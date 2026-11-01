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

  /**
   * dsh 绑定的规范形（同 dsh 的 normalizeBinding）：primary 在 macOS 是 meta、别处是 control，修饰键去重排序；
   * secondCode 是与 code 同时按住的第二个键（双键组合，不分先后）。认不出的键返回 null。
   */
  function codexPlusDshNormalizeBinding(binding, mac) {
    if (!binding || typeof binding.code !== "string") return null;
    const valid = (code) => /^(Key[A-Z]|Digit[0-9]|F([1-9]|1[0-9]|2[0-4]))$/.test(code)
      || ["Slash", "Comma", "Period", "Backslash", "Backquote", "Minus", "Equal", "BracketLeft", "BracketRight", "Semicolon", "Quote", "Enter", "Escape", "Space", "Tab", "Backspace", "Delete", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(code);
    const codes = [binding.code, ...(typeof binding.secondCode === "string" ? [binding.secondCode] : [])].sort();
    if (!codes.every(valid) || (codes.length === 2 && codes[0] === codes[1])) return null;
    const modifiers = new Set((Array.isArray(binding.modifiers) ? binding.modifiers : []).map((item) => (item === "primary" ? (mac ? "meta" : "control") : item)));
    return { code: codes[0], ...(codes[1] ? { secondCode: codes[1] } : {}), modifiers: ["control", "alt", "shift", "meta"].filter((item) => modifiers.has(item)) };
  }

  /** 规范绑定的键帽文字（双键组合两个键挨着写）。 */
  function codexPlusDshBindingKeys(normalized, mac) {
    if (!normalized) return [];
    const names = { Slash: "/", Comma: ",", Period: ".", Backslash: "\\", Backquote: "`", Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]", Semicolon: ";", Quote: "'", Escape: "Esc", ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→" };
    const key = (code) => names[code] ?? code.replace(/^(Key|Digit)/, "");
    const symbols = mac ? { control: "⌃", alt: "⌥", shift: "⇧", meta: "⌘" } : { control: "Ctrl", alt: "Alt", shift: "Shift", meta: "Meta" };
    return [...normalized.modifiers.map((item) => symbols[item]), key(normalized.code), ...(normalized.secondCode ? [key(normalized.secondCode)] : [])];
  }

  /**
   * 所有 dsh 前端插件的插槽登记（跨包共享）。对得上 Codex++ 位置的插槽直接渲染进去；对不上的作为「子插槽」，
   * 由别的 dsh 组件经 renderSlot / renderSlotChain 渲染（dsh 的复合组件就是这样拼起来的）；factory 经 renderFactorySlot。
   */
  const codexPlusDshSlotEntries = new Map();
  const codexPlusDshFactories = new Map();
  const codexPlusDshSlotListeners = new Set();
  /** 被 renderSlot / renderSlotChain 请求过的插槽名：对不上 Codex++ 位置、也没人渲染的才算「不会显示」。 */
  const codexPlusDshRequestedSlots = new Set();
  let codexPlusDshSlotRevision = 0;

  function codexPlusDshSlotsChanged() {
    codexPlusDshSlotRevision += 1;
    for (const listener of [...codexPlusDshSlotListeners]) {
      try { listener(); } catch {}
    }
  }

  function codexPlusDshAddSlotEntry(entry) {
    if (!codexPlusDshSlotEntries.has(entry.name)) codexPlusDshSlotEntries.set(entry.name, new Set());
    codexPlusDshSlotEntries.get(entry.name).add(entry);
    codexPlusDshSlotsChanged();
    return () => {
      codexPlusDshSlotEntries.get(entry.name)?.delete(entry);
      codexPlusDshSlotsChanged();
    };
  }

  function codexPlusDshEntriesOf(name) {
    return [...(codexPlusDshSlotEntries.get(name) ?? [])].sort((left, right) => left.order - right.order);
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
    // 当前会话：轮询 Codex 当前打开的会话 id（切会话没有事件可听），变了才通知订阅者。
    const selection = codexPlusDshSnapshotStore({ sessionId: codexPlusCurrentSessionId() || undefined }, (set) => {
      let last = codexPlusCurrentSessionId() || undefined;
      const timer = setInterval(() => {
        const next = codexPlusCurrentSessionId() || undefined;
        if (next !== last) {
          last = next;
          set({ sessionId: next });
        }
      }, 800);
      return () => clearInterval(timer);
    });
    const uiWorkspace = { selection, clearMain() {} };
    // 会话运行状态：拓展内核的 agent/status（running / idle / aborted）按会话记下来，给 useSessionStatus。
    // 有组件订阅时才听（agent/status 会让内核起会话轮询）。
    const statuses = codexPlusDshSnapshotStore({}, (set) => ctx.on("agent/status", (payload) => {
      const id = payload?.sessionId ?? payload?.agent?.session?.id;
      if (!id) return;
      set({ ...statuses.getSnapshot(), [id]: payload.status === "running" ? "running" : "idle" });
    }));
    // 前端半边连插件宿主半边：rpc.call 经配套拓展包的后台转给插件经 ctx.connection.rpc 注册的处理器。
    const connectionState = codexPlusDshSnapshotStore({ kind: "connected" });
    const connection = {
      hostname: globalThis.location?.hostname ?? "localhost",
      isLoopback: true,
      generation: codexPlusDshSnapshotStore({ id: 1 }),
      state: connectionState,
      rpc: {
        async call(channel, endpoint, payload) {
          try {
            const result = await ctx.backend.call("rpc", { channel, endpoint, payload }, { timeout: 120000 });
            if (result?.ok && Array.isArray(result.attachments)) {
              // 附件按 path 放回 value 里（字节经 JSON 往返时是 base64）。
              const value = structuredClone(result.value);
              for (const { path, base64 } of result.attachments) {
                const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
                let cursor = value;
                for (const step of path.slice(0, -1)) cursor = cursor?.[step];
                if (cursor && path.length) cursor[path[path.length - 1]] = bytes;
              }
              return { ok: true, value };
            }
            return result ?? { ok: false, error: { code: "EMPTY", message: "插件没有返回结果", details: {} } };
          } catch (error) {
            return { ok: false, error: { code: "TRANSPORT", message: String(error?.message || error), details: {} } };
          }
        },
        /**
         * 流式调用（如 typert 的 stream 方法）：后台在插件常驻进程里打开流，这里分批拉取；uplink 的值逐个送上去。
         * signal 取消或迭代提前结束时关掉流。
         */
        open(channel, endpoint, payload, signal, uplink) {
          return (async function* () {
            const opened = await ctx.backend.call("rpcOpen", { channel, endpoint, payload }, { timeout: 120000 });
            if (!opened?.ok) {
              const failure = opened?.error || { code: "EMPTY", message: "插件没有打开这个流" };
              throw Object.assign(new Error(failure.message), { code: failure.code, details: failure.details });
            }
            const id = opened.id;
            let closed = false;
            const close = () => {
              if (closed) return;
              closed = true;
              ctx.backend.call("rpcClose", { id }).catch(() => {});
            };
            signal?.addEventListener?.("abort", close, { once: true });
            if (uplink && typeof uplink[Symbol.asyncIterator] === "function") {
              void (async () => {
                for await (const item of uplink) {
                  if (closed) return;
                  await ctx.backend.call("rpcPush", { id, items: [item] });
                }
                if (!closed) await ctx.backend.call("rpcPush", { id, items: [], end: true });
              })().catch(() => {});
            }
            try {
              while (!closed) {
                const batch = await ctx.backend.call("rpcPull", { id, waitMs: 1000 }, { timeout: 30000 });
                for (const item of batch?.items || []) yield item;
                if (batch?.error) throw Object.assign(new Error(batch.error.message), { code: batch.error.code, details: batch.error.details });
                if (batch?.done) {
                  closed = true;
                  return;
                }
              }
            } finally {
              signal?.removeEventListener?.("abort", close);
              close();
            }
          })();
        },
      },
      reconnect() {},
      registerGenerationSource: () => () => {},
      start: () => ({ stop() {} }),
      stop() {},
    };
    let localeSnapshot = { id: ctx.locale.current, revision: 0 };
    const localeService = {
      register(ns, langOrDicts, dict) {
        return ctx.locale.register(String(ns), langOrDicts, dict);
      },
      bind: (ns) => ctx.locale.bind(String(ns)),
      getLocale: () => ({ id: ctx.locale.current }),
      getSnapshot: () => localeSnapshot,
      subscribe: (listener) => ctx.on("locale/change", () => {
        localeSnapshot = { id: ctx.locale.current, revision: localeSnapshot.revision + 1 };
        listener();
      }),
      resolveText: ctx.locale.resolve,
      // 界面语言跟着 Codex 走，插件不能改；记一条说明。
      setLocale(id) {
        if (id && id !== ctx.locale.current) fail(`setLocale(${id}) 不生效：界面语言跟随 Codex 的设置`);
      },
    };

    // dsh 的快捷键命令 → Codex++ 快捷键：取默认绑定（macOS 优先按平台挑），按下时问命令的 resolve。
    // 单键（加修饰键）的绑定交给 Codex++ 的快捷键；双键组合（secondCode：两个键同时按住）与固定序列的输入
    // （registerFixed / observeFixedInput，如 `g g`，插件自己按 stopSequenceMs 判断）由这里的键盘监听处理，同 dsh 的 installKeyboard。
    const mac = Boolean(codexPlusIsMacPlatform);
    const chordCommands = new Map();
    const fixedListeners = new Set();
    const shortcutCatalog = codexPlusDshSnapshotStore([]);
    const fixedCatalog = codexPlusDshSnapshotStore([]);
    const catalogRows = new Map();
    const fixedRows = new Map();
    const refreshCatalog = () => shortcutCatalog.set([...catalogRows.values()]);
    const refreshFixed = () => fixedCatalog.set([...fixedRows.values()]);
    let keyboard = null;
    const shortcutContext = (event) => {
      const path = typeof event?.composedPath === "function" ? event.composedPath() : [];
      const element = path.find((item) => item && typeof item.closest === "function") ?? event?.target ?? document.activeElement ?? null;
      const closest = (selector) => (element && typeof element.closest === "function" ? element.closest(selector) : null);
      const region = closest(".xterm") ? "terminal" : closest("input, textarea, select, [contenteditable=\"true\"], [contenteditable=\"\"]") ? "editable" : "page";
      const dialogs = typeof document.querySelectorAll === "function" ? document.querySelectorAll("[role=dialog][aria-modal=true], dialog[open]") : [];
      const top = dialogs[dialogs.length - 1];
      return { source: "keyboard", region, modal: top ? top.dataset?.shortcutModal ?? "other" : null, target: element };
    };
    const installKeyboard = () => {
      if (keyboard || typeof document.addEventListener !== "function") return;
      const held = new Set();
      const modifierCodes = /^(Shift|Control|Alt|Meta|OS)(Left|Right)?$/;
      const reset = () => {
        for (const listener of [...fixedListeners]) {
          try { listener({ type: "reset" }); } catch {}
        }
      };
      const keydown = (event) => {
        if (!event || typeof event.code !== "string" || modifierCodes.test(event.code)) return;
        const others = [...held].filter((code) => code !== event.code);
        held.add(event.code);
        const composing = Boolean(event.isComposing) || event.key === "Process" || event.key === "Dead";
        const gesture = {
          code: event.code, ...(others.length === 1 ? { secondCode: others[0] } : {}),
          control: Boolean(event.ctrlKey), alt: Boolean(event.altKey), shift: Boolean(event.shiftKey), meta: Boolean(event.metaKey),
          repeat: Boolean(event.repeat), composing, defaultPrevented: Boolean(event.defaultPrevented),
        };
        const context = shortcutContext(event);
        const consume = () => event.preventDefault?.();
        for (const listener of [...fixedListeners]) {
          try { listener({ type: "keydown", gesture, context, consume }); } catch (error) { fail(`固定按键输入的监听器出错：${error?.message || error}`); }
        }
        if (gesture.repeat || composing || !gesture.secondCode) return;
        const pair = [gesture.code, gesture.secondCode].sort().join("+");
        for (const { command, binding } of chordCommands.values()) {
          if ([binding.code, binding.secondCode].join("+") !== pair) continue;
          if (!["control", "alt", "shift", "meta"].every((item) => gesture[item] === binding.modifiers.includes(item))) continue;
          if (Array.isArray(command.regions) && command.regions.length && !command.regions.includes(context.region)) continue;
          if (context.modal !== null && !(Array.isArray(command.modals) && command.modals.includes(context.modal))) continue;
          const resolution = command.resolve?.(context);
          if (resolution?.status === "handled") {
            consume();
            resolution.run();
            return;
          }
        }
      };
      const keyup = (event) => {
        if (event && typeof event.code === "string") held.delete(event.code);
      };
      const blur = () => {
        held.clear();
        reset();
      };
      document.addEventListener("keydown", keydown);
      document.addEventListener("keyup", keyup, true);
      for (const type of ["compositionstart", "compositionend", "focusin", "pointerdown"]) document.addEventListener(type, reset, true);
      globalThis.addEventListener?.("blur", blur);
      keyboard = () => {
        document.removeEventListener?.("keydown", keydown);
        document.removeEventListener?.("keyup", keyup, true);
        for (const type of ["compositionstart", "compositionend", "focusin", "pointerdown"]) document.removeEventListener?.(type, reset, true);
        globalThis.removeEventListener?.("blur", blur);
        keyboard = null;
      };
      ctx.effect(() => () => keyboard?.());
    };
    const shortcutsService = {
      runtime: "web",
      platform: mac ? "macos" : "windows",
      // dsh 的默认值（固定序列两次按键之间的最长间隔，插件按它判断序列是否作废）。
      stopSequenceMs: 500,
      catalog: shortcutCatalog,
      config: codexPlusDshSnapshotStore({}),
      fixedCatalog,
      register(command) {
        const defaults = command?.defaults || {};
        const binding = Object.entries(defaults).find(([profile]) => profile === `web:${mac ? "macos" : "windows"}` || profile.endsWith(`:${mac ? "mac" : "windows"}`) || profile.endsWith(`:${mac ? "macos" : "windows"}`))?.[1] ?? Object.values(defaults)[0];
        const normalized = codexPlusDshNormalizeBinding(binding, mac);
        const id = String(command?.id || "");
        const label = typeof command?.label === "function" ? command.label() : id;
        catalogRows.set(id, { id, label, aliases: command?.aliases ?? [], keys: codexPlusDshBindingKeys(normalized, mac), aria: normalized && !normalized.secondCode ? codexPlusDshShortcutKey(binding) : undefined, binding: normalized, modified: false, conflicts: [], issue: normalized ? null : "unsupported-key" });
        refreshCatalog();
        const removeRow = () => {
          catalogRows.delete(id);
          refreshCatalog();
        };
        if (normalized?.secondCode) {
          // 双键组合：Codex++ 的快捷键只认一个键，由这里的键盘监听匹配。
          const entry = { command, binding: normalized };
          chordCommands.set(id, entry);
          installKeyboard();
          return () => {
            if (chordCommands.get(id) === entry) chordCommands.delete(id);
            removeRow();
          };
        }
        const key = codexPlusDshShortcutKey(binding);
        if (!key) {
          fail(`快捷键命令 ${id || "?"} 的默认绑定 Codex 里表达不了，没有注册`);
          return removeRow;
        }
        try {
          const dispose = ctx.registerShortcut({
            key,
            description: label,
            run: ({ event }) => {
              const resolution = command.resolve?.({ source: "keyboard", region: "page", modal: null, target: event?.target ?? null });
              if (resolution?.status === "handled") resolution.run();
            },
          });
          return () => {
            dispose?.();
            removeRow();
          };
        } catch (error) {
          fail(`快捷键 ${key} 注册失败：${error?.message || error}`);
          return removeRow;
        }
      },
      // 固定按键动作（含序列的每一步）：只进目录给界面展示；真正的输入由插件经 observeFixedInput 自己判断。
      registerFixed(command) {
        const id = String(command?.id || "");
        const bindings = (Array.isArray(command?.bindings) ? command.bindings : []).map((binding) => codexPlusDshNormalizeBinding(binding, mac)).filter(Boolean);
        fixedRows.set(id, { id, label: typeof command?.label === "function" ? command.label() : id, keys: Array.isArray(command?.keys) ? command.keys : [], bindings, group: command?.group ?? "application" });
        refreshFixed();
        return () => {
          fixedRows.delete(id);
          refreshFixed();
        };
      },
      observeFixedInput(listener) {
        if (typeof listener !== "function") return () => {};
        fixedListeners.add(listener);
        installKeyboard();
        return () => fixedListeners.delete(listener);
      },
      describeBinding: (binding) => {
        const normalized = codexPlusDshNormalizeBinding(binding, mac);
        return { keys: codexPlusDshBindingKeys(normalized, mac), aria: normalized && !normalized.secondCode ? codexPlusDshShortcutKey(binding) : undefined };
      },
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
    // 子插槽与 factory 的渲染：订阅跨包登记表，登记变化时重画；每个条目各自的错误边界。
    const useSlotRevision = () => React.useSyncExternalStore((listener) => {
      codexPlusDshSlotListeners.add(listener);
      return () => codexPlusDshSlotListeners.delete(listener);
    }, () => codexPlusDshSlotRevision);
    const SlotOutlet = ({ name, owner, opts, chain }) => {
      useSlotRevision();
      codexPlusDshRequestedSlots.add(name);
      let entries = codexPlusDshEntriesOf(name);
      if (opts?.entryKey !== undefined) entries = entries.filter((entry) => entry.key === String(opts.entryKey));
      if (opts?.only !== undefined) entries = entries.filter((entry) => entry.id === String(opts.only) || entry.key === String(opts.only));
      if (chain) {
        for (const entry of entries) {
          let matched = null;
          try {
            matched = typeof entry.select === "function" ? entry.select(owner) : owner;
          } catch (error) {
            entry.fail(`插槽 ${name} 的 select 出错：${error?.message || error}`);
          }
          if (matched !== null && matched !== undefined) return entry.element({ ...owner, matched });
        }
        return opts?.fallback ?? null;
      }
      if (!entries.length) return opts?.fallback ?? null;
      return React.createElement(React.Fragment, null, ...entries.map((entry) => entry.element({ ...owner }, entry.id)));
    };
    const renderSlot = (name, owner = {}, opts = {}) => React.createElement(SlotOutlet, { name: String(name), owner, opts });
    const renderSlotChain = (name, owner = {}, opts = {}) => React.createElement(SlotOutlet, { name: String(name), owner, opts, chain: true });
    const renderFactorySlot = (name, props = {}, options = {}) => {
      const FactoryOutlet = () => {
        useSlotRevision();
        const factory = codexPlusDshFactories.get(String(name));
        if (!factory) return options.fallback ?? null;
        return factory.element({
          ...props,
          useFactorySlot: (slot, fallback) => options.slots?.[slot] ?? factory.localSlots?.[slot] ?? fallback,
        });
      };
      return React.createElement(FactoryOutlet);
    };
    const SessionProvider = ({ children }) => children ?? null;
    const standardProps = (sessionId) => ({
      renderSlot,
      renderSlotChain,
      SessionProvider,
      sessionId,
      useSessions: () => React.useSyncExternalStore(sessionList.subscribe, sessionList.getSnapshot),
      useSession: (selector) => {
        const list = React.useSyncExternalStore(sessionList.subscribe, sessionList.getSnapshot);
        const session = sessionId ? list.byId[sessionId] : undefined;
        return typeof selector === "function" ? selector(session) : session;
      },
      useSessionStatus: (id) => {
        const map = React.useSyncExternalStore(statuses.subscribe, statuses.getSnapshot);
        return map[id ?? sessionId] ?? "idle";
      },
      useWorkspaces: () => React.useSyncExternalStore(workspaceList.subscribe, workspaceList.getSnapshot),
      usePanelInfo: () => ({ visible: true }),
      useProjection: () => undefined,
      renderFactorySlot,
    });
    // 本包登记的、对不上 Codex++ 位置的插槽：全部 apply 后过一会儿还没有任何组件渲染它，才记「不会显示」。
    const unmappedNames = new Set();

    let anonymous = 0;
    const register = (options = {}, Component) => {
      const name = String(options?.name || "");
      const target = codexPlusDshSlotMap[name];
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
      // 先登记进跨包登记表：父组件的 renderSlot 能渲染它；对得上 Codex++ 位置的再挂到那里。
      const entry = {
        id,
        name,
        key: options.key !== undefined ? String(options.key) : undefined,
        order,
        select: options.select,
        fail,
        element: (extra, key) => React.createElement(Boundary, { key }, React.createElement(Component, props(extra))),
      };
      const unregister = ctx.effect(() => codexPlusDshAddSlotEntry(entry));
      if (!target) {
        unmappedNames.add(name);
        return unregister;
      }
      const dispose = mapToCodex(target, { name, Component, options, order, props, label, id });
      return () => {
        unregister();
        dispose?.();
      };
    };
    const mapToCodex = (target, { name, Component, options, order, props, label, id }) => {
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
      entries: () => [...codexPlusDshSlotEntries.values()].flatMap((set) => [...set]).map(({ id, name, key, order }) => ({ id, name, key, order })),
      entriesOfSlot: (key) => codexPlusDshEntriesOf(String(key)).map(({ id, name, key: entryKey, order }) => ({ id, name, key: entryKey, order })),
      spec: (key) => (codexPlusDshSlotMap[key] || codexPlusDshSlotEntries.has(String(key)) ? { key } : undefined),
      snapshot: () => ({ revision: codexPlusDshSlotRevision }),
      onEntryError: () => () => {},
      subscribe(listener) {
        codexPlusDshSlotListeners.add(listener);
        return ctx.effect(() => () => codexPlusDshSlotListeners.delete(listener));
      },
      // factory：别的插件经 renderFactorySlot(name, props, { slots }) 渲染，useFactorySlot 取局部子插槽。
      registerFactory(options = {}, Component) {
        const name = String(options?.name || "");
        if (!name || (typeof Component !== "function" && typeof Component !== "object")) return () => {};
        let face = {};
        try {
          const raw = typeof options.inject === "function" ? options.inject() || {} : {};
          const { hooks, ...rest } = raw;
          face = { ...rest };
          for (const [key, hook] of Object.entries(hooks || {})) face[`use${key[0].toUpperCase()}${key.slice(1)}`] = hook;
        } catch (error) {
          fail(`factory ${name} 的 inject 出错：${error?.message || error}`);
        }
        const t = options.locale ? ctx.locale.bind(String(options.locale)) : (key) => key;
        let store;
        try {
          store = typeof options.store === "function" ? options.store() : options.store;
        } catch {}
        const factory = {
          localSlots: options.slots && typeof options.slots === "object" ? {} : undefined,
          element: (extra) => React.createElement(Boundary, null, React.createElement(Component, {
            ...standardProps(codexPlusCurrentSessionId() || undefined), t, store, ...face, ...extra,
          })),
        };
        codexPlusDshFactories.set(name, factory);
        codexPlusDshSlotsChanged();
        return ctx.effect(() => () => {
          if (codexPlusDshFactories.get(name) === factory) codexPlusDshFactories.delete(name);
          codexPlusDshSlotsChanged();
        });
      },
    };
    // typert 的前端半边（dsh 的 ctx.remote）：ctx.remote.<namespace>.<method>(...) 经 connection.rpc 的 /api 通道调宿主半边
    // 登记的类型化方法；stream 方法返回可迭代的句柄（send / end / dispose）。描述清单向宿主要一次，$mount 可以再补。
    const remoteNamespaces = {};
    const remoteFailure = (result) => Object.assign(new Error(result?.error?.message || "调用失败"), { code: result?.error?.code, details: result?.error?.details });
    const remotePayload = (descriptor, args) => {
      if (!descriptor?.parameters) return { positional: args };
      const named = {};
      descriptor.parameters.forEach((parameter, index) => {
        if (args[index] !== undefined || !parameter.optional) named[parameter.wire] = args[index];
      });
      return { args: named };
    };
    const remoteMethod = (namespace, method, descriptor) => {
      const endpoint = `${namespace}/${method}`;
      if (descriptor?.mode !== "stream") {
        return async (...args) => {
          const result = await connection.rpc.call("/api", endpoint, remotePayload(descriptor, args));
          if (!result?.ok) throw remoteFailure(result);
          return result.value;
        };
      }
      return (...args) => {
        const controller = new AbortController();
        const queue = [];
        let wake = null;
        let ended = false;
        const resume = () => {
          const next = wake;
          wake = null;
          next?.();
        };
        const uplink = descriptor.uplink ? (async function* () {
          for (;;) {
            if (queue.length) {
              yield queue.shift();
              continue;
            }
            if (ended || controller.signal.aborted) return;
            await new Promise((done) => { wake = done; });
          }
        })() : undefined;
        const items = connection.rpc.open("/api", endpoint, remotePayload(descriptor, args), controller.signal, uplink);
        return {
          [Symbol.asyncIterator]: () => items[Symbol.asyncIterator](),
          send(item) {
            queue.push(item);
            resume();
          },
          end() {
            ended = true;
            resume();
          },
          dispose() {
            controller.abort();
            ended = true;
            resume();
          },
        };
      };
    };
    const addRemoteDescriptor = (descriptor) => {
      if (!descriptor?.namespace || !descriptor?.method) return;
      const namespace = (remoteNamespaces[descriptor.namespace] ||= {});
      namespace[descriptor.method] = remoteMethod(descriptor.namespace, descriptor.method, {
        mode: descriptor.mode,
        uplink: Boolean(descriptor.uplink),
        parameters: (descriptor.parameters || []).map((parameter) => ({ wire: parameter.wire, optional: Boolean(parameter.optional || parameter.acceptsUndefined) })),
      });
      services[`remote.${descriptor.namespace}`] = namespace;
    };
    const remote = new Proxy({
      async $mount(contribution) {
        for (const descriptor of contribution?.descriptors || []) addRemoteDescriptor(descriptor);
        return async () => {};
      },
      // 宿主转发的事件：对上 Codex++ 拓展内核的同名事件（agent/status 等）。
      $on: (event, listener) => ctx.on(String(event), listener),
    }, {
      get(target, key) {
        if (key in target) return target[key];
        if (typeof key !== "string" || key === "then") return undefined;
        // 没有描述的 namespace 也能调：按位置传参，由宿主半边按它登记的描述对上参数。
        return remoteNamespaces[key] || new Proxy({}, {
          get: (_inner, method) => (typeof method === "string" && method !== "then" ? remoteMethod(key, method) : undefined),
        });
      },
    });

    const services = {
      slots, locale: localeService, sessions: clientSessions, workspaces: { list: workspaceList }, uiWorkspace,
      shortcuts: shortcutsService, commandUi, theme: themeService, inputTriggers, connection, remote,
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
    // 先求出各模块的插件对象：要用 remote 的话先向宿主半边要一次 typert 描述，remote.<namespace> 才能被 inject。
    const loaded = [];
    for (const entry of Array.isArray(modules) ? modules : []) {
      try {
        const exported = entry?.factory?.(require);
        loaded.push({ entry, plugin: exported?.default && typeof exported.default.apply === "function" ? exported.default : exported });
      } catch (error) {
        fail(`加载 ${entry?.id || packageName} 失败：${error?.message || error}`);
      }
    }
    const injectNames = (plugin) => (Array.isArray(plugin?.inject) ? plugin.inject : Object.keys(plugin?.inject || {})).map(String);
    if (loaded.some(({ plugin }) => injectNames(plugin).some((name) => name === "remote" || name.startsWith("remote.")))) {
      try {
        const listed = await connection.rpc.call("/api", "$codexpp/typert-descriptors", null);
        if (listed?.ok) for (const descriptor of listed.value || []) addRemoteDescriptor(descriptor);
      } catch {}
    }
    for (const { entry, plugin } of loaded) {
      if (ctx.disposed) return;
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
    // 对不上 Codex++ 位置的插槽：给页面一点时间画出父组件，仍没人渲染才记一条说明。
    if (unmappedNames.size) {
      ctx.setTimeout(() => {
        const orphaned = [...unmappedNames].filter((name) => !codexPlusDshRequestedSlots.has(name));
        if (orphaned.length) fail(`插槽 ${orphaned.join("、")} 在 Codex 里没有对应位置，也没有别的 dsh 组件渲染它，不会显示`);
      }, 3000);
    }
    for (const handler of readyHandlers.splice(0)) {
      try {
        await handler();
      } catch (error) {
        fail(`ready 监听器出错：${error?.message || error}`);
      }
    }
  }
