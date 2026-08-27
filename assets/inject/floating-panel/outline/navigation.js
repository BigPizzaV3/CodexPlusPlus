/* Answer Outline navigation: bounded, direction-aware scrolling and anchors. */

  function outlineFindScrollContainer(fromElement) {
    let node = fromElement instanceof Element ? fromElement.parentElement : null;
    while (node && node !== document.documentElement) {
      const style = window.getComputedStyle(node);
      const overflowY = style.overflowY || style.overflow;
      if (/(auto|scroll|overlay)/.test(overflowY) && node.scrollHeight > node.clientHeight + 4) return node;
      node = node.parentElement;
    }
    return document.scrollingElement || document.documentElement;
  }

  function outlineIsDocumentScroller(container) {
    return container === document.scrollingElement
      || container === document.documentElement
      || container === document.body;
  }

  // Codex thread scrollers may use column-reverse, where valid scrollTop values are negative.
  function outlineScrollBounds(container) {
    const maxDistance = Math.max(0, container.scrollHeight - container.clientHeight);
    const style = window.getComputedStyle(container);
    const reversed = !outlineIsDocumentScroller(container)
      && (style.flexDirection === "column-reverse" || container.scrollTop < -1);
    return reversed
      ? { min: -maxDistance, max: 0 }
      : { min: 0, max: maxDistance };
  }

  function outlineScrollViewportTop(container) {
    const safeTop = Math.max(0, contentSafeBounds().top - PANEL_SAFE_MARGIN);
    if (outlineIsDocumentScroller(container)) return safeTop;
    return Math.max(safeTop, container.getBoundingClientRect().top);
  }

  function outlineScrollScale(container) {
    const layoutHeight = container.clientHeight;
    const visualHeight = container.getBoundingClientRect().height;
    if (!(layoutHeight > 0) || !(visualHeight > 0)) return 1;
    const scale = visualHeight / layoutHeight;
    return Number.isFinite(scale) && scale > 0.01 ? scale : 1;
  }

  function outlineTargetScrollTop(element, container) {
    const bounds = outlineScrollBounds(container);
    const elementTop = element.getBoundingClientRect().top;
    const targetViewportTop = outlineScrollViewportTop(container) + OUTLINE_TARGET_TOP_OFFSET;
    const delta = elementTop - targetViewportTop;
    const deltaInScrollSpace = delta / outlineScrollScale(container);
    return clamp(container.scrollTop + deltaInScrollSpace, bounds.min, bounds.max);
  }

  function outlineScheduleScrollSettle(element, container) {
    state.outlineScrollCleanup?.();
    let settleTimer = 0;
    let recheckTimer = 0;
    let finished = false;
    const cleanup = () => {
      if (settleTimer) window.clearTimeout(settleTimer);
      if (recheckTimer) window.clearTimeout(recheckTimer);
      settleTimer = 0;
      recheckTimer = 0;
      container.removeEventListener("scrollend", settle);
      container.removeEventListener("wheel", cancel);
      container.removeEventListener("pointerdown", cancel);
      if (state.outlineScrollCleanup === cancel) state.outlineScrollCleanup = null;
    };
    const cancel = () => {
      if (finished) return;
      finished = true;
      cleanup();
    };
    const correct = () => {
      if (!isCurrentRuntime() || !element.isConnected || !container.isConnected) return false;
      const targetTop = outlineTargetScrollTop(element, container);
      if (Math.abs(container.scrollTop - targetTop) > 1) container.scrollTop = targetTop;
      return true;
    };
    const finish = () => {
      if (finished) return;
      finished = true;
      cleanup();
    };
    const settle = () => {
      if (finished) return;
      container.removeEventListener("scrollend", settle);
      if (settleTimer) window.clearTimeout(settleTimer);
      settleTimer = 0;
      if (!correct()) {
        finish();
        return;
      }
      recheckTimer = window.setTimeout(() => {
        recheckTimer = 0;
        correct();
        finish();
      }, OUTLINE_SCROLL_RECHECK_MS);
    };
    state.outlineScrollCleanup = cancel;
    container.addEventListener("scrollend", settle, { once: true });
    container.addEventListener("wheel", cancel, { once: true, passive: true });
    container.addEventListener("pointerdown", cancel, { once: true, passive: true });
    settleTimer = window.setTimeout(settle, OUTLINE_SCROLL_SETTLE_MS);
  }

  function outlineScrollToElement(element) {
    const container = outlineFindScrollContainer(element);
    if (!(container instanceof Element)) return false;
    // Use one bounded destination instead of chaining scrollIntoView with a corrective scroll.
    const targetTop = outlineTargetScrollTop(element, container);
    if (!Number.isFinite(targetTop)) return false;
    if (Math.abs(container.scrollTop - targetTop) < 0.5) {
      state.outlineScrollCleanup?.();
      return true;
    }
    outlineScheduleScrollSettle(element, container);
    try {
      container.scrollTo({ top: targetTop, behavior: "smooth" });
    } catch {
      state.outlineScrollCleanup?.();
      container.scrollTop = targetTop;
    }
    return true;
  }

  function outlineScrollToEnd(fromElement) {
    const container = outlineFindScrollContainer(fromElement);
    if (!(container instanceof Element)) return false;
    state.outlineScrollCleanup?.();
    const targetTop = outlineScrollBounds(container).max;
    if (Math.abs(container.scrollTop - targetTop) < 0.5) return true;
    try {
      container.scrollTo({ top: targetTop, behavior: "smooth" });
    } catch {
      container.scrollTop = targetTop;
    }
    return true;
  }

  function outlineResolveElement(id) {
    const item = state.outlineItems.find((entry) => entry.id === id) || null;
    if (item?.el?.isConnected) return item.el;
    const marked = Array.from(document.querySelectorAll(`[${MARK_ATTR}]`))
      .find((node) => node.getAttribute(MARK_ATTR) === String(id));
    if (marked instanceof Element) {
      if (item) item.el = marked;
      return marked;
    }
    const latest = state.outlineMessage?.isConnected ? { node: state.outlineMessage } : findLatestAssistantMessage();
    const root = outlineMarkdownRoot(latest?.node);
    if (!root || !item?.text) return null;
    const kind = item.kind === "semantic" ? "semantic" : "pseudo";
    const selector = kind === "semantic" ? OUTLINE_SEMANTIC_HEADING_SELECTOR : OUTLINE_PSEUDO_HEADING_SELECTOR;
    const candidates = root.querySelectorAll(selector);
    for (const node of candidates) {
      if (kind === "pseudo" && node.closest(OUTLINE_SEMANTIC_HEADING_SELECTOR)) continue;
      const candidate = outlineHeadingCandidate(node, kind);
      if (!candidate || !outlineTitlesEquivalent(candidate.text, item.text)) continue;
      node.setAttribute(MARK_ATTR, id);
      item.el = node;
      return node;
    }
    return null;
  }

  function outlineFlash(element) {
    if (!(element instanceof Element)) return;
    element.classList.add(HIGHLIGHT_CLASS);
    if (state.flashTimer) window.clearTimeout(state.flashTimer);
    state.flashTimer = window.setTimeout(() => {
      element.classList.remove(HIGHLIGHT_CLASS);
      state.flashTimer = 0;
    }, FLASH_MS);
  }

  function outlineSetActiveTarget({ id = "", anchor = "" } = {}) {
    state.panel?.querySelectorAll("[data-outline-id],[data-outline-anchor]").forEach((button) => {
      const isActive = id
        ? button.dataset.outlineId === id
        : anchor && button.dataset.outlineAnchor === anchor;
      button.dataset.active = isActive ? "true" : "false";
      if (isActive) button.setAttribute("aria-current", "location");
      else button.removeAttribute("aria-current");
    });
  }

  function outlineJumpTo(id) {
    const element = outlineResolveElement(id);
    if (!(element instanceof Element)) return false;
    outlineSetActiveTarget({ id });
    outlineScrollToElement(element);
    outlineFlash(element);
    return true;
  }

  function outlineCurrentMessageElement() {
    if (state.outlineMessage?.isConnected) return state.outlineMessage;
    const latest = findLatestAssistantMessage();
    return latest?.node instanceof Element ? latest.node : null;
  }

  function outlineTurnStartElement(message) {
    const turn = message?.closest?.(CONVERSATION_TURN_SELECTOR)
      || (state.latestTurnAnchor?.turnNode?.isConnected ? state.latestTurnAnchor.turnNode : null);
    if (!(turn instanceof Element)) return message;
    return labeledMessageContainer(turn, "user") || turn;
  }

  function outlineJumpToAnchor(anchor) {
    const message = outlineCurrentMessageElement();
    if (!(message instanceof Element)) return false;
    if (anchor === "start") {
      const startElement = outlineTurnStartElement(message);
      if (!(startElement instanceof Element)) return false;
      outlineSetActiveTarget({ anchor });
      outlineScrollToElement(startElement);
      outlineFlash(startElement);
      return true;
    }
    if (anchor === "end") {
      outlineSetActiveTarget({ anchor });
      return outlineScrollToEnd(message);
    }
    return false;
  }
