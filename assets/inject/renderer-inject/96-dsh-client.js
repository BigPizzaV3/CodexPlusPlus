  /**
   * dsh 前端（client）插件的运行时：codexPlus.dshClient(ctx, modules, options)。
   *
   * dsh 的前端插件是懒 CJS 工厂（window.__ModuleLoader__.load({ id, factory })），factory(require) 返回
   * { inject, apply(ctx) }，apply 里用 ctx.slots.register(options, Component) 往 dsh Web 客户端的 React 插槽里放组件。
   * Codex 的页面也是 React（19），这里把 Codex 自己的 React / ReactDOM 交给插件的 require，再把 dsh 插槽一一对到
   * Codex++ 的界面插槽：组件用 createRoot 渲染进插槽容器，随拓展实例卸载。对不上的插槽不显示，记一条失败说明。
   *
   * 配套拓展包把插件的 client.js 夹在两段胶水之间：前一段截住 __ModuleLoader__.load 并给 fetch 换成经后台转给插件
   * 宿主半边（ctx.connection.fetch 注册的路由）的版本，后一段调这里。设计见 docs/plans/2026-10-30-dsh-plugin-import-design.md。
   */

  let codexPlusReactModulesPromise = null;

  /**
   * 找 Codex 页面里的 React、ReactDOM、react-dom/client、jsx-runtime。打包器把 CJS 依赖做成惰性 getter
   * （`()=>(t||(e((t={exports:{}}).exports,t,t.exports),e=null),t.exports)`）导出，调用它就拿到模块本身；
   * 这些 getter 只做一次初始化、之后返回缓存，调用没有副作用。
   */
  function loadCodexPlusReactModules() {
    codexPlusReactModulesPromise ||= (async () => {
      const getter = /^\(\)=>\(\w\|\|\(\w\(\(\w=\{exports:\{\}\}\)\.exports,\w,\w\.exports\),\w=null\),\w\.exports\)$/;
      const urls = codexAppAssetCandidateUrls().filter((url) => /\.js(\?|$)/.test(url));
      urls.sort((left, right) => Number(right.includes("app-shared")) - Number(left.includes("app-shared")));
      const found = {};
      for (const url of urls) {
        let module;
        try {
          module = await import(url);
        } catch {
          continue;
        }
        for (const value of Object.values(module)) {
          if (typeof value !== "function") continue;
          let source = "";
          try {
            source = Function.prototype.toString.call(value);
          } catch {}
          if (!getter.test(source)) continue;
          let exports;
          try {
            exports = value();
          } catch {
            continue;
          }
          if (!exports || (typeof exports !== "object" && typeof exports !== "function")) continue;
          if (!found.react && typeof exports.createElement === "function" && typeof exports.useState === "function") found.react = exports;
          else if (!found.reactDomClient && typeof exports.createRoot === "function") found.reactDomClient = exports;
          else if (!found.reactDom && typeof exports.createPortal === "function" && typeof exports.flushSync === "function") found.reactDom = exports;
          else if (!found.jsxRuntime && typeof exports.jsx === "function" && typeof exports.jsxs === "function") found.jsxRuntime = exports;
        }
        if (found.react && found.reactDomClient && found.reactDom && found.jsxRuntime) return found;
      }
      if (!found.react || !found.reactDomClient) throw new Error("没有在 Codex 页面里找到 React（Codex 版本不兼容）");
      return found;
    })();
    codexPlusReactModulesPromise.catch(() => {
      codexPlusReactModulesPromise = null;
    });
    return codexPlusReactModulesPromise;
  }

  /**
   * dsh 插槽 → Codex++ 位置。kind：slot（界面插槽）、rowAction（会话行「更多」菜单的自绘项）、
   * message（助手消息下方）、page（整页，带图标栏入口）。
   */
  const codexPlusDshSlotMap = {
    "shell.overlay": { kind: "slot", slot: "overlay" },
    "sidebar.workspaces.session.menu.item": { kind: "rowAction" },
    "sidebar.workspaces.session.row.action": { kind: "slot", slot: "sessionRow.suffix" },
    "sidebar.session.row.leading": { kind: "slot", slot: "sessionRow.prefix" },
    "sidebar.footer.action": { kind: "slot", slot: "sidebar.bottom" },
    "conversation.chat.turnTail": { kind: "slot", slot: "turn.after" },
    "conversation.input.left": { kind: "slot", slot: "composer.left" },
    "conversation.input.right": { kind: "slot", slot: "composer.right" },
    "conversation.input.dock": { kind: "slot", slot: "composer.above" },
    "conversation.input.overlay": { kind: "slot", slot: "composer.above" },
    "conversation.composer.dock": { kind: "slot", slot: "composer.below" },
    "conversation.session.header.actions": { kind: "slot", slot: "header.title" },
    "conversation.session.header.utilities": { kind: "slot", slot: "titlebar" },
    "conversation.session.header.corner": { kind: "slot", slot: "titlebar" },
    // 右侧栏的页签（如 dsh 的终端、文件、文档预览）：Codex 里放进悬浮球面板的页签。
    "sidebar.right.pane.tab": { kind: "floating" },
    "conversation.chat.assistant-actions": { kind: "message" },
    "conversation.input.model": { kind: "slot", slot: "composer.model" },
    "tool.call.toolview": { kind: "toolView" },
    "conversation.plan-review.actions": { kind: "toolView", kinds: ["plan"] },
    "settings.section": { kind: "settings" },
    "settings.general.item": { kind: "settings" },
    "settings.plugins.tab": { kind: "settings" },
    "sidebar.panellist": { kind: "page" },
    "conversation.approval.detail": { kind: "slot", slot: "approval" },
    "conversation.input.activity": { kind: "slot", slot: "activity.after" },
    "conversation.hero.workspace": { kind: "slot", slot: "home.above" },
    "conversation.hero.agentPreset": { kind: "slot", slot: "home.above" },
    "conversation.hero.brand.mark": { kind: "slot", slot: "home.above" },
    "conversation.view": { kind: "slot", slot: "thread.top" },
    "main.conversation": { kind: "slot", slot: "thread.top" },
    "conversation.header": { kind: "slot", slot: "header.title" },
    "conversation.header.leading": { kind: "slot", slot: "header.title" },
    "conversation.session.header": { kind: "slot", slot: "header.title" },
    "conversation.session.header.lineage": { kind: "slot", slot: "header.title" },
    "sidebar.session.row.hover": { kind: "slot", slot: "sessionRow.suffix" },
    "shell.quota-notice": { kind: "slot", slot: "overlay" },
    // 右侧栏的会话面板：Codex 里放进顶栏开关打开的侧边面板。
    "rightbar.session": { kind: "sidePanel" },
    "tool.view.cordis": { kind: "toolView" },
  };

  /** dsh 快捷键绑定（{ code, modifiers }）→ Codex++ 的 "Mod+Shift+K" 写法；认不出时返回空串。 */
  function codexPlusDshShortcutKey(binding) {
    if (!binding || typeof binding.code !== "string" || binding.secondCode) return "";
    const names = { primary: "Mod", control: "Ctrl", alt: "Alt", shift: "Shift", meta: "Meta" };
    const modifiers = (Array.isArray(binding.modifiers) ? binding.modifiers : []).map((item) => names[item]).filter(Boolean);
    const code = binding.code;
    const key = /^Key[A-Z]$/.test(code) ? code.slice(3) : /^Digit\d$/.test(code) ? code.slice(5) : /^F\d{1,2}$/.test(code) ? code
      : ({ Comma: ",", Period: ".", Slash: "/", Semicolon: ";", Quote: "'", BracketLeft: "[", BracketRight: "]", Backslash: "\\", Minus: "-", Equal: "=", Backquote: "`", Space: "Space", Enter: "Enter", Escape: "Escape" })[code];
    return key ? [...modifiers, key].join("+") : "";
  }

  /** 一个可订阅的快照（dsh 的 ObservableSnapshot），给 useSyncExternalStore 用。 */
  function codexPlusDshSnapshotStore(initial, start) {
    let value = initial;
    const listeners = new Set();
    let stop = null;
    return {
      getSnapshot: () => value,
      subscribe(listener) {
        listeners.add(listener);
        if (listeners.size === 1 && typeof start === "function") stop = start((next) => {
          value = next;
          for (const item of [...listeners]) {
            try { item(); } catch {}
          }
        });
        return () => {
          listeners.delete(listener);
          if (!listeners.size && typeof stop === "function") {
            stop();
            stop = null;
          }
        };
      },
      set(next) {
        value = next;
        for (const item of [...listeners]) {
          try { item(); } catch {}
        }
      },
    };
  }

  async function codexPlusRunDshClient(ctx, modules, options = {}) {
    const packageName = String(options.packageName || ctx.id);
    const libs = await loadCodexPlusReactModules();
    if (ctx.disposed) return;
    const { react: React, reactDomClient } = libs;
    const fail = (message) => ctx.fail(new Error(`[dsh ${packageName}] ${message}`));

    // ---- 客户端服务 ----
    const sessionQuery = codexPlusServices.get("sessionQuery")?.value;
    const sessionsWrite = codexPlusServices.get("sessions")?.value;
    const emptyList = { ids: [], byId: {}, phase: "loading", projectionsBySession: {} };
    const toList = (records) => {
      const byId = {};
      for (const record of records || []) {
        byId[record.id] = { sessionId: record.id, agentAvailable: true, updatedAt: Number(record.updatedAt) || 0, running: false, blank: false, cwd: record.cwd || undefined, title: record.title || "" };
      }
      return { ids: Object.keys(byId), byId, phase: "ready", projectionsBySession: {} };
    };
    const refreshList = async () => {
      if (!sessionQuery) return;
      try {
        sessionList.set(toList(await sessionQuery.listSessions({ limit: 100 })));
      } catch (error) {
        fail(`读取会话列表失败：${error?.message || error}`);
      }
    };
    const sessionList = codexPlusDshSnapshotStore(emptyList, () => {
      void refreshList();
      const timer = setInterval(() => void refreshList(), 15000);
      return () => clearInterval(timer);
    });
    const workspaceList = codexPlusDshSnapshotStore({ items: [], archivedSessionIds: [], pinnedSessionIds: [] });
    const clientSessions = {
      list: sessionList,
      refresh: refreshList,
      async create({ cwd } = {}) {
        const result = await sessionsWrite?.create(undefined, cwd ? { cwd } : {});
        return result?.thread?.id;
      },
      async fork({ sessionId } = {}) {
        return (await sessionsWrite?.fork(sessionId))?.id;
      },
      retainInfo: () => undefined,
    };
    const uiWorkspace = {
      selection: { getSnapshot: () => ({ sessionId: codexPlusCurrentSessionId() || undefined }), subscribe: () => () => {} },
      clearMain() {},
    };
    const localeService = {
      register(ns, langOrDicts, dict) {
        return ctx.locale.register(String(ns), langOrDicts, dict);
      },
      bind: (ns) => ctx.locale.bind(String(ns)),
      getLocale: () => ({ id: ctx.locale.current }),
      getSnapshot: () => ({ id: ctx.locale.current, revision: 0 }),
      subscribe: (listener) => ctx.on("locale/change", () => listener()),
      resolveText: ctx.locale.resolve,
      setLocale() {},
    };

    // dsh 的快捷键命令 → Codex++ 快捷键：取默认绑定（macOS 优先按平台挑），按下时问命令的 resolve。
    const shortcutsService = {
      runtime: "web",
      platform: codexPlusIsMacPlatform ? "mac" : "windows",
      stopSequenceMs: 1000,
      catalog: codexPlusDshSnapshotStore([]),
      config: codexPlusDshSnapshotStore({}),
      fixedCatalog: codexPlusDshSnapshotStore([]),
      register(command) {
        const defaults = command?.defaults || {};
        const platform = codexPlusIsMacPlatform ? "mac" : "windows";
        const binding = Object.entries(defaults).find(([profile]) => profile.endsWith(`:${platform}`))?.[1] ?? Object.values(defaults)[0];
        const key = codexPlusDshShortcutKey(binding);
        if (!key) {
          fail(`快捷键命令 ${command?.id || "?"} 的默认绑定 Codex 里表达不了，没有注册`);
          return () => {};
        }
        try {
          return ctx.registerShortcut({
            key,
            description: typeof command.label === "function" ? command.label() : String(command.id || ""),
            run: ({ event }) => {
              const resolution = command.resolve?.({ source: "keyboard", region: "page", modal: null, target: event?.target ?? null });
              if (resolution?.status === "handled") resolution.run();
            },
          });
        } catch (error) {
          fail(`快捷键 ${key} 注册失败：${error?.message || error}`);
          return () => {};
        }
      },
      registerFixed: () => () => {},
      observeFixedInput: () => () => {},
      describeBinding: (binding) => ({ keys: codexPlusDshShortcutKey(binding).split("+").filter(Boolean), aria: codexPlusDshShortcutKey(binding) }),
      edit: async () => ({ ok: false, reason: "Codex++ 里快捷键在拓展里改" }),
      recording: async () => {},
      closeWindow: async () => {},
    };
    // dsh 的命令面板贡献 → Codex 输入框里的斜杠命令：action 直接跑；popupSelect 先列选项让用户挑。
    const commandUi = {
      register(contribution) {
        const name = String(contribution?.name || "");
        const ui = contribution?.ui;
        if (!name || !ui) return () => {};
        const session = () => ({ sessionId: codexPlusCurrentSessionId() || undefined });
        try {
          return ctx.registerCommand({
            name,
            description: typeof contribution.description === "function" ? contribution.description() : (typeof contribution.label === "function" ? contribution.label() : ""),
            async run(_args, { toast }) {
              if (typeof contribution.available === "function" && !contribution.available(session())) {
                toast(`/${name} 现在不可用`, { type: "warning" });
                return;
              }
              if (ui.kind === "action") return ui.run(session());
              if (ui.kind !== "popupSelect") return;
              const controller = new AbortController();
              const options = await ui.options(session(), controller.signal);
              if (!options?.length) {
                toast(ui.searchLabels?.().empty || "没有可选项", { type: "info" });
                return;
              }
              const picked = await ctx.ask({
                title: typeof contribution.label === "function" ? contribution.label() : `/${name}`,
                options: options.slice(0, 12).map((option) => ({ value: option.id, label: option.badge ? `${option.label}（${option.badge}）` : option.label })),
              });
              const option = options.find((item) => item.id === picked?.value);
              if (option) await ui.onSelect(option, session());
            },
          });
        } catch (error) {
          fail(`命令 /${name} 注册失败：${error?.message || error}`);
          return () => {};
        }
      },
      decorate: () => () => {},
      dismiss() {},
      popupFor: () => undefined,
    };
    // dsh 主题（tokens）→ Codex++ 主题：token 名写成 CSS 变量；切换走 codexPlus.themes。
    const themeService = {
      getTheme: () => ({ preference: "system", fontSize: 14, active: { id: codexPlusThemesService().current || "default", colorScheme: document.documentElement.classList.contains("dark") ? "dark" : "light", tokens: {} }, themes: [], revision: 0 }),
      exportInspectTokens: () => [],
      setTheme: (id) => codexPlusThemesService().apply(String(id)),
      setFontSize() {},
      register(definition) {
        if (!definition?.id) return () => {};
        const variables = {};
        for (const [name, value] of Object.entries(definition.tokens || {})) variables[name.startsWith("--") ? name : `--${name}`] = String(value);
        return ctx.registerTheme({ id: String(definition.id), name: String(definition.id), variables });
      },
      overrideTokens: () => () => {},
    };
    // dsh 的 / 与 @ 菜单在 Codex 里归 Codex 自己：输入触发源注册不进去，记一条说明但不挡插件加载。
    const inputTriggers = new Proxy({}, {
      get: (_target, key) => (key === "then" ? undefined : (...args) => {
        if (key === "register" || key === "registerSource") fail(`输入触发源 ${args[0]?.trigger || ""}${args[0]?.name || ""} 用的是 Codex 自己的 / 或 @ 菜单，Codex++ 不接管，不会出现`);
        return () => {};
      }),
    });

    // ---- 渲染 ----
    class Boundary extends React.Component {
      constructor(props) {
        super(props);
        this.state = { failed: false };
      }
      static getDerivedStateFromError() {
        return { failed: true };
      }
      componentDidCatch(error) {
        fail(`组件出错：${error?.stack || error?.message || error}`);
      }
      render() {
        return this.state.failed ? null : this.props.children;
      }
    }
    const mount = (container, Component, props) => {
      const root = reactDomClient.createRoot(container);
      root.render(React.createElement(Boundary, null, React.createElement(Component, props)));
      // React 不允许在渲染过程中同步卸载另一棵树：挪到微任务里。
      return () => queueMicrotask(() => {
        try { root.unmount(); } catch {}
      });
    };
    const standardProps = (sessionId) => ({
      sessionId,
      useSessions: () => React.useSyncExternalStore(sessionList.subscribe, sessionList.getSnapshot),
      useSession: (selector) => {
        const list = React.useSyncExternalStore(sessionList.subscribe, sessionList.getSnapshot);
        const session = sessionId ? list.byId[sessionId] : undefined;
        return typeof selector === "function" ? selector(session) : session;
      },
      useSessionStatus: () => "idle",
      useWorkspaces: () => React.useSyncExternalStore(workspaceList.subscribe, workspaceList.getSnapshot),
      usePanelInfo: () => ({ visible: true }),
      useProjection: () => undefined,
      renderFactorySlot: () => null,
    });

    let anonymous = 0;
    const register = (options = {}, Component) => {
      const name = String(options?.name || "");
      const target = codexPlusDshSlotMap[name];
      if (!target) {
        fail(`插槽 ${name} 在 Codex 里没有对应位置，这个组件不会显示`);
        return () => {};
      }
      if (typeof Component !== "function" && typeof Component !== "object") {
        fail(`插槽 ${name} 注册的不是 React 组件`);
        return () => {};
      }
      const t = options.locale ? ctx.locale.bind(String(options.locale)) : (key) => key;
      let face = {};
      if (typeof options.inject === "function") {
        try {
          const raw = options.inject() || {};
          const { hooks, ...rest } = raw;
          face = { ...rest };
          for (const [key, hook] of Object.entries(hooks || {})) face[`use${key[0].toUpperCase()}${key.slice(1)}`] = hook;
        } catch (error) {
          fail(`插槽 ${name} 的 inject 出错：${error?.message || error}`);
        }
      }
      const order = Number.isFinite(options.order) ? Number(options.order) : 0;
      const props = (extra) => ({ ...standardProps(extra.sessionId ?? (codexPlusCurrentSessionId() || undefined)), t, ...face, ...extra });
      const label = typeof options.label === "function" ? options.label() : String(options.label || options.id || packageName);
      const id = `dsh-${name}-${options.id || options.key || ++anonymous}`;
      switch (target.kind) {
        case "slot":
          return ctx.registerSlot({
            slot: target.slot,
            order,
            render: ({ container, sessionId, title, turnKey }) => mount(container, Component, props({
              ...(sessionId ? { sessionId, displayTitle: title } : {}),
              ...(turnKey ? { turn: { key: turnKey }, seq: 0, openFile: () => {} } : {}),
              wide: true,
            })),
          }, { id });
        case "rowAction":
          return ctx.registerRowAction({
            label,
            order,
            render: ({ container, session_id, title, close }) => mount(container, Component, props({
              sessionId: session_id,
              displayTitle: title,
              useMenuOpenState: () => [true, (open) => {
                if (!open) close();
              }],
            })),
          }, { id });
        case "message":
          return ctx.registerMessageDecoration({
            roles: ["assistant"],
            order,
            render: ({ message, container }) => mount(container, Component, props({ messageId: message.id, sessionId: message.conversationId || undefined })),
          }, { id });
        case "toolView":
          // keyed 插槽按工具名分派（key 就是工具名）；计划评审按条目类型。
          return ctx.registerToolView({
            ...(options.key ? { tools: [String(options.key)] } : {}),
            ...(target.kinds ? { kinds: target.kinds } : {}),
            order,
            render: ({ container, call }) => mount(container, Component, props({
              sessionId: call.sessionId || undefined,
              callId: call.callId,
              toolName: call.name,
              phase: "result",
              block: { type: "tool-call", id: call.callId, name: call.name, arguments: JSON.stringify(call.input ?? {}), result: call.output },
              useDisclosure: () => [true, () => {}],
              openFile: () => {},
            })),
          }, { id });
        case "settings":
          return ctx.registerSettingsSection({
            title: label,
            render: ({ container, close }) => mount(container, Component, props({ close: close || (() => {}) })),
          }, { id });
        case "sidePanel":
          return ctx.registerSidePanel({
            title: label,
            render: ({ container, close }) => mount(container, Component, props({ close })),
          }, { id });
        case "floating":
          return ctx.registerFloatingTab({
            title: label,
            render: ({ container, close }) => mount(container, Component, props({ dismiss: close, tab: { id } })),
          }, { id });
        case "page":
          return ctx.registerPage({
            title: label,
            render: ({ container, close }) => mount(container, Component, props({ close: close || (() => {}) })),
          }, { id, navLabel: label });
        default:
          return () => {};
      }
    };
    const slots = {
      register,
      // Codex++ 的插槽一开始就都在：立刻执行，回调返回的撤销函数随拓展卸载。
      inject(key, callback) {
        let result;
        try {
          result = callback();
        } catch (error) {
          fail(`插槽 ${key} 的注册出错：${error?.message || error}`);
          return () => {};
        }
        const disposers = typeof result === "function" ? [result] : result && typeof result[Symbol.iterator] === "function" ? [...result] : [];
        return ctx.effect(() => () => disposers.forEach((dispose) => {
          try { dispose?.(); } catch {}
        }));
      },
      entries: () => [],
      entriesOfSlot: () => [],
      spec: (key) => (codexPlusDshSlotMap[key] ? { key } : undefined),
      snapshot: () => ({}),
      onEntryError: () => () => {},
      subscribe: () => () => {},
      registerFactory: () => () => {},
    };
    const services = {
      slots, locale: localeService, sessions: clientSessions, workspaces: { list: workspaceList }, uiWorkspace,
      shortcuts: shortcutsService, commandUi, theme: themeService, inputTriggers,
    };
    // Cordis 的 ready / dispose 只对本插件包：ready 在全部 apply 跑完后发，dispose 随拓展卸载。
    const readyHandlers = [];
    let ready = false;
    const clientCtx = new Proxy({
      get: (name) => services[name],
      effect(callback) {
        const result = callback();
        if (typeof result === "function") return ctx.effect(() => result);
        if (result && typeof result.next === "function") {
          // generator effect：yield 出来的撤销函数随拓展卸载。
          const disposers = [];
          for (let step = result.next(); !step.done; step = result.next()) if (typeof step.value === "function") disposers.push(step.value);
          return ctx.effect(() => () => disposers.reverse().forEach((dispose) => {
            try { dispose(); } catch {}
          }));
        }
        return () => {};
      },
      on(event, handler) {
        if (event === "ready") {
          if (ready) queueMicrotask(() => handler());
          else readyHandlers.push(handler);
          return () => {
            const index = readyHandlers.indexOf(handler);
            if (index >= 0) readyHandlers.splice(index, 1);
          };
        }
        if (event === "dispose") return ctx.effect(() => handler);
        return ctx.on(String(event), handler);
      },
      emit: (event, ...args) => ctx.emit(String(event), ...args),
      // 可选依赖的激活写法：依赖都在才调用；Codex 里没有的服务不调用，也不算错（Cordis 同样只是不激活）。
      inject(names, callback) {
        const list = Array.isArray(names) ? names : Object.keys(names || {});
        const missing = list.filter((item) => !(item in services));
        if (missing.length) {
          console.info(`[dsh ${packageName}] ctx.inject 的 ${missing.join("、")} 在 Codex 里没有，这一段不激活`);
          return () => {};
        }
        return clientCtx.effect(() => callback(clientCtx));
      },
      setTimeout: (handler, delay) => ctx.setTimeout(handler, delay),
      setInterval: (handler, delay) => ctx.setInterval(handler, delay),
      logger: Object.assign((scope) => console.info.bind(console, `[dsh ${packageName}:${scope}]`), {
        info: console.info.bind(console, `[dsh ${packageName}]`),
        warn: console.warn.bind(console, `[dsh ${packageName}]`),
        error: console.error.bind(console, `[dsh ${packageName}]`),
        debug: () => {},
      }),
    }, {
      get(target, key) {
        if (key in target) return target[key];
        if (typeof key === "string" && key in services) return services[key];
        return undefined;
      },
    });

    const builtinModules = {
      react: React,
      "react/jsx-runtime": libs.jsxRuntime,
      "react/jsx-dev-runtime": libs.jsxRuntime,
      "react-dom": libs.reactDom,
      "react-dom/client": reactDomClient,
    };
    const require = (name) => {
      if (name in builtinModules && builtinModules[name]) return builtinModules[name];
      throw new Error(`dsh 前端插件 require 了 ${name}，Codex++ 只提供 React 与 ReactDOM`);
    };
    for (const entry of Array.isArray(modules) ? modules : []) {
      if (ctx.disposed) return;
      let plugin;
      try {
        const exported = entry?.factory?.(require);
        plugin = exported?.default && typeof exported.default.apply === "function" ? exported.default : exported;
      } catch (error) {
        fail(`加载 ${entry?.id || packageName} 失败：${error?.message || error}`);
        continue;
      }
      const apply = typeof plugin === "function" ? plugin : plugin?.apply;
      if (typeof apply !== "function") {
        fail(`${entry?.id || packageName} 没有导出 apply`);
        continue;
      }
      // inject 的对象写法里 { name: { required: false } } 是可选依赖：缺了照样加载。
      const inject = Array.isArray(plugin?.inject)
        ? plugin.inject
        : Object.entries(plugin?.inject || {}).filter(([, spec]) => spec?.required !== false).map(([name]) => name);
      const missing = inject.filter((name) => !(name in services));
      if (missing.length) {
        fail(`${entry?.id || packageName} 依赖 ${missing.join("、")}，Codex 里没有这些前端服务，不加载`);
        continue;
      }
      try {
        await apply(clientCtx, options.config ?? {});
      } catch (error) {
        fail(`${entry?.id || packageName} 的 apply 出错：${error?.stack || error?.message || error}`);
      }
    }
    if (ctx.disposed) return;
    ready = true;
    for (const handler of readyHandlers.splice(0)) {
      try {
        await handler();
      } catch (error) {
        fail(`ready 监听器出错：${error?.message || error}`);
      }
    }
  }
