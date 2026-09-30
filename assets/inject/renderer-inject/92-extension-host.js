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
    positionCodexPlusPage(overlay);
    // 拓展入口不在内置的三个 id 里，setCodexPlusSidebarNavActive 认不出来，
    // 所以自己点亮该入口，再调一次 sync 让原生选中态被压下去。
    setCodexPlusExtensionNavActive(id);
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    window.__codexPlusPageResizeHandler = () => positionCodexPlusPage(overlay);
    window.addEventListener("resize", window.__codexPlusPageResizeHandler);
    // 与内置页面一致：点图标栏上的任何原生按钮就关掉这个覆盖层。
    const rail = document.querySelector(codexPlusRailSelector);
    rail?.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      if (target?.closest(`#${codexPlusRailNavId}, #${codexPlusRailExtensionsId}, #${codexPlusRailSponsorId}`)) return;
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
    const cleanup = window.__codexPlusExtensionPageCleanup;
    window.__codexPlusExtensionPageCleanup = null;
    if (typeof cleanup === "function") {
      try {
        cleanup();
      } catch {}
    }
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    document.querySelectorAll(`.${codexPlusPageClass}`).forEach((node) => node.remove());
    setCodexPlusSidebarNavActive(false);
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
      const elementId = `codex-plus-ext-rail-${entry.id.replace(/[^\w-]/g, "_")}`;
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
   * 会话行「更多操作」里的拓展项。
   *
   * 由 attachButton 在构建 moreMenu 时调用。返回的节点直接 append 进菜单，
   * 所以样式与内置项一致；点击后关闭菜单再执行回调。
   */
  function appendCodexPlusExtensionRowActions(moreMenu, row, moreButton) {
    const items = codexPlusExtensionItems(codexPlusRegistry.rowActions);
    if (!items.length) return;
    items.forEach((definition) => {
      const item = createSessionMoreMenuItem(definition.label || definition.id, definition.icon || "◇", (event) => {
        stopActionButtonEvent(row, moreButton, event);
        closeSessionMoreMenus();
        runCodexPlusExtensionCallback(definition.scriptKey, "rowAction.onActivate", () =>
          definition.onActivate({
            row,
            session_id: sessionRefFromRow(row).session_id,
            close: () => closeSessionMoreMenus(),
          }));
      });
      // 加分隔线：拓展项与内置项在语义上没有关联，挨着排会让人以为是一组。
      item.dataset.codexPlusExtensionItem = definition.id;
      markCodexPlusExtensionNode(item, definition.scriptKey);
      moreMenu.appendChild(item);
    });
  }
