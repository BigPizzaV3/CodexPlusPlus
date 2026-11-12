(async () => {
  "use strict";
  const service = "codex_plus_collaboration";
  const queryKey = ["mcp", "servers", "status", "local", "full"];
  const expectedTools = [
    "list_agents", "delegate_task", "delegate_to_deepseek", "delegate_to_claude_code",
    "delegate_to_codex", "delegate_to_antigravity", "send_task", "get_task", "wait_task", "list_tasks", "cancel_task",
  ];
  const report = {
    status: "unavailable", message: "当前 Codex 尚未建立可用的本机连接，未重读配置。",
    reloadSucceeded: false, toolsReady: false, toolCount: 0, threadVerified: false, cacheRefreshed: false,
  };
  const result = () => JSON.stringify(report);
  const timeoutCode = "CODEX_PLUS_REFRESH_TIMEOUT";
  const bounded = (promise, milliseconds) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(Object.assign(new Error("operation timeout"), { code: timeoutCode })), milliseconds);
    Promise.resolve(promise).then(
      value => { clearTimeout(timer); resolve(value); },
      () => { clearTimeout(timer); reject(new Error("operation failed")); },
    );
  });
  function localClient() {
    const registry = window.__codexPlusNativeHostClients;
    if (!(registry instanceof Map)) return null;
    const entry = registry.get("local");
    if (!entry || entry.hostId !== "local" || entry.getterBound === true || typeof entry.ref?.deref !== "function") return null;
    const client = entry.ref.deref();
    return client && client.hostId === "local" && client.disposed !== true && typeof client.sendRequest === "function" ? client : null;
  }
  async function request(method, params, milliseconds = 1500) {
    const client = localClient();
    if (!client) throw new Error("local client unavailable");
    const reply = await bounded(client.sendRequest(method, params), milliseconds);
    if (reply?.error || reply?.status === "failed") throw new Error("request failed");
    return reply;
  }
  function entryFromPayload(payload) {
    const data = Array.isArray(payload) ? payload : Array.isArray(payload?.data) ? payload.data : payload?.result?.data;
    if (!Array.isArray(data)) return null;
    const entries = data.filter(entry => entry?.name === service);
    return entries.length === 1 ? entries[0] : null;
  }
  function toolNames(entry) {
    if (!entry?.tools || typeof entry.tools !== "object" || Array.isArray(entry.tools)) return [];
    return Object.entries(entry.tools).map(([key, tool]) => typeof tool?.name === "string" ? tool.name : key);
  }
  function toolsReady(entry) {
    const names = toolNames(entry);
    return !entry?.toolsError && expectedTools.every(name => names.includes(name));
  }
  function currentThreadId() {
    const source = `${window.location?.pathname || ""}${window.location?.search || ""}${window.location?.hash || ""}`;
    const match = source.match(/(?:session|conversation|thread)(?:\/|=|:|-)([A-Za-z0-9_.-]+)/i)
      || source.match(/\/([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})(?:[/?#]|$)/)
      || source.match(/\/([A-Za-z0-9_-]{24,})(?:[/?#]|$)/);
    if (!match) return "";
    let id = "";
    try { id = decodeURIComponent(match[1]); } catch { return ""; }
    return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id) ? id : "";
  }
  const statusParams = () => ({ serverName: service, detail: "toolsAndAuthOnly", limit: 100 });
  if (!localClient()) return result();
  // 旧 runtime 的全局清单可能缺少刚接入的 server，甚至发现请求失败。
  // 管理器已确认配置归属；不能要求旧清单先含有新工具，才允许本次重读。
  try { await request("mcpServerStatus/list", statusParams()); } catch {}
  try {
    await request("config/mcpServer/reload", null, 3500);
    report.reloadSucceeded = true;
  } catch {
    report.status = "error";
    report.message = "本机 MCP 配置整体重读结果未确认；未重复发送，请稍后检查。";
    return result();
  }
  const threadId = currentThreadId();
  try {
    const params = statusParams();
    if (threadId) params.threadId = threadId;
    const entry = entryFromPayload(await request("mcpServerStatus/list", params));
    report.toolCount = toolNames(entry).length;
    report.threadVerified = !!threadId && currentThreadId() === threadId && entry?.runtimeStatus === "connected" && toolsReady(entry);
    report.toolsReady = threadId ? report.threadVerified : toolsReady(entry);
  } catch {
    report.toolsReady = false;
  }

  function isQueryClient(value) {
    return !!value && typeof value.getQueryCache === "function" && typeof value.setQueryData === "function"
      && typeof value.invalidateQueries === "function" && typeof value.refetchQueries === "function";
  }
  function queryClientFromFiber(fiber, inspect) {
    const seen = new Set();
    const stack = [fiber];
    while (stack.length && seen.size < 8000) {
      const node = stack.pop();
      if (!node || typeof node !== "object" || seen.has(node)) continue;
      seen.add(node);
      const props = node.memoizedProps || node.pendingProps;
      for (const value of [props?.client, props?.queryClient, props?.value, node.stateNode, node.memoizedState?.memoizedState]) {
        const client = inspect(value);
        if (client) return client;
      }
      if (node.child) stack.push(node.child);
      if (node.sibling) stack.push(node.sibling);
    }
    return null;
  }
  function findQueryClient() {
    let sawQueryClient = false;
    let lookupFailed = false;
    const seen = new Set();
    const inspect = value => {
      if (seen.has(value)) return null;
      try {
        if (!isQueryClient(value)) return null;
        seen.add(value);
        sawQueryClient = true;
        return exactQuery(value) ? value : null;
      } catch {
        lookupFailed = true;
        return null;
      }
    };
    for (const value of [window.__REACT_QUERY_CLIENT__, window.__codexQueryClient]) {
      const client = inspect(value);
      if (client) return { client, sawQueryClient, lookupFailed };
    }
    const roots = [document.getElementById?.("root"), document.body, document.documentElement].filter(Boolean);
    for (const root of roots) {
      let key;
      try { key = Object.keys(root).find(name => name.startsWith("__reactContainer$") || name.startsWith("__reactFiber$")); }
      catch { continue; }
      if (!key) continue;
      let fiber = root[key];
      if (fiber?.stateNode?.current) fiber = fiber.stateNode.current;
      const client = queryClientFromFiber(fiber, inspect);
      if (client) return { client, sawQueryClient, lookupFailed };
    }
    return { client: null, sawQueryClient, lookupFailed };
  }
  function exactQuery(client) {
    const cache = client.getQueryCache();
    const matches = query => Array.isArray(query?.queryKey) && query.queryKey.length === queryKey.length
      && query.queryKey.every((value, index) => value === queryKey[index]);
    if (typeof cache?.find === "function") {
      const query = cache.find({ queryKey, exact: true });
      return matches(query) ? query : null;
    }
    return typeof cache?.getAll === "function" ? cache.getAll().find(matches) || null : null;
  }
  let cacheFailure = "";
  let cachePhase = "find";
  try {
    const { client, sawQueryClient, lookupFailed } = findQueryClient();
    if (!client) {
      cacheFailure = lookupFailed ? "无法定位 MCP 列表缓存，请重新打开 /mcp 后重试。" : sawQueryClient
        ? "尚未建立列表查询，请先在 Codex 中打开 /mcp，再点击刷新。"
        : "未找到 QueryClient，无法刷新 MCP 列表缓存。";
    } else {
      await bounded((async () => {
        cachePhase = "invalidate";
        await client.invalidateQueries({ queryKey, exact: true, refetchType: "none" });
        cachePhase = "refetch";
        await client.refetchQueries({ queryKey, exact: true, type: "all" }, { throwOnError: true, cancelRefetch: false });
      })(), 1800);
      cachePhase = "verify";
      const query = exactQuery(client);
      const cached = typeof client.getQueryData === "function" ? client.getQueryData(queryKey) : query?.state?.data;
      report.cacheRefreshed = query?.state?.isInvalidated !== true && toolsReady(entryFromPayload(cached));
      if (!report.cacheRefreshed) cacheFailure = "列表数据尚未更新，请稍后再次检查 /mcp。";
    }
  } catch (error) {
    report.cacheRefreshed = false;
    cacheFailure = error?.code === timeoutCode
      ? "列表仍在后台刷新，请稍后重新打开 /mcp 查看，无需重新接入。"
      : cachePhase === "find" ? "无法定位 MCP 列表缓存，请重新打开 /mcp 后重试。"
        : cachePhase === "invalidate" ? "无法准备列表刷新，请重新打开 /mcp 后重试。"
          : cachePhase === "verify" ? "无法核验列表更新，请重新打开 /mcp 查看。"
            : "重读列表失败，请稍后重试刷新。";
  }
  if (threadId && currentThreadId() !== threadId) {
    report.threadVerified = false;
    report.toolsReady = false;
  }
  report.status = report.toolsReady && report.cacheRefreshed ? "ready" : "partial";
  report.message = report.status === "ready"
    ? threadId ? "本机 MCP 配置已整体重读，当前会话工具与 MCP 列表均已刷新。" : "本机 MCP 配置已整体重读，工具与 MCP 列表已刷新；当前页面没有可核验的会话。"
    : report.toolsReady ? `本机 MCP 配置已整体重读，工具已就绪；${cacheFailure}`
      : `本机 MCP 配置已整体重读，但当前会话工具尚未确认就绪。${cacheFailure}`;
  return result();
})()
