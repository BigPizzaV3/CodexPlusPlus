/* Floating-panel runtime lifecycle and feature coordination. */

  function scan(generation = state.runtimeGeneration, timerId = 0) {
    if (!isCurrentRuntime(generation)) return;
    if (timerId && state.timer !== timerId) return;
    if (timerId) state.timer = 0;
    state.scans += 1;
    installStyle();
    installFloat();
    const stepwiseActive = stepwiseEnabled();
    const outlineActive = outlineEnabled();

    if (!chatSurfaceReady()) {
      if (outlineActive && (state.outlineItems.length || state.outlineMessage)) invalidateOutline();
      const statusChanged = setScanStatus("not-ready", {
        hasRoot: Boolean(chatRoot()),
        composerCount: composerCandidates().length,
        busy: chatBusy(),
      });
      if (statusChanged) renderFloat();
      return;
    }

    const message = findLatestAssistantMessage();
    if (!message) {
      if (outlineActive && (state.outlineItems.length || state.outlineMessage)) invalidateOutline();
      const statusChanged = setScanStatus("no-assistant-message", {
        messageCandidateCount: messageCandidates().length,
        actionRowCount: allActionRows().length,
      });
      if (statusChanged) renderFloat();
      return;
    }

    const stepwisePayload = stepwiseActive
      ? extractStepwisePayload(message)
      : { payload: null, prompts: [], textWithoutPayload: "" };
    if (stepwiseActive) hideStepwisePayload(message.node);

    const nextAssistantMessageId = assistantMessageId(message);
    if (state.activeContext.assistantMessageId !== nextAssistantMessageId) {
      state.activeContext.assistantMessageId = nextAssistantMessageId;
    }

    const assistantText = shortText(stepwiseActive
      ? stepwisePayload.textWithoutPayload || message.text
      : message.text);
    const hash = hashText(assistantText);
    const now = Date.now();

    if (hash !== state.lastAssistantHash) {
      state.lastAssistantHash = hash;
      state.lastAssistantAt = now;
      state.surpriseUntil = now + NEW_ANSWER_EXPRESSION_MS;
      scheduleExpressionRefresh(NEW_ANSWER_EXPRESSION_MS);
      setScanStatus("assistant-changed", { hash, textLength: assistantText.length });
      if (outlineActive) invalidateOutline(message, hash);
      if (stepwiseActive) {
        clearPromptsForNewAssistant(hash);
      } else {
        renderFloat();
      }
      scheduleScan(STREAM_IDLE_MS + 120);
      return;
    }

    if (now - state.lastAssistantAt < STREAM_IDLE_MS) {
      setScanStatus("assistant-settling", { hash });
      scheduleScan(STREAM_IDLE_MS);
      return;
    }

    if (outlineActive && state.outlineSourceHash !== hash && !state.outlineRefreshPromise) {
      void refreshOutline({ message, assistantHash: hash });
    }
    if (!stepwiseActive) {
      const statusChanged = setScanStatus("ready", {
        hash,
        outlineOnly: true,
        outlineCount: state.outlineItems.length,
      });
      if (statusChanged) renderFloat();
      return;
    }

    const userText = findPreviousUserText(message);
    const bridgeKey = bridgeRequestKey(userText, assistantText);
    const generationMode = stepwiseGenerationMode();
    const manualRequestPending = generationMode === "manual"
      && state.bridgeStatus === "pending"
      && state.bridgePendingHash === bridgeKey;
    state.bridgeActiveKey = bridgeKey;
    const bridgeResult = state.bridgeCache.get(bridgeKey);
    const hasSuccessfulCache = bridgeResult?.status === "ok";
    let prompts = [];

    const manualResultVisible = generationMode === "manual"
      && state.bridgeStatus === "ok"
      && state.bridgeActiveKey === bridgeKey;

    if (generationMode === "manual" && !manualResultVisible && !manualRequestPending) {
      state.bridgeStatus = "manual-ready";
      state.bridgeError = "";
      state.promptContext = contextSnapshot();
    } else if (manualRequestPending) {
      state.bridgeError = "";
      state.promptContext = contextSnapshot();
    } else if (hasSuccessfulCache) {
      prompts = Array.isArray(bridgeResult.prompts) ? bridgeResult.prompts : [];
      state.bridgeStatus = "ok";
      state.bridgeError = "";
      state.promptContext = contextSnapshot();
    } else {
      prompts = bridgeResult ? [] : stepwisePayload.prompts;
      if (bridgeResult) {
        state.bridgeStatus = bridgeResult.status || (bridgeResult.error ? "failed" : bridgeResult.disabled ? "disabled" : "ok");
        state.bridgeError = bridgeResult.error || "";
        state.promptContext = contextSnapshot();
      } else {
        requestBridgeStepwise(bridgeKey, userText, assistantText, "auto");
      }
    }
    setScanStatus("ready", {
      hash,
      bridgeCached: Boolean(bridgeResult),
      promptCount: prompts.length,
    });

    const nextHash = hashText(`${generationMode}:${state.bridgeStatus}:${prompts.map((item) => `${item.label}\n${item.prompt}`).join("\n\n")}`);
    const renderedHash = `${hash}:${nextHash}`;
    if (state.currentHash !== renderedHash) {
      state.currentHash = renderedHash;
      state.prompts = prompts;
      state.promptContext = contextSnapshot();
      state.promptPreviewIndex = 0;
      renderFloat();
    }
  }

  function scheduleScan(delay = SCAN_DELAY_MS) {
    if (!isCurrentRuntime()) return;
    if (state.timer) window.clearTimeout(state.timer);
    const generation = state.runtimeGeneration;
    const timer = window.setTimeout(() => scan(generation, timer), delay);
    state.timer = timer;
  }

  function installObserver() {
    if (!isCurrentRuntime()) return false;
    const root = document.body || document.documentElement;
    if (!root) return false;

    const generation = state.runtimeGeneration;
    state.observer = new MutationObserver((mutations) => {
      if (!isCurrentRuntime(generation)) return;
      const relevant = mutations.some((mutation) => {
        if (state.root?.contains(mutation.target)) return false;
        return mutation.addedNodes.length || mutation.type === "characterData";
      });
      if (relevant) scheduleScan();
    });
    state.observer.observe(root, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    return true;
  }

  // Stopping invalidates every generation, removes observers, and leaves no page-owned runtime behind.
  function stopRuntime() {
    state.runtimeActive = false;
    state.runtimeGeneration += 1;
    state.latestTurnAnchor = null;
    if (state.domReadyHandler) document.removeEventListener("DOMContentLoaded", state.domReadyHandler);
    state.domReadyHandler = null;
    if (state.timer) window.clearTimeout(state.timer);
    if (state.expressionTimer) window.clearTimeout(state.expressionTimer);
    if (state.keepAliveTimer) window.clearTimeout(state.keepAliveTimer);
    if (state.flashTimer) window.clearTimeout(state.flashTimer);
    if (state.materialAnimTimer) window.clearTimeout(state.materialAnimTimer);
    if (state.completionBeamTimer) window.clearTimeout(state.completionBeamTimer);
    if (state.snapTimer) window.clearTimeout(state.snapTimer);
    if (state.eyeRaf) window.cancelAnimationFrame(state.eyeRaf);
    state.timer = 0;
    state.expressionTimer = 0;
    state.keepAliveTimer = 0;
    state.flashTimer = 0;
    state.materialAnimTimer = 0;
    state.completionBeamTimer = 0;
    state.snapTimer = 0;
    state.eyeRaf = 0;
    state.surpriseUntil = 0;
    state.bridgeActiveKey = "";
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.viewTransitioning = false;
    state.pendingTab = "";
    state.pendingRender = false;
    state.popover?.removeAttribute?.("data-snap-right");
    cancelViewAnimation();
    cancelSourceCueAnimation();
    cancelMorphAnimations();
    state.dragCleanup?.();
    state.resizeCleanup?.();
    state.viewReorderCleanup?.();
    state.contentFadeCleanup?.();
    state.eyeCleanup?.();
    state.eyeCleanup = null;
    state.contentFadeCleanup = null;
    state.eyePointer = null;
    document.querySelectorAll(".codex-stepwise-active-pane, .codex-stepwise-pane-flash").forEach((node) => {
      node.classList.remove("codex-stepwise-active-pane", "codex-stepwise-pane-flash");
    });
    removeContextTracking();
    if (state.keyHandler) document.removeEventListener("keydown", state.keyHandler, true);
    state.keyHandler = null;
    window.removeEventListener("resize", onResize);
    state.observer?.disconnect();
    state.observer = null;
    state.themeObserver?.disconnect();
    state.themeObserver = null;
    state.typographyObserver?.disconnect();
    state.typographyObserver = null;
    clearPromptInteractionTimers();
    clearStepwisePayloadMarks();
    outlineClearMarks();
    state.outlineItems = [];
    state.outlineRefreshPromise = null;
    state.outlineMessage = null;
    state.outlineScrollCleanup?.();
    state.outlineScrollCleanup = null;
    state.outlineSourceHash = "";
    state.outlineFingerprint = "";
    state.outlineStatus = "idle";
    state.outlineError = "";
    state.root?.remove();
    state.root = null;
    state.fab = null;
    state.popover = null;
    state.glass = null;
    state.rim = null;
    state.completionBeam = null;
    state.clearFilter = null;
    state.clearDisplacement = null;
    state.clearDistortion = null;
    state.liquidFilter = null;
    state.crystalFilter = null;
    state.displacementTexture = null;
    state.panel = null;
    state.layout = null;
    state.drag = null;
    state.resizeDrag = null;
    state.dragCleanup = null;
    state.resizeCleanup = null;
    state.viewReorderCleanup = null;
    state.suppressViewTabClickUntil = 0;
    state.focusAfterMorph = "";
    state.pinnedThreadRoot = null;
    state.pinnedThreadAt = 0;
    state.activeContext = {
      paneRoot: null,
      paneKey: "",
      sessionId: "",
      assistantMessageId: "",
      generation: state.activeContext.generation + 1,
    };
    document.getElementById(STYLE_ID)?.remove();
    state.open = false;
  }

  function activateRuntime() {
    if (!isCurrentInstance()) return false;
    if (!state.runtimeActive) {
      state.runtimeGeneration += 1;
      state.runtimeActive = true;
    }
    const generation = state.runtimeGeneration;
    state.activeTab = normalizeActiveTab();
    installStyle();
    installFloat();
    installContextTracking();
    if (!state.observer && !installObserver()) {
      const domReadyHandler = () => {
        if (state.domReadyHandler === domReadyHandler) state.domReadyHandler = null;
        if (!isCurrentRuntime(generation)) return;
        installObserver();
        installFloat();
        void ensureSettings();
        scheduleScan(0);
      };
      state.domReadyHandler = domReadyHandler;
      document.addEventListener("DOMContentLoaded", domReadyHandler, { once: true });
    }
    scheduleScan(0);
    return true;
  }
