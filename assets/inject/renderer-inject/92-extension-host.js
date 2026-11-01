  /**
   * 拓展宿主：把注册中心里的第三方项渲染出来。
   *
   * 与 91-extension-api.js 的分工：那边负责「收」（校验、配额、挂 API），这边负责
   * 「画」（把数据变成 DOM）。分开是因为画的部分要贴着既有 UI 的类名与结构走，
   * 而收的部分只需要一份数据契约。
   *
   * 全部采用「追加」而不是「重写」：内置项仍由原路径渲染，拓展项在其后补上。
   * 这样内置 UI 的行为零变化，出问题时摘掉这个分片即可回滚。
   */

  /**
   * 拓展节点的统一标记，扫描调度靠它识别（见 01-registry.js 的注释）。
   *
   * 选择器按「有归属/无归属」两档登记，而不是按脚本 key 逐个登记：一个脚本可能
   * 注册很多项，按 key 登记会白白吃掉全局选择器配额（上限 64），而扫描调度只需要
   * 知道「这个节点是我们的」——精确到脚本对排除自喂循环没有任何额外价值。
   */
  function markCodexPlusExtensionNode(node, scriptKey) {
    node.setAttribute(codexPlusExtensionConstants.extensionAttribute, scriptKey || "");
    registerCodexPlusExtensionSelector(`[${codexPlusExtensionConstants.extensionAttribute}]`);
    return node;
  }

  /** 取一个拓展项的图标：允许传 SVG 字符串，没给就用默认字形。 */
  function codexPlusExtensionIconMarkup(definition) {
    const icon = definition?.icon;
    if (typeof icon !== "string" || !icon.trim()) return `<span aria-hidden="true">◇</span>`;
    // 只接受 svg 或文本字形：注入任意 HTML 会让拓展有机会破坏内置 UI 结构。
    if (/^\s*<svg[\s>]/i.test(icon)) return `<span class="codex-plus-ext-icon" aria-hidden="true">${icon}</span>`;
    return `<span class="codex-plus-ext-icon" aria-hidden="true">${escapeHtml(icon)}</span>`;
  }

  /**
   * 打开一个拓展注册的整页视图。
   *
   * 复用内置的页面骨架（rail 高亮同步、缩放跟随、原生选中态压制都白拿），只是把
   * 内容区换掉。注意 overlay 每次打开都重建，所以 render 每次都要重新调用。
   */
  function openCodexPlusExtensionPage(id) {
    const definition = codexPlusRegistry.pages.get(id);
    if (!definition) return false;
    openCodexPlusModalForExtension(id, definition);
    return true;
  }

  /**
   * 渲染拓展页面。
   *
   * 不走 openCodexPlusModal 是因为那个函数的内容区来自内置模板字符串；这里要的是
   * 同一套外壳 + 自定义内容，所以单独走一遍，但外壳结构与类名完全对齐。
   */
  function openCodexPlusModalForExtension(id, definition) {
    closeCodexPlusPage();
    document.querySelectorAll(".codex-plus-modal-overlay").forEach((node) => node.remove());
    document.querySelectorAll(`.${codexPlusPageClass}, [data-codex-plus-dialog="true"]`).forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = codexPlusPageClass;
    overlay.dataset.codexPlusPage = "true";
    overlay.dataset.codexPlusExtensionPage = id;
    applyCodexPlusTheme(overlay);
    // 必须在写 innerHTML 之前设好缩放，否则内部 calc 会先按 1 算一遍（见内置实现注释）。
    applyCodexPlusZoom(overlay);
    overlay.innerHTML = `
      <div class="codex-plus-modal-content" role="dialog" aria-modal="true" aria-label="${escapeHtml(definition.title || "拓展页面")}">
        <div class="codex-plus-modal-header">
          <div class="codex-plus-modal-title"><span class="codex-plus-backend-indicator" data-codex-backend-indicator="true" data-status="checking"></span><span>${escapeHtml(definition.title || "拓展页面")}</span></div>
        </div>
        <div class="codex-plus-modal-body">
          <div class="codex-plus-panel" data-codex-plus-panel="extension" data-codex-plus-extension-panel="${id}"></div>
        </div>
      </div>
    `;
    document.body.appendChild(overlay);
    observeCodexPlusPageLayout(overlay);
    // 拓展入口不在内置的三个 id 里，setCodexPlusSidebarNavActive 认不出来，
    // 所以自己点亮该入口，再调一次 sync 让原生选中态被压下去。
    setCodexPlusExtensionNavActive(id);
    // 与内置页面一致：点图标栏上的任何原生按钮就关掉这个覆盖层。
    //
    // 注意必须连拓展自己的入口一起排除：拓展入口 id 是动态生成的，不在那三个内置
    // id 里，若只排除内置项，点自己的入口会被当成「点了原生按钮」，页面刚打开就
    // 被这条监听关掉。
    const rail = document.querySelector(codexPlusRailSelector);
    rail?.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      if (target?.closest(`#${codexPlusRailNavId}, #${codexPlusRailExtensionsId}, #${codexPlusRailSponsorId}`)) return;
      if (target?.closest(`[${codexPlusExtensionConstants.extensionAttribute}]`)) return;
      if (target?.closest("button, a")) closeCodexPlusPageAfterNativeNavigation();
    }, true);

    const panel = overlay.querySelector(`[data-codex-plus-extension-panel="${id}"]`);
    if (!panel) return;
    // 拓展的 render 每次打开都重新调用，禁止缓存 DOM——见 91 顶部的生命周期约定。
    const outcome = runCodexPlusExtensionCallback(definition.scriptKey, "page.render", () =>
      definition.render({ container: panel, close: () => closeCodexPlusPage(), script: definition.scriptKey }));
    if (!outcome.ok) {
      panel.innerHTML = `<div class="codex-plus-row"><div><div class="codex-plus-row-title">拓展页面加载失败</div><div class="codex-plus-row-description">${escapeHtml(definition.scriptKey || "")}：${escapeHtml(outcome.error)}</div></div></div>`;
      panel.dataset.extensionError = "true";
    }
    definition.onCleanup && runCodexPlusExtensionCallback(definition.scriptKey, "page.onCleanup", () => {
      window.__codexPlusExtensionPageCleanup = definition.onCleanup;
    });
  }

  /**
   * 点亮某个拓展的图标栏入口。
   *
   * 内置的 setCodexPlusSidebarNavActive 只认三个固定 id，拓展入口的 id 是动态的，
   * 所以这里单独处理：先把内置项全部置为未选中，再点亮目标，最后统一压原生选中态。
   */
  function setCodexPlusExtensionNavActive(pageId) {
    setCodexPlusSidebarNavActive(false);
    const entry = codexPlusExtensionItems(codexPlusRegistry.navEntries)
      .find((item) => item.pageId === pageId);
    const elementId = entry ? `codex-plus-ext-rail-${entry.id.replace(/[^\w-]/g, "_")}` : "";
    // 先清掉所有拓展入口的选中态，避免两个页面之间切换时残留。
    document.querySelectorAll('[data-codex-plus-ext-rail-active="true"]').forEach((node) => {
      node.removeAttribute("data-codex-plus-ext-rail-active");
      const button = node.querySelector("button") || node;
      button?.removeAttribute("data-selected");
      button?.removeAttribute("aria-current");
    });
    if (!elementId) return;
    const wrapper = document.getElementById(elementId);
    if (!wrapper) return;
    wrapper.setAttribute("data-codex-plus-ext-rail-active", "true");
    const button = wrapper.querySelector("button") || wrapper;
    button.dataset.active = "true";
    button.setAttribute("aria-current", "page");
    button.setAttribute("data-selected", "");
    // setCodexPlusSidebarNavActive(false) 内部的 sync 是在还没有选中项时跑的，
    // 这里要再跑一次，否则原生选中态压制会基于过期状态。
    syncCodexPlusRailNativeSelection();
  }

  /** 关闭当前拓展页面并执行其 onCleanup。 */
  function closeCodexPlusPage() {
    clearTimeout(window.__codexPlusPageNavigationCloseTimer);
    window.__codexPlusPageNavigationCloseTimer = null;
    const cleanup = window.__codexPlusExtensionPageCleanup;
    window.__codexPlusExtensionPageCleanup = null;
    if (typeof cleanup === "function") {
      try {
        cleanup();
      } catch {}
    }
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    window.__codexPlusPageResizeHandler = null;
    window.__codexPlusPageLayoutObserver?.disconnect();
    window.__codexPlusPageLayoutObserver = null;
    window.__codexPlusPageLayoutTargets = null;
    document.querySelectorAll(`.${codexPlusPageClass}`).forEach((node) => node.remove());
    setCodexPlusSidebarNavActive(false);
    restoreCodexPlusPageBackButton();
  }

  /** 拓展入口的 DOM 标记：用它反查注册表项，dispose 后据此清理。 */
  const codexPlusExtensionRailAttribute = "data-codex-plus-ext-rail";

  /** 拓展入口的稳定 id。用注册表 id 推导，dispose 与重建都能算回同一个值。 */
  function codexPlusExtensionRailElementId(entry) {
    return `codex-plus-ext-rail-${String(entry.id).replace(/[^\w-]/g, "_")}`;
  }

  /**
   * 图标栏上的拓展入口。
   *
   * 与内置的三个入口并列插在 primary 锚点之后。内置项由
   * installCodexPlusRailNavigation 负责，这里只补第三方项，靠 id 幂等。
   */
  function refreshCodexPlusRailNavigation() {
    const rail = document.querySelector(codexPlusRailSelector);
    if (!rail) return false;
    const entries = codexPlusExtensionItems(codexPlusRegistry.navEntries);
    // 注意值域：属性里存的是注册表 id，所以这里也必须用注册表 id 比对。
    // 若拿元素 id（codex-plus-ext-rail-xxx）去比，两边永远不等，每次刷新都会把
    // 自己的入口当孤儿删掉，表现为「点了入口高亮立刻消失」。
    const liveIds = new Set(entries.map((entry) => entry.id));
    // 先清掉已不在注册表里的入口。dispose 之后没人来删 DOM，必须在这里收口，
    // 否则用户点一个已经注销的入口会什么都不发生。
    document.querySelectorAll(`[${codexPlusExtensionRailAttribute}]`).forEach((node) => {
      if (!liveIds.has(node.getAttribute(codexPlusExtensionRailAttribute) || "")) node.remove();
    });
    if (!entries.length) return false;
    const anchor = codexPlusRailPrimaryAnchor(rail);
    const host = anchor?.parentElement || rail;
    const template = codexPlusRailTemplateButton(rail);
    let cursor = anchor;
    // 内置三项先占位，第三方从它们之后开始排。
    [codexPlusRailNavId, codexPlusRailExtensionsId, codexPlusRailSponsorId].forEach((id) => {
      const node = document.getElementById(id);
      if (node) cursor = node;
    });
    entries.forEach((entry) => {
      const elementId = codexPlusExtensionRailElementId(entry);
      let wrapper = document.getElementById(elementId);
      if (!wrapper || wrapper.parentElement !== host) {
        wrapper?.remove();
        wrapper = createCodexPlusRailButton({
          id: elementId,
          template,
          label: entry.label || entry.id,
          iconMarkup: codexPlusExtensionIconMarkup(entry),
          withStatus: false,
          onActivate: () => {
            // 注册时若带了 pageId 就打开对应页面；否则交给拓展自己的 onActivate。
            const navigate = () => {
              if (entry.pageId && codexPlusRegistry.pages.has(entry.pageId)) {
                entry.navId = elementId;
                openCodexPlusExtensionPage(entry.pageId);
              } else if (typeof entry.onActivate === "function") {
                runCodexPlusExtensionCallback(entry.scriptKey, "navEntry.onActivate", () => entry.onActivate());
              }
            };
            navigate();
          },
        });
        if (!wrapper) return;
        // 这个属性是 dispose 后清理 DOM 的唯一线索，必须写。只靠
        // data-codex-plus-ext 认不出「这是 rail 入口」还是别的什么扩展节点。
        wrapper.setAttribute(codexPlusExtensionRailAttribute, entry.id);
        markCodexPlusExtensionNode(wrapper, entry.scriptKey);
        markCodexPlusExtensionNode(wrapper.firstElementChild || wrapper, entry.scriptKey);
      }
      if (cursor?.nextSibling) {
        if (cursor.nextSibling !== wrapper) host.insertBefore(wrapper, cursor.nextSibling);
      } else if (cursor) {
        host.appendChild(wrapper);
      }
      cursor = wrapper;
    });
    return true;
  }

  /**
   * 输入区按钮插槽：拓展用 registerComposerAction 注册的按钮，插在输入区 footer 里。
   *
   * footer 的选择沿用听写入口那套打分（同一输入框只挂一份、优先包含输入框和发送按钮的
   * footer），Codex 改结构时两边一起修。按注册表指纹决定是否重建，避免每轮扫描都改 DOM。
   */
  function codexPlusComposerActionHosts() {
    const hosts = new Map();
    document.querySelectorAll(".composer-footer, [data-composer-footer-responsive]").forEach((footer) => {
      if (!visibleElement(footer) || isExtensionUiNode(footer)) return;
      const scope = codexPlusDictationScope(footer);
      const editor = scope && codexPlusDictationEditor(scope);
      if (!editor) return;
      const score = codexPlusDictationFooterScore(footer, editor);
      const current = hosts.get(editor);
      if (!current || score >= current.score) hosts.set(editor, { footer, editor, score });
    });
    return [...hosts.values()];
  }

  /** 把文字插入输入框：先聚焦，再走浏览器的 insertText，让编辑器自己处理撤销栈与状态。 */
  function codexPlusInsertComposerText(editor, text) {
    if (!editor || typeof text !== "string" || !text) return false;
    editor.focus();
    return document.execCommand("insertText", false, text);
  }

  function refreshCodexPlusComposerActions() {
    const items = codexPlusExtensionItems(codexPlusRegistry.composerActions);
    const hosts = items.length ? codexPlusComposerActionHosts() : [];
    const hostFooters = new Set(hosts.map((host) => host.footer));
    document.querySelectorAll("[data-codex-plus-composer-actions]").forEach((node) => {
      if (!hostFooters.has(node.parentElement)) node.remove();
    });
    if (!items.length) return;
    if (!document.getElementById("codex-plus-composer-actions-style")) {
      const style = document.createElement("style");
      style.id = "codex-plus-composer-actions-style";
      markCodexPlusExtensionNode(style, "builtin-composer-actions");
      style.textContent = `[data-codex-plus-composer-actions]{display:inline-flex;align-items:center;gap:2px;margin-inline:2px}
        .codex-plus-composer-action{display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;padding:0;border:0;border-radius:8px;background:transparent;color:var(--color-token-text-primary,currentColor);opacity:.9;cursor:pointer;font-size:15px;line-height:1}
        .codex-plus-composer-action:hover{background:color-mix(in srgb,currentColor 12%,transparent);opacity:1}
        .codex-plus-composer-action:focus-visible{outline:2px solid currentColor;outline-offset:2px}
        .codex-plus-composer-action svg{width:16px;height:16px}`;
      document.head.append(style);
    }
    const renderKey = items.map((item) => `${item.id}|${item.label || ""}|${item.icon || ""}`).join("\n");
    for (const { footer, editor } of hosts) {
      let root = [...footer.children].find((node) => node.hasAttribute?.("data-codex-plus-composer-actions"));
      if (!root) {
        root = document.createElement("div");
        root.setAttribute("data-codex-plus-composer-actions", "");
        markCodexPlusExtensionNode(root, "builtin-composer-actions");
        // 放在听写入口前面；没有听写入口就追加在末尾。
        const dictation = [...footer.children].find((node) => node.hasAttribute?.("data-dictation-controls"));
        footer.insertBefore(root, dictation || null);
      }
      root.__codexPlusComposerEditor = editor;
      if (root.dataset.renderKey === renderKey) continue;
      root.dataset.renderKey = renderKey;
      root.textContent = "";
      for (const item of items) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "codex-plus-composer-action";
        button.setAttribute("aria-label", item.label || item.id);
        markCodexPlusExtensionNode(button, item.scriptKey);
        button.innerHTML = codexPlusExtensionIconMarkup(item);
        attachCodexPlusRailTooltip(button, item.label || item.id, "top");
        // 按下时不抢走输入框焦点，否则插入文字会落在别处。
        button.addEventListener("pointerdown", (event) => event.preventDefault());
        button.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          const target = root.__codexPlusComposerEditor;
          runCodexPlusExtensionCallback(item.scriptKey, "composerAction.onActivate", () => item.onActivate?.({
            editor: target,
            insertText: (text) => codexPlusInsertComposerText(target, text),
            close: () => hideCodexPlusRailTooltip(),
          }));
        });
        root.appendChild(button);
      }
    }
  }

  /**
   * 斜杠命令：拓展用 registerCommand 注册，在 Codex 输入框里「/name 参数」回车触发。
   *
   * 键盘监听挂在 document 的捕获阶段，先于输入框自己的处理；只拦截「整段输入恰好是一条已注册
   * 拓展命令」的回车，其它输入（包括 Codex 自己的斜杠命令）原样放行。命令名按注册表查，
   * 同名时先注册的生效。提示浮层带拓展标记，不触发扫描自喂。
   */
  const codexPlusCommandPattern = /^\/([A-Za-z0-9][\w-]*)(?:\s+([\s\S]*))?$/;

  function codexPlusRegisterExtensionCommand(definition, options = {}) {
    const name = String(definition?.name || "").trim();
    if (!/^[A-Za-z0-9][\w-]{0,31}$/.test(name)) throw new Error("命令名只能是字母、数字、_、-，最长 32 个字符");
    if (typeof definition?.run !== "function") throw new Error(`命令 /${name} 必须提供 run(args, context)`);
    const dispose = codexPlusRegisterExtensionItem("command", codexPlusRegistry.commands, { ...definition, name }, { ...options, id: options.id || `cmd-${name}` });
    installCodexPlusCommandHandlers();
    return dispose;
  }

  /** 当前打开的 Codex 会话 id（首页、新对话还没有 id 时为空串）。 */
  function codexPlusCurrentSessionId() {
    try {
      const fromRow = normalizedCodexThreadUuid(currentSessionRef()?.session_id);
      if (fromRow) return fromRow;
    } catch {}
    // 侧栏收起或当前会话不在列表里时，取可见输入框上方标着的会话 id（多个标签页只认看得见的那个）。
    const marked = [...document.querySelectorAll("[data-above-composer-conversation-id]")]
      // 这个标记节点本身常是空的（empty:hidden），按它所在输入框是否可见判断。
      .find((node) => node.closest("[data-codex-composer-root]")?.getBoundingClientRect().width > 0);
    return normalizedCodexThreadUuid(marked?.getAttribute("data-above-composer-conversation-id")) || "";
  }

  function codexPlusFindExtensionCommand(name) {
    return codexPlusExtensionItems(codexPlusRegistry.commands).find((item) => item.name === name) || null;
  }

  function codexPlusComposerEditorFrom(target) {
    const editor = target?.closest?.("[data-codex-composer], .ProseMirror, [contenteditable=true]");
    return editor && !isExtensionUiNode(editor) ? editor : null;
  }

  function codexPlusClearComposer(editor) {
    editor.focus();
    document.execCommand("selectAll");
    document.execCommand("delete");
  }

  function hideCodexPlusCommandHints() {
    document.getElementById("codex-plus-command-hints")?.remove();
  }

  /** 输入「/前缀」时在输入框上方列出匹配的拓展命令。 */
  function showCodexPlusCommandHints(editor, prefix) {
    const matches = codexPlusExtensionItems(codexPlusRegistry.commands)
      .filter((item) => item.name.startsWith(prefix))
      .slice(0, 8);
    if (!matches.length) {
      hideCodexPlusCommandHints();
      return;
    }
    let panel = document.getElementById("codex-plus-command-hints");
    if (!panel) {
      panel = document.createElement("div");
      panel.id = "codex-plus-command-hints";
      panel.setAttribute("role", "listbox");
      markCodexPlusExtensionNode(panel, "builtin-commands");
      document.body.appendChild(panel);
    }
    panel.textContent = "";
    for (const item of matches) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "codex-plus-command-hint";
      row.setAttribute("role", "option");
      row.innerHTML = `<strong>/${escapeHtml(item.name)}</strong><span>${escapeHtml(item.description || "")}</span>`;
      row.addEventListener("pointerdown", (event) => event.preventDefault());
      row.addEventListener("click", () => {
        codexPlusClearComposer(editor);
        document.execCommand("insertText", false, `/${item.name} `);
        hideCodexPlusCommandHints();
      });
      panel.appendChild(row);
    }
    const rect = editor.getBoundingClientRect();
    panel.style.left = `${Math.round(rect.left)}px`;
    panel.style.bottom = `${Math.round(window.innerHeight - rect.top + 8)}px`;
    panel.style.minWidth = `${Math.round(Math.min(rect.width, 420))}px`;
  }

  // 「已装过」记在本份注入代码里，不记在 window 上：重建后新代码注入到没刷新的页面时，
  // window 上的标记是旧代码留下的，照它跳过就只剩读旧注册表的旧监听器。旧监听器按 window 上存的引用摘掉。
  let codexPlusCommandHandlersInstalled = false;

  function installCodexPlusCommandHandlers() {
    if (codexPlusCommandHandlersInstalled) return;
    codexPlusCommandHandlersInstalled = true;
    if (!document.getElementById("codex-plus-command-hints-style")) {
      const style = document.createElement("style");
      style.id = "codex-plus-command-hints-style";
      markCodexPlusExtensionNode(style, "builtin-commands");
      style.textContent = `#codex-plus-command-hints{position:fixed;z-index:2147483645;display:flex;flex-direction:column;gap:2px;padding:6px;border-radius:12px;border:1px solid color-mix(in srgb,currentColor 14%,transparent);background:var(--color-token-main-surface-primary,var(--codex-plus-bg-elevated,#262626));color:var(--color-token-text-primary,var(--codex-plus-text,#eee));box-shadow:0 8px 24px rgba(0,0,0,.25);font-size:13px}
        .codex-plus-command-hint{display:flex;gap:10px;align-items:baseline;padding:6px 10px;border:0;border-radius:8px;background:transparent;color:inherit;text-align:left;cursor:pointer}
        .codex-plus-command-hint:hover,.codex-plus-command-hint:focus-visible{background:color-mix(in srgb,currentColor 10%,transparent)}
        .codex-plus-command-hint span{opacity:.65}`;
      document.head.append(style);
    }
    const onKeyDown = (event) => {
      if (event.isComposing) return;
      const editor = codexPlusComposerEditorFrom(event.target);
      if (!editor) return;
      const text = (editor.innerText || "").trim();
      if (event.key === "Escape") {
        hideCodexPlusCommandHints();
        return;
      }
      if (event.key === "Tab" && document.getElementById("codex-plus-command-hints") && /^\/[\w-]*$/.test(text)) {
        const first = codexPlusExtensionItems(codexPlusRegistry.commands).find((item) => item.name.startsWith(text.slice(1)));
        if (!first) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        codexPlusClearComposer(editor);
        document.execCommand("insertText", false, `/${first.name} `);
        hideCodexPlusCommandHints();
        return;
      }
      if (event.key !== "Enter" || event.shiftKey || event.altKey || event.ctrlKey || event.metaKey) return;
      const match = codexPlusCommandPattern.exec(text);
      const command = match && codexPlusFindExtensionCommand(match[1]);
      if (!command) return;
      // 是拓展命令：不让 Codex 把它当消息发出去。
      event.preventDefault();
      event.stopImmediatePropagation();
      hideCodexPlusCommandHints();
      codexPlusClearComposer(editor);
      const result = runCodexPlusExtensionCallback(command.scriptKey, `command./${command.name}`, () => command.run(match[2] || "", {
        editor,
        insertText: (value) => codexPlusInsertComposerText(editor, value),
        setText: (value) => {
          codexPlusClearComposer(editor);
          return codexPlusInsertComposerText(editor, value);
        },
        toast: (message, toastOptions) => showToast(String(message ?? ""), toastOptions || {}),
        sessionId: codexPlusCurrentSessionId(),
      }));
      if (result.ok && result.value && typeof result.value.then === "function") {
        result.value.catch((error) => codexPlusMarkExtensionFailure(command.scriptKey, `command./${command.name}: ${String(error?.stack || error?.message || error)}`));
      }
    };
    const onInput = (event) => {
      const editor = codexPlusComposerEditorFrom(event.target);
      if (!editor) return;
      const text = (editor.innerText || "").trim();
      const prefix = /^\/([\w-]*)$/.exec(text);
      if (prefix && codexPlusRegistry.commands.size) showCodexPlusCommandHints(editor, prefix[1]);
      else hideCodexPlusCommandHints();
    };
    document.removeEventListener("keydown", window.__codexPlusCommandKeyHandler, true);
    document.removeEventListener("input", window.__codexPlusCommandInputHandler, true);
    window.__codexPlusCommandKeyHandler = onKeyDown;
    window.__codexPlusCommandInputHandler = onInput;
    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("input", onInput, true);
  }

  /**
   * 拓展注册的菜单项。
   *
   * 接入方式是「在 home 面板末尾追加一块」而不是把内置的一百多行模板拆成数组——
   * 拆模板动的是内置 UI 主干，出问题会影响所有人；追加只影响新内容，回滚时删掉
   * 这个调用即可。
   *
   * 每次 openCodexPlusModal 都会重新调用，所以不需要在别处维护刷新逻辑。
   */
  function renderCodexPlusExtensionMenuRows() {
    const items = codexPlusExtensionItems(codexPlusRegistry.menuItems);
    if (!items.length) return "";
    const rows = items.map((item) => {
      const title = escapeHtml(item.label || item.id);
      const description = escapeHtml(item.description || "");
      // 有 onChange 的渲染成开关，否则渲染成动作按钮。
      let control;
      if (typeof item.onChange === "function") {
        const enabled = typeof item.toggleValue === "function" ? item.toggleValue() === true : false;
        control = `<button type="button" class="codex-plus-toggle" data-codex-plus-ext-setting="${escapeHtml(item.id)}" data-enabled="${String(enabled)}" aria-pressed="${String(enabled)}"><span></span></button>`;
      } else {
        control = `<button type="button" class="codex-plus-action-button" data-codex-plus-ext-action="${escapeHtml(item.id)}">${escapeHtml(item.buttonLabel || "打开")}</button>`;
      }
      return `<div class="codex-plus-row" data-codex-plus-ext-row="${escapeHtml(item.id)}">`
        + `<div><div class="codex-plus-row-title">${title}</div>`
        + (description ? `<div class="codex-plus-row-description">${description}</div>` : "")
        + `</div>${control}</div>`;
    }).join("");
    // 整块包一层：dispose 后能一次性摘掉，测试也好定位。
    return `<div data-codex-plus-ext-menu="true">${rows}</div>`;
  }

  /**
   * 处理拓展菜单项的点击。
   *
   * 由 openCodexPlusModal 的委托监听调用；返回 true 表示已处理，调用方应 return。
   */
  function handleCodexPlusExtensionMenuClick(target) {
    const action = target?.closest?.("[data-codex-plus-ext-action]");
    if (action) {
      const id = action.getAttribute("data-codex-plus-ext-action") || "";
      const item = codexPlusRegistry.menuItems.get(id);
      if (!item) return true;
      runCodexPlusExtensionCallback(item.scriptKey, "menuItem.onActivate", () =>
        item.onActivate({ close: () => document.querySelector(".codex-plus-modal-close")?.click() }));
      return true;
    }
    const toggle = target?.closest?.("[data-codex-plus-ext-setting]");
    if (toggle) {
      const id = toggle.getAttribute("data-codex-plus-ext-setting") || "";
      const item = codexPlusRegistry.menuItems.get(id);
      if (!item) return true;
      const next = toggle.getAttribute("data-enabled") !== "true";
      toggle.setAttribute("data-enabled", String(next));
      toggle.setAttribute("aria-pressed", String(next));
      runCodexPlusExtensionCallback(item.scriptKey, "menuItem.onChange", () => item.onChange(next));
      return true;
    }
    return false;
  }

  /**
   * 会话行「更多操作」里的拓展项。
   *
   * 由 attachButton 在构建 moreMenu 时调用。返回的节点直接 append 进菜单，
   * 所以样式与内置项一致；点击后关闭菜单再执行回调。
   */
  function appendCodexPlusExtensionRowActions(moreMenu, row, moreButton) {
    const items = sessionRowActionItems().filter((item) => item.placement !== "inline");
    if (!items.length) return;
    items.forEach((definition) => {
      // 自绘项：render({ container, session_id, title, close }) 往菜单里的一个容器画（返回的清理函数在菜单拆掉时调用）。
      if (typeof definition.render === "function") {
        const item = document.createElement("div");
        item.className = "codex-session-more-menu-custom";
        item.dataset.codexPlusExtensionItem = definition.id;
        markCodexPlusExtensionNode(item, definition.scriptKey);
        moreMenu.appendChild(item);
        const ref = sessionRefFromRow(row);
        const result = runCodexPlusExtensionCallback(definition.scriptKey, "rowAction.render", () => definition.render({
          container: item,
          row,
          session_id: ref.session_id,
          title: ref.title || row.getAttribute("data-app-action-sidebar-thread-title") || "",
          close: () => closeSessionMoreMenus(),
        }));
        if (result.ok && typeof result.value === "function") item.__codexPlusRowActionCleanup = result.value;
        return;
      }
      const item = createSessionMoreMenuItem(definition.label || definition.id, definition.icon || "◇", (event) => {
        stopActionButtonEvent(row, moreButton, event);
        closeSessionMoreMenus();
        runCodexPlusExtensionCallback(definition.scriptKey, "rowAction.onActivate", () =>
          definition.onActivate(sessionRowActionContext(row, { event })));
      });
      // 加分隔线：拓展项与内置项在语义上没有关联，挨着排会让人以为是一组。
      item.dataset.codexPlusExtensionItem = definition.id;
      markCodexPlusExtensionNode(item, definition.scriptKey);
      moreMenu.appendChild(item);
    });
  }
