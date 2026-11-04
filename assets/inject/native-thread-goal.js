(() => {
  const root = window;
  const existing = root.__codexPlusNativeThreadGoal;
  if (existing) {
    if (existing.version === 1 && existing.revision == null && !existing.__reloadGuard) {
      const reloadError = () => {
        const error = new Error("会话目标组件已更新，请刷新或重新打开此窗口后重试");
        error.code = "goal-runtime-reload-required";
        return error;
      };
      const install = existing.install;
      let source = "";
      try { source = Function.prototype.toString.call(install); } catch {}
      const knownLegacy = /^async function install\(threadId, options\) \{\s*adapters = options;/.test(source);
      const canGuard = Object.getOwnPropertyDescriptor(existing, "install")?.writable
        && Object.getOwnPropertyDescriptor(existing, "probe")?.writable && Object.isExtensible(existing);
      if (knownLegacy && canGuard) {
        // v1 的请求表和原始方法在闭包中；禁用转发但保留闭包，不能替换运行时或叠加包装器。
        const suspended = {
          enabled: () => true,
          forward: async () => { throw reloadError(); },
          get hostId() { throw reloadError(); },
        };
        try { Promise.resolve(install.call(existing, null, suspended)).catch(() => {}); } catch {}
        existing.install = async (_threadId, options) => {
          try { options?.report?.("goal-runtime-reload-required"); } catch {}
          throw reloadError();
        };
        existing.probe = async () => { throw reloadError(); };
        Object.defineProperty(existing, "__reloadGuard", { value: true });
      }
      if (!root.__codexPlusNativeThreadGoalReloadWarned) {
        root.__codexPlusNativeThreadGoalReloadWarned = true;
        root.console?.warn?.("[Codex++] 会话目标组件已更新，请刷新或重新打开此窗口。");
      }
    }
    return;
  }
  const patches = new WeakMap();
  const requests = new Map();
  const moduleDiscoveries = new WeakMap();
  let modulePromise = null;
  let adapters = null;
  let installedFor = null;
  let missingInstall = null;
  let installGeneration = 0;
  const genericForwardMessage = "会话所在窗口暂不可用，请稍后重试设置目标";
  const forwardMessages = new Map([
    ["goal-invalid-request", "会话目标请求无效，请重新设置目标"],
    ["goal-enhancements-disabled", "Codex++ 增强已关闭"],
    ["goal-runtime-reload-required", "会话目标组件已更新，请刷新或重新打开此窗口后重试"],
    ...["goal-probe-failed", "goal-owner-unavailable", "goal-native-getter-ambiguous",
      "goal-owner-changed", "goal-request-id-conflict", "goal-request-limit", "goal-forward-failed"]
      .map(code => [code, genericForwardMessage]),
  ]);

  function runtimeError(code) {
    const error = new Error(code);
    error.code = code;
    return error;
  }

  async function nativeModule() {
    if (!modulePromise) {
      modulePromise = Promise.resolve().then(async () => {
        if (adapters?.loadModule) return adapters.loadModule();
        const urls = [
          ...Array.from(document.querySelectorAll("script[src],link[href]"), node => node.src || node.href),
          ...performance.getEntriesByType("resource").map(entry => entry.name),
        ];
        const shared = urls.find(value => {
          try {
            const url = new URL(value, root.location.href);
            return url.protocol === "app:" && url.hostname === "-"
              && /^\/assets\/app-shared-[\w-]+\.js$/.test(url.pathname);
          } catch { return false; }
        });
        if (!shared) throw runtimeError("native-module-unavailable");
        return import(shared);
      }).then(module => {
        if (!module || (typeof module !== "object" && typeof module !== "function")) {
          throw runtimeError("native-module-unavailable");
        }
        return module;
      }).catch(error => {
        modulePromise = null;
        throw error;
      });
    }
    return modulePromise;
  }

  function discoverModule(module) {
    let discovery = moduleDiscoveries.get(module);
    if (discovery) return discovery;
    const getters = new Set(), families = [];
    for (const value of Object.values(module)) {
      if (value && typeof value === "object" && value.kind === "signal-family") families.push(value);
      if (typeof value !== "function") continue;
      try {
        const source = Function.prototype.toString.call(value);
        if (source.includes(".getConversation(") && source.includes(".find(")
            && source.includes(".get(") && source.includes("return null")) getters.add(value);
      } catch {}
    }
    discovery = {
      getter: getters.size === 1 ? getters.values().next().value : null,
      ambiguous: getters.size > 1, families, reported: false, warned: false,
    };
    moduleDiscoveries.set(module, discovery);
    return discovery;
  }

  function appScopes(discovery) {
    const scopes = new Set(discovery.families.map(value => value.scope));
    const found = new Set(), seen = new Set(), stack = [];
    for (const node of [document.getElementById("root"), document.body, document.documentElement]) {
      if (!node) continue;
      const key = Object.keys(node).find(key => /^__react(Container|Fiber)\$/.test(key));
      const fiber = key && node[key];
      if (fiber) stack.push(fiber.stateNode?.current || fiber);
    }
    for (let count = 0; stack.length && count < 4000; count++) {
      const fiber = stack.pop();
      if (!fiber || typeof fiber !== "object" || seen.has(fiber)) continue;
      seen.add(fiber);
      for (let hook = fiber.memoizedState, index = 0; hook && index < 64; hook = hook.next, index++) {
        const scope = hook.memoizedState?.current;
        if (scope && typeof scope.get === "function" && scopes.has(scope.scope)) found.add(scope);
      }
      if (fiber.child) stack.push(fiber.child);
      if (fiber.sibling) stack.push(fiber.sibling);
    }
    return found;
  }

  async function managerFor(threadId, hostId) {
    const module = await nativeModule();
    // 已验证的原生 getter：按 thread 查 host，再从该 AppScope 的 managers 中查找。
    const discovery = discoverModule(module);
    if (discovery.ambiguous) {
      if (!discovery.reported && typeof adapters?.report === "function") {
        try { adapters.report("goal-native-getter-ambiguous"); discovery.reported = true; } catch {}
      }
      if (!discovery.warned) {
        discovery.warned = true;
        root.console?.warn?.("[Codex++] Native thread goal getter is ambiguous.");
      }
      throw runtimeError("goal-native-getter-ambiguous");
    }
    if (!discovery.getter) return null;
    for (const scope of appScopes(discovery)) {
      let manager;
      try { manager = discovery.getter(scope, threadId); } catch { continue; }
      if (manager && manager.disposed !== true && typeof manager.setThreadGoal === "function"
          && typeof manager.getStreamRole === "function" && typeof manager.getHostId === "function"
          && typeof manager.getConversation === "function" && manager.getHostId() === hostId
          && manager.getConversation(threadId)) return manager;
    }
    return null;
  }

  async function probe({ threadId, hostId }) {
    const manager = await managerFor(threadId, hostId);
    return { status: manager?.getStreamRole(threadId)?.role || "unavailable" };
  }

  async function execute(request) {
    const { threadId, hostId, requestId, params } = request;
    const key = JSON.stringify([hostId, threadId, params]);
    const previous = requests.get(requestId);
    if (previous) {
      if (previous.key !== key) throw runtimeError("goal-request-id-conflict");
      return previous.promise;
    }
    const manager = await managerFor(threadId, hostId);
    if (!manager || manager.getStreamRole(threadId)?.role !== "owner") {
      throw runtimeError("goal-owner-changed");
    }
    // 不驱逐未完成请求，也不在未知结果后重试另一窗口。
    const duplicate = requests.get(requestId);
    if (duplicate) {
      if (duplicate.key !== key) throw runtimeError("goal-request-id-conflict");
      return duplicate.promise;
    }
    for (const [id, entry] of requests) {
      if (entry.finishedAt && Date.now() - entry.finishedAt > 300_000) requests.delete(id);
    }
    if (requests.size >= 128) throw runtimeError("goal-request-limit");
    const original = patches.get(Object.getPrototypeOf(manager))?.original || manager.setThreadGoal;
    const entry = { key, promise: null, finishedAt: 0 };
    entry.promise = Promise.resolve().then(() => {
      if (manager.disposed || manager.getHostId() !== hostId || !manager.getConversation(threadId)
          || manager.getStreamRole(threadId)?.role !== "owner") throw runtimeError("goal-owner-changed");
      // 原生实现会在 resume 后再次校验 owner，并维护 settings、transcript 和广播。
      return original.call(manager, threadId, params);
    }).then(goal => ({ status: "ok", goal })).finally(() => { entry.finishedAt = Date.now(); });
    requests.set(requestId, entry);
    return entry.promise;
  }

  async function install(threadId, options) {
    adapters = options;
    const generation = ++installGeneration;
    if (installedFor?.threadId === threadId && installedFor.hostId === options.hostId
        && installedFor.manager.disposed !== true) {
      try {
        if (installedFor.manager.getHostId() === options.hostId
            && installedFor.manager.getConversation(threadId)) {
          missingInstall = null;
          return true;
        }
      } catch {}
    }
    const sameMissing = missingInstall?.threadId === threadId && missingInstall.hostId === options.hostId;
    const now = () => typeof performance.now === "function" ? performance.now() : Date.now();
    if (sameMissing && now() < missingInstall.retryAt) return false;
    const manager = await managerFor(threadId, options.hostId);
    if (generation !== installGeneration) return false;
    if (!manager) {
      const delay = sameMissing ? Math.min(missingInstall.delay * 2, 8_000) : 4_000;
      missingInstall = { threadId, hostId: options.hostId, delay, retryAt: now() + delay };
      return false;
    }
    missingInstall = null;
    const proto = Object.getPrototypeOf(manager);
    if (patches.has(proto)) {
      installedFor = { threadId, hostId: options.hostId, manager };
      return true;
    }
    const descriptor = Object.getOwnPropertyDescriptor(proto, "setThreadGoal");
    if (!descriptor?.writable || typeof descriptor.value !== "function") return false;
    const original = descriptor.value;
    proto.setThreadGoal = async function (id, params) {
      if (!adapters?.enabled()) return original.call(this, id, params);
      const forward = async () => {
        let result;
        try {
          result = await adapters.forward({
            threadId: id, hostId: this.getHostId(), requestId: crypto.randomUUID(), params,
          });
        } catch {
          try { adapters.report("goal-forward-failed"); } catch {}
          throw new Error(genericForwardMessage);
        }
        if (result?.status !== "ok") {
          const code = forwardMessages.has(result?.code) ? result.code : "goal-forward-failed";
          try { adapters.report(code); } catch {}
          throw new Error(forwardMessages.get(code));
        }
        return result.goal;
      };
      if (this.getStreamRole(id)?.role === "follower") return forward();
      // 原生调用失败后不能确认是否已经写入，不能按错误文案自动重试。
      return original.call(this, id, params);
    };
    patches.set(proto, { original });
    installedFor = { threadId, hostId: options.hostId, manager };
    return true;
  }

  // 兼容旧脚本的 version===1 判断，revision 用于区分具体实现。
  root.__codexPlusNativeThreadGoal = { version: 1, revision: 2, probe, execute, install };
})();
