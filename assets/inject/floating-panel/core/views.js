/* Floating-panel views: markup, status presentation, settings, and view events. */

    state.activeTab = "outline";
    renderFloat({ preserveMorph: true });
    void refreshOutline();
    if (!state.open) setOpen(true, "panel");
  }

  function faceEyeHtml() {
    return `<span class="csw-fab-eye"><svg class="csw-fab-happy-arc" viewBox="0 0 18 12" aria-hidden="true" focusable="false"><path d="M1.5 9 C4.6 3.2 13.4 3.2 16.5 9"></path></svg></span>`;
  }

  function faceHtml() {
    return `
      <span class="csw-fab-face" aria-hidden="true">
        ${faceEyeHtml()}
        ${faceEyeHtml()}
      </span>
    `;
  }

  function statusStageHtml() {
    return `<span class="csw-status-stage">${faceHtml()}</span>`;
  }

  function viewTabHtml(view) {
    const isNext = view === "next";
    const active = state.activeTab === view;
    return `<button class="csw-icon" type="button" data-view="${view}" data-reorderable="true" data-active="${active}" role="tab" aria-selected="${active}" title="${isNext ? "下一步建议" : "回答大纲"}" aria-label="${isNext ? "下一步建议" : "回答大纲"}">${iconSvg(isNext ? "next" : "outline")}</button>`;
  }

  function sourceTrackHtml(paneCue = { direction: "single", angle: null }, trackHeight = CHIP_HEIGHT) {
    const angle = Number.isFinite(state.sourceCueAngle) && paneCue.direction !== "single"
      ? state.sourceCueAngle
      : paneCue.angle;
    const cue = paneCueForTrack({ direction: paneCue.direction, angle }, trackHeight);
    return `<span class="csw-source-track" style="--csw-source-track-height:${trackHeight}px" aria-hidden="true"><span class="csw-source-dot" data-direction="${escapeAttr(cue.direction)}" style="--csw-source-x:${cue.x}px;--csw-source-y:${cue.y}px"></span></span>`;
  }

  function normalizeSourceCueDelta(fromAngle, toAngle) {
    return ((toAngle - fromAngle + Math.PI * 3) % (Math.PI * 2)) - Math.PI;
  }

  function cancelSourceCueAnimation() {
    if (!state.sourceCueAnimation) return;
    cancelAnimationFrame(state.sourceCueAnimation);
    state.sourceCueAnimation = 0;
  }

  function applySourceCueAngle(angle, direction) {
    state.sourceCueAngle = angle;
    [
      [state.fab?.querySelector(".csw-source-dot"), CHIP_HEIGHT],
      [state.panel?.querySelector(".csw-head-face .csw-source-dot"), 32],
    ].forEach(([dot, trackHeight]) => {
      if (!dot) return;
      dot.setAttribute("data-direction", direction);
      if (!Number.isFinite(angle)) return;
      const point = capsuleBoundaryPoint(angle, CHIP_WIDTH, trackHeight);
      dot.style.setProperty("--csw-source-x", `${point.x}px`);
      dot.style.setProperty("--csw-source-y", `${point.y}px`);
    });
  }

  function animateSourceCue(paneCue) {
    if (!isCurrentRuntime()) return;
    cancelSourceCueAnimation();
    if (paneCue.direction === "single" || !Number.isFinite(paneCue.angle)) {
      applySourceCueAngle(null, "single");
      return;
    }

    const targetAngle = paneCue.angle;
    if (!Number.isFinite(state.sourceCueAngle) || prefersReducedMotion()) {
      applySourceCueAngle(targetAngle, paneCue.direction);
      return;
    }

    const startAngle = state.sourceCueAngle;
    const delta = normalizeSourceCueDelta(startAngle, targetAngle);
    if (Math.abs(delta) < 0.001) {
      applySourceCueAngle(targetAngle, paneCue.direction);
      return;
    }

    const duration = 180 + Math.min(1, Math.abs(delta) / Math.PI) * 120;
    const generation = state.runtimeGeneration;
    const startedAt = performance.now();
    const tick = (now) => {
      if (!isCurrentRuntime(generation)) return;
      const progress = clamp((now - startedAt) / duration, 0, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      applySourceCueAngle(startAngle + delta * eased, paneCue.direction);
      if (progress < 1) state.sourceCueAnimation = requestAnimationFrame(tick);
      else {
        state.sourceCueAnimation = 0;
        applySourceCueAngle(targetAngle, paneCue.direction);
      }
    };
    state.sourceCueAnimation = requestAnimationFrame(tick);
  }

  function bridgeErrorPresentation(error = state.bridgeError) {
    const text = normalizeText(error);
    const match = FRIENDLY_BRIDGE_ERRORS.find((item) => item.pattern.test(text));
    return match || {
      title: "生成失败，稍后重试",
      message: "",
    };
  }

  function outlineErrorTitle(error = state.outlineError) {
    const text = normalizeText(error);
    if (/找不到对应的小节/i.test(text)) return "找不到对应内容，刷新后再试";
    return FRIENDLY_BRIDGE_ERRORS.find((item) => item.pattern.test(text))?.title || "大纲暂不可用，稍后重试";
  }

  function statusTone(expression) {
    if (expression === "error") return "error";
    if (expression === "answering" || expression === "generating") return "busy";
    if (expression === "ready" || expression === "surprise") return "ready";
    return "idle";
  }

  function statusToneForView(expression) {
    if (state.activeTab === "outline") {
      if (state.outlineStatus === "pending") return "busy";
      if (state.outlineStatus === "error") return "error";
      if (state.outlineItems.length) return "ready";
      return "idle";
    }
    if (state.activeTab === "settings") {
      if (!state.settingsLoaded) return "busy";
      if (/失败|错误|不可用/i.test(state.settingsStatus)) return "error";
      if (outlineEnabled() && !stepwiseEnabled()) return "ready";
      if (stepwiseEnabled()
        && state.settings.baseUrlConfigured
        && state.settings.model
        && state.settings.apiKeyConfigured) return "ready";
      return "idle";
    }
    return statusTone(expression);
  }

  function refreshControlState() {
    if (state.activeTab === "settings") {
      return { blocked: false, title: "重新读取设置" };
    }
    if (state.activeTab === "outline") {
      const blocked = state.outlineStatus === "pending";
      return { blocked, title: blocked ? "正在整理大纲" : "刷新大纲" };
    }
    const blocked = state.bridgeStatus === "pending" || chatBusy();
    return { blocked, title: blocked ? "等待回答完成" : "刷新建议" };
  }

  function refreshCurrentView() {
    if (state.activeTab === "settings") return reloadSettings();
    if (state.activeTab === "outline") return refreshOutline();
    if (!stepwiseEnabled()) return;
    return forceRefreshStepwise();
  }

  // Rendering preserves scroll, active view, and in-flight morph state while replacing only view content.
  function renderFloat(options = {}) {
    if (!isCurrentRuntime()) return;
    if (!options.allowDuringTransition && (state.viewTransitioning || state.morphAnimation)) {
      deferRender();
      return;
    }
    state.activeTab = normalizeActiveTab();
    const viewScroll = captureViewScroll();
    clearPromptInteractionTimers();
    state.viewReorderCleanup?.();
    state.viewReorderCleanup = null;
    cancelViewAnimation();
    installStyle();
    installFloat();
    if (!state.fab || !state.popover || !state.panel || !state.glass) return;
    syncTheme();
    normalizePromptState();
    const expressionNow = Date.now();
    const outlineExpression = usesOutlineExpression(expressionNow);
    const expression = resolveFabExpression(expressionNow);
    const expressionCount = outlineExpression ? state.outlineItems.length : state.prompts.length;
    const expressionLabel = fabExpressionLabel(expression, outlineExpression);
    const featureLabel = stepwiseEnabled() && outlineEnabled()
      ? "悬浮球"
      : stepwiseEnabled() ? "下一步" : "回答大纲";
    const hidden = expression === "hidden";
    if (hidden) {
      settleMorph(0);
    }
    state.fabExpression = expression;
    state.fab.dataset.expression = expression;
    state.fab.dataset.count = String(expressionCount);
    state.fab.title = state.open ? "收起" : `${featureLabel} · ${expressionLabel}`;
    state.fab.setAttribute("aria-label", state.open
      ? "收起"
      : expressionCount > 0 && expression === "ready"
        ? `${featureLabel} · ${expressionLabel} · ${expressionCount} ${outlineExpression ? "个章节" : "条"}`
        : `${featureLabel} · ${expressionLabel}`);
    state.fab.setAttribute("aria-expanded", String(state.open));
    state.root.dataset.hidden = String(hidden);
    state.popover.dataset.open = state.open ? "true" : "false";
    state.popover.dataset.expression = expression;
    state.popover.dataset.view = state.activeTab;
    applyPosition();

    const refreshState = refreshControlState();
    const refreshBlocked = refreshState.blocked;
    const refreshTitle = refreshState.title;
    const headExpression = state.activeTab === "settings" ? "curious" : expression;
    const tone = statusToneForView(expression);
    const paneCue = activePaneCue();
    const viewTabs = enabledViewOrder().map(viewTabHtml).join("");
    state.panel.innerHTML = `
      <div class="csw-head">
        <div class="csw-head-side csw-head-left">
          <div class="csw-tabs csw-view-tabs" role="tablist" aria-label="悬浮球视图">
            <span class="csw-view-indicator" aria-hidden="true"></span>
            ${viewTabs}
          </div>
        </div>
        <button class="csw-head-face" type="button" data-action="collapse" data-expression="${escapeAttr(headExpression)}" data-tone="${tone}" title="收起" aria-label="收起">${statusStageHtml()}${sourceTrackHtml(paneCue, 32)}</button>
        <div class="csw-head-side csw-head-right">
          <button class="csw-icon" type="button" data-action="refresh" title="${escapeAttr(refreshTitle)}" aria-label="${escapeAttr(refreshTitle)}" ${refreshBlocked ? "disabled" : ""}>${iconSvg("refresh")}</button>
          <button class="csw-icon" type="button" data-action="theme" title="${escapeAttr(themeLabel())}" aria-label="${escapeAttr(themeLabel())}">${themeIcon()}</button>
          <button class="csw-icon" type="button" data-view="settings" data-active="${state.activeTab === "settings"}" aria-pressed="${state.activeTab === "settings"}" title="设置" aria-label="设置">${iconSvg("settings")}</button>
        </div>
      </div>
      <div class="csw-body" data-view-body="${state.activeTab}">
        <div class="csw-mouth-stage" data-mouth-stage="${state.activeTab}">${state.activeTab === "settings" ? settingsHtml() : state.activeTab === "outline" ? outlineHtml() : nextHtml()}</div>
      </div>
    `;
    if (state.activeTab === "outline") alignOutlineNestedText();
    animateViewTabSelection(options.viewIndicatorFrom ?? state.activeTab, state.activeTab);
    animateSourceCue(paneCue);
    restoreViewScroll(viewScroll);
    installContentFadeTracking();
    state.panel.querySelectorAll("[data-view]").forEach((button) => {
      button.addEventListener("click", () => {
        if (button.dataset.reorderable === "true"
          && performance.now() < state.suppressViewTabClickUntil) {
          state.suppressViewTabClickUntil = 0;
          return;
        }
        const nextTab = button.dataset.view || "next";
        if (nextTab === state.activeTab) return;
        void switchView(nextTab);
      });
    });
    const headFace = state.panel.querySelector("[data-action='collapse']");
    headFace?.addEventListener("click", onHeadFaceClick);
    bindGlassPointerSurface(headFace);
    state.panel.querySelector("[data-action='refresh']")?.addEventListener("click", () => void refreshCurrentView());
    state.panel.querySelector("[data-action='theme']")?.addEventListener("click", toggleCodexTheme);
    applyMaterial({ animate: false });

    if (state.activeTab === "settings") attachSettingsEvents();
    else if (state.activeTab === "outline") attachOutlineEvents();
    else attachNextEvents();
    installViewTabReorder();
    installPanelDrag();
    syncEyeTracking();
    if (!options.preserveMorph && !state.morphAnimation) settleMorph(state.open ? 1 : 0);
  }

  function nextProgressState() {
    if (state.bridgeStatus === "pending") {
      return {
        title: "正在生成建议",
      };
    }
    if (stepwiseGenerationMode() === "manual") return null;
    if (state.scanStatus === "assistant-changed" || state.scanStatus === "assistant-settling") {
      return {
        title: "正在整理回答",
      };
    }
    if (state.scanStatus === "not-ready" && state.scanBusy) {
      return {
        title: "等待回答完成",
      };
    }
    return null;
  }

  function nextHtml() {
    const progress = nextProgressState();
    if (progress) {
      return `<div class="csw-progress" aria-label="${progress.title}">
        <span class="csw-progress-ring" aria-hidden="true"></span>
        <span class="csw-progress-copy">
          <span class="csw-progress-title">${progress.title}</span>
        </span>
      </div>`;
    }
    if (!state.prompts.length) {
      const empty = nextEmptyState();
      return `<div class="csw-empty" data-state="${escapeAttr(empty.state || "idle")}">
        <div class="csw-empty-title">${escapeHtml(empty.title)}</div>
      </div>`;
    }
    const previewIndex = clamp(Number(state.promptPreviewIndex) || 0, 0, state.prompts.length - 1);
    const previewItem = state.prompts[previewIndex];
    state.promptPreviewIndex = previewIndex;
    return `<div class="csw-next-layout">
      <div class="csw-list" data-label-only="${state.labelOnly}" aria-label="下一步建议">${state.prompts.map((item, index) => `
        <button class="csw-row" type="button" data-index="${index}" data-preview-active="${index === previewIndex}" aria-current="${index === previewIndex ? "true" : "false"}">
          <span class="csw-row-copy">
            <span class="csw-row-label">${escapeHtml(item.label || labelForPrompt(item.prompt))}</span>
            ${state.labelOnly ? "" : `<span class="csw-row-prompt">${escapeHtml(item.summary || summaryForPrompt(item.prompt))}</span>`}
          </span>
          <span class="csw-row-arrow" aria-hidden="true">›</span>
        </button>
      `).join("")}</div>
      <section class="csw-prompt-preview" data-preview-index="${previewIndex}" aria-label="建议完整内容">
        <div class="csw-prompt-preview-scroll" tabindex="0">
          <div class="csw-prompt-preview-content">
            <span class="csw-prompt-preview-kicker">${previewIndex + 1} / ${state.prompts.length}</span>
            <span class="csw-prompt-preview-title">${escapeHtml(previewItem.label || labelForPrompt(previewItem.prompt))}</span>
            <span class="csw-prompt-preview-body">${escapeHtml(previewItem.prompt)}</span>
          </div>
        </div>
      </section>
    </div>`;
  }

  function nextEmptyState() {
    if (state.bridgeError || state.bridgeStatus === "failed") return bridgeErrorPresentation();
    if (state.bridgeStatus === "ok") {
      return {
        title: "暂无建议",
        message: "",
      };
    }
    if (state.bridgeStatus === "disabled") {
      return {
        title: "功能已关闭",
        message: "",
      };
    }
    if (stepwiseGenerationMode() === "manual") {
      return {
        title: "当前为手动模式",
        message: "",
        state: "manual",
      };
    }
    return {
      title: "等待回答完成",
      message: "",
    };
  }

  function attachNextEvents() {
    state.panel.querySelectorAll(".csw-row").forEach((button) => {
      button.addEventListener("pointerenter", () => schedulePromptPreview(button));
      button.addEventListener("pointerleave", cancelScheduledPromptPreview);
      button.addEventListener("focus", () => showPromptPreview(button, true));
      button.addEventListener("click", (event) => {
        if (event.detail >= 2) {
          event.preventDefault();
          if (state.promptClickTimer) window.clearTimeout(state.promptClickTimer);
          state.promptClickTimer = 0;
          showPromptPreview(button, true);
          selectPrompt(button, promptClickSubmits(event.detail));
          return;
        }

        if (state.promptClickTimer) window.clearTimeout(state.promptClickTimer);
        const generation = state.runtimeGeneration;
        state.promptClickTimer = window.setTimeout(() => {
          state.promptClickTimer = 0;
          if (!isCurrentRuntime(generation) || !button.isConnected) return;
          showPromptPreview(button, true);
          selectPrompt(button, promptClickSubmits(1));
        }, PROMPT_CLICK_DELAY_MS);
      });
      button.addEventListener("dblclick", (event) => event.preventDefault());
    });
  }

  function clearPromptInteractionTimers() {
    if (state.promptPreviewTimer) window.clearTimeout(state.promptPreviewTimer);
    if (state.promptClickTimer) window.clearTimeout(state.promptClickTimer);
    state.promptPreviewTimer = 0;
    state.promptClickTimer = 0;
  }

  function schedulePromptPreview(button) {
    if (state.promptPreviewTimer) window.clearTimeout(state.promptPreviewTimer);
    state.promptPreviewTimer = 0;
    showPromptPreview(button);
  }

  function cancelScheduledPromptPreview() {
    if (state.promptPreviewTimer) window.clearTimeout(state.promptPreviewTimer);
    state.promptPreviewTimer = 0;
  }

  function showPromptPreview(button, immediate = false) {
    const index = Number(button.dataset.index);
    const item = state.prompts[index];
    const preview = state.panel?.querySelector(".csw-prompt-preview");
    if (!item?.prompt || !preview) return;

    if (Number(preview.dataset.previewIndex) === index) {
      state.panel.querySelectorAll(".csw-row").forEach((row) => {
        const active = row === button;
        row.dataset.previewActive = String(active);
        row.setAttribute("aria-current", active ? "true" : "false");
      });
      preview.removeAttribute("data-switching");
      return;
    }

    const applyPreview = () => {
      if (!button.isConnected || !preview.isConnected) return;
      state.panel.querySelectorAll(".csw-row").forEach((row) => {
        const active = row === button;
        row.dataset.previewActive = String(active);
        row.setAttribute("aria-current", active ? "true" : "false");
      });
      const title = preview.querySelector(".csw-prompt-preview-title");
      const kicker = preview.querySelector(".csw-prompt-preview-kicker");
      const body = preview.querySelector(".csw-prompt-preview-body");
      const scroll = preview.querySelector(".csw-prompt-preview-scroll");
      if (kicker) kicker.textContent = `${index + 1} / ${state.prompts.length}`;
      if (title) title.textContent = item.label || labelForPrompt(item.prompt);
      if (body) body.textContent = item.prompt;
      if (scroll) scroll.scrollTop = 0;
      preview.dataset.previewIndex = String(index);
      state.promptPreviewIndex = index;
      syncContentFade();
      window.requestAnimationFrame(() => {
        preview.removeAttribute("data-switching");
        if (preview.isConnected) syncContentFade();
      });
    };

    if (immediate) {
      if (state.promptPreviewTimer) window.clearTimeout(state.promptPreviewTimer);
      state.promptPreviewTimer = 0;
      preview.removeAttribute("data-switching");
      applyPreview();
      return;
    }
    const generation = state.runtimeGeneration;
    state.promptPreviewTimer = window.setTimeout(() => {
      state.promptPreviewTimer = 0;
      if (!isCurrentRuntime(generation) || !button.matches(":hover, :focus, :focus-within")) return;
      preview.dataset.switching = "true";
      applyPreview();
    }, PROMPT_PREVIEW_SWITCH_MS);
  }

  function selectPrompt(button, submit) {
    const item = state.prompts[Number(button.dataset.index)];
    if (!item?.prompt) return;
    pushDiagnostic("prompt:select", {
      submit,
      clickMode: state.promptClickMode,
      index: Number(button.dataset.index),
    });
    fillComposer(item.prompt, submit);
  }

  function promptClickSubmits(clickDetail, value = state.promptClickMode) {
    const mode = normalizePromptClickMode(value);
    if (mode === "direct") return true;
    if (mode === "fill") return false;
    return clickDetail >= 2;
  }

  function settingsModelLabel(settings) {
    if (settings && !stepwiseEnabled(settings)) {
      return outlineEnabled(settings) ? "回答大纲" : "未启用";
    }
    const raw = normalizeText(settings?.model);
    if (!raw) return settings ? "未配置" : "读取中";
    const leaf = raw.split("/").pop() || raw;
    return leaf
      .replace(/^gpt[-_:]?/i, "")
      .split(/[-_\s]+/)
      .filter(Boolean)
      .map((part) => (/^\d/.test(part) ? part : `${part.charAt(0).toUpperCase()}${part.slice(1)}`))
      .join(" ");
  }

  function settingsRuntimePresentation(settings) {
    if (!settings) return { label: "正在读取设置", tone: "busy" };
    if (!runtimeEnabled(settings)) return { label: "已关闭", tone: "idle" };
    if (stepwiseEnabled(settings) && (!settings.baseUrlConfigured || !settings.model || !settings.apiKeyConfigured)) {
      return { label: "等待配置", tone: "error" };
    }
    const expressionNow = Date.now();
    const outlineExpression = usesOutlineExpression(expressionNow);
    const expression = resolveFabExpression(expressionNow);
    const detail = (outlineExpression ? {
      idle: "等待回答",
      answering: "回答中",
      surprise: "正在整理回答",
      generating: "正在整理大纲",
      ready: `${state.outlineItems.length} 个章节已准备`,
      empty: "暂无大纲",
      error: "生成失败",
      hidden: "已关闭",
    } : {
      idle: "等待回答",
      answering: "回答中",
      surprise: "正在整理回答",
      generating: "正在生成建议",
      ready: `${state.prompts.length} 条建议已准备`,
      empty: "暂无建议",
      error: "生成失败",
      hidden: "已关闭",
    })[expression] || "等待回答";
    if (!outlineExpression && stepwiseWaitingForManualRefresh(settings)) {
      return { label: "当前为手动模式", tone: "idle" };
    }
    return { label: detail, tone: statusTone(expression) };
  }

  function settingsCommandHtml(action, icon, label, title, options = {}) {
    return `
      <button class="csw-command-button" type="button" data-action="${escapeAttr(action)}" title="${escapeAttr(title)}" aria-label="${escapeAttr(title)}" ${options.disabled ? "disabled" : ""}>
        <span class="csw-command-icon" data-busy="${options.busy === true}" aria-hidden="true">${iconSvg(icon)}</span>
        <span class="csw-command-label">${escapeHtml(label)}</span>
      </button>
    `;
  }

  function promptClickModeLabel(value = state.promptClickMode) {
    return {
      direct: "直接发送",
      hybrid: "单击填入 · 双击发送",
      fill: "仅填入",
    }[normalizePromptClickMode(value)];
  }

  function generationModeLabel(value = stepwiseGenerationMode()) {
    return normalizeGenerationMode(value) === "manual" ? "手动刷新" : "自动生成";
  }

  function nextGenerationMode(value = stepwiseGenerationMode()) {
    const index = GENERATION_MODES.indexOf(normalizeGenerationMode(value));
    return GENERATION_MODES[(index + 1) % GENERATION_MODES.length];
  }

  function nextPromptClickMode(value = state.promptClickMode) {
    const index = PROMPT_CLICK_MODES.indexOf(normalizePromptClickMode(value));
    return PROMPT_CLICK_MODES[(index + 1) % PROMPT_CLICK_MODES.length];
  }

  function generationModeButtonLabel(value = stepwiseGenerationMode()) {
    return `模式：${generationModeLabel(value)}；切换为${generationModeLabel(nextGenerationMode(value))}`;
  }

  function promptClickModeButtonLabel(value = state.promptClickMode) {
    return `点击：${promptClickModeLabel(value)}；切换为${promptClickModeLabel(nextPromptClickMode(value))}`;
  }

  function toggleGenerationMode(event) {
    event?.preventDefault();
    event?.stopPropagation();
    return setGenerationMode(nextGenerationMode());
  }

  function togglePromptClickMode(event) {
    event?.preventDefault();
    event?.stopPropagation();
    return writePromptClickMode(nextPromptClickMode());
  }

  function appearanceSettingsHtml() {
    return `
      <div class="csw-control-deck" aria-label="外观、字号与显示">
        <div class="csw-control-group">
          <span class="csw-control-label">外观</span>
          <span class="csw-control-row">
            <button class="csw-control-button" type="button" data-action="material" data-material="${state.material}" title="${escapeAttr(materialButtonLabel())}" aria-label="${escapeAttr(materialButtonLabel())}"><span data-material-value>${materialValueLabel()}</span></button>
          </span>
        </div>
        <div class="csw-control-group">
          <span class="csw-control-label" title="同时调整下一步与大纲内容字号">字号</span>
          <span class="csw-stepper" aria-label="下一步与大纲内容字号">
            <button class="csw-step-button" type="button" data-action="font-dec" title="减小字体" aria-label="减小字体" ${effectiveFontSize() <= MIN_FONT ? "disabled" : ""}>−</button>
            <span class="csw-step-value" aria-live="polite">${fontSizeLabel()}</span>
            <button class="csw-step-button" type="button" data-action="font-inc" title="增大字体" aria-label="增大字体" ${effectiveFontSize() >= MAX_FONT ? "disabled" : ""}>+</button>
          </span>
        </div>
        <div class="csw-control-group">
          <span class="csw-control-label">显示</span>
          <span class="csw-control-row">
            <button class="csw-control-button" type="button" data-action="label-only" aria-pressed="${state.labelOnly}" title="切换显示方式：${state.labelOnly ? "标题 + 摘要" : "仅标题"}"><span data-label-only-value>${state.labelOnly ? "仅标题" : "标题 + 摘要"}</span></button>
          </span>
        </div>
      </div>
    `;
  }

  function settingsHtml() {
    const settings = state.settingsLoaded ? state.settings : null;
    const runtime = settingsRuntimePresentation(settings);
    const model = settingsModelLabel(settings);
    const notice = settings ? settingsNotice(settings) : "";
    const noticeTone = /失败|错误|未配置|关闭|不可用|需要/i.test(notice) ? "warn" : "plain";
    const testing = state.settingsStatus === "正在检查连接";
    return `
      <div class="csw-settings">
        <section class="csw-settings-surface" data-loading="${!settings}" aria-label="悬浮球设置" aria-busy="${!settings}">
          <div class="csw-settings-hero">
            <div class="csw-model-pane">
              <strong class="csw-model-value" title="${escapeAttr(settings?.model || model)}">${escapeHtml(model)}</strong>
              <span class="csw-runtime-line">
                <span class="csw-runtime-dot" data-tone="${escapeAttr(runtime.tone)}" aria-hidden="true"></span>
                <span class="csw-runtime-copy">${escapeHtml(runtime.label)}</span>
              </span>
            </div>
            ${appearanceSettingsHtml()}
          </div>
          <div class="csw-settings-footer" aria-label="配置摘要与设置操作">
            <div class="csw-runtime-grid" aria-label="配置摘要">
              <div class="csw-generation-mode" data-generation-mode-control>
                <span class="csw-metric-label">模式</span>
                <button class="csw-metric-action" type="button" data-action="generation-mode" title="${escapeAttr(generationModeButtonLabel())}" aria-label="${escapeAttr(generationModeButtonLabel())}">
                  <span data-generation-mode-value>${generationModeLabel()}</span>
                </button>
              </div>
              <div class="csw-click-mode" data-prompt-click-control>
                <span class="csw-metric-label">点击</span>
                <button class="csw-metric-action" type="button" data-action="prompt-click-mode" title="${escapeAttr(promptClickModeButtonLabel())}" aria-label="${escapeAttr(promptClickModeButtonLabel())}">
                  <span data-prompt-click-mode-value>${promptClickModeLabel()}</span>
                </button>
              </div>
            </div>
            <div class="csw-command-deck" aria-label="设置操作">
              ${settingsCommandHtml("open-manager", "open-config", "配置", "在 Codex++ 中配置")}
              ${settingsCommandHtml("test-settings", testing ? "refresh" : "connection", "检查", "检查连接", { disabled: settings?.enabled !== true, busy: testing })}
            </div>
            ${notice ? `<div class="csw-settings-notice" data-tone="${noticeTone}" aria-live="polite">${escapeHtml(notice)}</div>` : ""}
          </div>
        </section>
      </div>
    `;
  }

  function settingsNotice(settings) {
    const status = state.settingsStatus || "";
    const line = statusLine(settings);
    if (!status || status === line) {
      if (stepwiseEnabled(settings) && settings.baseUrlConfigured && settings.model && settings.apiKeyConfigured) return "";
      if (outlineEnabled(settings) && !stepwiseEnabled(settings)) return "";
      return line;
    }
    return status;
  }

  function statusLine(settings) {
    if (!runtimeEnabled(settings)) return "悬浮球已关闭";
    if (!stepwiseEnabled(settings)) return "仅显示大纲";
    if (!settings.baseUrlConfigured || !settings.model) return "尚未配置服务地址或模型";
    if (!settings.apiKeyConfigured) return "尚未配置密钥";
    return `连接就绪 · ${settings.model || ""}`.replace(/\s+·\s+$/, "");
  }

  function attachSettingsEvents() {
    state.panel.querySelector("[data-action='material']")?.addEventListener("click", toggleMaterial);
    state.panel.querySelector("[data-action='label-only']")?.addEventListener("click", toggleLabelOnly);
    state.panel.querySelector("[data-action='font-dec']")?.addEventListener("click", () => bumpFontSize(-1));
    state.panel.querySelector("[data-action='font-inc']")?.addEventListener("click", () => bumpFontSize(1));
    state.panel.querySelector("[data-action='open-manager']")?.addEventListener("click", () => void openManager());
    state.panel.querySelector("[data-action='test-settings']")?.addEventListener("click", () => void testSettings());
    state.panel.querySelector("[data-action='generation-mode']")?.addEventListener("click", (event) => {
      void toggleGenerationMode(event);
    });
    state.panel.querySelector("[data-action='prompt-click-mode']")?.addEventListener("click", togglePromptClickMode);
  }

  function writePromptClickMode(value) {
    state.promptClickMode = normalizePromptClickMode(value);
    storage.set(PROMPT_CLICK_MODE_KEY, state.promptClickMode);
    const trigger = state.panel?.querySelector("[data-action='prompt-click-mode']");
    const display = trigger?.querySelector("[data-prompt-click-mode-value]");
    const label = promptClickModeButtonLabel();
    if (trigger) {
      trigger.title = label;
      trigger.setAttribute("aria-label", label);
    }
    if (display) display.textContent = promptClickModeLabel();
    return state.promptClickMode;
  }

  function updateGenerationModeControl(value = stepwiseGenerationMode(), busy = false) {
    const mode = normalizeGenerationMode(value);
    const trigger = state.panel?.querySelector("[data-action='generation-mode']");
    const display = trigger?.querySelector("[data-generation-mode-value]");
    const label = generationModeButtonLabel(mode);
    if (trigger) {
      trigger.title = label;
      trigger.setAttribute("aria-label", label);
      trigger.setAttribute("aria-busy", String(busy));
      trigger.disabled = busy;
    }
    if (display) display.textContent = generationModeLabel(mode);
  }

  async function setGenerationMode(value) {
    if (!isCurrentRuntime() || !state.settingsLoaded || !stepwiseEnabled()) return;
    const runtimeGeneration = state.runtimeGeneration;
    const previousMode = stepwiseGenerationMode();
    const nextMode = normalizeGenerationMode(value);
    if (nextMode === previousMode) return;
    const previousSettings = state.settings;
    const cancelAutoRequestImmediately = previousMode === "auto" && nextMode === "manual";
    const requestEpoch = ++settingsSyncEpoch;
    settingsRequestId += 1;
    settingsPromise = null;
    if (cancelAutoRequestImmediately) {
      applyRuntimeSettings({ ...(state.settings || {}), generationMode: nextMode });
      scheduleScan(0);
    }
    updateGenerationModeControl(nextMode, true);

    const payload = await bridgeCall("/settings/set", {
      codexAppStepwiseGenerationMode: nextMode,
    });
    if (!isCurrentRuntime(runtimeGeneration) || requestEpoch !== settingsSyncEpoch) return;
    if (payload?.error) {
      if (cancelAutoRequestImmediately) {
        applyRuntimeSettings(previousSettings);
        scheduleScan(0);
      }
      state.settingsStatus = payload.error || "模式保存失败";
      renderFloat();
      return;
    }

    pendingSettingsPatch = { ...pendingSettingsPatch, generationMode: nextMode };
    if (!cancelAutoRequestImmediately) {
      applyRuntimeSettings({ ...(state.settings || {}), generationMode: nextMode });
    }
    state.settingsStatus = statusLine(state.settings);
    updateGenerationModeControl(nextMode);
    scheduleScan(0);

    settingsPromise = null;
    await reloadSettings();
  }

  // Manager settings are the source of truth; local UI state is updated only after request identity checks.
  async function loadSettings() {
    const requestId = ++settingsRequestId;
    const requestEpoch = settingsSyncEpoch;
    const payload = await bridgeCall("/stepwise/settings", {});
    if (!isCurrentInstance()
      || requestId !== settingsRequestId
      || requestEpoch !== settingsSyncEpoch) return null;
    let shouldRender = false;
    if (payload?.settings) {
      const nextSettings = { ...payload.settings, ...pendingSettingsPatch };
      if (!Object.prototype.hasOwnProperty.call(nextSettings, "generationMode")) {
        nextSettings.generationMode = stepwiseGenerationMode();
      }
      pendingSettingsPatch = {};
      const settingsChanged = !state.settingsLoaded
        || settingsFingerprint(nextSettings) !== state.settingsFingerprint;
      state.settingsLoaded = true;
      if (settingsChanged) applyRuntimeSettings(nextSettings);
      if (runtimeEnabled(nextSettings)) {
        if (!state.runtimeActive) activateRuntime();
        if (settingsChanged) {
          state.settingsStatus = statusLine(nextSettings);
          shouldRender = true;
          scheduleScan(0);
        }
      } else if (state.runtimeActive) {
        stopRuntime();
      }
    } else {
      const nextStatus = payload?.error || "Bridge 未就绪";
      shouldRender = nextStatus !== state.settingsStatus;
      state.settingsStatus = nextStatus;
    }
    if (shouldRender && isCurrentRuntime()) renderFloat();
    return state.settings;
  }

  function reloadSettings() {
    if (!settingsPromise) {
      const request = loadSettings();
      const tracked = request.finally(() => {
        if (settingsPromise === tracked) settingsPromise = null;
      });
      settingsPromise = tracked;
    }
    return settingsPromise;
  }

  function scheduleSettingsSync(delay = SETTINGS_SYNC_INTERVAL_MS) {
    if (!isCurrentInstance()) return;
    if (state.settingsSyncTimer) window.clearTimeout(state.settingsSyncTimer);
    state.settingsSyncTimer = window.setTimeout(async () => {
      state.settingsSyncTimer = 0;
      try {
        await reloadSettings();
      } catch (error) {
        pushDiagnostic("settings:sync-error", {
          message: String(error?.message || error || "settings sync failed"),
        });
      } finally {
        scheduleSettingsSync();
      }
    }, delay);
  }

  async function ensureSettings() {
    if (state.settingsLoaded) return state.settings;
    return reloadSettings();
  }

  async function testSettings() {
    if (!isCurrentRuntime()) return;
    const generation = state.runtimeGeneration;
    state.settingsStatus = "正在检查连接";
    renderFloat();
    const payload = await bridgeCall("/stepwise/test", {});
    if (!isCurrentRuntime(generation)) return;
    const count = Array.isArray(payload?.items) ? payload.items.length : 0;
    state.settingsStatus = payload?.error || (payload?.disabled ? "功能已关闭" : `连接正常 · ${count} 条`);
    renderFloat();
  }

  async function openManager() {
    if (!isCurrentRuntime()) return;
    const generation = state.runtimeGeneration;
    state.settingsStatus = "正在打开 Codex++...";
    renderFloat();
    const payload = await bridgeCall("/manager/open-transient", {
      page: "settings",
      section: "stepwise",
    });
    if (!isCurrentRuntime(generation)) return;
    state.settingsStatus = payload?.status === "ok" ? "已打开 Codex++" : payload?.message || "打开失败";
    renderFloat();
  }
