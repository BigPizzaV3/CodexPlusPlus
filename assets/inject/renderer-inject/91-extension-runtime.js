  // 注入代际（00-prelude 在每个实例启动时写入）。重新注入后旧实例留下的常驻循环
  // 靠它自检并退出，否则每注入一次就多叠一套轮询。测试片段可能只抽这个文件，
  // 所以对「变量不存在」做一次兜底。
  const codexPlusInjectionGeneration = typeof codexPlusRuntimeGeneration === "undefined"
    ? (window.__codexPlusRuntimeGeneration || 0)
    : codexPlusRuntimeGeneration;
  const codexPlusInjectionStillCurrent = () =>
    (window.__codexPlusRuntimeGeneration || 0) === codexPlusInjectionGeneration;

  /**
   * 拓展运行时内核：codexPlus.extension(apply)
   *
   * 在 register* 之上加一层「拓展实例」：ctx 上的每一次注册（UI 项、事件监听、
   * 中间件、DOM 监听、定时器、自定义副作用）都绑定到实例的寿命上，卸载时按注册
   * 逆序自动撤销。拓展作者不必再自己维护 dispose 列表，热重载也不会残留旧实例。
   *
   * 两种拓展间耦合方式：
   *   - 事件：ctx.on / ctx.emit，一对多通知，处理器抛错互不影响
   *   - waterfall：ctx.intercept / ctx.waterfall，中间件链，handler(value, next)
   *     调 next(v) 交给下一个，不调即短路；没有中间件时原样返回
   *
   * 宿主后续会在协议层、界面层的关键点调用 codexPlusRunWaterfall /
   * codexPlusEmitExtensionEvent，拓展用同一套 ctx 接口挂上去。设计见
   * docs/plans/2026-10-30-codexpp-extension-system-design.md。
   *
   * 和 91-extension-api.js 一样只做定义、不读 DOM，挂载在 99-startup。
   */

  /** 事件名 → 监听者集合。监听者带 scriptKey，出错时记到对应脚本。 */
  const codexPlusExtensionListeners = new Map();
  /** 中间件链名 → 中间件数组，按 order、再按注册先后排序。 */
  const codexPlusExtensionInterceptors = new Map();
  let codexPlusExtensionInterceptorSeq = 0;
  /** 存活的拓展实例，供诊断查看。 */
  const codexPlusExtensionInstances = new Set();

  /**
   * 拓展间服务。名字是一个扁平命名空间：ctx.provide(name, value) 提供，声明了 inject 的拓展
   * 在依赖齐全时才应用；提供者卸载或被替换时，依赖方先卸载、回到挂起，提供者回来后自动
   * 重新应用。内置服务（如 sessionQuery）由 Codex++ 提供，归属为 "builtin"。
   */
  const codexPlusServices = new Map();
  let codexPlusServiceSeq = 0;
  /** 等待依赖或依赖服务的拓展（生命周期句柄），服务变化时逐个复查。 */
  const codexPlusServiceFibers = new Set();
  const codexPlusServiceNamePattern = /^[A-Za-z][\w.-]{0,63}$/;

  function codexPlusNormalizeInject(inject) {
    if (!inject) return [];
    const names = Array.isArray(inject) ? inject : typeof inject === "object" ? Object.keys(inject) : [inject];
    return [...new Set(names.map(String).filter((name) => codexPlusServiceNamePattern.test(name)))];
  }

  /** 必需依赖：对象写法里 { name: { required: false } } 是可选依赖，缺了也应用，变了照样重载。 */
  function codexPlusRequiredInject(inject) {
    if (!inject || Array.isArray(inject) || typeof inject !== "object") return codexPlusNormalizeInject(inject);
    return codexPlusNormalizeInject(Object.keys(inject).filter((name) => inject[name]?.required !== false));
  }

  /**
   * 管理器保存拓展配置后先调这里：这个拓展的所有实例都接住新配置时返回 "applied"，否则 "reload"（由管理器整体热重载）。
   * 多实例时每份叠上自己实例的配置。
   */
  function codexPlusUpdateExtensionConfig(scriptKey, nextConfig) {
    const instances = [...codexPlusExtensionInstances].filter((ctx) => ctx.scriptKey === scriptKey && !ctx.disposed);
    if (!instances.length) return "reload";
    for (const ctx of instances) {
      const merged = ctx.instance ? { ...nextConfig, ...(ctx.manifest?.instance?.config ?? ctx.manifest?.instances?.find?.((item) => item.id === ctx.instance.id)?.config ?? {}) } : nextConfig;
      if (!ctx.__tryConfigUpdate(merged)) return "reload";
    }
    const script = window.__codexPlusUserScripts?.scripts?.[scriptKey];
    if (script?.manifest) script.manifest = { ...script.manifest, config: nextConfig };
    return "applied";
  }
  if (typeof window !== "undefined") {
    window.__codexPlusUserScripts ||= { scripts: {} };
    window.__codexPlusUserScripts.updateConfig = codexPlusUpdateExtensionConfig;
  }

  function codexPlusServiceChanged() {
    for (const fiber of [...codexPlusServiceFibers]) {
      try {
        fiber.check();
      } catch {}
    }
  }

  function codexPlusProvideService(name, value, owner) {
    if (!codexPlusServiceNamePattern.test(String(name))) throw new Error(`服务名不合法：${name}`);
    const existing = codexPlusServices.get(name);
    if (existing) throw new Error(`服务 ${name} 已由 ${existing.owner || "其它拓展"} 提供`);
    const entry = { value, owner, token: ++codexPlusServiceSeq };
    codexPlusServices.set(name, entry);
    codexPlusServiceChanged();
    return () => {
      if (codexPlusServices.get(name) !== entry) return;
      codexPlusServices.delete(name);
      codexPlusServiceChanged();
    };
  }

  /** 内置的只读会话查询服务：方法名与参数对齐会话查询接口，经桥接转到 Codex++。 */
  function codexPlusSessionQueryService() {
    const invoke = (method, params) => {
      const bridge = window.__codexSessionDeleteBridge;
      if (typeof bridge !== "function") return Promise.reject(new Error("Codex 页面尚未连接，请稍后重试"));
      return Promise.resolve(bridge("/extensions/history", { method, params: params ?? null })).then((result) => {
        if (result?.status === "failed" || result?.error) throw new Error(result.message || result.error || `${method} 调用失败`);
        return result?.result;
      });
    };
    const withId = (method) => (id, options) => invoke(method, { id, options: options ?? {} });
    const readSession = (id) => invoke("readSession", { id });
    const readEventsSince = (query) => invoke("readEventsSince", query ?? {});
    return Object.freeze({
      listSessions: (options) => invoke("listSessions", options ?? {}),
      filterSessions: (filters) => invoke("filterSessions", filters ?? {}),
      readSession,
      readTitle: (id) => invoke("readTitle", { id }),
      listEvents: withId("listEvents"),
      filterEvents: withId("filterEvents"),
      readEvent: (query) => invoke("readEvent", query ?? {}),
      readEventsSince,
      searchSessions: (query) => invoke("searchSessions", query ?? {}),
      searchEvents: (query) => invoke("searchEvents", query ?? {}),
      /** 把会话事件日志折叠成状态：apply(state, event) 返回新状态（返回 undefined 视为不变）。 */
      async project(id, { init, apply } = {}) {
        if (typeof apply !== "function") throw new Error("project 需要 apply(state, event)");
        const { events } = await readSession(id);
        let state = typeof init === "function" ? init() : init;
        for (const event of events) {
          const next = apply(state, event);
          if (next !== undefined) state = next;
        }
        return state;
      },
      /**
       * 订阅会话的新事件：按 intervalMs（默认 2 秒）增量读取，handler(event) 逐条收到。
       * 默认从当前末尾开始（不回放历史），fromSeq 指定起点。返回撤销函数。
       */
      /** 订阅全部会话的新事件：handler(event, { sessionId })，返回撤销函数。等同 ctx.on("session/event")。 */
      observeAll(handler) {
        if (typeof handler !== "function") throw new Error("observeAll 需要 handler(event, { sessionId })");
        const listener = { handler, scriptKey: "builtin:sessionQuery" };
        if (!codexPlusExtensionListeners.has("session/event")) codexPlusExtensionListeners.set("session/event", new Set());
        codexPlusExtensionListeners.get("session/event").add(listener);
        codexPlusEnsureSessionWatcher();
        return () => {
          const listeners = codexPlusExtensionListeners.get("session/event");
          listeners?.delete(listener);
          if (listeners && !listeners.size) codexPlusExtensionListeners.delete("session/event");
        };
      },
      observeSession(id, handler, options = {}) {
        if (typeof handler !== "function") throw new Error("observeSession 需要 handler(event)");
        let stopped = false;
        let next = Number.isFinite(options.fromSeq) ? Number(options.fromSeq) : null;
        let timer = null;
        const interval = Math.max(500, Number(options.intervalMs) || 2000);
        const tick = async () => {
          // 重新注入后旧实例的订阅循环必须停：它只认自己闭包里的 stopped，
          // 没人会替它置位，不停就是每次注入多一条永久轮询。
          if (!stopped && !codexPlusInjectionStillCurrent()) {
            stopped = true;
            clearTimeout(timer);
          }
          if (stopped) return;
          try {
            if (next === null) {
              const tail = await invoke("listEvents", { id, options: { fromEnd: true, limit: 1 } });
              next = tail.length ? tail[0].seq + 1 : 0;
            } else {
              const events = await readEventsSince({ sessionId: id, seq: next, limit: 200 });
              for (const event of events) {
                if (stopped) return;
                next = event.seq + 1;
                try { handler(event); } catch {}
              }
            }
          } catch {}
          if (!stopped) timer = setTimeout(tick, interval);
        };
        void tick();
        return () => {
          stopped = true;
          clearTimeout(timer);
        };
      },
    });
  }

  /**
   * 会话写入：经 Codex 自己的 AppServerManager 调 app-server。
   *
   * 新版 Codex 在页面里用 `scope.get(管理器原子).forHost(hostId).sendRequest(method, params)` 发请求；原子是入口
   * 模块的一个导出（名字随构建变），这里按「scope.get 出来的值带 forHost」把它找出来并缓存。scope 取自任意
   * 带 React fiber 的界面节点，每次调用现取（界面重建后旧 scope 会失效）。只开放下面几个方法，不开放删除等。
   */
  const codexPlusSessionMethods = {
    list: "thread/list",
    create: "thread/start",
    fork: "thread/fork",
    rename: "thread/name/set",
    append: "thread/inject_items",
    startTurn: "turn/start",
    steer: "turn/steer",
    interrupt: "turn/interrupt",
    compact: "thread/compact/start",
  };
  let codexPlusManagerAtom = null;

  function codexPlusFindAppScope() {
    const candidates = document.querySelectorAll("[data-app-action-sidebar-thread-id], [data-codex-composer-root], main, aside");
    for (const node of candidates) {
      const key = Object.getOwnPropertyNames(node).find((name) => name.startsWith("__reactFiber$"));
      let fiber = key ? node[key] : null;
      for (let depth = 0; fiber && depth < 40; depth += 1, fiber = fiber.return) {
        let hook = fiber.memoizedState;
        for (let index = 0; hook && index < 64; index += 1, hook = hook.next) {
          const scope = hook.memoizedState?.current;
          if (scope && typeof scope.get === "function" && scope.node) return scope;
        }
      }
    }
    return null;
  }

  async function codexPlusAppServerClient(hostId = "local") {
    const scope = codexPlusFindAppScope();
    if (!scope) throw new Error("Codex 界面还没准备好，稍后再试");
    const managerFrom = (atom) => {
      try {
        const manager = scope.get(atom);
        return manager && typeof manager.forHost === "function" ? manager : null;
      } catch {
        return null;
      }
    };
    let manager = codexPlusManagerAtom ? managerFrom(codexPlusManagerAtom) : null;
    if (!manager) {
      // 数字分块只检查当前页面已执行的模块缓存，不执行候选工厂或新加载懒模块。
      const entries = Array.from(document.scripts || []).map((script) => script.src)
        .filter((url) => /\/assets\/\d+\.[a-f\d]+\.js$/.test(url));
      for (const url of entries) {
        const runtime = (await import(url)).__webpack_require__;
        if (!runtime?.c || !runtime?.m) continue;
        for (const [key, factory] of Object.entries(runtime.m)) {
          if (typeof factory !== "function" || !runtime.c[key]) continue;
          const source = Function.prototype.toString.call(factory);
          if (!source.includes("AppServerManager RPC is unavailable for hostId:") || !source.includes(".forHost(")) continue;
          for (const value of Object.values(runtime.c[key].exports || {})) {
            if (!value || (typeof value !== "object" && typeof value !== "function")) continue;
            manager = managerFrom(value);
            if (manager) {
              codexPlusManagerAtom = value;
              break;
            }
          }
          if (manager) break;
        }
        if (manager) break;
      }
    }
    if (!manager) {
      const module = await loadOptionalCodexAppModule("app-initial-");
      for (const value of Object.values(module || {})) {
        if (!value || (typeof value !== "object" && typeof value !== "function")) continue;
        manager = managerFrom(value);
        if (manager) {
          codexPlusManagerAtom = value;
          break;
        }
      }
    }
    if (!manager) throw new Error("没有找到 Codex 的 app-server 连接（Codex 版本不兼容）");
    return manager.forHost(hostId);
  }

  function installCodexThreadContextObserver() {
    if (!codexPerModelContextEnabled()) return;
    const state = window.__codexPlusThreadContextObserver
      ||= { pending: null, subscription: null, handle: null, retryAt: 0 };
    if (state.pending || state.handle || Date.now() < state.retryAt) return state.pending;
    state.pending = (async () => {
      const client = await codexPlusAppServerClient("local");
      if (!codexPerModelContextEnabled()) return;
      const subscription = client.subscribe({
        type: "notification",
        key: { hostId: "local" },
        methods: ["thread/tokenUsage/updated"],
        listener: ({ params }) => {
          const size = params?.tokenUsage?.modelContextWindow;
          if (typeof params?.threadId !== "string" || !Number.isSafeInteger(size) || size <= 0) return;
          const observed = window.__codexPlusObservedContextWindows ||= new Map();
          observed.set(params.threadId, size);
          if (observed.size > 256) observed.delete(observed.keys().next().value);
        },
      });
      state.subscription = subscription;
      const handle = await subscription;
      state.handle = handle;
      handle.onRpcBroken(() => {
        if (state.handle !== handle) return;
        state.handle = null;
        state.subscription = null;
        state.retryAt = Date.now() + 5000;
        handle[Symbol.dispose]();
        subscription[Symbol.dispose]();
        sendCodexPlusDiagnostic("thread_context_observer_disconnected", {});
      });
    })().catch((error) => {
      state.retryAt = Date.now() + 5000;
      state.subscription?.[Symbol.dispose]();
      state.subscription = null;
      state.handle = null;
      sendCodexPlusDiagnostic("thread_context_observer_failed", {
        errorMessage: String(error?.message || error),
      });
    }).finally(() => {
      state.pending = null;
    });
    return state.pending;
  }

  /** 把 append 的条目整理成 app-server 的 ResponseItem：字符串当作用户消息，{ role, text } 按角色转成消息。 */
  function codexPlusSessionItem(entry) {
    if (typeof entry === "string") return { type: "message", role: "user", content: [{ type: "input_text", text: entry }] };
    if (entry && typeof entry === "object" && typeof entry.type === "string") return entry;
    const role = entry?.role === "assistant" || entry?.role === "developer" ? entry.role : "user";
    return { type: "message", role, content: [{ type: role === "assistant" ? "output_text" : "input_text", text: String(entry?.text ?? "") }] };
  }

  /** 按方法名调用会话写入；params 形状见 codexPlusSessionsService。也给 Codex++ 进程经 CDP 调（后台、工具）。 */
  async function codexPlusSessionsCall(name, params = {}) {
    const method = codexPlusSessionMethods[name];
    if (!method) throw new Error(`会话服务没有方法 ${name}`);
    const client = await codexPlusAppServerClient(params.hostId || "local");
    switch (name) {
      case "list":
        return client.sendRequest(method, { limit: params.limit ?? 50, ...(params.cursor ? { cursor: params.cursor } : {}), ...(params.archived ? { archived: true } : {}) });
      case "fork":
        return (await client.sendRequest(method, { threadId: String(params.id), ...(params.options || {}) }))?.thread ?? null;
      case "append": {
        const items = (Array.isArray(params.items) ? params.items : [params.items]).map(codexPlusSessionItem);
        await client.sendRequest(method, { threadId: String(params.id), items });
        return { appended: items.length };
      }
      case "startTurn": {
        const input = typeof params.input === "string" ? [{ type: "text", text: params.input }] : params.input;
        if (!Array.isArray(input) || !input.length) throw new Error("startTurn 需要输入文字");
        return (await client.sendRequest(method, { threadId: String(params.id), input, ...(params.options || {}) }))?.turn ?? null;
      }
      case "interrupt": {
        // 没给 turnId 时取会话里正在跑的那一轮（dsh 的 agent.cancel 只知道会话）。
        let turnId = String(params.turnId || "");
        if (!turnId) {
          const read = await client.sendRequest("thread/read", { threadId: String(params.id), includeTurns: true });
          const turns = Array.isArray(read?.thread?.turns) ? read.thread.turns : [];
          const running = [...turns].reverse().find((turn) => /^in[_-]?progress$/i.test(String(turn?.status?.type ?? turn?.status ?? "")));
          turnId = String(running?.id ?? "");
          if (!turnId) return { interrupted: false, reason: "会话里没有正在进行的一轮" };
        }
        return client.sendRequest(method, { threadId: String(params.id), turnId });
      }
      case "create":
        // 新建会话；给了 input 时紧接着发起第一轮。
        {
          const thread = (await client.sendRequest(method, { ...(params.options || {}) }))?.thread ?? null;
          if (thread?.id && params.input) {
            const turn = await codexPlusSessionsCall("startTurn", { id: thread.id, input: params.input, hostId: params.hostId });
            return { thread, turn };
          }
          return { thread, turn: null };
        }
      case "rename":
        if (!String(params.name || "").trim()) throw new Error("rename 需要新名字");
        return client.sendRequest(method, { threadId: String(params.id), name: String(params.name).trim() });
      case "steer": {
        // 会话正忙时往当前这一轮插话。
        const input = typeof params.input === "string" ? [{ type: "text", text: params.input }] : params.input;
        if (!Array.isArray(input) || !input.length) throw new Error("steer 需要输入文字");
        return client.sendRequest(method, { threadId: String(params.id), input, ...(params.turnId ? { expectedTurnId: String(params.turnId) } : {}) });
      }
      case "compact":
        return client.sendRequest(method, { threadId: String(params.id) });
      default:
        throw new Error(`会话服务没有方法 ${name}`);
    }
  }

  /** 内置的会话写入服务（inject: ["sessions"]）：新建、分叉、改名、追加条目、发起 / 插话 / 中断一轮、压缩。读请用 sessionQuery。 */
  function codexPlusSessionsService() {
    return Object.freeze({
      list: (options = {}) => codexPlusSessionsCall("list", options),
      create: (input, options) => codexPlusSessionsCall("create", { input, options }),
      fork: (id, options) => codexPlusSessionsCall("fork", { id, options }),
      rename: (id, name) => codexPlusSessionsCall("rename", { id, name }),
      append: (id, items) => codexPlusSessionsCall("append", { id, items }),
      startTurn: (id, input, options) => codexPlusSessionsCall("startTurn", { id, input, options }),
      steer: (id, input, turnId) => codexPlusSessionsCall("steer", { id, input, turnId }),
      interrupt: (id, turnId) => codexPlusSessionsCall("interrupt", { id, turnId }),
      compact: (id) => codexPlusSessionsCall("compact", { id }),
    });
  }

  function installCodexPlusBuiltinServices() {
    window.__codexPlusSessionsCall = codexPlusSessionsCall;
    // 后台、工具、钩子经 CDP 调：window.__codexPlusUiCall("ask", 参数)。
    window.__codexPlusUiCall = (name, params) => {
      if (name === "ask") return codexPlusAsk(params || {});
      if (name === "toast") return showToast(String(params?.message ?? ""), params?.options || {}), true;
      throw new Error(`界面服务没有方法 ${name}`);
    };
    if (!codexPlusServices.has("themes") && typeof codexPlusThemesService === "function") codexPlusProvideService("themes", codexPlusThemesService(), "builtin");
    if (!codexPlusServices.has("sessions")) codexPlusProvideService("sessions", codexPlusSessionsService(), "builtin");
    if (!codexPlusServices.has("sessionQuery")) codexPlusProvideService("sessionQuery", codexPlusSessionQueryService(), "builtin");
    // 全局的多语言服务：写成 inject: ["locale"] 的拓展也能用（ctx.locale 是按实例自动撤销的同一套接口）。
    if (!codexPlusServices.has("locale")) {
      codexPlusProvideService("locale", Object.freeze({
        get current() {
          return codexPlusCurrentLocale();
        },
        register(ns, langOrDicts, dict) {
          if (typeof langOrDicts === "string") return codexPlusRegisterLocale(String(ns), langOrDicts, dict);
          const disposers = Object.entries(langOrDicts || {}).map(([lang, value]) => codexPlusRegisterLocale(String(ns), lang, value));
          return () => disposers.forEach((dispose) => dispose());
        },
        bind: (ns) => (key, params) => codexPlusTranslate(String(ns), key, params),
        resolve: codexPlusResolveText,
      }), "builtin");
    }
    installCodexPlusLocaleWatcher();
    window.__codexPlusSyncBackendServices = syncCodexPlusBackendServices;
    syncCodexPlusBackendServices();
  }

  /**
   * 拓展后台声明的服务（codexpp.json 的 backend.provides）：登记成页面服务，值是一个代理，
   * 调用任意方法都经桥接转给提供方后台（没起就先起）。清单随注入包下发，热重载时增删。
   * 页面里已有同名服务时以页面的为准。
   */
  const codexPlusBackendServiceEntries = new Map();

  function codexPlusBackendServiceProxy(name) {
    return new Proxy({}, {
      get: (_, method) => (typeof method !== "string" || method === "then") ? undefined : (...args) => {
        const bridge = window.__codexSessionDeleteBridge;
        if (typeof bridge !== "function") return Promise.reject(new Error("Codex 页面尚未连接，请稍后重试"));
        return Promise.resolve(bridge("/extensions/services/call", { name, method, args })).then((result) => {
          if (result?.status === "failed" || result?.error) throw new Error(result.message || result.error || `${name}.${method} 调用失败`);
          return result?.result;
        });
      },
    });
  }

  function syncCodexPlusBackendServices() {
    const list = Array.isArray(window.__codexPlusBackendServices) ? window.__codexPlusBackendServices : [];
    const wanted = new Map(
      list
        .filter((item) => item && codexPlusServiceNamePattern.test(String(item.name)))
        .map((item) => [String(item.name), String(item.key || "")]),
    );
    for (const [name, entry] of [...codexPlusBackendServiceEntries]) {
      if (wanted.get(name) === entry.key) continue;
      codexPlusBackendServiceEntries.delete(name);
      entry.dispose();
    }
    for (const [name, key] of wanted) {
      if (codexPlusBackendServiceEntries.has(name) || codexPlusServices.has(name)) continue;
      try {
        const dispose = codexPlusProvideService(name, codexPlusBackendServiceProxy(name), `backend:${key}`);
        codexPlusBackendServiceEntries.set(name, { key, dispose });
      } catch {}
    }
  }

  /**
   * 多语言：词条按命名空间登记（同一命名空间可多次登记，后登记的优先）。查找顺序：当前语言 →
   * 主语言（zh-CN → zh）→ en，先在指定命名空间，再在 common；都没有就返回 key 本身。
   * 占位符写成 {name}。拓展包 locale/<语言>.json 自动登记到「拓展 id」命名空间。
   */
  const codexPlusLocaleEntries = new Map();
  const codexPlusFallbackLocale = "en";

  function codexPlusCurrentLocale() {
    const raw = (typeof document !== "undefined" && document.documentElement?.lang) || globalThis.navigator?.language || codexPlusFallbackLocale;
    return String(raw).trim() || codexPlusFallbackLocale;
  }

  function codexPlusLocaleChain(locale = codexPlusCurrentLocale()) {
    const exact = String(locale);
    const primary = exact.split(/[-_]/)[0].toLowerCase();
    return [...new Set([exact, exact.toLowerCase(), primary, codexPlusFallbackLocale])];
  }

  function codexPlusRegisterLocale(ns, lang, dict) {
    if (!dict || typeof dict !== "object") throw new Error(`词条必须是对象：${ns}/${lang}`);
    const entry = { lang: String(lang), dict };
    if (!codexPlusLocaleEntries.has(ns)) codexPlusLocaleEntries.set(ns, []);
    codexPlusLocaleEntries.get(ns).unshift(entry);
    return () => {
      const list = codexPlusLocaleEntries.get(ns) || [];
      const index = list.indexOf(entry);
      if (index >= 0) list.splice(index, 1);
      if (!list.length) codexPlusLocaleEntries.delete(ns);
    };
  }

  function codexPlusLocaleLookup(dict, key) {
    if (typeof dict[key] === "string") return dict[key];
    let node = dict;
    for (const part of String(key).split(".")) {
      if (!node || typeof node !== "object") return undefined;
      node = node[part];
    }
    return typeof node === "string" ? node : undefined;
  }

  function codexPlusTranslate(ns, key, params) {
    const chain = codexPlusLocaleChain();
    let text;
    for (const space of [ns, "common"]) {
      const list = codexPlusLocaleEntries.get(space);
      if (!list) continue;
      for (const lang of chain) {
        for (const entry of list) {
          if (entry.lang.toLowerCase() !== lang.toLowerCase()) continue;
          text = codexPlusLocaleLookup(entry.dict, key);
          if (text !== undefined) break;
        }
        if (text !== undefined) break;
      }
      if (text !== undefined) break;
    }
    if (text === undefined) text = String(key);
    if (!params || typeof params !== "object") return text;
    return text.replace(/\{(\w+)\}/g, (match, name) => (params[name] === undefined ? match : String(params[name])));
  }

  /** 解析 { en, zh, ... } 形式的多语言文本；本身是字符串就原样返回。 */
  function codexPlusResolveText(value) {
    if (value === null || value === undefined) return "";
    if (typeof value !== "object") return String(value);
    for (const lang of codexPlusLocaleChain()) {
      const hit = Object.keys(value).find((key) => key.toLowerCase() === lang.toLowerCase());
      if (hit && typeof value[hit] === "string") return value[hit];
    }
    const first = Object.values(value).find((item) => typeof item === "string");
    return first ?? "";
  }

  /** 页面语言变化时广播 locale/change（参数 { locale }）。 */
  let codexPlusLocaleWatcherInstalled = false;
  function installCodexPlusLocaleWatcher() {
    if (codexPlusLocaleWatcherInstalled || typeof MutationObserver !== "function" || !document?.documentElement) return;
    codexPlusLocaleWatcherInstalled = true;
    let last = codexPlusCurrentLocale();
    new MutationObserver(() => {
      const next = codexPlusCurrentLocale();
      if (next === last) return;
      last = next;
      codexPlusEmitExtensionEvent("locale/change", [{ locale: next }]);
    }).observe(document.documentElement, { attributes: true, attributeFilter: ["lang"] });
  }

  /** 拓展包的清单（Rust 侧写进脚本状态记录）；单文件脚本没有清单。 */
  function codexPlusExtensionManifest(scriptKey) {
    const manifest = window.__codexPlusUserScripts?.scripts?.[scriptKey]?.manifest;
    if (!manifest || typeof manifest !== "object") return null;
    // 给拓展一份副本，避免它改到状态记录里的原件。
    try {
      return JSON.parse(JSON.stringify(manifest));
    } catch {
      return null;
    }
  }

  function codexPlusIsThenable(value) {
    return !!value && typeof value.then === "function";
  }

  /**
   * 全部会话的事件流：有拓展监听 `session/event` / `agent/status` 时，按会话更新时间增量读 rollout，
   * 把新事件逐条发出去。新版 Codex 的通知不经过页面，rollout 是页面能拿到的唯一完整来源。
   *   - `session/event`：(event, { sessionId })，event 形状同 sessionQuery 的事件日志
   *   - `agent/status`：({ sessionId, status: "running" | "idle" | "aborted", turn, reason? })
   *   - 更细的通知（agent 只是 { id, session: { id } } 描述，没有 steer / inject）：
   *     `tools/result`：(exec { callId, name, arguments }, result { isError: false, content })，工具结果写进会话时；
   *     `agent/created`：({ agent, source: "startup" })，观察开始后新出现的会话；
   *     `agent/error`：({ agent, turn, error })，一轮被中止（turn/aborted）时
   * 没人监听时自动停；监听开始前已有的事件不重放，之后新建的会话从头发。
   */
  const codexPlusSessionWatchEvents = ["session/event", "agent/status", "tools/result", "agent/created", "agent/error"];
  let codexPlusSessionWatcher = null;

  function codexPlusWantsSessionWatch() {
    return codexPlusSessionWatchEvents.some((event) => codexPlusExtensionListeners.get(event)?.size);
  }

  function codexPlusSessionStatusOf(event) {
    if (event.type === "turn/start") return { status: "running", turn: event.data?.turn };
    if (event.type === "turn/complete") return { status: "idle", turn: event.data?.turn };
    if (event.type === "turn/aborted") return { status: "aborted", turn: event.data?.turn, reason: event.data?.reason ?? null };
    return null;
  }

  function codexPlusEnsureSessionWatcher() {
    if (codexPlusSessionWatcher || !codexPlusWantsSessionWatch()) return;
    // 重新注入会带来一整套新的闭包，而旧的 tick 循环还活着、只认自己闭包里的变量，
    // 于是每注入一次就多叠一套轮询。用注入代际让旧循环发现「已经被接管」后自己收摊。
    const stillCurrent = codexPlusInjectionStillCurrent;
    const query = codexPlusSessionQueryService();
    const cursors = new Map();
    // callId → { name, arguments }：tools/result 要带上调用时的工具名与参数。
    const calls = new Map();
    let primed = false;
    // 会话列表的变化比会话内容慢一个量级：每轮都拉一次 listSessions 等于两秒一次全量，
    // 实测把 /extensions/history 顶到 137 次/分钟。这里缓存列表，每 4 轮刷新一次。
    let cachedSessions = null;
    let round = 0;
    const tick = async () => {
      if (!stillCurrent()) {
        clearTimeout(codexPlusSessionWatcher);
        codexPlusSessionWatcher = null;
        return;
      }
      if (!codexPlusWantsSessionWatch()) {
        codexPlusSessionWatcher = null;
        return;
      }
      try {
        if (!cachedSessions || round % 4 === 0) {
          cachedSessions = await query.listSessions({ limit: 20 });
        }
        round += 1;
        // 只有正在看的会话需要每轮实时读；其余会话按更新时间变化再读（精度到秒）。
        // 原来对前 3 个会话无条件读，等于每轮固定 3 次桥往返，是空转的大头。
        const activeId = codexPlusCurrentSessionId();
        for (const session of cachedSessions || []) {
          let cursor = cursors.get(session.id);
          if (!cursor) {
            if (primed) codexPlusEmitExtensionEvent("agent/created", [{ agent: codexPlusAgentDescriptor(session.id), source: "startup" }]);
            let next = 0;
            if (!primed) {
              const tail = await query.listEvents(session.id, { fromEnd: true, limit: 1 }).catch(() => []);
              next = tail.length ? tail[0].seq + 1 : 0;
            }
            cursor = { updatedAt: primed ? null : session.updatedAt, next };
            cursors.set(session.id, cursor);
            if (!primed) continue;
          }
          const isActive = Boolean(activeId) && session.id === activeId;
          if (!isActive && cursor.updatedAt === session.updatedAt) continue;
          cursor.updatedAt = session.updatedAt;
          const events = await query.readEventsSince({ sessionId: session.id, seq: cursor.next, limit: 200 }).catch(() => []);
          for (const event of events || []) {
            cursor.next = event.seq + 1;
            codexPlusEmitExtensionEvent("session/event", [event, { sessionId: session.id }]);
            const status = codexPlusSessionStatusOf(event);
            if (status) codexPlusEmitExtensionEvent("agent/status", [{ sessionId: session.id, ...status }]);
            codexPlusEmitSessionNotices(session.id, event, calls);
          }
        }
        primed = true;
      } catch {}
      // 后台标签页里没人看，降到一个量级更低的节拍，别在用户切走之后继续占桥。
      codexPlusSessionWatcher = setTimeout(tick, document.hidden ? 30000 : 1500);
    };
    codexPlusSessionWatcher = setTimeout(tick, 0);
  }

  /** 页面通知里的 agent 描述：只有会话 id，不能往会话里插话。 */
  function codexPlusAgentDescriptor(sessionId) {
    return Object.freeze({ id: sessionId, session: Object.freeze({ id: sessionId }) });
  }

  /** 会话事件 → tools/result、agent/error 通知。calls 记着工具调用，工具结果到了再配对。 */
  function codexPlusEmitSessionNotices(sessionId, event, calls) {
    if (event.type === "tool/call") {
      calls.set(event.data?.callId, { name: event.data?.name ?? "", arguments: event.data?.input ?? "" });
      if (calls.size > 500) calls.delete(calls.keys().next().value);
      return;
    }
    if (event.type === "tool/result" && codexPlusExtensionListeners.get("tools/result")?.size) {
      const call = calls.get(event.data?.callId) || {};
      let parsed = call.arguments;
      try { parsed = typeof call.arguments === "string" ? JSON.parse(call.arguments) : call.arguments; } catch {}
      const agent = codexPlusAgentDescriptor(sessionId);
      codexPlusEmitExtensionEvent("tools/result", [
        { callId: event.data?.callId ?? "", name: call.name ?? "", arguments: parsed, agent },
        { isError: false, value: event.data?.output ?? "", content: [{ type: "text", text: String(event.data?.output ?? "") }] },
      ]);
      return;
    }
    if (event.type === "turn/aborted") {
      codexPlusEmitExtensionEvent("agent/error", [{ agent: codexPlusAgentDescriptor(sessionId), turn: event.data?.turn, error: new Error(String(event.data?.reason ?? "aborted")) }]);
    }
  }

  /**
   * Codex++ 后台经 CDP 发给拓展的公开事件（只认 codexpp/ 前缀），例如会话索引修复结束的
   * `codexpp/session-index-repaired`。拓展用 ctx.on 订阅。
   */
  window.__codexPlusEmitHostEvent = (event, payload) => {
    if (typeof event !== "string" || !event.startsWith("codexpp/")) return false;
    codexPlusEmitExtensionEvent(event, [payload]);
    return true;
  };
  /** 通知所有监听者。逐个隔离：一个处理器抛错不影响其余。 */
  function codexPlusEmitExtensionEvent(event, args = []) {
    const listeners = codexPlusExtensionListeners.get(event);
    if (!listeners) return;
    for (const listener of [...listeners]) {
      runCodexPlusExtensionCallback(listener.scriptKey, `on:${event}`, () => listener.handler(...args));
    }
  }

  /**
   * 跑一条中间件链。
   *
   * 中间件可以是异步的：返回 Promise 时整条链的结果也是 Promise。中间件抛错（或
   * reject）时跳过它——若它已经调过 next，沿用下游的结果，避免下游被执行两次。
   */
  function codexPlusRunWaterfall(name, value) {
    const chain = [...(codexPlusExtensionInterceptors.get(name) || [])];
    const step = (index, current) => {
      if (index >= chain.length) return current;
      const item = chain[index];
      let called = false;
      let downstream;
      const next = (nextValue = current) => {
        called = true;
        downstream = step(index + 1, nextValue);
        return downstream;
      };
      const recover = () => (called ? downstream : step(index + 1, current));
      const outcome = runCodexPlusExtensionCallback(item.scriptKey, `intercept:${name}`, () => item.handler(current, next));
      if (!outcome.ok) return recover();
      if (codexPlusIsThenable(outcome.value)) {
        return Promise.resolve(outcome.value).catch((error) => {
          codexPlusMarkExtensionFailure(item.scriptKey, `intercept:${name}: ${String(error?.stack || error?.message || error)}`);
          return recover();
        });
      }
      return outcome.value;
    };
    return step(0, value);
  }

  /**
   * Codex 会话事件：把 app-server 推给页面的通知转成拓展事件 `codex/<method>`。
   *
   * 复用 30-service-tier 找到的通知分发器（dispatcher.subscribe），只订阅一组固定的通知；
   * 没有拓展监听时 emit 只是一次 Map 查找。分发器换了（Codex 重建客户端）就重新订阅。
   */
  function installCodexExtensionEventBridge(dispatcher) {
    if (!dispatcher || typeof dispatcher.subscribe !== "function") return false;
    // 列表放在函数里：本函数可能在内核分片的常量初始化之前被调用（测试单独执行前面的分片），
    // 顶层 const 会落进暂时性死区。
    const events = [
      "thread/started",
      "turn/started",
      "turn/completed",
      "item/started",
      "item/completed",
      "item/agentMessage/delta",
      "error",
      // 用量与等待用户的通知（Agent 桌宠拓展的大肥鱼用它显示本轮用量、等待审批 / 提问）。
      "thread/tokenUsage/updated",
      "item/tool/requestUserInput",
      "item/commandExecution/requestApproval",
      "item/fileChange/requestApproval",
      "item/permissions/requestApproval",
      "mcpServer/elicitation/request",
      "serverRequest/resolved",
      "item/tool/userInputAnswered",
      "item/approval/responded",
    ];
    try {
      const current = window.__codexPlusExtensionEventBridge;
      if (current?.dispatcher === dispatcher) return true;
      try {
        current?.unsubscribe?.();
      } catch {}
      const unsubscribers = events.map((method) => {
        try {
          return dispatcher.subscribe(method, (params) => {
            // 转发失败（例如内核尚未初始化）不能影响 Codex 自己的通知处理。
            try {
              codexPlusEmitExtensionEvent(`codex/${method}`, [params, { method }]);
            } catch {}
          });
        } catch {
          return null;
        }
      });
      window.__codexPlusExtensionEventBridge = {
        dispatcher,
        unsubscribe: () => unsubscribers.forEach((unsubscribe) => {
          try {
            if (typeof unsubscribe === "function") unsubscribe();
          } catch {}
        }),
      };
      return true;
    } catch {
      return false;
    }
  }

  /**
   * 发往 Codex app-server 的请求中间件：`codex/request` 链，值为 `{ method, params }`。
   *
   * 两个接入点：新版 Codex 的请求走 app-server 客户端 sendRequest（异步，见
   * codexPlusApplyExtensionRequestMiddlewareAsync）；旧的消息分发路径走这里，是同步的，
   * 中间件返回 Promise 时视为出错并原样放行。没有中间件时原样返回同一个对象，
   * 调用方靠对象身份判断「是否改过」。method 不允许改。
   */
  function codexPlusApplyExtensionRequestMiddleware(method, params) {
    try {
      if (!codexPlusExtensionInterceptors.has("codex/request")) return params;
      const result = codexPlusRunWaterfall("codex/request", { method, params });
      if (codexPlusIsThenable(result)) {
        Promise.resolve(result).catch(() => {});
        codexPlusMarkExtensionFailure("", "codex/request 中间件必须同步返回，本次请求已原样发出");
        return params;
      }
      return result && typeof result === "object" && result.params !== undefined ? result.params : params;
    } catch {
      // 页面早期（内核尚未初始化）或中间件整体失败时，绝不能挡住 Codex 自己的请求。
      return params;
    }
  }

  /** 有拓展挂了 codex/request 中间件：需要 app-server 客户端补丁（以及它依赖的类捕获）。 */
  function codexPlusExtensionWantsRequestMiddleware() {
    try {
      return codexPlusExtensionInterceptors.has("codex/request");
    } catch {
      return false;
    }
  }

  /**
   * 异步版：app-server 客户端的 sendRequest 本身是异步的，这里允许中间件返回 Promise。
   * 出错或超时（5 秒）一律原样放行，绝不能让 Codex 自己的请求卡住。
   */
  async function codexPlusApplyExtensionRequestMiddlewareAsync(method, params) {
    if (!codexPlusExtensionWantsRequestMiddleware()) return params;
    try {
      const result = await Promise.race([
        Promise.resolve(codexPlusRunWaterfall("codex/request", { method, params })),
        new Promise((resolve) => setTimeout(() => resolve(null), 5000)),
      ]);
      return result && typeof result === "object" && result.params !== undefined ? result.params : params;
    } catch {
      return params;
    }
  }

  /**
   * 创建一个拓展实例并执行 apply(ctx)。
   *
   * definition 可以是 apply 函数本身，也可以是 { id, apply }。apply 同步抛错时
   * 先回滚它已经做过的注册，再把错误抛给调用方——脚本初始化期间调用的话，
   * wrap_script 会把该脚本标成 failed。异步 apply reject 时同样回滚并上报。
   *
   * 必须在脚本同步初始化期间调用才能接上热重载：此时内核向用户脚本运行时登记
   * 一个清理函数，重载前整体卸载。之后（await 之后）创建的实例只能靠 ctx.dispose()。
   */
  function codexPlusCreateExtension(definition, options = {}) {
    const apply = typeof definition === "function" ? definition : definition?.apply;
    if (typeof apply !== "function") throw new Error("拓展必须提供 apply(ctx) 函数");
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const packageManifest = codexPlusExtensionManifest(scriptKey);
    // 清单声明了 instances：同一个拓展按实例各应用一次（同 dsh 同一插件开多份），配置是包配置叠上实例配置。
    const instances = Array.isArray(packageManifest?.instances) ? packageManifest.instances : [];
    if (instances.length && !options.instance) {
      const handles = instances.map((instance) => codexPlusCreateExtension(definition, { ...options, scriptKey, instance }));
      return handles[0];
    }
    const instance = options.instance ?? null;
    const manifest = instance
      ? { ...packageManifest, instance: { id: String(instance.id), name: instance.name ?? String(instance.id) }, config: { ...(packageManifest?.config ?? {}), ...(instance.config ?? {}) } }
      : packageManifest;
    const baseId = String(
      (typeof definition === "object" && definition?.id) || options.id || packageManifest?.id || scriptKey || "anonymous",
    );
    const extensionId = instance ? `${baseId}#${instance.id}` : baseId;
    const rawInject = (typeof definition === "object" && definition?.inject) || options.inject;
    const inject = codexPlusNormalizeInject(rawInject);
    const required = codexPlusRequiredInject(rawInject);
    const runtime = window.__codexPlusUserScripts;
    const reloadable = !!scriptKey && runtime?.currentKey === scriptKey && typeof runtime.registerCleanup === "function";
    if (!inject.length) {
      const ctx = codexPlusInstantiateExtension({ apply, scriptKey, manifest, extensionId, inject });
      if (reloadable) runtime.registerCleanup(() => ctx.dispose());
      return ctx;
    }
    const fiber = codexPlusCreateExtensionFiber({ apply, scriptKey, manifest, extensionId, inject, required });
    if (reloadable) runtime.registerCleanup(() => fiber.dispose());
    return fiber;
  }

  /**
   * 声明了 inject 的拓展：依赖齐全才创建实例；依赖消失或换了提供者就卸载实例、回到挂起，
   * 提供者回来再重新应用。返回的句柄可以随时 dispose。
   */
  function codexPlusCreateExtensionFiber({ apply, scriptKey, manifest, extensionId, inject, required = inject }) {
    let tokens = "";
    // apply 里提供服务会触发全体复查，又回到这个句柄：正在检查时只记下「需要再查一次」。
    let checking = false;
    let recheck = false;
    const fiber = {
      id: extensionId,
      scriptKey,
      inject: [...inject],
      state: "pending",
      ctx: null,
      check() {
        if (fiber.state === "disposed") return;
        if (checking) {
          recheck = true;
          return;
        }
        checking = true;
        try {
          fiber.update();
        } finally {
          checking = false;
        }
        if (recheck) {
          recheck = false;
          fiber.check();
        }
      },
      update() {
        const ready = required.every((name) => codexPlusServices.has(name));
        const current = inject.map((name) => codexPlusServices.get(name)?.token ?? "").join("|");
        if (fiber.ctx && (!ready || current !== tokens)) {
          const ctx = fiber.ctx;
          fiber.ctx = null;
          fiber.state = "pending";
          ctx.dispose();
        }
        if (!fiber.ctx && ready && fiber.state !== "disposed") {
          tokens = current;
          fiber.state = "active";
          try {
            fiber.ctx = codexPlusInstantiateExtension({ apply, scriptKey, manifest, extensionId, inject, fiber });
          } catch (error) {
            fiber.state = "failed";
            codexPlusMarkExtensionFailure(scriptKey, `${extensionId}: ${String(error?.stack || error?.message || error)}`);
          }
        }
      },
      dispose() {
        if (fiber.state === "disposed") return;
        fiber.state = "disposed";
        codexPlusServiceFibers.delete(fiber);
        const ctx = fiber.ctx;
        fiber.ctx = null;
        ctx?.dispose();
      },
    };
    codexPlusServiceFibers.add(fiber);
    fiber.check();
    return fiber;
  }

  /** 创建并应用一个拓展实例。inject 里的服务挂到 ctx 上；fiber 是它所属的生命周期句柄（可无）。 */
  function codexPlusInstantiateExtension({ apply, scriptKey, manifest, extensionId, inject = [], fiber = null }) {
    const disposers = [];
    let disposed = false;

    /** 登记一个撤销函数，返回「提前撤销」。实例已卸载时立刻执行。 */
    const track = (dispose) => {
      if (typeof dispose !== "function") return () => {};
      let active = true;
      const entry = () => {
        if (!active) return;
        active = false;
        dispose();
      };
      if (disposed) {
        runCodexPlusExtensionCallback(scriptKey, `${extensionId}.dispose`, entry);
        return () => {};
      }
      disposers.push(entry);
      return () => {
        const index = disposers.indexOf(entry);
        if (index >= 0) disposers.splice(index, 1);
        entry();
      };
    };

    // 多实例时注册项的 id 带上实例 id，同一个拓展的几份不会互相顶掉。
    const instanceId = manifest?.instance?.id;
    const registerOptions = (itemOptions) => ({
      ...(itemOptions || {}),
      ...(instanceId && itemOptions?.id ? { id: `${itemOptions.id}@${instanceId}` } : {}),
      scriptKey,
    });
    // 按拓展隔离的本地存储：键名自动带拓展 id 前缀，值按 JSON 存取。
    const storagePrefix = `codexpp:ext:${extensionId}:`;
    const storage = {
      get(key, fallback = null) {
        try {
          const raw = localStorage.getItem(storagePrefix + key);
          return raw === null ? fallback : JSON.parse(raw);
        } catch {
          return fallback;
        }
      },
      set(key, value) {
        localStorage.setItem(storagePrefix + key, JSON.stringify(value));
      },
      remove(key) {
        localStorage.removeItem(storagePrefix + key);
      },
      keys() {
        const out = [];
        for (let index = 0; index < localStorage.length; index += 1) {
          const key = localStorage.key(index);
          if (key?.startsWith(storagePrefix)) out.push(key.slice(storagePrefix.length));
        }
        return out;
      },
    };
    const info = { id: extensionId, scriptKey };
    const readyHandlers = new Set();
    let applied = false;
    const locale = Object.freeze({
      /** 当前语言 id（取 Codex 页面的 lang，如 zh-CN）。 */
      get current() {
        return codexPlusCurrentLocale();
      },
      /** register(ns, { zh: {...}, en: {...} }) 或 register(ns, "zh", {...})；随实例卸载撤销。 */
      register(ns, langOrDicts, dict) {
        const space = String(ns || extensionId);
        if (typeof langOrDicts === "string") return track(codexPlusRegisterLocale(space, langOrDicts, dict));
        const disposers = Object.entries(langOrDicts || {}).map(([lang, value]) => codexPlusRegisterLocale(space, lang, value));
        return track(() => disposers.forEach((dispose) => dispose()));
      },
      /** 绑定命名空间，返回 t(key, params)。 */
      bind(ns) {
        const space = String(ns || extensionId);
        return (key, params) => codexPlusTranslate(space, key, params);
      },
      /** 在本拓展的命名空间里翻译（包内 locale/*.json 已自动登记在这里）。 */
      t(key, params) {
        return codexPlusTranslate(extensionId, key, params);
      },
      /** 解析 { en, zh, ... } 形式的多语言文本。 */
      resolve: codexPlusResolveText,
    });

    // 生效配置（清单 config 默认值叠加用户在管理器 / 对话里设置的值）。改配置会热重载拓展，这里是只读快照。
    let config = Object.freeze({ ...(manifest?.config && typeof manifest.config === "object" ? manifest.config : {}) });
    const configHandlers = new Set();
    // 常驻后台：call 调方法（首次调用时 Codex++ 启动后台进程），on 订阅后台 emit 的事件（长轮询，
    // 随实例卸载停止；on 本身不启动后台）。走内部桥接路由，不经拓展路由白名单，key 固定为本脚本。
    // 用过后台的实例卸载时通知 Codex++ 停掉它；带实例标识，热重载时不会误停新实例拉起的后台。
    const backendHandlers = new Map();
    const backendInstance = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
    let backendPolling = false;
    let backendStopTracked = false;
    const trackBackendStop = () => {
      if (backendStopTracked) return;
      backendStopTracked = true;
      track(() => {
        void backendBridge("/extensions/backend/stop", {}).catch(() => {});
      });
    };
    const backendBridge = (path, payload) => {
      const bridge = window.__codexSessionDeleteBridge;
      if (typeof bridge !== "function") return Promise.reject(new Error("Codex 页面尚未连接，请稍后重试"));
      return Promise.resolve(bridge(path, { key: scriptKey, instance: backendInstance, ...payload })).then((result) => {
        if (result?.status === "failed" || result?.error) throw new Error(result.message || result.error || `${path} 调用失败`);
        return result;
      });
    };
    const pollBackendEvents = async () => {
      if (backendPolling) return;
      backendPolling = true;
      // 首次订阅从队尾开始：后台已在跑时，订阅前的旧事件不重放给这个页面实例。
      let since = "latest";
      try {
        while (!disposed && backendHandlers.size) {
          let result;
          try {
            result = await backendBridge("/extensions/backend/events", { since, waitMs: since === "latest" ? 0 : 20000 });
          } catch {
            await new Promise((resolve) => setTimeout(resolve, 2000));
            continue;
          }
          for (const item of result.events || []) {
            for (const name of [item.event, "*"]) {
              for (const handler of [...(backendHandlers.get(name) || [])]) {
                runCodexPlusExtensionCallback(scriptKey, `backend.on:${item.event}`, () => handler(item.payload, { event: item.event }));
              }
            }
          }
          since = Number(result.next) || (since === "latest" ? 0 : since);
          // 后台还没起：稍后再看，不空转。
          if (!result.running) await new Promise((resolve) => setTimeout(resolve, 3000));
        }
      } finally {
        backendPolling = false;
      }
    };
    // 调模型：经 Codex++ 协议代理用当前供应商（不跑拓展网关中间件）。
    const llm = Object.freeze({
      complete(options) {
        const payload = typeof options === "string" ? { input: options } : options ?? {};
        return backendBridge("/extensions/llm/complete", payload).then((result) => result.result);
      },
      listModels: () => backendBridge("/extensions/llm/models", {}).then((result) => result.result),
    });
    const backend = {
      call(method, params = null, callOptions = {}) {
        const timeoutMs = Number.isFinite(callOptions.timeout) ? Number(callOptions.timeout) : undefined;
        trackBackendStop();
        return backendBridge("/extensions/backend/call", { method: String(method), params, timeoutMs }).then((result) => result.result);
      },
      on(event, handler) {
        if (typeof handler !== "function") throw new Error("后台事件处理器必须是函数");
        const name = String(event);
        if (!backendHandlers.has(name)) backendHandlers.set(name, new Set());
        backendHandlers.get(name).add(handler);
        void pollBackendEvents();
        return track(() => {
          const set = backendHandlers.get(name);
          set?.delete(handler);
          if (set && !set.size) backendHandlers.delete(name);
        });
      },
    };
    const ctx = {
      id: extensionId,
      scriptKey,
      manifest,
      /** 清单 instances 里的哪一份（{ id, name }）；没声明实例时为 null。 */
      instance: manifest?.instance ?? null,
      get config() {
        return config;
      },
      /**
       * 配置原地更新：handler(新配置, 旧配置) 返回 true 表示接住了（ctx.config 换成新值，不重载）；
       * 这个拓展只要有一个实例没接，就照常整体热重载（同 Cordis 的 internal/update 否决重启）。
       */
      onConfigUpdate(handler) {
        if (typeof handler !== "function") throw new Error("onConfigUpdate 需要一个函数");
        configHandlers.add(handler);
        return track(() => configHandlers.delete(handler));
      },
      /** 内核用：问本实例能否原地接住新配置。 */
      __tryConfigUpdate(next) {
        if (!configHandlers.size) return false;
        const frozen = Object.freeze({ ...(next && typeof next === "object" ? next : {}) });
        for (const handler of [...configHandlers]) {
          const result = runCodexPlusExtensionCallback(scriptKey, `${extensionId}.onConfigUpdate`, () => handler(frozen, config));
          if (!result.ok || result.value !== true) return false;
        }
        config = frozen;
        return true;
      },
      backend,
      llm,
      get themes() {
        return codexPlusServices.get("themes")?.value;
      },
      get disposed() {
        return disposed;
      },
      storage,

      locale,

      /**
       * 包内资源的地址（data URL），可直接给 img.src / audio.src / CSS url()。资源放在包的 assets/ 下，
       * 例如 ctx.asset("assets/pet.png")；找不到时返回空串（单文件脚本没有资源）。
       */
      asset(path) {
        const key = String(path || "").replace(/^\.?\//, "");
        const assets = manifest?.assets && typeof manifest.assets === "object" ? manifest.assets : {};
        return assets[key] || assets[`assets/${key}`] || "";
      },

      /** 提供一个服务（扁平命名空间，重名抛错）；随实例卸载撤销，依赖它的拓展随之卸载。 */
      provide(name, value) {
        return track(codexPlusProvideService(String(name), value, scriptKey || extensionId));
      },
      /** 取一个服务，不存在时返回 undefined（可选依赖用这个；必需依赖请在 inject 里声明）。 */
      get(name) {
        return codexPlusServices.get(String(name))?.value;
      },
      /** 子作用域：deps 里的服务齐全时以子 ctx 调用 apply，服务消失时卸载、回来时重新应用。 */
      inject(deps, apply) {
        if (typeof apply !== "function") throw new Error("inject 需要 apply(ctx) 函数");
        const child = codexPlusCreateExtensionFiber({
          apply, scriptKey, manifest, extensionId, inject: codexPlusNormalizeInject(deps), required: codexPlusRequiredInject(deps),
        });
        track(() => child.dispose());
        return child;
      },

      /** 立即执行 fn；它返回的函数在卸载时调用。 */
      effect(fn) {
        if (typeof fn !== "function") throw new Error("effect 需要一个函数");
        const result = fn();
        if (!codexPlusIsThenable(result)) return track(result);
        // 异步 effect：等它给出撤销函数；实例先卸载了就在拿到时立刻撤销。
        let cancelled = false;
        let dispose = null;
        Promise.resolve(result).then((value) => {
          if (typeof value !== "function") return;
          if (cancelled) runCodexPlusExtensionCallback(scriptKey, `${extensionId}.dispose`, value);
          else dispose = value;
        }, (error) => ctx.fail(error));
        return track(() => {
          cancelled = true;
          dispose?.();
        });
      },

      registerRowAction(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("rowAction", codexPlusRegistry.rowActions, definition, registerOptions(itemOptions)));
      },
      registerNavEntry(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("navEntry", codexPlusRegistry.navEntries, definition, registerOptions(itemOptions)));
      },
      registerMenuItem(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("menuItem", codexPlusRegistry.menuItems, definition, registerOptions(itemOptions)));
      },
      registerPage(definition, itemOptions) {
        return track(codexPlusRegisterExtensionPage(definition, registerOptions(itemOptions)));
      },
      registerCommand(definition, itemOptions) {
        return track(codexPlusRegisterExtensionCommand(definition, registerOptions(itemOptions)));
      },
      registerComposerAction(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("composerAction", codexPlusRegistry.composerActions, definition, registerOptions(itemOptions)));
      },
      registerMessageAction(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("messageAction", codexPlusRegistry.messageActions, definition, registerOptions(itemOptions)));
      },
      registerMessageDecoration(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("messageDecoration", codexPlusRegistry.messageDecorations, definition, registerOptions(itemOptions)));
      },
      registerSlot(definition, itemOptions) {
        const slot = codexPlusRegisterExtensionSlot(definition, registerOptions(itemOptions));
        const handle = track(slot);
        handle.refresh = slot.refresh;
        return handle;
      },
      registerFloatingTab(definition, itemOptions) {
        return track(codexPlusRegisterFloatingTab(definition, registerOptions(itemOptions)));
      },
      registerSettingsPanel(definition, itemOptions) {
        return track(codexPlusRegisterSettingsPanel(definition, registerOptions(itemOptions)));
      },
      registerToolView(definition, itemOptions) {
        const view = codexPlusRegisterToolView(definition, registerOptions(itemOptions));
        const handle = track(view);
        handle.refresh = view.refresh;
        return handle;
      },
      registerSettingsSection(definition, itemOptions) {
        return track(codexPlusRegisterSettingsSection(definition, registerOptions(itemOptions)));
      },
      registerInputTrigger(definition, itemOptions) {
        return track(codexPlusRegisterInputTrigger(definition, registerOptions(itemOptions)));
      },
      registerShortcut(definition, itemOptions) {
        return track(codexPlusRegisterShortcut(definition, registerOptions(itemOptions)));
      },
      registerTheme(definition, itemOptions) {
        return track(codexPlusRegisterTheme(definition, registerOptions(itemOptions)));
      },
      /** 当前打开的 Codex 会话 id；首页、新对话还没有 id 时为空串。 */
      currentSessionId() {
        return codexPlusCurrentSessionId();
      },
      /** 弹对话框问用户，返回 { value, text }；取消为 null。 */
      ask(question) {
        return codexPlusAsk(question);
      },
      registerSidePanel(definition, itemOptions) {
        const panel = codexPlusRegisterExtensionSidePanel(definition, registerOptions(itemOptions));
        const handle = track(panel);
        handle.open = panel.open;
        handle.close = panel.close;
        handle.toggle = panel.toggle;
        return handle;
      },

      on(event, handler) {
        if (typeof event !== "string" || !event) throw new Error("事件名不能为空");
        if (typeof handler !== "function") throw new Error("事件处理器必须是函数");
        // ready / dispose 只属于本实例（同 Cordis）：ready 在 apply（含异步部分）完成后触发，dispose 随卸载。
        if (event === "ready") {
          if (applied) {
            queueMicrotask(() => !disposed && runCodexPlusExtensionCallback(scriptKey, `${extensionId}.ready`, handler));
            return () => {};
          }
          readyHandlers.add(handler);
          return track(() => readyHandlers.delete(handler));
        }
        if (event === "dispose") return track(() => runCodexPlusExtensionCallback(scriptKey, `${extensionId}.dispose`, handler));
        const listener = { handler, scriptKey };
        if (!codexPlusExtensionListeners.has(event)) codexPlusExtensionListeners.set(event, new Set());
        codexPlusExtensionListeners.get(event).add(listener);
        if (codexPlusSessionWatchEvents.includes(event)) codexPlusEnsureSessionWatcher();
        return track(() => {
          const listeners = codexPlusExtensionListeners.get(event);
          listeners?.delete(listener);
          if (listeners && !listeners.size) codexPlusExtensionListeners.delete(event);
        });
      },
      emit(event, ...args) {
        codexPlusEmitExtensionEvent(event, args);
      },

      intercept(name, handler, interceptOptions = {}) {
        if (typeof name !== "string" || !name) throw new Error("中间件链名不能为空");
        if (typeof handler !== "function") throw new Error("中间件必须是函数");
        const item = {
          handler,
          scriptKey,
          order: Number.isFinite(interceptOptions.order) ? Number(interceptOptions.order) : 0,
          seq: ++codexPlusExtensionInterceptorSeq,
        };
        const chain = [...(codexPlusExtensionInterceptors.get(name) || []), item];
        chain.sort((left, right) => left.order - right.order || left.seq - right.seq);
        codexPlusExtensionInterceptors.set(name, chain);
        return track(() => {
          const rest = (codexPlusExtensionInterceptors.get(name) || []).filter((entry) => entry !== item);
          if (rest.length) codexPlusExtensionInterceptors.set(name, rest);
          else codexPlusExtensionInterceptors.delete(name);
        });
      },
      waterfall(name, value) {
        return codexPlusRunWaterfall(name, value);
      },

      listen(target, type, handler, listenOptions) {
        if (!target || typeof target.addEventListener !== "function") throw new Error("listen 的目标必须支持 addEventListener");
        target.addEventListener(type, handler, listenOptions);
        return track(() => target.removeEventListener(type, handler, listenOptions));
      },
      setTimeout(handler, delay, ...args) {
        let timer = null;
        const revoke = track(() => window.clearTimeout(timer));
        timer = window.setTimeout(() => {
          revoke();
          runCodexPlusExtensionCallback(scriptKey, `${extensionId}.setTimeout`, () => handler(...args));
        }, delay);
        return revoke;
      },
      setInterval(handler, delay, ...args) {
        const timer = window.setInterval(() => {
          runCodexPlusExtensionCallback(scriptKey, `${extensionId}.setInterval`, () => handler(...args));
        }, delay);
        return track(() => window.clearInterval(timer));
      },
      /**
       * 跟着 Codex++ 的页面扫描跑：Codex 的界面变了（会话切换、侧栏重建、输入框重建……）时调用 handler，
       * 按帧合并、页面隐藏时不跑。phase：deferred（默认，完整扫描）或 light（每次变化都跑的轻量扫描，handler 要便宜）。
       * handler 插进页面的节点必须带 data-codex-plus-ext 属性，否则会触发自喂扫描循环。返回的撤销函数带 run()：立即跑一次。
       */
      onScan(handler, scanOptions = {}) {
        if (typeof handler !== "function") throw new Error("onScan 需要一个函数");
        const step = () => runCodexPlusExtensionCallback(scriptKey, `${extensionId}.onScan`, handler);
        const revoke = track(registerCodexPlusScanStep(scriptKey, step, scanOptions?.phase));
        revoke.run = () => {
          if (!disposed) step();
        };
        return revoke;
      },

      /**
       * 调后端路由。拓展包只能调清单 permissions.routes 声明过的路由；单文件脚本
       * 没有清单，沿用全局白名单。这是声明约束而非安全边界：拓展与页面同权。
       */
      call(route, payload, callOptions) {
        if (manifest) {
          const routes = Array.isArray(manifest.permissions?.routes) ? manifest.permissions.routes : [];
          if (!routes.includes(route)) {
            return Promise.reject(new Error(`拓展 ${extensionId} 未在 codexpp.json 声明路由权限：${route}`));
          }
        }
        return codexPlusExtensionCall(route, payload, callOptions);
      },
      toast(message, toastOptions) {
        return runCodexPlusExtensionCallback(scriptKey, `${extensionId}.toast`, () => showToast(String(message ?? ""), toastOptions || {}));
      },
      fail(error) {
        codexPlusMarkExtensionFailure(scriptKey, `${extensionId}: ${String(error?.stack || error?.message || error)}`);
      },

      /** 卸载：按注册逆序撤销，单项撤销抛错只记录、不中断其余撤销。 */
      dispose() {
        if (disposed) return;
        disposed = true;
        for (const entry of disposers.splice(0).reverse()) {
          runCodexPlusExtensionCallback(scriptKey, `${extensionId}.dispose`, entry);
        }
        codexPlusExtensionInstances.delete(ctx);
        codexPlusEmitExtensionEvent("extension/dispose", [info]);
        if (typeof scheduleCodexPlusExtensionsChanged === "function") scheduleCodexPlusExtensionsChanged();
      },
    };

    // 拓展包自带的 locale/<语言>.json 登记到本拓展的命名空间。
    if (manifest?.locales && typeof manifest.locales === "object") {
      for (const [lang, dict] of Object.entries(manifest.locales)) {
        if (dict && typeof dict === "object") track(codexPlusRegisterLocale(extensionId, lang, dict));
      }
    }
    // 声明过的服务挂到 ctx 上（读的是当前提供者的值；换了提供者时整个实例会被重建）。
    for (const name of inject) {
      if (Object.prototype.hasOwnProperty.call(ctx, name)) continue;
      Object.defineProperty(ctx, name, { enumerable: true, get: () => codexPlusServices.get(name)?.value });
    }
    if (fiber) Object.defineProperty(ctx, "fiber", { value: fiber });
    codexPlusExtensionInstances.add(ctx);

    let result;
    try {
      result = apply(ctx);
    } catch (error) {
      ctx.dispose();
      throw error;
    }
    // apply 完成（异步 apply 等它 resolve）后才算应用好：发 extension/apply，再调本实例的 ready 监听器。
    const settle = () => {
      if (disposed) return;
      applied = true;
      codexPlusEmitExtensionEvent("extension/apply", [info]);
      if (typeof scheduleCodexPlusExtensionsChanged === "function") scheduleCodexPlusExtensionsChanged();
      for (const handler of [...readyHandlers]) runCodexPlusExtensionCallback(scriptKey, `${extensionId}.ready`, handler);
      readyHandlers.clear();
    };
    if (codexPlusIsThenable(result)) {
      Promise.resolve(result).then(settle, (error) => {
        ctx.fail(error);
        ctx.dispose();
      });
    } else {
      settle();
    }
    return ctx;
  }
