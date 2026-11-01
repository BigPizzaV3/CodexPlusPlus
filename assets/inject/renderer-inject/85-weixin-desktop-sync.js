  // 微信写入与桌面目录使用不同的 app-server；只通知同 home/SQLite 的持久线程。
  // 复用设置心跳，重注入沿用已处理版本，不另起计时器或扫描 DOM。
  const weixinDesktopSync = window.__codexPlusWeixinDesktopSync ||= {
    revisions: new Map(), flight: null, services: null, retryAt: 0,
  };
  weixinDesktopSync.retries ||= new Map();

  async function syncWeixinThreadProject(services, entry) {
    const project = entry.project;
    if (!project) return;
    if (!Array.isArray(project.rootPaths) || !project.rootPaths.length
        || project.rootPaths.some((root) => typeof root !== "string" || !root)
        || typeof project.name !== "string" || !project.name.trim()
        || typeof project.id !== "string") throw Error("微信项目记录无效");
    const normalize = (root) => {
      const path = root.replace(/\\/g, "/").replace(/^\/\/\?\//, "").replace(/\/+$/, "") || "/";
      return /^(?:[a-z]:|\/\/)/i.test(path) ? path.toLowerCase() : path;
    };
    const home = normalize(await services.executionPaths.readCodexHome("local"));
    const readMappings = async () => {
      const mappings = await weixinDesktopSync.readState(weixinDesktopSync.keys.APP_SERVER_PROJECT_ID_BY_LEGACY_PROJECT_ID_BY_HOST);
      const partitions = Object.entries(mappings || {}).filter(([identity]) =>
        identity.startsWith("local:") && normalize(identity.slice(6)) === home);
      if (partitions.length > 1) throw Error("微信项目的实例归属不唯一");
      return partitions[0]?.[1] || {};
    };
    const projects = await services.projects.getLocalProjects();
    const mappings = await readMappings();
    const matches = Object.entries(mappings).filter(([id, serverId]) =>
      serverId === project.id && Object.hasOwn(projects, id));
    if (matches.length !== 1) throw Error("微信项目的桌面归属暂不可确认");
    const assignments = await weixinDesktopSync.readState(weixinDesktopSync.keys.THREAD_PROJECT_ASSIGNMENTS);
    if (assignments != null && (typeof assignments !== "object" || Array.isArray(assignments)))
      throw Error("原生线程归属暂不可读");
    if (assignments?.[entry.threadId] != null) return;
    // 部分更新保留并发出现的非空归属；只修复注册前被标为无项目的线程。
    await services.threadProjectAssignments.setProjectless({ threadId: entry.threadId, projectless: false });
    await services.threadProjectAssignments.assignIfUnassigned([{
      threadId: entry.threadId, hostId: "local", projectless: false,
      assignment: { projectKind: "local", projectId: matches[0][0] },
    }]);
  }

  async function syncWeixinDesktopThreads() {
    const entries = codexPlusBackendSettings.weixinDesktopSync;
    const now = Date.now();
    if (!codexPlusBackendSettingsLoaded || !Array.isArray(entries)
        || entries.length > 128 || weixinDesktopSync.flight || now < weixinDesktopSync.retryAt) return;
    const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
    const currentIds = new Set(entries.map((entry) => entry?.threadId));
    for (const records of [weixinDesktopSync.revisions, weixinDesktopSync.retries]) {
      for (const id of records.keys()) {
        if (!currentIds.has(id)) records.delete(id);
      }
    }
    const pending = entries.filter((entry) => {
      const retry = weixinDesktopSync.retries.get(entry?.threadId);
      return uuid.test(entry?.threadId) && uuid.test(entry?.revision)
        && weixinDesktopSync.revisions.get(entry.threadId) !== entry.revision
        && (retry?.revision !== entry.revision || now >= retry.retryAt);
    }).map((entry) => ({ ...entry }));
    if (!pending.length) return;
    weixinDesktopSync.flight = (async () => {
      try {
        if (!weixinDesktopSync.services) {
          const module = await loadOptionalCodexAppModule("app-shared-");
          // 导出名随打包变化；只接受拥有明确服务能力的唯一原生服务集合。
          const services = Object.values(module || {}).filter((value) => value && typeof value === "object"
            && Object.hasOwn(value, "localThreadCatalog")
            && typeof value.localThreadCatalog?.notifyThread === "function"
            && typeof value.localThreadCatalog?.readEntries === "function"
            && typeof value.projects?.getLocalProjects === "function"
            && typeof value.executionPaths?.readCodexHome === "function"
            && typeof value.threadProjectAssignments?.setProjectless === "function"
            && typeof value.threadProjectAssignments?.assignIfUnassigned === "function");
          const keys = Object.values(module || {}).filter((value) => value && typeof value === "object"
            && typeof value.APP_SERVER_PROJECT_ID_BY_LEGACY_PROJECT_ID_BY_HOST === "string"
            && typeof value.THREAD_PROJECT_ASSIGNMENTS === "string");
          const clients = Object.values(module || {}).filter((value) => typeof value === "function"
            && Object.hasOwn(value, "prototype") && Object.hasOwn(value, "getInstance")
            && typeof value.getInstance === "function"
            && ["post", "sendRequest", "onFetchResponse", "cancelStream"]
              .every((method) => typeof value.prototype?.[method] === "function"));
          if (services.length !== 1 || keys.length !== 1 || clients.length !== 1)
            throw Error("原生会话目录暂不可用");
          // 直接使用原生请求客户端；不按函数源码中的读写关键字猜测调用器。
          weixinDesktopSync.readState = async (key) => {
            const response = await clients[0].getInstance().post("vscode://codex/get-global-state", JSON.stringify({ key }));
            if (!response?.body || !Object.hasOwn(response.body, "value")) throw Error("原生状态暂不可读");
            return response.body.value;
          };
          weixinDesktopSync.services = services[0];
          weixinDesktopSync.keys = keys[0];
        }
        for (const entry of pending) {
          try {
            await syncWeixinThreadProject(weixinDesktopSync.services, entry);
            await weixinDesktopSync.services.localThreadCatalog.notifyThread({ hostId: "local", threadId: entry.threadId }, "update");
            weixinDesktopSync.revisions.set(entry.threadId, entry.revision);
            weixinDesktopSync.retries.delete(entry.threadId);
          } catch {
            weixinDesktopSync.retries.set(entry.threadId, { revision: entry.revision, retryAt: Date.now() + 30000 });
          }
        }
      } catch {
        // 桌面启动或升级期间可能没有 catalog；下一次心跳退避重试，不重载页面。
        weixinDesktopSync.services = null;
        weixinDesktopSync.retryAt = Date.now() + 30000;
      }
    })();
    try { await weixinDesktopSync.flight; }
    finally { weixinDesktopSync.flight = null; }
  }
