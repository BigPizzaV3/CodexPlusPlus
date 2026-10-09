  function syncActionGroupsLayout() {
    sessionRows().forEach((row) => {
      const group = actionGroupFromRow(row);
      if (group) syncActionGroupLayout(row, group);
    });
  }

  function removeActionGroups(row) {
    document.querySelectorAll(`.${moreMenuClass}`).forEach((menu) => {
      if (menu.__codexSessionMoreRow === row) menu.remove();
    });
    row.querySelectorAll(`.${actionGroupClass}`).forEach((group) => group.remove());
  }

  function stopActionButtonEvent(row, button, event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    releaseDeleteFocus(row, button);
  }

  function installActionButtonEvents(row, button, onActivate) {
    ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
      button.addEventListener(eventName, (event) => stopActionButtonEvent(row, button, event), true);
    });
    button.addEventListener("pointerenter", () => showActionButtonTooltip(button));
    button.addEventListener("pointerleave", hideActionButtonTooltip);
    button.addEventListener("focus", () => showActionButtonTooltip(button));
    button.addEventListener("blur", hideActionButtonTooltip);
    button.addEventListener("click", (event) => {
      hideActionButtonTooltip();
      onActivate(event);
    }, true);
  }

  function installMoreButtonEvents(row, button, onActivate) {
    ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
      button.addEventListener(eventName, (event) => stopActionButtonEvent(row, button, event), true);
    });
    button.addEventListener("pointerup", onActivate, true);
    button.addEventListener("click", (event) => {
      hideActionButtonTooltip();
      stopActionButtonEvent(row, button, event);
    }, true);
  }

  function hideActionButtonTooltip() {
    document.querySelectorAll(`.${actionTooltipClass}`).forEach((node) => node.remove());
  }

  function closeSessionMoreMenus(exceptMenu = null) {
    document.querySelectorAll(`.${moreMenuClass}`).forEach((menu) => {
      if (menu !== exceptMenu) {
        menu.hidden = true;
        menu.closest?.("[data-codex-delete-row]")?.classList.remove("codex-session-more-open");
        menu.__codexSessionMoreRow?.classList?.remove("codex-session-more-open");
      }
    });
  }

  function toggleSessionMoreMenu(row, button, menu) {
    const nextHidden = !menu.hidden;
    closeSessionMoreMenus(menu);
    menu.hidden = nextHidden;
    row.classList.toggle("codex-session-more-open", !menu.hidden);
    button.setAttribute("aria-expanded", String(!menu.hidden));
  }

  function installSessionMoreMenuAutoClose(row, menu) {
    const group = menu.__codexSessionMoreGroup || menu.closest?.(`.${actionGroupClass}`);
    const closeIfOutside = () => {
      window.setTimeout(() => {
        if (menu.hidden) return;
        const active = document.activeElement;
        if (group?.matches?.(":hover") || menu.matches?.(":hover") || menu.contains(active)) return;
        menu.hidden = true;
        row.classList.remove("codex-session-more-open");
        group?.querySelector?.(`.${moreButtonClass}`)?.setAttribute("aria-expanded", "false");
      }, 80);
    };
    group?.addEventListener("pointerleave", closeIfOutside, true);
    menu.addEventListener("pointerleave", closeIfOutside, true);
    menu.addEventListener("focusout", closeIfOutside, true);
  }

  function updateSessionMoreMenuDirection(button, menu) {
    menu.classList.remove("codex-session-more-menu-open-up");
    const buttonRect = button.getBoundingClientRect();
    const estimatedMenuHeight = Math.max(80, menu.getBoundingClientRect().height || 76);
    if (buttonRect.bottom + 30 + estimatedMenuHeight > window.innerHeight - 8) {
      menu.classList.add("codex-session-more-menu-open-up");
    }
  }

  function positionSessionMoreMenu(button, menu) {
    const rect = button.getBoundingClientRect();
    const menuWidth = Math.max(104, menu.getBoundingClientRect().width || 104);
    const left = Math.min(window.innerWidth - menuWidth - 8, Math.max(8, rect.right - menuWidth));
    menu.style.left = `${left}px`;
    menu.style.top = `${Math.max(8, rect.bottom + 4)}px`;
  }

  function createSessionMoreMenuItem(label, icon, onActivate) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "codex-session-more-menu-item";
    item.innerHTML = `<span class="codex-session-more-menu-icon">${icon}</span><span>${label}</span>`;
    item.addEventListener("click", onActivate, true);
    return item;
  }

  function showActionButtonTooltip(button) {
    const label = button.dataset.codexActionLabel || button.getAttribute("aria-label") || "";
    if (!label) return;
    hideActionButtonTooltip();
    const tooltip = document.createElement("div");
    tooltip.className = actionTooltipClass;
    tooltip.textContent = label;
    document.body.appendChild(tooltip);
    const buttonRect = button.getBoundingClientRect();
    const tooltipRect = tooltip.getBoundingClientRect();
    const gap = 8;
    const left = Math.min(
      window.innerWidth - tooltipRect.width - 8,
      Math.max(8, buttonRect.left + buttonRect.width / 2 - tooltipRect.width / 2),
    );
    const top = Math.min(
      window.innerHeight - tooltipRect.height - 8,
      buttonRect.bottom + gap,
    );
    tooltip.style.left = `${left}px`;
    tooltip.style.top = `${Math.max(8, top)}px`;
  }

  function refreshActionButton(originalButton, row, onActivate) {
    if (!originalButton.isConnected) return;
    const replacement = originalButton.cloneNode(true);
    installActionButtonEvents(row, replacement, onActivate);
    originalButton.replaceWith(replacement);
    return replacement;
  }

  function configureActionButton(button, label, icon) {
    button.setAttribute("aria-label", label);
    button.dataset.codexActionLabel = label;
    button.removeAttribute("title");
    button.textContent = icon;
  }

  function trashIconSvg() {
    return `
      <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 6h18"></path>
        <path d="M8 6V4h8v2"></path>
        <path d="M19 6l-1 14H6L5 6"></path>
        <path d="M10 11v5"></path>
        <path d="M14 11v5"></path>
      </svg>
    `;
  }

  function configureSvgActionButton(button, label, svg) {
    button.setAttribute("aria-label", label);
    button.dataset.codexActionLabel = label;
    button.removeAttribute("title");
    button.innerHTML = svg;
  }

  function attachButton(row) {
    const settings = codexPlusSettings();
    const sessionMenuEnabled = codexPlusBackendSettings.enhancementsEnabled !== false;
    if (!settings.sessionDelete && !settings.markdownExport && !sessionMenuEnabled) {
      removeActionGroups(row);
      row.dataset.codexDeleteRow = "false";
      return;
    }
    const existingGroup = actionGroupFromRow(row);
    const existingDeleteButton = existingGroup?.querySelector(`.${buttonClass}`);
    const existingMoreButton = existingGroup?.querySelector(`.${moreButtonClass}`);
    const existingExportButton = existingGroup?.querySelector(`.${exportButtonClass}`);
    const needsMoreMenu = sessionMenuEnabled;
    const hasUnexpectedDelete = !settings.sessionDelete && !!existingDeleteButton;
    const hasUnexpectedMore = !needsMoreMenu && !!existingMoreButton;
    const hasUnexpectedExport = !!existingExportButton;
    const missingDelete = settings.sessionDelete && !existingDeleteButton;
    const missingMore = needsMoreMenu && !existingMoreButton;
    const deleteReady = !settings.sessionDelete || existingDeleteButton?.dataset.codexDeleteVersion === codexDeleteVersion;
    const groupReady = existingGroup?.dataset.codexActionGroupVersion === codexActionGroupVersion;
    if (groupReady && deleteReady && !hasUnexpectedDelete && !hasUnexpectedMore && !hasUnexpectedExport && !missingDelete && !missingMore) {
      return;
    }
    removeActionGroups(row);
    row.dataset.codexDeleteRow = "false";
    const ref = sessionRefFromRow(row);
    if (!ref.session_id) return;
    row.dataset.codexDeleteRow = "true";
    const group = document.createElement("div");
    group.className = actionGroupClass;
    group.dataset.codexActionGroupVersion = codexActionGroupVersion;
    if (needsMoreMenu) {
      const moreButton = document.createElement("button");
      moreButton.type = "button";
      moreButton.className = `${actionButtonClass} ${moreButtonClass}`;
      moreButton.setAttribute("aria-haspopup", "menu");
      moreButton.setAttribute("aria-expanded", "false");
      configureActionButton(moreButton, "更多操作", "…");
      const moreMenu = document.createElement("div");
      moreMenu.className = moreMenuClass;
      moreMenu.setAttribute("role", "menu");
      moreMenu.hidden = true;
      if (settings.markdownExport) {
        moreMenu.appendChild(createSessionMoreMenuItem("导出", "⇩", (event) => {
          stopActionButtonEvent(row, moreButton, event);
          closeSessionMoreMenus();
          exportMarkdown(ref);
        }));
      }
      if (sessionMenuEnabled) {
        const sessionCopyItem = createSessionMoreMenuItem("原地复制会话 - Codex++", "⧉", activateSessionCopyMenuItem);
        sessionCopyItem.dataset.codexSessionCopyMenu = "true";
        sessionCopyItem.dataset.codexSessionCopyVersion = sessionCopyMenuItemVersion;
        sessionCopyItem.__codexSessionCopyRow = row;
        moreMenu.appendChild(sessionCopyItem);
        const sessionAutoRenameItem = createSessionMoreMenuItem("自动重命名当前会话", "✦", activateSessionAutoRenameMenuItem);
        sessionAutoRenameItem.dataset.codexSessionAutoRenameMenu = "true";
        sessionAutoRenameItem.__codexSessionAutoRenameRow = row;
        moreMenu.appendChild(sessionAutoRenameItem);
      }
      // 拓展注册的会话行操作追加在内置项之后。菜单每次重建（版本号变化）都会
      // 重新走一遍这里，所以拓展项不会因为重建而丢失。
      appendCodexPlusExtensionRowActions(moreMenu, row, moreButton);
      const openMoreMenu = (event) => {
        stopActionButtonEvent(row, moreButton, event);
        hideActionButtonTooltip();
        toggleSessionMoreMenu(row, moreButton, moreMenu);
        if (!moreMenu.hidden) {
          positionSessionMoreMenu(moreButton, moreMenu);
          updateSessionMoreMenuDirection(moreButton, moreMenu);
        }
      };
      installMoreButtonEvents(row, moreButton, openMoreMenu);
      group.appendChild(moreButton);
      moreMenu.__codexSessionMoreRow = row;
      moreMenu.__codexSessionMoreGroup = group;
      document.body.appendChild(moreMenu);
      installSessionMoreMenuAutoClose(row, moreMenu);
    }
    if (settings.sessionDelete) {
      const deleteButton = document.createElement("button");
      deleteButton.type = "button";
      deleteButton.className = `${actionButtonClass} ${buttonClass}`;
      deleteButton.dataset.codexDeleteVersion = codexDeleteVersion;
      configureSvgActionButton(deleteButton, "删除", trashIconSvg());
      const openDeleteConfirm = (event) => openDeleteConfirmForRow(row, deleteButton, sessionRefFromRow(row), event);
      installActionButtonEvents(row, deleteButton, openDeleteConfirm);
      group.appendChild(deleteButton);
      setTimeout(() => refreshActionButton(deleteButton, row, openDeleteConfirm), 0);
    }
    row.appendChild(group);
    syncActionGroupLayout(row, group);
  }

  function tryAttachButton(row) {
    try {
      attachButton(row);
    } catch (error) {
      window.__codexSessionDeleteAttachButtonFailures = window.__codexSessionDeleteAttachButtonFailures || [];
      window.__codexSessionDeleteAttachButtonFailures.push(String(error?.stack || error));
    }
  }

  function reactArchivedThreadFromNode(node) {
    const reactKey = Object.keys(node).find((key) => key.startsWith("__reactFiber$") || key.startsWith("__reactInternalInstance$"));
    let fiber = reactKey ? node[reactKey] : null;
    for (let depth = 0; fiber && depth < 20; depth += 1, fiber = fiber.return) {
      const props = fiber.memoizedProps || fiber.pendingProps || {};
      if (props.archivedThread?.id) return props.archivedThread;
      const childThread = props.children?.props?.archivedThread;
      if (childThread?.id) return childThread;
    }
    return null;
  }

  function archivedThreadFromRow(row) {
    for (const node of [row, ...row.querySelectorAll("*")]) {
      const thread = reactArchivedThreadFromNode(node);
      if (thread?.id || thread?.sessionId) return thread;
    }
    return null;
  }

  function archivedRefFromRow(row) {
    const archivedThread = archivedThreadFromRow(row);
    if (archivedThread?.id || archivedThread?.sessionId) {
      return { session_id: archivedThread.id || archivedThread.sessionId, title: archivedThread.title || row.querySelector(".truncate.text-base")?.textContent?.trim() || "Untitled session" };
    }
    const sidebarRef = sessionRefFromRow(row);
    if (sidebarRef.session_id) return sidebarRef;
    const titleNode = row.querySelector(".truncate.text-base, [data-thread-title], a, div");
    const title = ((titleNode || row).textContent || "Untitled session")
      .replace("取消归档", "")
      .replace("删除", "")
      .replace(/\d{4}年\d{1,2}月\d{1,2}日.*$/, "")
      .replace(/\s+·\s+.*$/, "")
      .trim()
      .slice(0, 160);
    return { session_id: "", title };
  }

  async function resolveArchivedThread(row) {
    const ref = archivedRefFromRow(row);
    if (ref.session_id) return ref;
    const resolved = await postJson("/archived-thread", { title: ref.title });
    return resolved?.session_id ? resolved : ref;
  }

  function stopArchivedButtonEvent(event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
  }

  function attachArchivedPageDeleteButton(row) {
    const settings = codexPlusSettings();
    row.querySelectorAll("[data-codex-archive-row-action]").forEach((button) => button.remove());
    row.dataset.codexArchiveDeleteRow = "false";
    if (!settings.sessionDelete && !settings.markdownExport) return;
    const unarchiveButton = Array.from(row.querySelectorAll("button")).find((button) => (button.textContent || "").trim() === "取消归档");
    if (!unarchiveButton) return;
    row.dataset.codexArchiveDeleteRow = "true";
    row.dataset.codexArchiveRowActionsVersion = codexArchiveRowActionsVersion;
    let insertionPoint = unarchiveButton;
    if (settings.markdownExport) {
      const exportButton = document.createElement("button");
      exportButton.type = "button";
      exportButton.className = `codex-archive-delete-all codex-archive-row-button ${exportButtonClass}`;
      exportButton.dataset.codexArchiveRowAction = "export";
      exportButton.textContent = "导出";
      ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
        exportButton.addEventListener(eventName, stopArchivedButtonEvent, true);
      });
      exportButton.addEventListener("click", async (event) => {
        stopArchivedButtonEvent(event);
        const ref = await resolveArchivedThread(row);
        if (!ref.session_id) {
          showToast("导出失败：未找到归档会话 ID", null);
          return;
        }
        await exportMarkdown(ref);
      }, true);
      insertionPoint.insertAdjacentElement("afterend", exportButton);
      insertionPoint = exportButton;
    }
  }

  function conversationRoot() {
    return document.querySelector(".thread-scroll-container") || document.querySelector("main") || document.querySelector('[role="main"]');
  }

  function nodeOrAncestorLooksLikeCodexUserBubble(node) {
    if (node.nodeType !== 1) return false;
    const className = String(node.className || "");
    if (className.includes("bg-token-foreground/5") && node.parentElement?.classList?.contains("items-end")) return true;
    const bubble = node.closest?.("[class*='bg-token-foreground/5']");
    return !!bubble?.parentElement?.classList?.contains("items-end");
  }

  function nodeLooksLikeCodexUserBubble(node) {
    if (nodeOrAncestorLooksLikeCodexUserBubble(node)) return true;
    return !!node.querySelector?.(".group.flex.w-full.flex-col.items-end.justify-end.gap-1 > [class*='bg-token-foreground/5']");
  }

  function scrollerViewportTop(scroller) {
    if (scroller === document.scrollingElement || scroller === document.documentElement || scroller === document.body) return 0;
    return scroller.getBoundingClientRect().top;
  }

  function nearestScrollableAncestor(node) {
    for (let current = node?.parentElement; current; current = current.parentElement) {
      const style = getComputedStyle(current);
      if (/(auto|scroll)/.test(style.overflowY) && current.scrollHeight > current.clientHeight) return current;
    }
    return document.querySelector(".thread-scroll-container") || document.scrollingElement || document.documentElement;
  }

  // 旧版（26.9xx 之前）内容容器类名清单。保留它当候选之一，但**不再当唯一判据**：
  // 新版 Codex 把 `max-w-(--thread-content-max-width)` 换成了 `max-w-(--thread-body-max-width)`，
  // `pb-8` 也并入 `has-[[…]]:pb-0` 的条件组合，全等匹配必然归零（issue #2258）。
  const conversationViewContentClasses = [
    "mx-auto",
    "w-full",
    "max-w-(--thread-content-max-width)",
    "px-toolbar",
    "relative",
    "flex",
    "shrink-0",
    "flex-col",
    "pb-8",
  ];
  const conversationViewComposerClasses = [
    "relative",
    "z-10",
    "flex",
    "flex-col",
    "mx-auto",
    "w-full",
    "max-w-(--thread-content-max-width)",
    "px-toolbar",
  ];
  // Codex 把中间栏宽度的工具类写成 `max-w-(--thread-<用途>-max-width)`，用途词换过好几轮
  // （content → body、content-responsive…）。所以只钉住「结构」——`max-w-(--thread-*-max-width)`
  // 这个形状本身——而不是某个具体用途词。哈希类名（`_shell_151xi_3` 那类）一律不写死。
  const conversationViewThreadWidthTokenPattern = /^(?:[a-z-]+:)*max-w-\(--thread-[a-z-]+-max-width\)$/;
  // 内容容器的新版稳定锚点。它是虚拟列表宿主（data-mcp-app-portal-target 同节点），
  // 由 Codex 自己维护在滚动容器内部，比类名抗改。选择器统一登记在 00-prelude.js 的
  // selectors 表里，不在这里另起一份。
  const conversationViewContentAnchorSelector = selectors.conversationViewContentAnchor;
  const conversationViewScrollContainerSelector = selectors.conversationViewScrollContainer;
  // 页脚包裹层同样带 `max-w-(--thread-…-max-width)`，会被结构候选误当成内容容器。
  // 用 Codex 自己的页脚标记把它排掉。
  const conversationViewFooterSelector = selectors.conversationViewFooter;
  const conversationViewPaneBoundarySelector = "#app-shell-sidebar, .app-shell-left-panel, .sidebar-navigation, nav[data-app-navigation-rail], [data-summary-panel-variant]";
  // 两侧留白：Codex 的 `--padding-toolbar` 是 `calc(var(--spacing) * 2)`（= 8px * 2）。
  // 仅在拿不到父节点 computed style 时作为回落的单侧留白。
  const conversationViewSideInset = 8;
  const conversationViewState = {
    contentEl: null,
    composerEl: null,
    rafId: 0,
    settleFramesLeft: 0,
    mo: null,
    ro: null,
    pollId: 0,
    runtimeStarted: false,
    moObserved: false,
    targetsReported: false,
    observed: new WeakSet(),
    elements: new Set(),
  };

  function conversationViewTokenSet(el) {
    return new Set(String(el?.className || "").split(/\s+/).filter(Boolean));
  }

  function conversationViewHasAllClasses(el, classes) {
    const set = conversationViewTokenSet(el);
    return classes.every((cls) => set.has(cls));
  }

  function conversationViewFindByClasses(classes, root, accept) {
    return Array.from(root?.querySelectorAll("div") || [])
      .find((el) => conversationViewHasAllClasses(el, classes) && accept(el)) || null;
  }

  function conversationViewHasThreadWidthToken(el) {
    for (const token of conversationViewTokenSet(el)) {
      if (conversationViewThreadWidthTokenPattern.test(token)) return true;
    }
    return false;
  }

  // 结构性判定：居中 + 满宽 + 线程宽度工具类。不依赖任何具体用途词或哈希类名。
  function conversationViewLooksLikeThreadWidthBox(el) {
    if (el?.tagName !== "DIV") return false;
    const set = conversationViewTokenSet(el);
    if (!set.has("mx-auto") || !set.has("w-full")) return false;
    return conversationViewHasThreadWidthToken(el);
  }

  // 页脚包裹层**自身**也带宽度工具类，所以这里不仅要排掉它的后代，还要排掉它本身。
  function conversationViewIsInsideFooter(el) {
    if (!el) return false;
    try {
      return el.matches?.(conversationViewFooterSelector) === true
        || el.closest?.(conversationViewFooterSelector) != null;
    } catch (_) {
      return false;
    }
  }

  function conversationViewScrollContainer() {
    const scrollers = Array.from(document.querySelectorAll(conversationViewScrollContainerSelector))
      .filter((el) => typeof visibleElement !== "function" || visibleElement(el));
    // 多个可见会话没有可靠的当前目标，不把任一 pane 当成整页正文。
    return scrollers.length === 1 ? scrollers[0] : null;
  }

  function conversationViewSafeWidthTarget(el, scope) {
    if (!el || !scope || el === scope || !scope.contains?.(el)) return false;
    if (scope.matches?.(conversationViewScrollContainerSelector)
        && el.closest?.(conversationViewScrollContainerSelector) !== scope) return false;
    return conversationViewSafeWidthNode(el);
  }

  function conversationViewSafeWidthNode(el) {
    if (!el) return false;
    if (["MAIN", "ASIDE", "NAV", "HEADER", "BODY", "HTML"].includes(el.tagName)) return false;
    if (el.closest?.(`${conversationViewPaneBoundarySelector}, [data-codex-plus-ext]`)) return false;
    // CSS 变量可继承给整棵布局树；包含其他 pane 的祖先不能改 width/margin/left。
    // composer 内的状态提示也会用 aside，不能仅按语义标签把它误判为侧栏。
    return !el.querySelector?.(`${conversationViewPaneBoundarySelector}, .thread-scroll-container`);
  }

  function conversationViewSamePane(scroller, el) {
    if (scroller.contains?.(el)) return el.closest?.(conversationViewScrollContainerSelector) === scroller;
    if (el.closest?.(conversationViewScrollContainerSelector)) return false;
    if (el.parentElement === scroller.parentElement && el.parentElement !== document.body) return true;
    for (let pane = scroller.parentElement; pane && pane !== document.body; pane = pane.parentElement) {
      if (!pane.contains?.(el)) continue;
      return !pane.querySelector?.(conversationViewPaneBoundarySelector);
    }
    return false;
  }

  function conversationViewFootersFor(scroller) {
    return Array.from(document.querySelectorAll(conversationViewFooterSelector)).filter((footer) => {
      if (typeof visibleElement === "function" && !visibleElement(footer)) return false;
      return !footer.closest?.(`${conversationViewPaneBoundarySelector}, [data-codex-plus-ext]`) && conversationViewSamePane(scroller, footer);
    });
  }

  function conversationViewFindNativeComposer(scroller) {
    const roots = Array.from(document.querySelectorAll("[data-codex-composer-root]"))
      .filter((el) => (typeof visibleElement !== "function" || visibleElement(el))
        && !el.closest?.(`${conversationViewPaneBoundarySelector}, [data-codex-plus-ext]`) && (!scroller || conversationViewSamePane(scroller, el)));
    if (roots.length !== 1) return null;
    const root = roots[0];
    const accept = (el) => conversationViewLooksLikeThreadWidthBox(el) && conversationViewSafeWidthNode(el)
      && !el.matches?.(conversationViewFooterSelector) && !el.querySelector?.(conversationViewContentAnchorSelector);
    const inside = [root, ...root.querySelectorAll("div")].find(accept);
    if (inside) return inside;
    // 原生 composer 锚点可能在宽度宿主内部；只爬到局部宿主，不收窄含正文的布局。
    for (let host = root.parentElement; host && host !== document.body; host = host.parentElement) {
      if (accept(host)) return host;
      if (host.matches?.(conversationViewScrollContainerSelector) || host.querySelector?.(conversationViewScrollContainerSelector)) break;
    }
    return null;
  }

  function conversationViewCollectThreadWidthBoxes(root) {
    if (!root?.querySelectorAll) return [];
    return Array.from(root.querySelectorAll("div")).filter(conversationViewLooksLikeThreadWidthBox);
  }

  /**
   * 按候选顺序找内容容器，任一候选命中即返回。
   *
   * 候选链刻意从「最精确」排到「最宽松」：
   * 所有候选都必须在唯一的会话滚动区内，并排除布局祖先：
   *   1. 旧版类名全等（老版本 Codex 上仍然最准）；
   *   2. Codex 自己的 data-* 锚点（当前版本）；
   *   3. 结构判定（滚动容器内、居中满宽、带 thread 宽度工具类）；
   *   4. #2085 报告里提到的兜底：两处类名都没命中时，按 CSS 变量反查宿主节点。
   *
   * 顺序不能反：结构判定会把页脚包裹层也算进来，而它和内容容器在同一棵子树里。
   */
  function conversationViewFindContentEl() {
    const scroller = conversationViewScrollContainer();
    if (!scroller) return null;
    const accept = (el) => conversationViewSafeWidthTarget(el, scroller) && !conversationViewIsInsideFooter(el)
      && !el.querySelector?.(conversationViewFooterSelector);
    const legacy = conversationViewFindByClasses(conversationViewContentClasses, scroller, accept);
    if (legacy) return legacy;
    const anchored = Array.from(scroller.querySelectorAll(conversationViewContentAnchorSelector)).find(accept);
    if (anchored) return anchored;
    const structural = conversationViewCollectThreadWidthBoxes(scroller)
      // 页脚包裹层（data-thread-scroll-footer）也带同样的宽度工具类，必须排掉。
      .find(accept);
    if (structural) return structural;
    return conversationViewFindByThreadWidthVariable(scroller, accept);
  }

  function conversationViewFindComposerEl() {
    const scroller = conversationViewScrollContainer();
    if (!scroller) {
      // 首页没有消息 scroller；只用明确的原生 composer 锚点，不能全页猜宽度变量。
      if (Array.from(document.querySelectorAll(conversationViewScrollContainerSelector))
          .some((el) => typeof visibleElement !== "function" || visibleElement(el))) return null;
      const native = conversationViewFindNativeComposer(null);
      if (native) return native;
      const legacy = Array.from(document.querySelectorAll("div")).filter((el) =>
        conversationViewHasAllClasses(el, conversationViewComposerClasses) && conversationViewSafeWidthNode(el)
        && !conversationViewIsInsideFooter(el) && el.querySelector?.('textarea, [contenteditable="true"]'));
      return legacy.length === 1 ? legacy[0] : null;
    }
    // 页脚包裹层带的是和作曲器同一套工具类，会被旧清单全等命中，所以要排除它。
    const footers = conversationViewFootersFor(scroller);
    if (footers.length > 1) return null;
    const footer = footers[0];
    const accept = (el) => conversationViewSafeWidthTarget(el, footer || scroller)
      && !el.matches?.(conversationViewFooterSelector) && !conversationViewIsContentCandidate(el);
    // 新版作曲器在页脚包裹层内部——页脚自身也是 max-w 盒子，得往里再找一层。
    const insideFooter = conversationViewCollectThreadWidthBoxes(footer).find(accept);
    if (insideFooter) return insideFooter;
    if (footer) return conversationViewFindByThreadWidthVariable(footer, (el) => el !== footer && accept(el));
    const native = conversationViewFindNativeComposer(scroller);
    if (native) return native;
    // 保留同一会话内的旧类名；无 footer 的新版结构还必须包含明确编辑器。
    const legacy = conversationViewFindByClasses(conversationViewComposerClasses, scroller, (el) => accept(el) && !conversationViewIsInsideFooter(el));
    if (legacy) return legacy;
    return conversationViewCollectThreadWidthBoxes(scroller).find((el) => accept(el)
      && !conversationViewIsInsideFooter(el) && !el.querySelector?.(conversationViewContentAnchorSelector)
      && el.querySelector?.('textarea, [contenteditable="true"]')) || null;
  }

  // 内容容器的判定（锚点或全等类名），供作曲器查找排除同形节点用。
  function conversationViewIsContentCandidate(el) {
    if (!el) return false;
    if (el.matches?.(conversationViewContentAnchorSelector)) return true;
    return conversationViewHasAllClasses(el, conversationViewContentClasses);
  }

  // 兜底：类名全不对时，看计算样式里 Codex 是否在该节点上定义了线程宽度变量。
  // 变量名只按 `--thread-*-max-width` 这个形状匹配，同样不绑具体用途词。
  // accept 为 null 时默认排除页脚内部节点（内容容器的用法）；作曲器查找会传自己的判定。
  function conversationViewFindByThreadWidthVariable(root, accept = null) {
    if (!root?.querySelectorAll) return null;
    const candidates = Array.from(root.querySelectorAll("div"));
    return candidates.find((el) => {
      if (accept ? !accept(el) : conversationViewIsInsideFooter(el)) return false;
      try {
        const style = getComputedStyle(el);
        // --thread-* 在后代继承，不代表该节点自身受 max-width 约束（#2414）。
        if (!style.maxWidth || style.maxWidth === "none") return false;
        for (const name of conversationViewThreadWidthCustomProperties(style)) {
          if (String(style.getPropertyValue(name) || "").trim()) return true;
        }
      } catch (_) {
        return false;
      }
      return false;
    }) || null;
  }

  function conversationViewThreadWidthCustomProperties(style) {
    // CSSStyleDeclaration 的索引属性在 Chromium 里可用；拿不到时退回固定候选名，
    // 保证兜底在受限环境（测试夹具）里也不会抛。
    const names = [];
    const length = Number(style?.length) || 0;
    for (let index = 0; index < length; index += 1) {
      const name = style[index];
      if (typeof name === "string" && name.startsWith("--thread-") && name.endsWith("-max-width")) names.push(name);
    }
    if (!names.length) names.push("--thread-body-max-width", "--thread-content-max-width");
    return names;
  }

  function codexServiceTierBadgeVisibleElement(element) {
    if (!(element instanceof HTMLElement) || !element.isConnected) return false;
    const style = getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden") return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }

  function codexServiceTierBadgeText(element) {
    return String(element?.textContent || "").replace(/\s+/g, " ").trim();
  }

  function codexServiceTierKnownProviderNames() {
    return uniqueValues([
      codexModelCatalog.provider_name,
      codexModelCatalog.model_provider,
    ]).map((value) => value.toLowerCase());
  }

  function codexServiceTierLooksLikeProviderButton(button, providerNames) {
    const text = codexServiceTierBadgeText(button);
    if (!text || text.length > 32) return false;
    const lower = text.toLowerCase();
    if (providerNames.includes(lower)) return true;
    if (/\s/.test(text)) return false;
    if (!/[a-z]/i.test(text)) return false;
    if (!/^[a-z0-9][a-z0-9._-]{1,31}$/i.test(text)) return false;
    if (/^(local|remote|cloud|standard|default|fast|worktree|new|send|stop|codex)$/i.test(text)) return false;
    if (/^(gpt|o[1-9]|claude|gemini|deepseek|qwen|kimi|moonshot|mistral|llama|sonnet|opus|haiku)[a-z0-9._-]*$/i.test(text)) return false;
    return true;
  }

  function codexServiceTierBadgeButtonCandidates(composer) {
    const composerRect = composer.getBoundingClientRect();
    return Array.from(composer.querySelectorAll("button, [role='button']"))
      .filter((button) => !button.closest?.(`[data-codex-service-tier-badge="true"]`))
      .filter(codexServiceTierBadgeVisibleElement)
      .filter((button) => {
        const rect = button.getBoundingClientRect();
        return rect.bottom >= composerRect.top + composerRect.height * 0.35;
      })
      .sort((left, right) => {
        const leftRect = left.getBoundingClientRect();
        const rightRect = right.getBoundingClientRect();
        return (rightRect.bottom - leftRect.bottom) || (leftRect.left - rightRect.left);
      });
  }

  function codexServiceTierVisibleComposerFooters(root = document) {
    const footers = [
      ...(root?.matches?.(".composer-footer") ? [root] : []),
      ...Array.from(root?.querySelectorAll?.(".composer-footer") || []),
    ];
    return footers
      .filter(codexServiceTierBadgeVisibleElement)
      .sort((left, right) => {
        const leftRect = left.getBoundingClientRect();
        const rightRect = right.getBoundingClientRect();
        return (rightRect.bottom - leftRect.bottom) || (rightRect.width - leftRect.width);
      });
  }

  function codexServiceTierComposerScore(composer) {
    const text = codexServiceTierBadgeText(composer).toLowerCase();
    const providerNames = codexServiceTierKnownProviderNames();
    let score = 0;
    if (providerNames.some((name) => name && text.includes(name))) score += 40;
    if (/完全访问权限|full access|model|超高|high|sub2api|provider/i.test(text)) score += 20;
    if (/本地模式|local mode|worktree|branch|codex\//i.test(text)) score -= 30;
    if (composer.matches?.(".composer-footer")) score += 4;
    if (composer.querySelector?.(".composer-footer")) score += 8;
    const buttons = Array.from(composer.querySelectorAll?.("button, [role='button']") || []).filter(codexServiceTierBadgeVisibleElement);
    if (buttons.some((button) => codexServiceTierLooksLikeProviderButton(button, providerNames))) score += 30;
    score += Math.min(10, buttons.length);
    return score;
  }

  function codexServiceTierComposerCandidates() {
    const candidates = new Set();
    const threadComposer = conversationViewFindComposerEl();
    if (threadComposer && codexServiceTierBadgeVisibleElement(threadComposer)) candidates.add(threadComposer);
    codexServiceTierVisibleComposerFooters().forEach((footer) => {
      candidates.add(footer);
      let node = footer.parentElement;
      for (let depth = 0; node instanceof HTMLElement && depth < 6; depth += 1, node = node.parentElement) {
        if (codexServiceTierBadgeVisibleElement(node)) candidates.add(node);
      }
    });
    return Array.from(candidates);
  }

  function codexServiceTierBestComposerFooter(root = document) {
    return codexServiceTierVisibleComposerFooters(root)
      .map((footer, index) => ({ footer, index, score: codexServiceTierComposerScore(footer) }))
      .sort((left, right) => (right.score - left.score) || (left.index - right.index))[0]?.footer || null;
  }

  function codexServiceTierFindComposerEl() {
    return codexServiceTierComposerCandidates()
      .map((composer, index) => ({ composer, index, score: codexServiceTierComposerScore(composer) }))
      .sort((left, right) => (right.score - left.score) || (left.index - right.index))[0]?.composer || null;
  }

  function codexServiceTierBadgeAnchor(composer) {
    const providerNames = codexServiceTierKnownProviderNames();
    const buttons = codexServiceTierBadgeButtonCandidates(composer);
    const exact = buttons.find((button) => providerNames.includes(codexServiceTierBadgeText(button).toLowerCase()));
    if (exact) return exact;
    const composerRect = composer.getBoundingClientRect();
    return buttons.find((button) => {
      const rect = button.getBoundingClientRect();
      return rect.left >= composerRect.left + composerRect.width * 0.42 && codexServiceTierLooksLikeProviderButton(button, providerNames);
    }) || null;
  }

  function codexServiceTierComposerFooter(composer) {
    if (composer?.matches?.(".composer-footer")) return composer;
    return codexServiceTierBestComposerFooter(composer) || codexServiceTierBestComposerFooter() || null;
  }

  function codexServiceTierBadgeFooterGroup(composer) {
    const footer = codexServiceTierComposerFooter(composer);
    if (!footer) return null;
    const children = Array.from(footer.children).filter(codexServiceTierBadgeVisibleElement);
    if (!children.length) return footer;
    const providerNames = codexServiceTierKnownProviderNames();
    const providerGroup = children.find((child) => {
      const text = codexServiceTierBadgeText(child).toLowerCase();
      return providerNames.some((name) => name && text.includes(name));
    });
    return providerGroup || children[children.length - 1] || footer;
  }

  function codexServiceTierBadgePlacement(composer) {
    const anchor = composer ? codexServiceTierBadgeAnchor(composer) : null;
    if (anchor?.parentElement) return { parent: anchor.parentElement, before: anchor };
    const group = composer ? codexServiceTierBadgeFooterGroup(composer) : null;
    if (group) return { parent: group, before: group.firstChild };
    return null;
  }

  function wireCodexServiceTierBadge(badge) {
    if (!badge || badge.dataset.codexServiceTierBadgeWired === codexServiceTierBadgeVersion) return;
    badge.dataset.codexServiceTierBadgeWired = codexServiceTierBadgeVersion;
    badge.setAttribute("role", "button");
    badge.setAttribute("tabindex", "0");
    badge.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (codexServiceTierState.status === "loading") return;
      toggleCodexServiceTierFromBadge();
    });
    badge.addEventListener("keydown", (event) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      event.stopPropagation();
      if (codexServiceTierState.status === "loading") return;
      toggleCodexServiceTierFromBadge();
    });
  }

  function installCodexServiceTierBadge() {
    if (!codexPlusSettings().serviceTierControls) {
      removeCodexServiceTierBadges();
      return;
    }
    const composer = codexServiceTierFindComposerEl();
    const placement = composer ? codexServiceTierBadgePlacement(composer) : null;
    const existingBadges = Array.from(document.querySelectorAll(`[data-codex-service-tier-badge="true"]`));
    if (!composer || !placement?.parent) {
      existingBadges.forEach((badge) => badge.remove());
      return;
    }
    let badge = existingBadges.find((node) => node.closest?.(".composer-footer") || node.closest?.("button") == null) || existingBadges[0];
    existingBadges.forEach((node) => {
      if (node !== badge) node.remove();
    });
    if (!badge || badge.dataset.codexServiceTierBadgeVersion !== codexServiceTierBadgeVersion) {
      badge?.remove();
      badge = document.createElement("span");
      badge.className = codexServiceTierBadgeClass;
      badge.dataset.codexServiceTierBadge = "true";
      badge.dataset.codexServiceTierBadgeVersion = codexServiceTierBadgeVersion;
    }
    wireCodexServiceTierBadge(badge);
    const before = placement.before?.parentElement === placement.parent ? placement.before : null;
    if (badge.parentElement !== placement.parent || badge.nextSibling !== before) {
      placement.parent.insertBefore(badge, before);
    }
    refreshCodexServiceTierBadges();
  }

  function removeCodexServiceTierBadges() {
    document.querySelectorAll(`[data-codex-service-tier-badge="true"]`).forEach((badge) => badge.remove());
  }

  function conversationViewRememberOriginals(el) {
    if (!el) return;
    conversationViewState.elements.add(el);
    const original = {
      width: el.style.width || "",
      maxWidth: el.style.maxWidth || "",
      marginLeft: el.style.marginLeft || "",
      marginRight: el.style.marginRight || "",
      left: el.style.left || "",
      transform: el.style.transform || "",
      boxSizing: el.style.boxSizing || "",
    };
    if (!("codexPlusConversationViewOriginalWidth" in el.dataset)) el.dataset.codexPlusConversationViewOriginalWidth = original.width;
    if (!("codexPlusConversationViewOriginalMaxWidth" in el.dataset)) el.dataset.codexPlusConversationViewOriginalMaxWidth = original.maxWidth;
    if (!("codexPlusConversationViewOriginalMarginLeft" in el.dataset)) el.dataset.codexPlusConversationViewOriginalMarginLeft = original.marginLeft;
    if (!("codexPlusConversationViewOriginalMarginRight" in el.dataset)) el.dataset.codexPlusConversationViewOriginalMarginRight = original.marginRight;
    if (!("codexPlusConversationViewOriginalLeft" in el.dataset)) el.dataset.codexPlusConversationViewOriginalLeft = original.left;
    if (!("codexPlusConversationViewOriginalTransform" in el.dataset)) el.dataset.codexPlusConversationViewOriginalTransform = original.transform;
    if (!("codexPlusConversationViewOriginalBoxSizing" in el.dataset)) el.dataset.codexPlusConversationViewOriginalBoxSizing = original.boxSizing;
  }

  function conversationViewRestoreElement(el) {
    if (!el) return;
    if ("codexPlusConversationViewOriginalWidth" in el.dataset) {
      el.style.width = el.dataset.codexPlusConversationViewOriginalWidth;
      delete el.dataset.codexPlusConversationViewOriginalWidth;
    }
    if ("codexPlusConversationViewOriginalMaxWidth" in el.dataset) {
      el.style.maxWidth = el.dataset.codexPlusConversationViewOriginalMaxWidth;
      delete el.dataset.codexPlusConversationViewOriginalMaxWidth;
    }
    if ("codexPlusConversationViewOriginalMarginLeft" in el.dataset) {
      el.style.marginLeft = el.dataset.codexPlusConversationViewOriginalMarginLeft;
      delete el.dataset.codexPlusConversationViewOriginalMarginLeft;
    }
    if ("codexPlusConversationViewOriginalMarginRight" in el.dataset) {
      el.style.marginRight = el.dataset.codexPlusConversationViewOriginalMarginRight;
      delete el.dataset.codexPlusConversationViewOriginalMarginRight;
    }
    if ("codexPlusConversationViewOriginalLeft" in el.dataset) {
      el.style.left = el.dataset.codexPlusConversationViewOriginalLeft;
      delete el.dataset.codexPlusConversationViewOriginalLeft;
    }
    if ("codexPlusConversationViewOriginalTransform" in el.dataset) {
      el.style.transform = el.dataset.codexPlusConversationViewOriginalTransform;
      delete el.dataset.codexPlusConversationViewOriginalTransform;
    }
    if ("codexPlusConversationViewOriginalBoxSizing" in el.dataset) {
      el.style.boxSizing = el.dataset.codexPlusConversationViewOriginalBoxSizing;
      delete el.dataset.codexPlusConversationViewOriginalBoxSizing;
    }
  }

  function conversationViewResetOwnOffset(el) {
    if (!el) return;
    const originalTransform = el.dataset.codexPlusConversationViewOriginalTransform || "";
    const originalLeft = el.dataset.codexPlusConversationViewOriginalLeft || "";
    if (el.style.left !== originalLeft) el.style.left = originalLeft;
    if (el.style.transform !== originalTransform) el.style.transform = originalTransform;
    const transform = String(el.style.transform || "").trim();
    if (/^(translateX\([^)]*\)\s*)+$/i.test(transform)) {
      el.style.transform = "";
    }
  }

  // #2085：设置值是**上限**，不是必须写死的宽度。容器比上限窄时按容器可用宽度
  // 收敛，否则 900px 会让内容溢出滚动容器、两侧被裁。
  // 容器宽度已由调用方在读取阶段量好，这里只做纯计算，不读几何——见 conversationViewAlignNow。
  function conversationViewEffectiveWidth(containerWidth) {
    const configured = conversationViewWidth();
    if (!Number.isFinite(containerWidth) || containerWidth <= 0) return configured;
    return Math.max(conversationViewMinWidth, Math.min(configured, Math.round(containerWidth)));
  }

  function conversationViewApplyNativeWidth(el, effectiveWidth) {
    conversationViewRememberOriginals(el);
    const width = Number.isFinite(effectiveWidth) ? effectiveWidth : conversationViewWidth();
    const maxWidth = `${width}px`;
    if (el.style.boxSizing !== "border-box") el.style.boxSizing = "border-box";
    if (el.style.width !== "100%") el.style.width = "100%";
    if (el.style.maxWidth !== maxWidth) el.style.maxWidth = maxWidth;
    if (el.style.marginLeft !== "auto") el.style.marginLeft = "auto";
    if (el.style.marginRight !== "auto") el.style.marginRight = "auto";
  }

  function conversationViewSessionRectFor(el) {
    return el?.parentElement?.getBoundingClientRect() || null;
  }

  // 容器可用宽度：取宿主节点的内容盒宽（rect.width 含内边距，减掉左右 padding 才是可用空间）。
  // 拿不到几何（离屏、display:none、父节点缺失）时返回 0，由 effectiveWidth 回落设置上限。
  function conversationViewAvailableWidth(el) {
    const host = el?.parentElement;
    const rect = conversationViewSessionRectFor(el);
    if (!host || !rect || !(rect.width > 0)) return 0;
    let inlinePadding = conversationViewSideInset * 2;
    try {
      const style = getComputedStyle(host);
      inlinePadding = (Number.parseFloat(style.paddingLeft) || 0) + (Number.parseFloat(style.paddingRight) || 0);
    } catch (_) {
      inlinePadding = conversationViewSideInset * 2;
    }
    return Math.max(0, rect.width - inlinePadding);
  }

  function conversationViewHtmlCenter() {
    const rect = document.documentElement.getBoundingClientRect();
    return rect.left + rect.width / 2;
  }

  function conversationViewObserveIfNeeded(el) {
    if (!el || !conversationViewState.ro || conversationViewState.observed.has(el)) return;
    conversationViewState.observed.add(el);
    conversationViewState.ro.observe(el);
  }

  function conversationViewResolveTargets() {
    for (const [key, next] of [
      ["contentEl", conversationViewFindContentEl()],
      ["composerEl", conversationViewFindComposerEl()],
    ]) {
      const previous = conversationViewState[key];
      if (previous && previous !== next) {
        conversationViewRestoreElement(previous);
        conversationViewState.elements.delete(previous);
        [previous, previous.parentElement, previous.parentElement?.parentElement].forEach((el) => {
          if (!el) return;
          conversationViewState.ro?.unobserve?.(el);
          conversationViewState.observed.delete(el);
        });
      }
      conversationViewState[key] = next;
    }
    [
      document.documentElement,
      document.body,
      conversationViewState.contentEl,
      conversationViewState.contentEl?.parentElement,
      conversationViewState.contentEl?.parentElement?.parentElement,
      conversationViewState.composerEl,
      conversationViewState.composerEl?.parentElement,
      conversationViewState.composerEl?.parentElement?.parentElement,
    ].forEach(conversationViewObserveIfNeeded);
  }

  function conversationViewAlignNow() {
    if (!codexPlusSettings().conversationView) return;
    conversationViewResolveTargets();
    const targets = [
      conversationViewState.contentEl,
      conversationViewState.composerEl,
    ].filter((el) => el?.isConnected);
    if (!targets.length) {
      conversationViewReportMissingTargets();
      return;
    }
    conversationViewState.targetsReported = false;
    // 三阶段批量对齐，全程不出现读-写交替（否则退回 commit 82fb0924 修掉的强制重排）：
    //   ① 读：一次性量完全部目标的宿主可用宽度，算出各自的有效上限；
    //   ② 写：按算好的宽度统一写 style（宽度 + 复位自身偏移）；
    //   ③ 读 + 写 left：统一读几何，决定是否需要再写 left。
    // #2085 的自适应计算落在 ①，写动作仍集中在 ②，与原有两阶段结构一致。
    const availableWidths = targets.map((el) => conversationViewAvailableWidth(el));
    const effectiveWidths = availableWidths.map((width) => conversationViewEffectiveWidth(width));
    targets.forEach((el, index) => {
      conversationViewApplyNativeWidth(el, effectiveWidths[index]);
      conversationViewResetOwnOffset(el);
    });
    const htmlCenter = conversationViewHtmlCenter();
    targets.forEach((el) => {
      const nativeRect = el.getBoundingClientRect();
      const bounds = conversationViewSessionRectFor(el);
      if (!conversationViewHasRoomForHtmlCenterAt(nativeRect, bounds, htmlCenter)) return;
      const targetLeft = htmlCenter - nativeRect.width / 2;
      const delta = targetLeft - nativeRect.left;
      if (Math.abs(delta) > 0.5) {
        const nextLeft = `${delta.toFixed(2)}px`;
        if (el.style.left !== nextLeft) el.style.left = nextLeft;
      }
    });
  }

  /**
   * #2258 最贵的地方是「静默」：类名变化导致目标归零时，对齐整段直接 return，
   * 用户只看到居中失效，日志里什么都没有。这里每个会话只上报一次，
   * 并在下一次成功命中时重置，避免长时间运行时刷屏。
   */
  function conversationViewReportMissingTargets() {
    if (conversationViewState.targetsReported) return;
    conversationViewState.targetsReported = true;
    const scroller = conversationViewScrollContainer();
    sendCodexPlusDiagnostic("conversation_view_target_not_found", {
      hasScrollContainer: !!scroller,
      hasContentAnchor: !!document.querySelector(conversationViewContentAnchorSelector),
      hasFooter: !!document.querySelector(conversationViewFooterSelector),
      threadWidthBoxes: conversationViewCollectThreadWidthBoxes(scroller || document).length,
      configuredWidth: conversationViewWidth(),
    });
  }

  function conversationViewHasRoomForHtmlCenterAt(nativeRect, bounds, htmlCenter) {
    if (!nativeRect || !bounds) return false;
    const targetLeft = htmlCenter - nativeRect.width / 2;
    const targetRight = targetLeft + nativeRect.width;
    return targetLeft >= bounds.left - 0.5 && targetRight <= bounds.right + 0.5;
  }

  function scheduleConversationViewAlign(frames = 16) {
    conversationViewState.settleFramesLeft = Math.max(conversationViewState.settleFramesLeft, frames);
    if (conversationViewState.rafId) return;
    const tick = () => {
      conversationViewState.rafId = 0;
      conversationViewAlignNow();
      conversationViewState.settleFramesLeft -= 1;
      if (conversationViewState.settleFramesLeft > 0) {
        conversationViewState.rafId = requestAnimationFrame(tick);
      }
    };
    conversationViewState.rafId = requestAnimationFrame(tick);
  }

  function cleanupConversationView() {
    if (conversationViewState.rafId) cancelAnimationFrame(conversationViewState.rafId);
    if (conversationViewState.pollId) clearInterval(conversationViewState.pollId);
    conversationViewState.rafId = 0;
    conversationViewState.pollId = 0;
    conversationViewState.mo?.disconnect();
    conversationViewState.ro?.disconnect();
    conversationViewState.mo = null;
    conversationViewState.ro = null;
    conversationViewState.moObserved = false;
    conversationViewState.runtimeStarted = false;
    conversationViewState.observed = new WeakSet();
    conversationViewState.elements.forEach(conversationViewRestoreElement);
    conversationViewState.elements.clear();
    conversationViewState.contentEl = null;
    conversationViewState.composerEl = null;
  }

  window.__codexPlusConversationViewCleanup = cleanupConversationView;
