/* Stepwise generation: request lifecycle, cache, composer, and prompt submission. */

  function bridgeRequestKey(userText, assistantText) {
    return hashText(`${state.activeContext.sessionId}\n${shortText(userText, 2400)}\n\n--- assistant ---\n\n${shortText(assistantText, 5200)}`);
  }

  // Bridge requests are deduplicated by answer identity and guarded against late responses from older turns.
  function requestBridgeStepwise(key, userText, assistantText, requestMode = stepwiseGenerationMode(), options = {}) {
    if (!stepwiseEnabled() || !key || state.bridgePendingHash === key || state.bridgeCache.has(key)) return;

    const normalizedMode = normalizeGenerationMode(requestMode);
    if (normalizedMode === "manual" && options.userInitiated !== true) return;
    pushDiagnostic("bridge:generate-request", {
      userTextLength: userText.length,
      assistantTextLength: assistantText.length,
      mode: normalizedMode,
    });
    const requestContext = contextSnapshot();
    const requestEpoch = state.stepwiseEpoch;
    const requestId = ++state.bridgeRequestSequence;
    const requestAssistantMessageId = requestContext.assistantMessageId;
    const requestOwned = () => state.bridgePendingHash === key
      && state.bridgePendingRequestId === requestId
      && state.bridgePendingMode === normalizedMode;
    const requestCurrent = () => stepwiseEnabled()
      && stepwiseGenerationMode() === normalizedMode
      && requestEpoch === state.stepwiseEpoch
      && contextMatches(requestContext)
      && state.activeContext.assistantMessageId === requestAssistantMessageId
      && state.bridgeActiveKey === key
      && !chatBusy();
    state.bridgePendingHash = key;
    state.bridgePendingRequestId = requestId;
    state.bridgePendingMode = normalizedMode;
    state.bridgeStatus = "pending";
    state.bridgeError = "";
    state.promptContext = requestContext;
    renderFloat();

    bridgeCall(
      "/stepwise/generate",
      {
        request: {
        lastUserMessage: userText,
        lastAssistantMessage: assistantText,
        threadTitle: document.title || "",
        pageUrl: location.href,
      },
      }
    )
      .then((payload) => {
        if (!requestOwned() || !requestCurrent()) return;
        const prompts = payload?.disabled || payload?.error ? [] : payloadPrompts(payload);
        pushDiagnostic("bridge:generate-result", {
          status: payload?.status || "",
          disabled: Boolean(payload?.disabled),
          error: normalizeText(payload?.error || ""),
          rawItemCount: payloadItems(payload).length,
          promptCount: prompts.length,
          payloadKeys: payload && typeof payload === "object" ? Object.keys(payload).slice(0, 12) : [],
        });
        const bridgeStatus = payload?.disabled ? "disabled" : payload?.error ? "failed" : "ok";
        state.bridgeCache.set(key, {
          status: bridgeStatus,
          disabled: Boolean(payload?.disabled),
          error: normalizeText(payload?.error || ""),
          prompts,
        });
        state.bridgeStatus = bridgeStatus;
        state.bridgeError = normalizeText(payload?.error || "");
        state.promptContext = requestContext;
        if (bridgeStatus === "ok") triggerCompletionBeam(prompts.length);
      })
      .catch((error) => {
        if (!requestOwned() || !requestCurrent()) return;
        pushDiagnostic("bridge:generate-failed", { error: error.message });
        state.bridgeCache.set(key, {
          status: "failed",
          disabled: true,
          error: error.message,
          prompts: [],
        });
        state.bridgeStatus = "failed";
        state.bridgeError = error.message;
      })
      .finally(() => {
        if (!requestOwned()) return;
        state.bridgePendingHash = "";
        state.bridgePendingRequestId = 0;
        state.bridgePendingMode = stepwiseGenerationMode();
        if (state.bridgeStatus === "pending") {
          state.bridgeStatus = "idle";
          state.bridgeError = "";
          state.promptContext = null;
        }
        scheduleScan(0);
      });
  }

  function forceRefreshStepwise() {
    if (!isCurrentRuntime() || !stepwiseEnabled()) return;
    if (state.bridgeStatus === "pending") {
      setScanStatus("manual-refresh-pending", {});
      return;
    }
    if (chatBusy()) {
      if (!state.prompts.length) state.bridgeError = "回答生成中，结束后再刷新";
      setScanStatus("manual-refresh-busy", {});
      renderFloat();
      return;
    }

    const message = findLatestAssistantMessage();
    if (!message) {
      state.bridgeError = "未找到可用于生成的回答";
      state.prompts = [];
      state.promptContext = null;
      state.promptPreviewIndex = 0;
      setScanStatus("manual-refresh-no-assistant", {});
      renderFloat();
      return;
    }

    const nextAssistantMessageId = assistantMessageId(message);
    if (state.activeContext.assistantMessageId !== nextAssistantMessageId) {
      state.activeContext.assistantMessageId = nextAssistantMessageId;
    }

    const stepwisePayload = extractStepwisePayload(message);
    hideStepwisePayload(message.node);
    const assistantText = shortText(stepwisePayload.textWithoutPayload || message.text);
    const userText = findPreviousUserText(message);
    const bridgeKey = bridgeRequestKey(userText, assistantText);
    const generationMode = stepwiseGenerationMode();
    state.bridgeActiveKey = bridgeKey;
    state.stepwiseEpoch += 1;
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.bridgePendingMode = generationMode;
    if (bridgeKey) state.bridgeCache.delete(bridgeKey);

    state.lastAssistantHash = hashText(assistantText);
    state.lastAssistantAt = 0;
    state.currentHash = `${state.lastAssistantHash}:manual-refresh`;
    state.prompts = [];
    state.promptContext = contextSnapshot();
    state.promptPreviewIndex = 0;
    state.bridgeError = "";
    setScanStatus("manual-refresh", { hash: state.lastAssistantHash, textLength: assistantText.length });
    requestBridgeStepwise(bridgeKey, userText, assistantText, generationMode, { userInitiated: true });
    renderFloat();
  }

  function clearPromptsForNewAssistant(hash) {
    state.stepwiseEpoch += 1;
    state.bridgeActiveKey = "";
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.bridgePendingMode = stepwiseGenerationMode();
    state.bridgeStatus = state.bridgePendingMode === "manual" ? "manual-ready" : "idle";
    state.currentHash = `${hash}:pending`;
    state.prompts = [];
    state.promptContext = contextSnapshot();
    state.promptPreviewIndex = 0;
    state.bridgeError = "";
    renderFloat();
  }

  function composerRootForContext(snapshot = state.promptContext) {
    if (snapshot?.paneKey) return rootForContext(snapshot.paneKey, snapshot.sessionId);
    return chatRoot();
  }

  function composerTargetForContext(snapshot = state.promptContext) {
    const root = composerRootForContext(snapshot);
    if (!root) return null;
    return mainComposerCandidate(composerCandidates(root), root);
  }

  function setNativeValue(element, value) {
    const prototype = Object.getPrototypeOf(element);
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "value");
    if (descriptor && typeof descriptor.set === "function") descriptor.set.call(element, value);
    else element.value = value;
  }

  function composerText(target) {
    if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) return normalizeText(target.value);
    return normalizeText(target?.innerText || target?.textContent || "");
  }

  function pressEnter(target) {
    target.focus();
    const base = {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      which: 13,
      bubbles: true,
      cancelable: true,
      composed: true,
    };
    const down = target.dispatchEvent(new KeyboardEvent("keydown", base));
    target.dispatchEvent(new KeyboardEvent("keypress", base));
    target.dispatchEvent(new KeyboardEvent("keyup", base));
    pushDiagnostic("submit:enter-fallback", { defaultAllowed: down });
    return true;
  }

  function submitComposer(target, allowFallback = false) {
    if (!(target instanceof HTMLElement)) return false;
    if (composerBusy(target)) {
      pushDiagnostic("submit:blocked-local-stop", { attemptFallback: allowFallback });
      return false;
    }

    const button = nearbySubmitButton(target);
    if (button) {
      pushDiagnostic("submit:button-click", {
        label: buttonLabel(button),
        disabled: disabledButton(button),
        rect: rectSummary(button),
        className: String(button.className || "").slice(0, 160),
        composerTextLength: composerText(target).length,
        iconPath: iconPathData(button).slice(0, 160),
      });
      button.click();
      return true;
    }

    const pendingButton = nearbySubmitButton(target, { includeDisabled: true });
    if (pendingButton && disabledButton(pendingButton)) {
      pushDiagnostic("submit:button-disabled", {
        label: buttonLabel(pendingButton),
        rect: rectSummary(pendingButton),
        className: String(pendingButton.className || "").slice(0, 160),
        composerTextLength: composerText(target).length,
        iconPath: iconPathData(pendingButton).slice(0, 160),
      });
      return false;
    }

    const form = target.closest("form");
    if (form && allowFallback) {
      pushDiagnostic("submit:form-fallback", { rect: rectSummary(form) });
      try {
        form.requestSubmit();
      } catch {
        pushDiagnostic("submit:form-fallback-failed", {});
        return false;
      }
      return true;
    }

    if (allowFallback) return pressEnter(target);
    pushDiagnostic("submit:no-button-yet", { allowFallback });
    return false;
  }

  function submitComposerWhenReady(target, expectedText = "", attempt = 0) {
    let currentTarget = target;
    if (!(currentTarget instanceof HTMLElement)) return false;
    if (!document.contains(currentTarget)) {
      currentTarget = composerTargetForContext(state.promptContext || state.activeContext);
      pushDiagnostic("submit:target-detached", {
        attempt,
        rebound: Boolean(currentTarget),
        paneKey: state.promptContext?.paneKey || state.activeContext.paneKey,
        sessionId: state.promptContext?.sessionId || state.activeContext.sessionId,
      });
      if (!currentTarget) {
        if (attempt >= SUBMIT_RETRY_LIMIT) return false;
        window.setTimeout(() => submitComposerWhenReady(target, expectedText, attempt + 1), SUBMIT_RETRY_DELAY_MS);
        return false;
      }
    }
    if (normalizeText(expectedText) && composerText(currentTarget) !== normalizeText(expectedText)) {
      pushDiagnostic("submit:composer-changed", {
        attempt,
        expectedLength: normalizeText(expectedText).length,
        actualLength: composerText(currentTarget).length,
      });
      return false;
    }
    if (composerBusy(currentTarget)) {
      if (attempt === 0 || attempt % 10 === 0 || attempt >= SUBMIT_RETRY_LIMIT) {
        pushDiagnostic("submit:blocked-local-stop", {
          attempt,
          retrying: attempt < SUBMIT_RETRY_LIMIT,
          targetRect: rectSummary(currentTarget),
        });
      }
      if (attempt >= SUBMIT_RETRY_LIMIT) {
        pushDiagnostic("submit:blocked-local-stop-timeout", { attempt, targetRect: rectSummary(currentTarget) });
        return false;
      }
      window.setTimeout(() => submitComposerWhenReady(currentTarget, expectedText, attempt + 1), SUBMIT_RETRY_DELAY_MS);
      return false;
    }
    if (submitComposer(currentTarget, attempt >= SUBMIT_RETRY_LIMIT)) return true;
    if (attempt >= SUBMIT_RETRY_LIMIT) return false;
    window.setTimeout(() => submitComposerWhenReady(currentTarget, expectedText, attempt + 1), SUBMIT_RETRY_DELAY_MS);
    return false;
  }

  function setEditableText(target, prompt) {
    target.focus();
    const selection = window.getSelection?.();
    const range = document.createRange();
    range.selectNodeContents(target);
    selection?.removeAllRanges();
    selection?.addRange(range);

    let inserted = false;
    try {
      inserted = document.execCommand?.("insertText", false, prompt) === true;
    } catch {
      inserted = false;
    }
    if (!inserted) target.textContent = prompt;
  }

  function fillComposer(prompt, submit = false) {
    const context = state.promptContext || state.activeContext;
    const targetRoot = composerRootForContext(context);
    const candidates = targetRoot ? composerCandidates(targetRoot) : [];
    const target = targetRoot
      ? mainComposerCandidate(candidates, targetRoot)
      : null;
    pushDiagnostic("fill:start", {
      submit,
      candidateCount: candidates.length,
      paneKey: context?.paneKey || "",
      sessionId: context?.sessionId || "",
      targetTag: target?.tagName || "",
      targetRole: target?.getAttribute?.("role") || "",
      targetClass: String(target?.className || "").slice(0, 120),
      targetRect: rectSummary(target),
      chatRootRect: rectSummary(targetRoot),
      promptLength: normalizeText(prompt).length,
    });
    if (!target) {
      pushDiagnostic("fill:no-main-composer", { candidateCount: candidates.length });
      window.prompt("Copy Stepwise prompt", prompt);
      return false;
    }

    target.focus();
    if (target instanceof HTMLTextAreaElement || target instanceof HTMLInputElement) {
      setNativeValue(target, prompt);
      target.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: prompt }));
      target.dispatchEvent(new Event("change", { bubbles: true }));
      pushDiagnostic("fill:text-control", { valueLength: normalizeText(target.value).length });
      if (submit) submitComposerWhenReady(target, prompt);
      return true;
    }

    if (target.isContentEditable || target.getAttribute("role") === "textbox") {
      setEditableText(target, prompt);
      target.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: prompt }));
      pushDiagnostic("fill:editable", { valueLength: normalizeText(target.textContent).length });
      if (submit) window.setTimeout(() => submitComposerWhenReady(target, prompt), EDITABLE_SUBMIT_DELAY_MS);
      return true;
    }

    window.prompt("Copy Stepwise prompt", prompt);
    return false;
  }

  // Scanning observes the pinned conversation, settles streamed answers, and schedules only necessary work.
