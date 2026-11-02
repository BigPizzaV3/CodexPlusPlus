  /**
   * 更多视图接口：悬浮球页签、工具调用视图、Codex 设置页分区。
   *
   * - registerFloatingTab：右下角悬浮球面板里加一个页签（悬浮球的「下一步建议」或「回答大纲」至少开着一个时可见）。
   *   悬浮球是单独注入的脚本，这里把注册项同步到 window.__codexPlusFloatingTabs，再请它重画。
   * - registerToolView：对话里的工具调用 / 命令条目末尾挂一块自己的内容（在 activity.after 插槽上），
   *   按工具名或条目类型匹配；render 收到的 call 带工具名、参数、结果（从会话日志里按调用 id 找）。
   * - registerSettingsSection：Codex 设置页左侧导航末尾加一项，点开是拓展自己画的页面。
   * - registerSettingsPanel：「拓展」页面里本拓展详情下方的设置面板，由 40-backend-settings.js 挂载。
   */

  // ---------------------------------------------------------------- 悬浮球页签
  function syncCodexPlusFloatingTabs() {
    const tabs = new Map();
    for (const item of codexPlusExtensionItems(codexPlusRegistry.floatingTabs)) {
      tabs.set(item.id, {
        id: item.id,
        title: String(item.title || item.label || item.id),
        icon: item.icon,
        scriptKey: item.scriptKey,
        render: (context) => {
          const result = runCodexPlusExtensionCallback(item.scriptKey, "floatingTab.render", () => item.render(context));
          return result.ok ? result.value : undefined;
        },
        fail: (error) => codexPlusMarkExtensionFailure(item.scriptKey, `floatingTab: ${String(error?.stack || error?.message || error)}`),
      });
    }
    window.__codexPlusFloatingTabs = tabs;
    try {
      window.__codexStepwisePanel?.renderFloat?.({ preserveMorph: true });
    } catch {}
  }

  function codexPlusRegisterFloatingTab(definition, options = {}) {
    if (typeof definition?.render !== "function") throw new Error("悬浮球页签需要 render({ container, close })");
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const id = `${scriptKey}:floatingTab:${options.id || codexPlusExtensionIdSeed()}`;
    const dispose = registerCodexPlusExtension("floatingTab", codexPlusRegistry.floatingTabs, id, definition, scriptKey);
    syncCodexPlusFloatingTabs();
    return () => {
      dispose();
      syncCodexPlusFloatingTabs();
    };
  }

  // ---------------------------------------------------------------- 工具调用视图
  /** 会话的工具调用表（调用 id → 工具名、参数、结果），读一次缓存 3 秒。 */
  const codexPlusToolCallCache = new Map();

  function codexPlusToolCalls(sessionId) {
    const cached = codexPlusToolCallCache.get(sessionId);
    if (cached && Date.now() - cached.at < 3000) return cached.promise;
    const promise = (async () => {
      const query = codexPlusServices.get("sessionQuery")?.value;
      if (!query || !sessionId) return new Map();
      const { events } = await query.readSession(sessionId);
      const calls = new Map();
      for (const event of events || []) {
        if (event.type === "tool/call") {
          let input = event.data?.input;
          try { input = typeof input === "string" ? JSON.parse(input) : input; } catch {}
          calls.set(event.data?.callId, { callId: event.data?.callId, name: event.data?.name || "", input, output: null });
        } else if (event.type === "tool/result" && calls.has(event.data?.callId)) {
          calls.get(event.data.callId).output = event.data?.output ?? null;
        }
      }
      return calls;
    })();
    codexPlusToolCallCache.set(sessionId, { at: Date.now(), promise });
    if (codexPlusToolCallCache.size > 8) codexPlusToolCallCache.delete(codexPlusToolCallCache.keys().next().value);
    promise.catch(() => codexPlusToolCallCache.delete(sessionId));
    return promise;
  }

  /** 条目 id 里带着调用 id（命令条目是 exec-<调用 id>）：按包含关系找。 */
  function codexPlusFindToolCall(calls, itemIds) {
    for (const itemId of itemIds || []) {
      if (calls.has(itemId)) return calls.get(itemId);
      for (const [callId, call] of calls) if (callId && itemId.includes(callId)) return call;
    }
    return null;
  }

  function codexPlusToolViewMatches(definition, call) {
    const kinds = definition.kinds ? [].concat(definition.kinds) : null;
    if (kinds && !kinds.includes(call.kind)) return false;
    const tools = definition.tools ? [].concat(definition.tools) : null;
    if (!tools) return true;
    return tools.some((pattern) => pattern instanceof RegExp ? pattern.test(call.name) : String(pattern) === call.name || new RegExp(`^(?:${pattern})$`).test(call.name));
  }

  function codexPlusRegisterToolView(definition, options = {}) {
    if (typeof definition?.render !== "function") throw new Error("工具调用视图需要 render({ container, call })");
    return codexPlusRegisterExtensionSlot({
      slot: "activity.after",
      order: definition.order,
      render: ({ container, kind, itemIds, text }) => {
        let cleanup = null;
        let cancelled = false;
        const sessionId = codexPlusCurrentSessionId();
        const base = { kind, itemIds, text, sessionId, callId: null, name: "", input: null, output: null };
        const draw = (call) => {
          if (cancelled || !codexPlusToolViewMatches(definition, call)) return;
          const result = runCodexPlusExtensionCallback(options.scriptKey || codexPlusCurrentExtensionScriptKey(options), "toolView.render", () => definition.render({ container, call }));
          if (result.ok && typeof result.value === "function") cleanup = result.value;
        };
        if (!definition.tools && !definition.resolve) draw(base);
        else {
          codexPlusToolCalls(sessionId).then((calls) => {
            const found = codexPlusFindToolCall(calls, itemIds);
            draw(found ? { ...base, ...found, kind } : base);
          }).catch(() => draw(base));
        }
        return () => {
          cancelled = true;
          if (typeof cleanup === "function") cleanup();
        };
      },
    }, options);
  }

  // ---------------------------------------------------------------- Codex 设置页分区
  function codexPlusRegisterSettingsSection(definition, options = {}) {
    if (typeof definition?.render !== "function") throw new Error("设置分区需要 render({ container, close })");
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const id = options.id || codexPlusExtensionIdSeed();
    const disposePage = codexPlusRegisterExtensionPage(definition, { ...options, scriptKey, id: `settings-${id}`, nav: false });
    const pageId = `${scriptKey}:page:settings-${id}`;
    const title = String(definition.title || id);
    const disposeSlot = codexPlusRegisterExtensionSlot({
      slot: "settings.nav",
      order: definition.order,
      render: ({ container, sampleClass }) => {
        const button = document.createElement("button");
        button.type = "button";
        button.className = `${sampleClass || ""} codex-plus-settings-nav-item`.trim();
        button.innerHTML = `${codexPlusExtensionIconMarkup(definition)}<span>${escapeHtml(title)}</span>`;
        button.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          openCodexPlusExtensionPage(pageId);
        });
        container.appendChild(button);
      },
    }, { scriptKey, id: `settings-nav-${id}` });
    return () => {
      disposeSlot();
      disposePage();
    };
  }

  // ---------------------------------------------------------------- 拓展详情页的设置面板
  /**
   * 每个拓展一个设置面板，登记在注册表里（持久）；面板 DOM 由「拓展」详情页每次打开时向拓展要（瞬态）。
   * 注册 / 撤销时通知详情页：正显示着这个拓展时重画，热重载后看到的是新代码画的面板。
   */
  function codexPlusRegisterSettingsPanel(definition, options = {}) {
    if (typeof definition?.render !== "function") throw new Error("设置面板需要 render({ container, config, saveConfig })");
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const dispose = registerCodexPlusExtension("settingsPanel", codexPlusRegistry.settingsPanels, `${scriptKey}:settingsPanel`, definition, scriptKey);
    // 详情页宿主在 40-backend-settings.js；单独切出这一段测试时没有它。
    const changed = () => {
      if (typeof codexPlusSettingsPanelChanged === "function") codexPlusSettingsPanelChanged(scriptKey);
    };
    changed();
    return () => {
      dispose();
      changed();
    };
  }

  function codexPlusSettingsPanelFor(scriptKey) {
    return [...codexPlusRegistry.settingsPanels.values()].find((item) => item.scriptKey === scriptKey) || null;
  }
