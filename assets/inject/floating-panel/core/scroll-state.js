/* Shared panel view scroll state and content fade tracking. */

// Shared view scroll state belongs to the shell, not to Outline parsing.

  function viewScrollTargets(body = state.panel?.querySelector(".csw-body[data-view-body]")) {
    if (!body) return [];
    const targets = [body];
    const previewScroll = body.querySelector(".csw-prompt-preview-scroll");
    if (previewScroll) targets.push(previewScroll);
    return targets;
  }
  function captureViewScroll() {
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    if (!body || body.dataset.viewBody !== state.activeTab) return null;
    const preview = body.querySelector(".csw-prompt-preview");
    const previewScroll = preview?.querySelector(".csw-prompt-preview-scroll");
    return {
      view: state.activeTab,
      top: body.scrollTop,
      preview: preview && previewScroll ? {
        index: preview.dataset.previewIndex || "",
        prompt: preview.querySelector(".csw-prompt-preview-body")?.textContent || "",
        top: previewScroll.scrollTop,
      } : null,
    };
  }

  function restoreViewScroll(snapshot) {
    if (!snapshot || snapshot.view !== state.activeTab) return;
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    if (!body || body.dataset.viewBody !== snapshot.view) return;
    const maxTop = Math.max(0, body.scrollHeight - body.clientHeight);
    body.scrollTop = clamp(snapshot.top, 0, maxTop);

    const preview = body.querySelector(".csw-prompt-preview");
    const previewScroll = preview?.querySelector(".csw-prompt-preview-scroll");
    if (!snapshot.preview || !preview || !previewScroll) return;
    const prompt = preview.querySelector(".csw-prompt-preview-body")?.textContent || "";
    if (preview.dataset.previewIndex !== snapshot.preview.index || prompt !== snapshot.preview.prompt) return;
    const previewMaxTop = Math.max(0, previewScroll.scrollHeight - previewScroll.clientHeight);
    previewScroll.scrollTop = clamp(snapshot.preview.top, 0, previewMaxTop);
  }

  function syncContentFade() {
    const popover = state.popover;
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    if (!popover || !body) return;

    const preview = body.querySelector(".csw-prompt-preview");
    const previewScroll = preview?.querySelector(".csw-prompt-preview-scroll");
    if (preview && previewScroll) {
      const previewMaxTop = Math.max(0, previewScroll.scrollHeight - previewScroll.clientHeight);
      const previewOverflowing = previewMaxTop > 2;
      const previewAtEnd = !previewOverflowing || previewScroll.scrollTop >= previewMaxTop - 2;
      preview.dataset.scrollOverflow = String(previewOverflowing);
      preview.dataset.scrollAtEnd = String(previewAtEnd);
      preview.dataset.scrollFade = String(previewOverflowing && !previewAtEnd);
    }

    const view = body.dataset.viewBody || "";
    const compressed = popover.dataset.compressed === "true";
    const eligible = compressed && (view === "next" || view === "outline");
    const scrollStates = viewScrollTargets(body).map((target) => ({
      target,
      maxTop: Math.max(0, target.scrollHeight - target.clientHeight),
    }));
    const overflowing = eligible && scrollStates.some(({ maxTop }) => maxTop > 2);
    const atEnd = !overflowing || scrollStates.every(({ target, maxTop }) => (
      maxTop <= 2 || target.scrollTop >= maxTop - 2
    ));

    popover.dataset.contentOverflow = String(overflowing);
    popover.dataset.contentAtEnd = String(atEnd);
    popover.dataset.contentFade = String(overflowing && !atEnd);
  }

  function installContentFadeTracking() {
    state.contentFadeCleanup?.();
    state.contentFadeCleanup = null;

    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    const targets = viewScrollTargets(body);
    if (!body || !targets.length) return;

    const onScroll = () => syncContentFade();
    targets.forEach((target) => target.addEventListener("scroll", onScroll, { passive: true }));

    const resizeObserver = typeof window.ResizeObserver === "function"
      ? new window.ResizeObserver(onScroll)
      : null;
    const resizeTargets = new Set();
    targets.forEach((target) => {
      resizeTargets.add(target);
      if (target.firstElementChild) resizeTargets.add(target.firstElementChild);
    });
    resizeTargets.forEach((target) => resizeObserver?.observe(target));

    state.contentFadeCleanup = () => {
      targets.forEach((target) => target.removeEventListener("scroll", onScroll));
      resizeObserver?.disconnect();
    };

    syncContentFade();
    window.requestAnimationFrame(() => {
      if (body.isConnected && state.panel?.contains(body)) syncContentFade();
    });
  }
