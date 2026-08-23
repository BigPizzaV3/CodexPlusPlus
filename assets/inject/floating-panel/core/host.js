/* Floating-panel host: expressions, view transitions, and host-level state. */

  // Derived expressions turn backend, parser, and page states into one calm user-facing status.
  function expressionError() {
    const settings = state.settings;
    const configurationMissing = settings?.enabled === true
      && (!settings.baseUrlConfigured || !settings.model || !settings.apiKeyConfigured);
    return configurationMissing
      || state.bridgeStatus === "failed"
      || (state.bridgeStatus === "disabled" && Boolean(state.bridgeError))
      || state.scanStatus === "manual-refresh-no-assistant";
  }

  function stepwiseWaitingForManualRefresh(settings = state.settings) {
    return stepwiseEnabled(settings)
      && stepwiseGenerationMode(settings) === "manual"
      && state.bridgeStatus !== "pending"
      && state.bridgeStatus !== "ok"
      && !state.prompts.length
      && !expressionError();
  }

  function resolveStepwiseExpression(now = Date.now()) {
    if (!stepwiseEnabled()) return "hidden";
    if (state.bridgeStatus === "pending") return "generating";
    if (expressionError()) return "error";
    if (stepwiseGenerationMode() === "manual") {
      if (state.bridgeStatus === "disabled") return "hidden";
      if (state.prompts.length) return "ready";
      if (state.bridgeStatus === "ok") return "empty";
      return "idle";
    }
    if (state.scanBusy) return "answering";
    if (state.surpriseUntil > now) return "surprise";
    if (state.scanStatus === "assistant-changed" || state.scanStatus === "assistant-settling") {
      return "answering";
    }
    if (state.bridgeStatus === "disabled") return "hidden";
    if (state.prompts.length) return "ready";
    if (state.bridgeStatus === "ok") return "empty";
    return "idle";
  }

  function resolveOutlineExpression(now = Date.now()) {
    if (!outlineEnabled()) return "hidden";
    if (state.outlineStatus === "pending") return "generating";
    if (state.scanBusy) return "answering";
    if (state.surpriseUntil > now) return "surprise";
    if (state.outlineStatus === "error") return "error";
    if (state.outlineItems.length) return "ready";
    if (state.outlineStatus === "empty") return "empty";
    return "idle";
  }

  function usesOutlineExpression(now = Date.now()) {
    const stepwiseExpression = resolveStepwiseExpression(now);
    return outlineEnabled()
      && (state.activeTab === "outline"
        || stepwiseExpression === "hidden"
        || stepwiseWaitingForManualRefresh());
  }

  function resolveFabExpression(now = Date.now()) {
    if (!runtimeEnabled()) return "hidden";
    return usesOutlineExpression(now)
      ? resolveOutlineExpression(now)
      : resolveStepwiseExpression(now);
  }

  function fabExpressionLabel(expression, outlineExpression = usesOutlineExpression()) {
    if (outlineExpression) {
      return {
        idle: "空闲",
        answering: "回答中",
        surprise: "正在整理回答",
        generating: "正在整理大纲",
        ready: "大纲已准备",
        empty: "暂无大纲",
        error: "生成失败",
        curious: "查看设置",
        hidden: "已关闭",
      }[expression] || "空闲";
    }
    return {
      idle: "空闲",
      answering: "回答中",
      surprise: "正在整理回答",
      generating: "正在生成建议",
      ready: "建议已准备",
      empty: "暂无建议",
      error: "生成失败",
      curious: "查看设置",
      hidden: "已关闭",
    }[expression] || "空闲";
  }

  function scheduleExpressionRefresh(delay) {
    if (!isCurrentRuntime()) return;
    if (state.expressionTimer) window.clearTimeout(state.expressionTimer);
    const generation = state.runtimeGeneration;
    const timer = window.setTimeout(() => {
      if (state.expressionTimer === timer) state.expressionTimer = 0;
      if (isCurrentRuntime(generation)) renderFloat();
    }, delay);
    state.expressionTimer = timer;
  }

  function clearCompletionBeam() {
    if (state.completionBeamTimer) window.clearTimeout(state.completionBeamTimer);
    state.completionBeamTimer = 0;
    if (state.popover) state.popover.dataset.completionBeam = "false";
  }

  function triggerCompletionBeam(promptCount) {
    clearCompletionBeam();
    if (promptCount < 1 || prefersReducedMotion() || !state.popover) return;
    state.popover.dataset.completionBeam = "true";
    const timer = window.setTimeout(() => {
      if (state.completionBeamTimer !== timer) return;
      state.completionBeamTimer = 0;
      if (state.popover) state.popover.dataset.completionBeam = "false";
    }, COMPLETION_BEAM_MS);
    state.completionBeamTimer = timer;
  }

  // View transitions and shell morphs share deterministic completion and cancellation rules.
  function prefersReducedMotion() {
    try {
      return window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches === true;
    } catch {
      return false;
    }
  }

  function cancelViewAnimation() {
    cancelViewStageAnimation();
    cancelViewIndicatorAnimation();
  }

  function cancelViewStageAnimation() {
    const transition = state.viewAnimation;
    state.viewAnimation = null;
    if (!transition) return;
    transition.animations?.forEach((animation) => animation.cancel());
    transition.finish?.();
  }

  function cancelViewIndicatorAnimation() {
    if (state.viewIndicatorFrame) window.cancelAnimationFrame(state.viewIndicatorFrame);
    state.viewIndicatorFrame = 0;
  }

  function deferRender() {
    state.pendingRender = true;
  }

  function flushDeferredRender() {
    if (!state.pendingRender || !isCurrentRuntime()) return false;
    if (state.viewTransitioning || state.morphAnimation) return false;
    state.pendingRender = false;
    renderFloat({ preserveMorph: true, allowDuringTransition: true });
    return true;
  }

  function viewSlideDirection(fromTab, targetTab) {
    const order = viewNavigationOrder();
    const fromIndex = order.indexOf(fromTab);
    const targetIndex = order.indexOf(targetTab);
    if (fromIndex < 0 || targetIndex < 0 || fromIndex === targetIndex) return 1;
    return targetIndex > fromIndex ? 1 : -1;
  }

  function captureViewStage() {
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    const stage = body?.querySelector(":scope > .csw-mouth-stage");
    if (!body || !stage) return null;
    return {
      node: stage.cloneNode(true),
      scrollTop: body.scrollTop,
    };
  }

  function animateViewSlide(snapshot, direction) {
    const body = state.panel?.querySelector(".csw-body[data-view-body]");
    const incoming = body?.querySelector(":scope > .csw-mouth-stage");
    if (!snapshot?.node || !body || !incoming || prefersReducedMotion()
      || typeof incoming.animate !== "function") {
      return Promise.resolve();
    }

    cancelViewStageAnimation();
    const layer = document.createElement("div");
    const outgoing = snapshot.node;
    layer.className = "csw-view-transition-layer";
    outgoing.classList.add("csw-view-transition-copy");
    outgoing.style.top = `${2 - snapshot.scrollTop}px`;
    layer.appendChild(outgoing);
    body.appendChild(layer);
    body.dataset.viewTransition = "true";

    const distance = VIEW_SLIDE_DISTANCE * direction;
    const options = {
      duration: VIEW_SLIDE_MS,
      easing: "cubic-bezier(.2, .72, .2, 1)",
      fill: "forwards",
    };
    const outgoingAnimation = outgoing.animate([
      { opacity: 1, transform: "translate3d(0, 0, 0)" },
      { opacity: 0.08, transform: `translate3d(${-distance}px, 0, 0)` },
    ], options);
    const incomingAnimation = incoming.animate([
      { opacity: 0.42, transform: `translate3d(${distance}px, 0, 0)` },
      { opacity: 1, transform: "translate3d(0, 0, 0)" },
    ], options);
    let cleaned = false;
    const cleanup = () => {
      if (cleaned) return;
      cleaned = true;
      body.removeAttribute("data-view-transition");
      layer.remove();
    };
    let resolveCompletion;
    let settled = false;
    const completion = new Promise((resolve) => {
      resolveCompletion = resolve;
    });
    const transition = {
      animations: [outgoingAnimation, incomingAnimation],
      cleanup,
      fallbackTimer: 0,
      finish: () => {
        if (settled) return;
        settled = true;
        if (transition.fallbackTimer) window.clearTimeout(transition.fallbackTimer);
        cleanup();
        if (state.viewAnimation === transition) state.viewAnimation = null;
        resolveCompletion();
      },
    };
    state.viewAnimation = transition;
    transition.fallbackTimer = window.setTimeout(
      () => {
        transition.animations.forEach((animation) => {
          if (animation.playState !== "finished") animation.cancel();
        });
        transition.finish();
      },
      VIEW_SLIDE_MS + 120,
    );
    void Promise.all(transition.animations.map((animation) => animation.finished.catch(() => null)))
      .then(() => transition.finish());
    return completion;
  }

  function syncViewTabSelection(targetTab, animate = true) {
    const tabs = state.panel?.querySelector(".csw-view-tabs");
    const indicator = tabs?.querySelector(".csw-view-indicator");
    if (!tabs || !indicator) return;

    const buttons = Array.from(tabs.querySelectorAll(".csw-icon[data-view]"));
    const target = buttons.find((button) => button.dataset.view === targetTab) || null;
    buttons.forEach((button) => {
      const selected = button === target;
      button.dataset.active = String(selected);
      button.setAttribute("aria-selected", String(selected));
    });

    indicator.style.transition = animate && !prefersReducedMotion() ? "" : "none";
    if (!target) {
      indicator.style.opacity = "0";
      tabs.dataset.activeView = "";
      return;
    }

    tabs.dataset.activeView = targetTab;
    indicator.style.opacity = "1";
    indicator.style.transform = `translate3d(${target.offsetLeft - indicator.offsetLeft}px, 0, 0)`;
    if (!animate) indicator.getBoundingClientRect();
  }

  function animateViewTabSelection(fromTab, targetTab) {
    syncViewTabSelection(fromTab, false);
    if (fromTab === targetTab) return;
    state.viewIndicatorFrame = window.requestAnimationFrame(() => {
      state.viewIndicatorFrame = 0;
      if (!isCurrentRuntime()) return;
      syncViewTabSelection(targetTab, true);
    });
  }

  async function switchView(nextTab) {
    const generation = state.runtimeGeneration;
    const targetTab = normalizeActiveTab(nextTab);
    if (!isCurrentRuntime(generation) || targetTab === state.activeTab) return;
    if (state.viewTransitioning) {
      state.pendingTab = targetTab;
      return;
    }
    state.viewTransitioning = true;
    try {
      const sourceTab = state.activeTab;
      const snapshot = captureViewStage();
      const direction = viewSlideDirection(sourceTab, targetTab);
      state.activeTab = normalizeActiveTab(targetTab);
      state.pendingRender = false;
      renderFloat({
        preserveMorph: true,
        viewIndicatorFrom: sourceTab,
        allowDuringTransition: true,
      });
      await animateViewSlide(snapshot, direction);
      if (!isCurrentRuntime(generation)) return;
      if (targetTab === "settings") void reloadSettings();
    } finally {
      if (isCurrentRuntime(generation)) {
        state.viewTransitioning = false;
        const pendingTab = state.pendingTab;
        state.pendingTab = "";
        if (pendingTab && pendingTab !== state.activeTab) void switchView(pendingTab);
        else flushDeferredRender();
      }
    }
  }
