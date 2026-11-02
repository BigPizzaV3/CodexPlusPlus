  /**
   * 消息插槽与侧边面板。
   *
   * 拓展用 registerMessageAction 往每条对话消息的操作行里加按钮，用 registerMessageDecoration
   * 在消息正文下方挂一块自己的内容，用 registerSidePanel 在顶栏加一个开关、在右侧开一个抽屉。
   *
   * 消息节点由 Codex 虚拟化渲染、随滚动与流式输出反复重建，所以这里不缓存任何消息 DOM：每次
   * 刷新都按选择器重新找消息，缺了就补。普通扫描刻意不理会对话内容的变化（流式输出时会扫爆），
   * 所以只在有消息插槽注册时，单独给对话滚动区挂一个去抖观察器；注册项清空时摘掉。
   */
  const codexPlusMessageSlotAttribute = "data-codex-plus-message-slot";
  const codexPlusMessageSlotRefreshDelayMs = 300;

  /** 当前页面上的对话消息：助手消息是带标注目标的消息组，用户消息是带锚点的消息单元。 */
  function codexPlusMessageNodes() {
    const result = [];
    document.querySelectorAll("[data-response-annotation-target]").forEach((node) => {
      if (isExtensionUiNode(node) || !node.querySelector(':scope > [data-conversation-role="assistant"]')) return;
      result.push({ role: "assistant", node });
    });
    document.querySelectorAll("[data-local-conversation-user-anchor]").forEach((node) => {
      if (!isExtensionUiNode(node)) result.push({ role: "user", node });
    });
    return result;
  }

  /** 交给拓展的消息信息。每次现取，不要缓存：同一条消息的文字在流式输出时会变。 */
  function codexPlusMessageInfo(role, node) {
    const unitKey = (node.closest("[data-content-search-unit-key]") || node).getAttribute("data-content-search-unit-key") || "";
    const body = role === "assistant"
      ? node.querySelector('[data-markdown-text-style="assistant-message"]') || node.querySelector('[class*="MarkdownRoot"]')
      : node.querySelector("[data-user-message-bubble]");
    const conversationId = node.getAttribute("data-response-annotation-conversation")
      || node.closest(".thread-scroll-container")?.querySelector("[data-response-annotation-conversation]")?.getAttribute("data-response-annotation-conversation")
      || "";
    return {
      role,
      id: (role === "assistant" && node.getAttribute("data-response-annotation-target")) || unitKey,
      turnId: unitKey.split(":")[0] || "",
      conversationId,
      text: String(body?.textContent || "").trim(),
      element: node,
    };
  }

  /** 一条消息上插槽的挂载位置：操作按钮进原生操作行，装饰放在正文与操作行之间。 */
  function codexPlusMessageSlotHosts(role, node) {
    if (role === "assistant") {
      const actionRow = node.querySelector(":scope > .turn-action-controls");
      return { decorationParent: node, decorationBefore: actionRow, actionParent: actionRow?.firstElementChild || null };
    }
    const group = node.querySelector('[class*="group/user-message"]');
    const bubble = group?.querySelector(":scope > [data-user-message-bubble]");
    const actionRow = group?.querySelector(":scope > .flex-row-reverse");
    return {
      decorationParent: group || null,
      decorationBefore: bubble?.nextElementSibling || null,
      actionParent: actionRow?.querySelector(":scope > .isolate") || actionRow || null,
    };
  }

  function codexPlusMessageItemApplies(item, info) {
    const roles = typeof item.roles === "string" ? [item.roles] : item.roles;
    if (Array.isArray(roles) && roles.length && !roles.includes(info.role)) return false;
    if (typeof item.visible !== "function") return true;
    const result = runCodexPlusExtensionCallback(item.scriptKey, "message.visible", () => item.visible(info));
    return result.ok && result.value !== false;
  }

  function codexPlusMessageSlotChild(parent, slot) {
    return [...parent.children].find((child) => child.getAttribute?.(codexPlusMessageSlotAttribute) === slot) || null;
  }

  function codexPlusRenderMessageActions(parent, role, items, info) {
    let root = codexPlusMessageSlotChild(parent, "actions");
    const visible = items.filter((item) => codexPlusMessageItemApplies(item, info));
    if (!visible.length) {
      root?.remove();
      return;
    }
    const renderKey = visible.map((item) => `${item.id}|${item.label || ""}|${item.icon || ""}`).join("\n");
    if (root?.dataset.renderKey === renderKey) return;
    if (!root) {
      root = document.createElement("span");
      root.setAttribute(codexPlusMessageSlotAttribute, "actions");
      markCodexPlusExtensionNode(root, "builtin-message-slots");
      parent.appendChild(root);
    }
    root.dataset.renderKey = renderKey;
    root.textContent = "";
    // 借原生按钮的类名，悬停、尺寸、显隐都和复制 / 编辑按钮一致。
    const nativeClass = parent.querySelector(`button:not([${codexPlusExtensionConstants.extensionAttribute}])`)?.className || "";
    for (const item of visible) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = `${nativeClass} codex-plus-message-action`.trim();
      button.setAttribute("aria-label", item.label || item.id);
      markCodexPlusExtensionNode(button, item.scriptKey);
      button.innerHTML = codexPlusExtensionIconMarkup(item);
      attachCodexPlusRailTooltip(button, item.label || item.id, "bottom");
      button.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        const node = button.closest(role === "assistant" ? "[data-response-annotation-target]" : "[data-local-conversation-user-anchor]");
        if (!node) return;
        runCodexPlusExtensionCallback(item.scriptKey, "messageAction.onActivate", () => item.onActivate?.({
          message: codexPlusMessageInfo(role, node),
          close: () => hideCodexPlusRailTooltip(),
        }));
      });
      root.appendChild(button);
    }
  }

  function codexPlusRenderMessageDecorations(hosts, role, items, info) {
    const parent = hosts.decorationParent;
    if (!parent) return;
    let root = codexPlusMessageSlotChild(parent, "decorations");
    const applicable = items.filter((item) => codexPlusMessageItemApplies(item, info));
    if (!applicable.length) {
      root?.remove();
      return;
    }
    // 文字变了（流式输出、编辑后重发）才重画；只比长度和结尾，避免每轮都拼整段字符串。
    const renderKey = `${applicable.map((item) => item.id).join("\n")}|${info.id}|${info.text.length}|${info.text.slice(-32)}`;
    if (!root) {
      root = document.createElement("div");
      root.className = "codex-plus-message-decorations";
      root.dataset.role = role;
      root.setAttribute(codexPlusMessageSlotAttribute, "decorations");
      markCodexPlusExtensionNode(root, "builtin-message-slots");
    }
    // React 重排后操作行可能跑到装饰前面：每次都校正位置（位置已对时不动 DOM）。
    const before = hosts.decorationBefore && hosts.decorationBefore !== root && hosts.decorationBefore.parentElement === parent
      ? hosts.decorationBefore
      : null;
    if (root.parentElement !== parent || (before ? root.nextElementSibling !== before : parent.lastElementChild !== root)) {
      parent.insertBefore(root, before);
    }
    if (root.dataset.renderKey === renderKey) return;
    root.dataset.renderKey = renderKey;
    root.textContent = "";
    for (const item of applicable) {
      const container = document.createElement("div");
      container.className = "codex-plus-message-decoration";
      markCodexPlusExtensionNode(container, item.scriptKey);
      root.appendChild(container);
      runCodexPlusExtensionCallback(item.scriptKey, "messageDecoration.render", () => item.render({ message: info, container }));
    }
  }

  function installCodexPlusSlotStyle() {
    if (document.getElementById("codex-plus-slot-style")) return;
    const style = document.createElement("style");
    style.id = "codex-plus-slot-style";
    markCodexPlusExtensionNode(style, "builtin-message-slots");
    style.textContent = `[data-codex-plus-message-slot="actions"]{display:contents}
      .codex-plus-message-action .codex-plus-ext-icon,.codex-plus-message-action svg{display:inline-flex;width:16px;height:16px;align-items:center;justify-content:center;font-size:14px;line-height:1}
      .codex-plus-message-decorations{display:flex;flex-direction:column;gap:6px;margin-top:8px;min-width:0}
      .codex-plus-message-decorations[data-role="user"]{align-self:flex-end;align-items:flex-end;max-width:var(--user-chat-width,100%)}
      .codex-plus-message-decoration:empty,.codex-plus-message-decorations:has(> .codex-plus-message-decoration:empty:only-child){display:none}
      html[data-codex-plus-side-panel-open] main[data-app-shell-main-surface]{margin-right:var(--codex-plus-side-panel-width,360px)!important}
      .codex-plus-side-panel{position:fixed;right:0;bottom:0;top:var(--codex-plus-side-panel-top,44px);width:var(--codex-plus-side-panel-width,360px);z-index:40;display:flex;flex-direction:column;min-height:0;background:var(--color-token-main-surface-primary,var(--color-token-bg-primary,Canvas));color:var(--color-token-text-primary,CanvasText);border-left:1px solid var(--color-token-border-light,color-mix(in srgb,currentColor 12%,transparent))}
      .codex-plus-side-panel-header{display:flex;align-items:center;gap:8px;height:40px;padding:0 8px 0 14px;flex:none;border-bottom:1px solid var(--color-token-border-light,color-mix(in srgb,currentColor 10%,transparent))}
      .codex-plus-side-panel-title{flex:1;min-width:0;font-size:13px;font-weight:600;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
      .codex-plus-side-panel-close{display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;border:0;border-radius:8px;background:transparent;color:var(--color-token-text-secondary,currentColor);cursor:pointer}
      .codex-plus-side-panel-close:hover{background:var(--color-token-list-hover-background,color-mix(in srgb,currentColor 10%,transparent))}
      .codex-plus-side-panel-body{flex:1;min-height:0;overflow:auto;padding:12px 14px}
      .codex-plus-side-panel-toggle .codex-plus-ext-icon,.codex-plus-side-panel-toggle svg{display:inline-flex;width:16px;height:16px;align-items:center;justify-content:center;font-size:14px;line-height:1}
      .codex-plus-side-panel-toggle[aria-pressed="true"]{background:var(--color-token-list-hover-background,color-mix(in srgb,currentColor 12%,transparent))}
      .codex-plus-ui-slot{display:flex;flex-direction:column;gap:6px;min-width:0}
      .codex-plus-ui-slot:not(:has(.codex-plus-ui-slot-item:not(:empty):not(:has(> [data-hidden="true"]:only-child)))){display:none}
      .codex-plus-ui-slot-item:has(> [data-hidden="true"]:only-child){display:none}
      .codex-plus-ui-slot-item:empty{display:none}
      [data-codex-plus-ui-slot="composer.above"]{margin:0 0 8px}
      [data-codex-plus-ui-slot="thread.top"],[data-codex-plus-ui-slot="thread.bottom"]{width:min(100%,var(--thread-content-max-width,768px));margin:12px auto;padding:0 16px;box-sizing:border-box}
      [data-codex-plus-ui-slot="sidebar.top"],[data-codex-plus-ui-slot="sidebar.bottom"]{padding:6px 8px}
      [data-codex-plus-ui-slot="titlebar"]{display:contents}
      [data-codex-plus-ui-slot="titlebar"] .codex-plus-ui-slot-item{display:contents}
      [data-codex-plus-ui-slot="turn.after"]{margin-top:6px}
      [data-codex-plus-ui-slot="sessionRow.prefix"],[data-codex-plus-ui-slot="sessionRow.suffix"]{flex-direction:row;align-items:center;flex:none;gap:4px}
      [data-codex-plus-ui-slot="sessionRow.prefix"]{margin-right:4px}
      [data-codex-plus-ui-slot="sessionRow.suffix"]{margin-left:4px}
      [data-codex-plus-ui-slot="composer.left"],[data-codex-plus-ui-slot="composer.right"],[data-codex-plus-ui-slot="header.title"],[data-codex-plus-ui-slot="sidebar.section"],[data-codex-plus-ui-slot="projectRow.suffix"]{flex-direction:row;align-items:center;flex:none;gap:4px}
      [data-codex-plus-ui-slot="composer.right"]{margin-right:4px}
      [data-codex-plus-ui-slot="composer.model"]{flex-direction:row;align-items:center;flex:none;gap:4px;margin-left:2px}
      [data-codex-plus-ui-slot="settings.nav"]{gap:2px}
      .codex-plus-settings-nav-item{display:flex;align-items:center;gap:8px;width:100%;text-align:left;background:transparent;border:0;cursor:pointer}
      [data-codex-plus-ui-slot="header.title"]{margin-left:6px}
      [data-codex-plus-ui-slot="composer.below"]{margin:6px 0 0}
      [data-codex-plus-ui-slot="home.above"]{margin:0 0 10px}
      [data-codex-plus-ui-slot="approval"]{padding:0 16px 12px}
      [data-codex-plus-ui-slot="activity.after"]{margin:4px 0 2px}
      [data-codex-plus-ui-slot="overlay"]{position:fixed;inset:0;z-index:2147483600;pointer-events:none}
      [data-codex-plus-ui-slot="overlay"] .codex-plus-ui-slot-item>*{pointer-events:auto}`;
    document.head.append(style);
  }

  let codexPlusMessageSlotsRendered = false;
  let codexPlusMessageObserver = null;
  let codexPlusMessageObserverTargets = [];
  let codexPlusMessageRefreshTimer = 0;

  function scheduleCodexPlusMessageSlotsRefresh() {
    if (codexPlusMessageRefreshTimer) return;
    codexPlusMessageRefreshTimer = setTimeout(() => {
      codexPlusMessageRefreshTimer = 0;
      refreshCodexPlusMessageSlots();
      // 对话区里的界面插槽（每轮末尾、对话顶部 / 底部）也跟着对话内容走：切会话、新一轮出现时补上。
      if (codexPlusHasThreadUiSlots()) refreshCodexPlusUiSlots();
    }, codexPlusMessageSlotRefreshDelayMs);
  }

  /** 只是我们自己的节点在变就不理，否则「写入 → 观察到 → 再写入」会自喂（issue #1960）。 */
  function codexPlusMutationIsOurs(mutation) {
    const own = (node) => isCodexPlusExtensionNode(node?.nodeType === 1 ? node : node?.parentElement);
    if (own(mutation.target)) return true;
    const changed = [...mutation.addedNodes, ...mutation.removedNodes];
    return changed.length > 0 && changed.every((node) => node.nodeType === 1 && isCodexPlusExtensionNode(node));
  }

  /** Codex 的多个标签页各有一个对话滚动区（非当前的隐藏着），全部观察：切标签时不漏。 */
  function syncCodexPlusMessageObserver(active) {
    const targets = active ? [...document.querySelectorAll(".thread-scroll-container")] : [];
    if (targets.length === codexPlusMessageObserverTargets.length && targets.every((node, index) => node === codexPlusMessageObserverTargets[index])) return;
    codexPlusMessageObserver?.disconnect();
    codexPlusMessageObserverTargets = targets;
    if (!targets.length) return;
    codexPlusMessageObserver ||= new MutationObserver((mutations) => {
      if (mutations.some((mutation) => !codexPlusMutationIsOurs(mutation))) scheduleCodexPlusMessageSlotsRefresh();
    });
    for (const target of targets) codexPlusMessageObserver.observe(target, { childList: true, subtree: true, characterData: true });
  }

  /** 有没有挂在对话区里的界面插槽：它们和消息插槽共用对话区的观察器。 */
  function codexPlusHasThreadUiSlots() {
    return codexPlusExtensionItems(codexPlusRegistry.slots).some((item) => ["thread.top", "thread.bottom", "turn.after", "composer.above", "approval", "activity.after"].includes(item.slot));
  }

  function refreshCodexPlusMessageSlots() {
    const actions = codexPlusExtensionItems(codexPlusRegistry.messageActions);
    const decorations = codexPlusExtensionItems(codexPlusRegistry.messageDecorations);
    const active = actions.length > 0 || decorations.length > 0;
    syncCodexPlusMessageObserver(active || codexPlusHasThreadUiSlots());
    if (!active) {
      if (codexPlusMessageSlotsRendered) {
        document.querySelectorAll(`[${codexPlusMessageSlotAttribute}]`).forEach((node) => node.remove());
        codexPlusMessageSlotsRendered = false;
      }
      return;
    }
    installCodexPlusSlotStyle();
    codexPlusMessageSlotsRendered = true;
    for (const { role, node } of codexPlusMessageNodes()) {
      const info = codexPlusMessageInfo(role, node);
      const hosts = codexPlusMessageSlotHosts(role, node);
      if (hosts.actionParent) codexPlusRenderMessageActions(hosts.actionParent, role, actions, info);
      codexPlusRenderMessageDecorations(hosts, role, decorations, info);
    }
  }

  /**
   * 侧边面板：同一时间只开一个。面板挂在 body 上（不进 React 管的树），打开时给 html 打标记，
   * 由样式把 Codex 主区域往左让出面板宽度。开关按钮放在标题栏右上角「新建标签页」按钮前面
   * （首页和会话页都有）；找不到时退到会话顶栏右侧按钮组的最前面。
   */
  let codexPlusOpenSidePanelState = null;

  function codexPlusSidePanelToolbar() {
    const layoutToggle = [...document.querySelectorAll("button[data-app-shell-workspace-layout-toggle]")].find((node) => visibleElement(node));
    const wrapper = layoutToggle?.parentElement;
    if (wrapper?.parentElement) return { host: wrapper.parentElement, before: wrapper, sample: layoutToggle };
    const cluster = [...document.querySelectorAll("[data-app-shell-header-toolbar] > .ms-auto")].find((node) => visibleElement(node));
    if (!cluster) return null;
    return { host: cluster, before: cluster.firstChild, sample: cluster.querySelector(`button:not([${codexPlusExtensionConstants.extensionAttribute}])`) };
  }

  function codexPlusSidePanelWidth(definition) {
    const width = Number(definition?.width);
    return Number.isFinite(width) ? Math.min(720, Math.max(240, Math.round(width))) : 360;
  }

  /** 面板从主区域顶边开始，不盖住顶栏（开关按钮就在顶栏里）。隐藏标签页的 main 尺寸为 0，跳过。 */
  function syncCodexPlusSidePanelTop() {
    const main = [...document.querySelectorAll("main[data-app-shell-main-surface]")].find((node) => node.getBoundingClientRect().width > 0);
    const toolbar = document.querySelector("[data-app-shell-header-toolbar]");
    const top = main
      ? Math.max(0, Math.round(main.getBoundingClientRect().top))
      : toolbar ? Math.round(toolbar.getBoundingClientRect().bottom) : 44;
    document.documentElement.style.setProperty("--codex-plus-side-panel-top", `${top}px`);
  }

  function closeCodexPlusSidePanel() {
    const state = codexPlusOpenSidePanelState;
    if (!state) return false;
    codexPlusOpenSidePanelState = null;
    if (typeof state.cleanup === "function") {
      runCodexPlusExtensionCallback(state.scriptKey, "sidePanel.cleanup", state.cleanup);
    }
    state.panel.remove();
    delete document.documentElement.dataset.codexPlusSidePanelOpen;
    document.documentElement.style.removeProperty("--codex-plus-side-panel-width");
    refreshCodexPlusSidePanels();
    return true;
  }

  function openCodexPlusSidePanel(id) {
    const definition = codexPlusRegistry.sidePanels.get(id);
    if (!definition) return false;
    if (codexPlusOpenSidePanelState?.id === id) return true;
    closeCodexPlusSidePanel();
    installCodexPlusSlotStyle();
    const title = String(definition.title || definition.label || id);
    const panel = document.createElement("aside");
    panel.className = "codex-plus-side-panel";
    panel.dataset.codexPlusSidePanel = id;
    panel.setAttribute("role", "complementary");
    panel.setAttribute("aria-label", title);
    markCodexPlusExtensionNode(panel, definition.scriptKey);
    panel.innerHTML = `<div class="codex-plus-side-panel-header"><span class="codex-plus-side-panel-title">${escapeHtml(title)}</span><button type="button" class="codex-plus-side-panel-close" aria-label="关闭"><svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg></button></div><div class="codex-plus-side-panel-body"></div>`;
    panel.querySelector(".codex-plus-side-panel-close").addEventListener("click", () => closeCodexPlusSidePanel());
    document.documentElement.style.setProperty("--codex-plus-side-panel-width", `${codexPlusSidePanelWidth(definition)}px`);
    document.documentElement.dataset.codexPlusSidePanelOpen = "true";
    syncCodexPlusSidePanelTop();
    document.body.appendChild(panel);
    const state = { id, scriptKey: definition.scriptKey, panel, cleanup: null };
    codexPlusOpenSidePanelState = state;
    const container = panel.querySelector(".codex-plus-side-panel-body");
    const result = runCodexPlusExtensionCallback(definition.scriptKey, "sidePanel.render", () => definition.render({
      container,
      close: () => {
        if (codexPlusOpenSidePanelState === state) closeCodexPlusSidePanel();
      },
    }));
    if (result.ok && typeof result.value === "function") state.cleanup = result.value;
    refreshCodexPlusSidePanels();
    return true;
  }

  function toggleCodexPlusSidePanel(id) {
    return codexPlusOpenSidePanelState?.id === id ? closeCodexPlusSidePanel() : openCodexPlusSidePanel(id);
  }

  function refreshCodexPlusSidePanels() {
    const items = codexPlusExtensionItems(codexPlusRegistry.sidePanels);
    const openId = codexPlusOpenSidePanelState?.id || "";
    // 面板的注册被撤销（拓展卸载、热重载）时一并关掉。
    if (openId && !codexPlusRegistry.sidePanels.has(openId)) {
      closeCodexPlusSidePanel();
      return;
    }
    // 面板被别的代码摘掉了（例如 body 被整体重建）：重新挂回去，内容仍是拓展上次画的。
    if (openId && !codexPlusOpenSidePanelState.panel.isConnected) document.body.appendChild(codexPlusOpenSidePanelState.panel);
    if (openId) syncCodexPlusSidePanelTop();
    const toolbar = items.length ? codexPlusSidePanelToolbar() : null;
    document.querySelectorAll("[data-codex-plus-side-panel-toggles]").forEach((node) => {
      if (node.parentElement !== toolbar?.host) node.remove();
    });
    if (!toolbar) return;
    installCodexPlusSlotStyle();
    let root = [...toolbar.host.children].find((node) => node.hasAttribute?.("data-codex-plus-side-panel-toggles"));
    if (!root) {
      root = document.createElement("div");
      root.style.display = "contents";
      root.setAttribute("data-codex-plus-side-panel-toggles", "");
      markCodexPlusExtensionNode(root, "builtin-side-panels");
      toolbar.host.insertBefore(root, toolbar.before && toolbar.before.parentElement === toolbar.host ? toolbar.before : null);
    }
    const renderKey = `${openId}\n${items.map((item) => `${item.id}|${item.title || ""}|${item.icon || ""}`).join("\n")}`;
    if (root.dataset.renderKey === renderKey) return;
    root.dataset.renderKey = renderKey;
    root.textContent = "";
    const nativeClass = toolbar.sample?.className || "";
    for (const item of items) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = `${nativeClass} codex-plus-side-panel-toggle`.trim();
      const label = String(item.title || item.label || item.id);
      button.setAttribute("aria-label", label);
      button.setAttribute("aria-pressed", item.id === openId ? "true" : "false");
      markCodexPlusExtensionNode(button, item.scriptKey);
      button.innerHTML = codexPlusExtensionIconMarkup(item);
      attachCodexPlusRailTooltip(button, label, "bottom");
      button.addEventListener("click", (event) => {
        event.preventDefault();
        hideCodexPlusRailTooltip();
        toggleCodexPlusSidePanel(item.id);
      });
      root.appendChild(button);
    }
  }

  /** 注册侧边面板；返回的撤销函数上带 open / close，拓展可以自己控制开合。 */
  function codexPlusRegisterExtensionSidePanel(definition, options = {}) {
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const id = `${scriptKey}:sidePanel:${options.id || codexPlusExtensionIdSeed()}`;
    const dispose = registerCodexPlusExtension("sidePanel", codexPlusRegistry.sidePanels, id, definition, scriptKey);
    codexPlusRefreshExtensionHosts();
    const handle = () => {
      dispose();
      codexPlusRefreshExtensionHosts();
    };
    handle.open = () => openCodexPlusSidePanel(id);
    handle.close = () => (codexPlusOpenSidePanelState?.id === id ? closeCodexPlusSidePanel() : false);
    handle.toggle = () => toggleCodexPlusSidePanel(id);
    return handle;
  }

  /**
   * 通用界面插槽：拓展用 registerSlot({ slot, render }) 往 Codex 界面的固定位置放自己的内容。
   *
   * 位置：composer.above（输入框上方）、thread.top / thread.bottom（对话区顶部 / 底部）、
   * sidebar.top / sidebar.bottom（左侧栏列表顶部 / 底部）、titlebar（标题栏右侧按钮组）、
   * turn.after（每一轮对话的末尾，render 收到 turnKey）、sessionRow.prefix / sessionRow.suffix
   * （侧栏每个会话行标题前 / 后，render 收到 sessionId、title）、overlay（铺满窗口的浮层，默认不挡点击）、
   * activity.after（对话里每个活动条目——命令、工具 / 子 agent 调用、出错、推理——的末尾，render 收到
   * kind、itemIds、text；条目在 Codex 折叠的「用时 Ns」里，展开后才渲染）、
   * composer.left / composer.right（输入框底栏左侧按钮组末尾 / 右侧按钮组开头）、composer.below（输入框下方）、
   * home.above（新对话首页输入框上方）、header.title（顶栏标题后面）、approval（审批卡片底部，render 收到 kind）、
   * projectRow.suffix（侧栏项目行名称后，render 收到 projectId、label）、sidebar.section（侧栏分组标题右侧，
   * render 收到 section）。
   * 每个位置一个带拓展标记的宿主节点，里面每个注册项一个容器；容器新建时调用 render（DOM 会随 Codex 重建，
   * 重建后会再调一次），render 返回的函数在该项被撤下时调用。注册返回的句柄带 refresh() 主动重画。
   */
  const codexPlusUiSlotNames = [
    "composer.above", "composer.below", "composer.left", "composer.right", "composer.model", "home.above", "settings.nav",
    "thread.top", "thread.bottom", "turn.after", "header.title", "titlebar", "approval",
    "sidebar.top", "sidebar.bottom", "sidebar.section", "sessionRow.prefix", "sessionRow.suffix", "projectRow.suffix",
    "overlay",
    "activity.after",
  ];
  const codexPlusUiSlotVersions = new Map();
  // 只为资源撤销保留挂载记录；渲染位置仍现查 DOM，不复用这些节点。
  const codexPlusUiSlotCleanups = new Map();

  function codexPlusUiSlotAnchors(slot) {
    const visible = (selector) => [...document.querySelectorAll(selector)].filter((node) => visibleElement(node) && !isExtensionUiNode(node));
    switch (slot) {
      // place：start（最前）、end（最后），或一个兄弟节点（放在它前面）。
      case "composer.above":
        return visible("[data-codex-composer-root]").map((node) => ({ parent: node.parentElement, place: node }));
      case "thread.top":
      case "thread.bottom":
        // 对话内容层的末尾是一个给输入框让位的粘底空白层；底部插槽放在它前面，紧跟最后一条消息。
        return visible(".thread-scroll-container").map((scroll) => scroll.firstElementChild).filter(Boolean)
          .map((content) => ({
            parent: content,
            place: slot === "thread.top"
              ? "start"
              : [...content.children].find((child) => child.classList.contains("sticky") && !isExtensionUiNode(child)) || "end",
          }));
      case "sidebar.top":
      case "sidebar.bottom":
        return visible("[data-app-action-sidebar-scroll]")
          .map((list) => ({ parent: list, place: slot === "sidebar.top" ? "start" : "end" }));
      case "turn.after":
        // 每一轮的容器带 data-turn-key（虚拟列表只渲染可见的几轮）。
        return visible(".thread-scroll-container [data-turn-key]")
          .map((turn) => ({ parent: turn, place: "end", info: { turnKey: turn.getAttribute("data-turn-key") || "" } }));
      case "sessionRow.prefix":
      case "sessionRow.suffix":
        return visible("[data-app-action-sidebar-thread-row]").flatMap((row) => {
          const title = row.querySelector("[data-thread-title-trigger]");
          if (!title?.parentElement) return [];
          const info = {
            sessionId: String(row.getAttribute("data-app-action-sidebar-thread-id") || "").replace(/^local:/, ""),
            title: row.getAttribute("data-app-action-sidebar-thread-title") || "",
          };
          const next = title.nextElementSibling;
          return [{ parent: title.parentElement, place: slot === "sessionRow.prefix" ? title : (next && !isExtensionUiNode(next) ? next : "end"), info }];
        });
      case "composer.below":
        return visible("[data-codex-composer-root]").map((node) => ({ parent: node, place: "end" }));
      case "composer.left":
        // 底栏左侧按钮组（添加内容、权限……）：「添加内容」按钮所在的那一行 flex。
        return visible("[data-composer-footer-responsive]")
          .map((footer) => {
            // 只认带「添加内容 / 权限」的那一行；首页顶上的工作区栏（项目、运行位置）也带导航目标，不算。
            const row = footer.querySelector('[data-composer-navigation-target="add-context"], [data-composer-navigation-target="permissions"]')?.closest("div.flex");
            return row && footer.contains(row) ? row : null;
          })
          .filter(Boolean)
          .map((row) => ({ parent: row, place: "end" }));
      case "composer.right":
        // 底栏右侧按钮组（听写、发送）：放在最前面。
        return visible('[data-composer-footer-responsive] > [class*="col-start-3"]')
          .map((group) => group.querySelector(":scope div.shrink-0.items-center"))
          .filter(Boolean)
          .map((row) => ({ parent: row, place: "start" }));
      case "composer.model": {
        // 模型 / 推理强度选择按钮后面。
        // 按钮外面包着一层提示用的 span，挂在包着它的那一行 flex 里、紧跟在这一项后面。
        return visible('[data-composer-navigation-target="reasoning"]').flatMap((button) => {
          const row = button.parentElement?.closest("div.flex");
          if (!row) return [];
          const item = [...row.children].find((child) => child.contains(button));
          const next = item?.nextElementSibling;
          return [{ parent: row, place: next && !isExtensionUiNode(next) ? next : "end" }];
        });
      }
      case "settings.nav": {
        // Codex 设置页左侧导航（各项带 data-settings-panel-slug）的末尾；sampleClass 是原生导航项的类名，可以借来对齐样式。
        const items = visible("[data-settings-panel-slug]");
        if (!items.length) return [];
        let list = items[0].parentElement;
        while (list && !items.every((item) => list.contains(item))) list = list.parentElement;
        if (!list) return [];
        return [{ parent: list, place: "end", info: { sampleClass: items[0].className || "" } }];
      }
      case "home.above":
        return visible('[data-codex-composer-root][data-composer-placement="home"]').map((node) => ({ parent: node.parentElement, place: node }));
      case "header.title":
        return visible("[data-app-shell-titlebar-content]").map((node) => ({ parent: node, place: "end" }));
      case "approval":
        return visible("[data-codex-approval-surface]").map((card) => ({
          parent: card,
          place: "end",
          info: { kind: card.tagName === "FORM" || card.querySelector("form") ? "form" : "card" },
        }));
      case "sidebar.section":
        return visible("[data-app-action-sidebar-section]").flatMap((section) => {
          const actions = section.querySelector(':scope [class*="nav-section-title"] > div.shrink-0');
          return actions ? [{ parent: actions, place: "start", info: { section: section.getAttribute("data-app-action-sidebar-section-heading") || "" } }] : [];
        });
      case "projectRow.suffix":
        return visible("[data-app-action-sidebar-project-row]").flatMap((row) => {
          const label = row.querySelector("[data-marquee-text]")?.parentElement;
          if (!label?.parentElement) return [];
          return [{
            parent: label.parentElement,
            place: label.nextElementSibling && !isExtensionUiNode(label.nextElementSibling) ? label.nextElementSibling : "end",
            info: {
              projectId: row.getAttribute("data-app-action-sidebar-project-id") || "",
              label: row.getAttribute("data-app-action-sidebar-project-label") || "",
            },
          }];
        });
      case "overlay":
        return document.body ? [{ parent: document.body, place: "end" }] : [];
      case "activity.after":
        // 活动条目带 data-local-conversation-item-target-ids（空格分隔的条目 id，前缀即类型）；挂在条目自己里面的末尾。
        return visible(".thread-scroll-container [data-local-conversation-item-target-ids]").map((item) => {
          const itemIds = String(item.getAttribute("data-local-conversation-item-target-ids") || "").split(/\s+/).filter(Boolean);
          return { parent: item, place: "end", info: { kind: codexPlusActivityKind(itemIds[0] || ""), itemIds, text: (item.innerText || "").trim().slice(0, 300) } };
        });
      case "titlebar": {
        const toolbar = codexPlusSidePanelToolbar();
        if (!toolbar) return [];
        return [{ parent: toolbar.host, place: toolbar.before && toolbar.before.parentElement === toolbar.host ? toolbar.before : "end" }];
      }
      default:
        return [];
    }
  }

  /** 活动条目 id 的前缀 → 类型。 */
  function codexPlusActivityKind(id) {
    if (/^exec-/.test(id)) return "command";
    if (/^turn-error-/.test(id)) return "error";
    if (/plan/i.test(id)) return "plan";
    if (/reason/i.test(id)) return "reasoning";
    if (/patch|file-?change/i.test(id)) return "fileChange";
    if (/^call_|^fc_|tool|mcp/i.test(id)) return "toolCall";
    return "other";
  }

  function codexPlusRunSlotCleanup(node) {
    const cleanup = node.__codexPlusSlotCleanup;
    node.__codexPlusSlotCleanup = null;
    codexPlusUiSlotCleanups.delete(node);
    delete node.dataset.slotVersion;
    delete node.__codexPlusSlotInfo;
    if (typeof cleanup === "function") runCodexPlusExtensionCallback(node.dataset.scriptKey || "", "slot.cleanup", cleanup);
  }

  /** 共用主观察器的移除记录；同批移动已重新连接的节点不清理。 */
  function pruneRemovedCodexPlusUiSlots(mutations) {
    if (!codexPlusUiSlotsRendered && !codexPlusUiSlotCleanups.size) return false;
    const repairs = new Map();
    for (const mutation of mutations || []) {
      for (const node of mutation.removedNodes || []) {
        if (node.nodeType !== 1 || node.isConnected) continue;
        const containers = node.matches?.("[data-codex-plus-slot-item]") ? [node] : [];
        containers.push(...node.querySelectorAll("[data-codex-plus-slot-item]"));
        containers.forEach(codexPlusRunSlotCleanup);
        // 原生只删了插槽、锚点还在时补回；整块页面离开不为旧插槽另排扫描。
        const parent = mutation.target;
        if (!parent?.isConnected) continue;
        const slot = node.getAttribute("data-codex-plus-ui-slot")
          || (node.matches?.("[data-codex-plus-slot-item]") && parent.getAttribute("data-codex-plus-ui-slot"));
        if (slot) {
          if (!repairs.has(slot)) repairs.set(slot, new Set());
          repairs.get(slot).add(node.hasAttribute("data-codex-plus-ui-slot") ? parent : parent.parentElement);
        }
      }
    }
    for (const [slot, parents] of repairs) {
      if (!codexPlusExtensionItems(codexPlusRegistry.slots).some((item) => item.slot === slot)) continue;
      if (codexPlusUiSlotAnchors(slot).some((anchor) => parents.has(anchor.parent))) return true;
    }
    return false;
  }

  let codexPlusUiSlotsRendered = false;

  function refreshCodexPlusUiSlots() {
    for (const [node, item] of [...codexPlusUiSlotCleanups]) {
      if (!node.isConnected || codexPlusRegistry.slots.get(item.id) !== item) codexPlusRunSlotCleanup(node);
    }
    const items = codexPlusExtensionItems(codexPlusRegistry.slots);
    if (!items.length) {
      if (codexPlusUiSlotsRendered) {
        document.querySelectorAll("[data-codex-plus-ui-slot]").forEach((host) => {
          host.querySelectorAll("[data-codex-plus-slot-item]").forEach(codexPlusRunSlotCleanup);
          host.remove();
        });
        codexPlusUiSlotsRendered = false;
      }
      return;
    }
    codexPlusUiSlotsRendered = true;
    installCodexPlusSlotStyle();
    for (const slot of codexPlusUiSlotNames) {
      const slotItems = items.filter((item) => item.slot === slot);
      const anchors = slotItems.length ? codexPlusUiSlotAnchors(slot) : [];
      const parents = new Set(anchors.map((anchor) => anchor.parent));
      document.querySelectorAll(`[data-codex-plus-ui-slot="${slot}"]`).forEach((host) => {
        if (parents.has(host.parentElement)) return;
        host.querySelectorAll("[data-codex-plus-slot-item]").forEach(codexPlusRunSlotCleanup);
        host.remove();
      });
      for (const { parent, place, info } of anchors) {
        if (!parent) continue;
        let host = [...parent.children].find((node) => node.getAttribute?.("data-codex-plus-ui-slot") === slot);
        if (!host) {
          host = document.createElement("div");
          host.className = "codex-plus-ui-slot";
          host.setAttribute("data-codex-plus-ui-slot", slot);
          markCodexPlusExtensionNode(host, "builtin-ui-slots");
        }
        // 位置不对（Codex 重排了兄弟节点）时挪回去；位置已对时不动 DOM。
        if (place === "start") {
          if (parent.firstChild !== host) parent.insertBefore(host, parent.firstChild);
        } else if (place === "end" || !place || place.parentElement !== parent) {
          if (parent.lastChild !== host) parent.appendChild(host);
        } else if (host.nextSibling !== place) {
          parent.insertBefore(host, place);
        }
        const wanted = new Set(slotItems.map((item) => item.id));
        for (const child of [...host.children]) {
          if (!wanted.has(child.dataset.slotItemId)) {
            codexPlusRunSlotCleanup(child);
            child.remove();
          }
        }
        // 按注册顺序就位：只在顺序不对时挪动，已就位的节点不碰（挪动会让里面的输入框失焦）。
        let previous = null;
        const placeAfterPrevious = (container) => {
          const expected = previous ? previous.nextElementSibling : host.firstElementChild;
          if (expected !== container) host.insertBefore(container, expected);
          previous = container;
        };
        for (const item of slotItems) {
          const version = String(codexPlusUiSlotVersions.get(item.id) || 0);
          let container = [...host.children].find((child) => child.dataset.slotItemId === item.id);
          // 同一个宿主换了归属（会话行被复用给别的会话）时整项重画。
          const infoKey = info ? JSON.stringify(info) : "";
          if (container && container.dataset.slotVersion === version && (container.__codexPlusSlotInfo ?? "") === infoKey) {
            placeAfterPrevious(container);
            continue;
          }
          if (container) {
            codexPlusRunSlotCleanup(container);
            container.textContent = "";
          } else {
            container = document.createElement("div");
            container.className = "codex-plus-ui-slot-item";
            container.setAttribute("data-codex-plus-slot-item", "");
            container.dataset.slotItemId = item.id;
            container.dataset.scriptKey = item.scriptKey;
            markCodexPlusExtensionNode(container, item.scriptKey);
          }
          container.dataset.slotVersion = version;
          container.__codexPlusSlotInfo = infoKey;
          placeAfterPrevious(container);
          const result = runCodexPlusExtensionCallback(item.scriptKey, "slot.render", () => item.render({ container, slot, ...(info || {}) }));
          if (result.ok && typeof result.value === "function") {
            // 新注入可经旧节点撤销资源；包装器必须摘掉原闭包的记录，且重入只执行一次。
            const release = result.value;
            let cleaned = false;
            const cleanup = () => {
              if (cleaned) return;
              cleaned = true;
              codexPlusUiSlotCleanups.delete(container);
              delete container.dataset.slotVersion;
              delete container.__codexPlusSlotInfo;
              if (container.__codexPlusSlotCleanup === cleanup) container.__codexPlusSlotCleanup = null;
              release();
            };
            container.__codexPlusSlotCleanup = cleanup;
            codexPlusUiSlotCleanups.set(container, item);
            if (!container.isConnected || codexPlusRegistry.slots.get(item.id) !== item) codexPlusRunSlotCleanup(container);
          }
        }
      }
    }
  }

  /** 注册界面插槽；返回的撤销函数上带 refresh()，强制所有位置重画该项。 */
  function codexPlusRegisterExtensionSlot(definition, options = {}) {
    if (!codexPlusUiSlotNames.includes(definition?.slot)) {
      throw new Error(`未知的插槽位置 ${definition?.slot}，可用：${codexPlusUiSlotNames.join("、")}`);
    }
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const id = `${scriptKey}:slot:${options.id || codexPlusExtensionIdSeed()}`;
    const dispose = registerCodexPlusExtension("slot", codexPlusRegistry.slots, id, definition, scriptKey);
    codexPlusRefreshExtensionHosts();
    const handle = () => {
      dispose();
      codexPlusUiSlotVersions.delete(id);
      codexPlusRefreshExtensionHosts();
    };
    handle.refresh = () => {
      codexPlusUiSlotVersions.set(id, (codexPlusUiSlotVersions.get(id) || 0) + 1);
      refreshCodexPlusUiSlots();
    };
    return handle;
  }
