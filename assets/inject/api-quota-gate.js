(() => {
  // 本机官登只要勾了混入 Key，就解除官方订阅额度的发送锁。不看上游地址。
  function permitsExternalApi(settings, hostId) {
    if (hostId !== "local" || settings?.relayProfilesEnabled !== true) return false;
    if (!Array.isArray(settings.relayProfiles)) return false;
    const profile = settings.relayProfiles.find((item) => item?.id === settings.activeRelayId);
    return !!profile && profile.relayMode === "official" && profile.officialMixApiKey === true;
  }

  function locate(text, url) {
    if (typeof text !== "string" || !text || typeof url !== "string") return null;
    const id = "[A-Za-z_$][\\w$]*";
    // 绑定身份校验和额度判断的特征，不能仅按压缩后的函数名定位。
    const atomPattern = new RegExp(`(${id})=${id}\\(${id},\\(\\{get:${id}\\}\\)=>\\{[^}]{0,1400}?\\.authMethod!==\`chatgpt\`[^}]{0,1400}?\\.rate_limit\\?\\.allowed!==!1`, "g");
    const atoms = [...text.matchAll(atomPattern)];
    if (atoms.length !== 1) return null;
    const escape = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const gatePattern = new RegExp(`(${id})=${id}\\(${escape(atoms[0][1])}\\)&&(${id})===\`local\``, "g");
    const gates = [...text.matchAll(gatePattern)];
    if (gates.length !== 1) return null;
    const gate = gates[0];
    const tailStart = gate.index + gate[0].length;
    const tail = text.slice(tailStart, tailStart + 18000);
    const assignment = new RegExp(`(${id})=(${id}(?:\\|\\|${id})*\\|\\|${escape(gate[1])})(?=[,;])`).exec(tail);
    if (!assignment || !tail.includes(`submitDisabled:${assignment[1]}`)) return null;
    const offset = tailStart + assignment.index + assignment[1].length + 1;
    const before = text.slice(0, offset).split("\n");
    return {
      urlRegex: `^${escape(url)}$`,
      lineNumber: before.length - 1,
      columnNumber: before.at(-1).length,
      quotaVariable: gate[1],
      hostVariable: gate[2],
    };
  }

  function condition(location) {
    const identifier = /^[A-Za-z_$][\w$]*$/;
    if (!identifier.test(location?.quotaVariable) || !identifier.test(location?.hostVariable)) return null;
    const quota = location.quotaVariable;
    const host = location.hostVariable;
    // 条件始终返回 false，页面不会因这条断点暂停。
    return `(${quota}&&window.__codexPlusExternalApiQuotaAllowed?.(${host})===true&&(${quota}=false),false)`;
  }

  let refreshedComposers = new WeakMap();
  function refreshComposers(location, force = false) {
    if (!condition(location)) return 0;
    if (force) refreshedComposers = new WeakMap();
    const allowed = window.__codexPlusExternalApiQuotaAllowed?.("local") === true;
    let refreshed = 0;
    for (const root of document.querySelectorAll("[data-codex-composer-root]")) {
      let fiber = root[Object.keys(root).find(key => key.startsWith("__reactFiber$"))];
      for (let depth = 0; fiber && depth < 80; depth++, fiber = fiber.return) {
        if (typeof fiber.type !== "function") continue;
        const source = fiber.type.toString();
        if (!source.includes(`&&${location.hostVariable}===\`local\``)
          || !source.includes(`||${location.quotaVariable},`)
          || !source.includes("submitDisabled:")) continue;
        const hooks = [];
        for (let hook = fiber.memoizedState; hook; hook = hook.next) {
          if (hook.memoizedState instanceof Set && hook.memoizedState.size === 0
            && typeof hook.queue?.dispatch === "function") hooks.push(hook);
        }
        // 只重绘唯一可识别的空 Set 状态；不修改草稿或非空停止队列。
        if (hooks.length !== 1) break;
        const dispatch = hooks[0].queue.dispatch;
        const previous = refreshedComposers.get(dispatch);
        if (previous === allowed || (previous === undefined && !allowed && !force)) break;
        refreshedComposers.set(dispatch, allowed);
        dispatch(new Set());
        refreshed++;
        break;
      }
    }
    return refreshed;
  }

  // 复用增强模式的 Query 发布点，不依赖压缩变量名或 app 资源文件名。
  function installNative(settings) {
    if (window.__codexPlusNativeQuotaRuntime) {
      window.__codexPlusNativeQuotaRuntime.update(settings);
      return window.__codexPlusNativeQuotaRuntime;
    }
    let currentSettings = settings;
    const officialUsageRuntime = {
      rawPayloads: new WeakMap(), pendingPublications: new WeakMap(), rewriteDepth: 0,
    };
    function officialUsagePolicy() {
      const enabled = permitsExternalApi(currentSettings, "local");
      return { official: true, hideAlerts: false, unlockSend: enabled };
    }
    function officialUsagePolicyKey() {
      return officialUsagePolicy().unlockSend ? "official-mix" : "off";
    }
  function isOfficialUsageStatus(value) {
    if (!value || typeof value !== "object" || Array.isArray(value)) return false;
    const rateLimit = value.rate_limit;
    if (!rateLimit || typeof rateLimit !== "object" || typeof rateLimit.allowed !== "boolean") return false;
    return typeof value.plan_type === "string"
      || typeof value.user_id === "string"
      || typeof value.account_id === "string";
  }

  function isImageGenerationUpsell(value) {
    return String(value?.banner_type || "") === "image_generation_limit_reached";
  }

  function isMainRateLimitQueryKey(queryKey) {
    return Array.isArray(queryKey)
      && queryKey[0] === "rate-limit-status"
      && queryKey[1] !== "image-generation";
  }

  // 低额度提示和发送锁都只看当前是不是官登混入 Key。纯官登不改这份用量。
  // 桌面端发送按钮读 rate_limit.allowed；limit_reached 为 true 也会被当成已用完。
  // 混入时在查询发布前把 allowed 写成 true，并清掉 limit_reached。
  // 百分比、重置时间、账号、积分和消费上限不动。图片额度横幅单独留下。
  function rewriteOfficialUsageStatus(value, policy = officialUsagePolicy()) {
    if (!policy.unlockSend || !isOfficialUsageStatus(value)) return null;
    if (value.rate_limit.allowed === true && value.rate_limit.limit_reached !== true) return null;
    return { ...value, rate_limit: { ...value.rate_limit, allowed: true, limit_reached: false } };
  }

  function rewriteOfficialUsagePayload(value, policy = officialUsagePolicy()) {
    if (!value || typeof value !== "object") return value;
    if (isOfficialUsageStatus(value)) return rewriteOfficialUsageStatus(value, policy) || value;
    if (value.usage && value.usage !== value && isOfficialUsageStatus(value.usage)) {
      const usage = rewriteOfficialUsageStatus(value.usage, policy);
      return usage ? { ...value, usage } : value;
    }
    return value;
  }

  function rewriteTrackedOfficialUsagePayload(value) {
    const raw = officialUsageRuntime.rawPayloads.get(value) || value;
    if (officialUsagePolicyKey() === "off") return raw;
    const next = rewriteOfficialUsagePayload(value);
    if (next !== value) officialUsageRuntime.rawPayloads.set(next, raw);
    return next;
  }

  function looksLikeQueryClient(value) {
    return !!value
      && typeof value.getQueryCache === "function"
      && typeof value.setQueryData === "function";
  }

  // 图片额度使用同一条 /wham/usage，只能靠查询键 image-generation 排除。
  // 这里只在第一次挂上缓存时找客户端，不进每轮 DOM 扫描。
  function queryClientFromFiber(fiber) {
    const seen = new Set();
    const stack = [fiber];
    let visited = 0;
    while (stack.length && visited < 8000) {
      const node = stack.pop();
      if (!node || typeof node !== "object" || seen.has(node)) continue;
      seen.add(node);
      visited += 1;
      const props = node.memoizedProps || node.pendingProps;
      if (looksLikeQueryClient(props?.client)) return props.client;
      if (looksLikeQueryClient(props?.value)) return props.value;
      if (looksLikeQueryClient(node.stateNode)) return node.stateNode;
      const state = node.memoizedState;
      if (state && typeof state === "object" && looksLikeQueryClient(state.memoizedState)) return state.memoizedState;
      if (node.child) stack.push(node.child);
      if (node.sibling) stack.push(node.sibling);
    }
    return null;
  }

  let officialUsageClient = null;

  function findCodexQueryClient() {
    const explicit = window.__REACT_QUERY_CLIENT__ || window.__codexQueryClient;
    if (looksLikeQueryClient(explicit)) return explicit;
    if (looksLikeQueryClient(officialUsageClient)) return officialUsageClient;
    const roots = [document.getElementById?.("root"), document.body, document.documentElement].filter(Boolean);
    for (const root of roots) {
      let key = "";
      try {
        key = Object.keys(root).find((name) => name.startsWith("__reactContainer$") || name.startsWith("__reactFiber$")) || "";
      } catch {
        key = "";
      }
      if (!key) continue;
      let fiber = root[key];
      if (fiber?.stateNode?.current) fiber = fiber.stateNode.current;
      const client = queryClientFromFiber(fiber);
      if (client) {
        officialUsageClient = client;
        return client;
      }
    }
    return null;
  }

  function mainRateLimitQueries(client) {
    const cache = client.getQueryCache?.();
    if (cache && typeof cache.findAll === "function") {
      return cache.findAll({ queryKey: ["rate-limit-status"] }).filter((query) => isMainRateLimitQueryKey(query?.queryKey));
    }
    if (typeof client.getQueriesData === "function") {
      return client.getQueriesData({ queryKey: ["rate-limit-status"] })
        .filter(([queryKey]) => isMainRateLimitQueryKey(queryKey))
        .map(([queryKey, data]) => ({ queryKey, state: { data } }));
    }
    return [];
  }

  // Query.setData 是 GET /wham/usage 和 SSE snapshot 共用的发布点。
  // 在通知订阅者之前改写，RK 第一次读到的 allowed 就是结果。
  function patchOfficialUsageQueryPublication(client) {
    const cache = client.getQueryCache?.();
    if (!cache) return;
    const listed = typeof cache.getAll === "function"
      ? cache.getAll()
      : (typeof cache.findAll === "function" ? cache.findAll({ queryKey: ["rate-limit-status"] }) : []);
    const query = listed.find((item) => typeof Object.getPrototypeOf(item)?.setData === "function");
    if (!query) return;
    const proto = Object.getPrototypeOf(query);
    if (typeof proto.setData !== "function" || proto.setData.__codexPlusNativeQuotaPublication) return;
    const original = proto.setData;
    function codexPlusPublishUsageData(data, ...rest) {
      if (!isMainRateLimitQueryKey(this?.queryKey)) return original.call(this, data, ...rest);
      const raw = officialUsageRuntime.rawPayloads.get(data) || data;
      const next = officialUsageRuntime.rewrite(data);
      const previousPublication = officialUsageRuntime.pendingPublications.get(this);
      const publication = { raw };
      officialUsageRuntime.pendingPublications.set(this, publication);
      officialUsageRuntime.rewriteDepth += 1;
      try {
        const stored = original.call(this, next, ...rest);
        // TanStack 结构共享可能返回另一对象；快照绑定实际缓存对象，不绑定输入副本。
        // 若订阅者已嵌套发布更新，沿用它登记的快照，不能用外层旧值覆盖。
        if (publication.raw === raw && stored && typeof stored === "object") {
          if (next !== raw) officialUsageRuntime.rawPayloads.set(stored, raw);
          else officialUsageRuntime.rawPayloads.delete(stored);
        }
        if (previousPublication) previousPublication.raw = publication.raw;
        return stored;
      } finally {
        officialUsageRuntime.rewriteDepth -= 1;
        if (previousPublication) officialUsageRuntime.pendingPublications.set(this, previousPublication);
        else officialUsageRuntime.pendingPublications.delete(this);
      }
    }
    codexPlusPublishUsageData.__codexPlusNativeQuotaPublication = true;
    proto.setData = codexPlusPublishUsageData;
  }

  function patchOfficialUsageQueryClient(client) {
    if (!client || typeof client.setQueryData !== "function") return;
    patchOfficialUsageQueryPublication(client);
    if (client.__codexPlusNativeQuotaRewrite) return;
    const original = client.setQueryData;
    client.setQueryData = function codexPlusSetUsageQueryData(queryKey, updater, ...rest) {
      if (!isMainRateLimitQueryKey(queryKey)) {
        return original.call(this, queryKey, updater, ...rest);
      }
      const nextUpdater = typeof updater === "function"
        ? (previous) => {
          // setData 的同步订阅者可能马上写回，此时结构共享对象尚未返回。
          const query = this.getQueryCache?.()?.find?.({ queryKey, exact: true });
          const publishing = query && officialUsageRuntime.pendingPublications.get(query);
          const raw = publishing ? publishing.raw : (officialUsageRuntime.rawPayloads.get(previous) || previous);
          return officialUsageRuntime.rewrite(updater(raw));
        }
        : officialUsageRuntime.rewrite(updater);
      officialUsageRuntime.rewriteDepth += 1;
      try {
        return original.call(this, queryKey, nextUpdater, ...rest);
      } finally {
        officialUsageRuntime.rewriteDepth -= 1;
      }
    };
    const cache = client.getQueryCache?.();
    if (cache && typeof cache.subscribe === "function") {
      cache.subscribe((event) => {
        if (officialUsageRuntime.rewriteDepth > 0) return;
        const query = event?.query;
        if (!isMainRateLimitQueryKey(query?.queryKey)) return;
        const current = query.state?.data;
        const next = officialUsageRuntime.rewrite(current);
        if (next === current) return;
        client.setQueryData(query.queryKey, next);
      });
    }
    client.__codexPlusNativeQuotaRewrite = true;
  }

  function rewriteCachedOfficialUsage(client) {
    if (!client || !officialUsagePolicy().official) return;
    for (const query of mainRateLimitQueries(client)) {
      const current = query.state?.data;
      const next = officialUsageRuntime.rewrite(current);
      if (next !== current) client.setQueryData(query.queryKey, next);
    }
  }

  function invalidateMainRateLimitQueries(client) {
    if (!client || typeof client.invalidateQueries !== "function") return;
    for (const query of mainRateLimitQueries(client)) {
      try {
        Promise.resolve(client.invalidateQueries({ queryKey: query.queryKey, exact: true })).catch(() => {});
      } catch {
      }
    }
  }

  let officialUsagePolicyApplied = "";
  let officialUsageClientTimer = null;

  function syncOfficialUsagePolicy() {
    const key = officialUsagePolicyKey();
    const client = findCodexQueryClient();
    if (client) {
      officialUsageClient = client;
      patchOfficialUsageQueryClient(client);
      if (officialUsageClientTimer) {
        clearTimeout(officialUsageClientTimer);
        officialUsageClientTimer = null;
      }
    } else if (!officialUsageClientTimer) {
      let attempts = 0;
      const retry = () => {
        attempts += 1;
        officialUsageClientTimer = null;
        if (findCodexQueryClient()) {
          syncOfficialUsagePolicy();
          return;
        }
        if (attempts < 20) officialUsageClientTimer = setTimeout(retry, 300);
      };
      officialUsageClientTimer = setTimeout(retry, 300);
      return;
    } else {
      return;
    }
    if (key === officialUsagePolicyApplied) {
      if (key !== "off") rewriteCachedOfficialUsage(client);
      return;
    }
    const previous = officialUsagePolicyApplied;
    officialUsagePolicyApplied = key;
    if (key === "off") {
      // 先同步恢复真实用量；断网或刷新悬挂时也不能沿用混入模式的解锁结果。
      for (const query of mainRateLimitQueries(client)) {
        const raw = officialUsageRuntime.rawPayloads.get(query.state?.data);
        if (raw) client.setQueryData(query.queryKey, raw);
      }
      if (previous) invalidateMainRateLimitQueries(client);
      return;
    }
    rewriteCachedOfficialUsage(client);
    if (previous) invalidateMainRateLimitQueries(client);
  }


    const runtime = {
      update(next) {
        currentSettings = next;
        syncOfficialUsagePolicy();
      },
      refresh: syncOfficialUsagePolicy,
    };
    officialUsageRuntime.rewrite = rewriteTrackedOfficialUsagePayload;
    window.__codexPlusNativeQuotaRuntime = runtime;
    syncOfficialUsagePolicy();
    return runtime;
  }

  const api = { permitsExternalApi, locate, condition, refreshComposers, installNative };
  if (typeof module === "object" && module.exports) module.exports = api;
  if (typeof window !== "undefined") {
    window.__codexPlusApiQuotaGate = api;
    if (typeof window.__codexPlusExternalApiQuotaAllowed !== "function") {
      window.__codexPlusExternalApiQuotaAllowed = (hostId) => permitsExternalApi(window.__codexPlusSettings, hostId);
    }
  }
})();
