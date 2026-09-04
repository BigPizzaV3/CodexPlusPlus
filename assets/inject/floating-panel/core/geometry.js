/* Floating-panel geometry: position, morphing, and shell boundaries. */

  // Shell geometry is sampled from the capsule through the horizontal intermediate to the panel.
  function lerp(from, to, progress) {
    return from + (to - from) * progress;
  }

  function axisEase(progress) {
    const value = clamp(progress, 0, 1);
    const eased = 1 - Math.pow(1 - value, 1.25);
    return eased * 0.4 + value * 0.6;
  }

  function expandMotionU(progress) {
    return clamp(progress, 0, 1);
  }

  function defaultPosition() {
    const bounds = contentSafeBounds();
    return clampPosition({
      x: bounds.right - CHIP_WIDTH,
      y: Math.min(bounds.bottom - CHIP_HEIGHT, bounds.top + 44),
    }, false);
  }

  function savedPosition() {
    try {
      const parsed = JSON.parse(localStorage.getItem(POSITION_KEY) || "null");
      if (Number.isFinite(parsed?.x) && Number.isFinite(parsed?.y)) return clampPosition(parsed);
    } catch {}
    return defaultPosition();
  }

  function contentSafeBounds() {
    const viewportWidth = Math.max(80, window.innerWidth || 0);
    const viewportHeight = Math.max(80, window.innerHeight || 0);
    let left = PANEL_SAFE_MARGIN;
    let top = PANEL_SAFE_MARGIN;
    let right = viewportWidth - PANEL_SAFE_MARGIN;
    let bottom = viewportHeight - PANEL_SAFE_MARGIN;

    const leftPanel = document.querySelector("aside.app-shell-left-panel");
    if (leftPanel instanceof Element) {
      const rect = leftPanel.getBoundingClientRect();
      if (rect.width >= 48 && rect.right > 40 && rect.right < viewportWidth * 0.62) {
        left = Math.max(left, rect.right + PANEL_SAFE_MARGIN);
      }
    }

    const mainStage = document.querySelector(
      "main.main-surface, .app-shell-main-content-viewport, .app-shell-main-content-frame"
    );
    if (mainStage instanceof Element) {
      const rect = mainStage.getBoundingClientRect();
      if (rect.width >= 160) {
        if (rect.left > 40 && rect.left < viewportWidth * 0.62) {
          left = Math.max(left, rect.left + PANEL_SAFE_MARGIN);
        }
        if (rect.right > left + 80 && rect.right <= viewportWidth + 2) {
          right = Math.min(right, rect.right - PANEL_SAFE_MARGIN);
        }
        if (rect.top >= 0 && rect.top < viewportHeight * 0.4) {
          top = Math.max(top, rect.top + PANEL_SAFE_MARGIN);
        }
        if (rect.bottom > top + 80 && rect.bottom <= viewportHeight + 2) {
          bottom = Math.min(bottom, rect.bottom - PANEL_SAFE_MARGIN);
        }
      }
    }

    const rightRail = document.querySelector(
      "aside.app-shell-right-panel, [data-testid='right-sidebar'], aside.app-shell-secondary-panel"
    );
    if (rightRail instanceof Element) {
      const rect = rightRail.getBoundingClientRect();
      if (rect.width >= 48 && rect.left > viewportWidth * 0.45 && rect.left < viewportWidth - 40) {
        right = Math.min(right, rect.left - PANEL_SAFE_MARGIN);
      }
    }

    document.querySelectorAll(
      ".app-header-tint, .draggable.flex.h-toolbar, [class*='h-toolbar'].draggable, header"
    ).forEach((bar) => {
      if (!(bar instanceof Element)) return;
      const rect = bar.getBoundingClientRect();
      if (rect.height < 28 || rect.height > 96) return;
      if (rect.top > 24 || rect.width < viewportWidth * 0.45) return;
      top = Math.max(top, rect.bottom + PANEL_SAFE_MARGIN);
    });

    if (bottom - top < CHIP_HEIGHT) {
      top = PANEL_SAFE_MARGIN;
    }

    if (right - left < CHIP_WIDTH) {
      left = PANEL_SAFE_MARGIN;
      right = viewportWidth - PANEL_SAFE_MARGIN;
    }

    return {
      left,
      top,
      right,
      bottom,
      width: Math.max(0, right - left),
      height: Math.max(0, bottom - top),
    };
  }

  function clampPosition(position) {
    const bounds = contentSafeBounds();
    const visibleWidth = Math.min(CHIP_WIDTH, bounds.width);
    const visibleHeight = Math.min(CHIP_HEIGHT, bounds.height);
    const sourceX = Number(position?.x);
    const sourceY = Number(position?.y);
    return {
      x: clamp(Number.isFinite(sourceX) ? sourceX : bounds.left, bounds.left, Math.max(bounds.left, bounds.right - visibleWidth)),
      y: clamp(Number.isFinite(sourceY) ? sourceY : bounds.top, bounds.top, Math.max(bounds.top, bounds.bottom - visibleHeight)),
    };
  }

  function persistPosition() {
    if (!state.position) return;
    try { localStorage.setItem(POSITION_KEY, JSON.stringify(state.position)); } catch {}
  }

  function setPosition(position, persist = false) {
    state.position = clampPosition(position);
    if (persist) persistPosition();
    applyPosition();
  }

  function dockRightKeepHeight(persist = true) {
    const layout = shellLayout();
    setPosition({
      x: layout.bounds.right - layout.chip.width,
      y: layout.anchor.y,
    }, persist);
  }

  function snapRightIfNear(persist = false, animate = false) {
    const layout = shellLayout();
    const visibleRight = state.open
      ? layout.left + layout.width
      : layout.anchor.x + layout.chip.width;
    if (layout.bounds.right - visibleRight > RIGHT_EDGE_SNAP_DISTANCE) return false;
    if (animate && state.popover && !prefersReducedMotion()) {
      if (state.snapTimer) window.clearTimeout(state.snapTimer);
      state.popover.dataset.snapRight = "true";
      const timer = window.setTimeout(() => {
        if (state.snapTimer !== timer) return;
        state.snapTimer = 0;
        state.popover?.removeAttribute("data-snap-right");
      }, 220);
      state.snapTimer = timer;
    }
    dockRightKeepHeight(persist);
    return true;
  }

  function shellLayout() {
    const bounds = contentSafeBounds();
    const width = Math.max(CHIP_WIDTH, Math.min(state.width, bounds.width));
    const anchor = clampPosition(state.position || defaultPosition());
    const chipWidth = Math.min(CHIP_WIDTH, width);
    const chipHeight = Math.min(CHIP_HEIGHT, bounds.height);
    const minimumPanelHeight = Math.min(PANEL_MIN_HEIGHT, bounds.height);
    const roomBelow = Math.max(chipHeight, bounds.bottom - anchor.y);
    const roomAbove = Math.max(chipHeight, anchor.y + chipHeight - bounds.top);
    const panelDrag = state.drag?.source === "panel" ? state.drag : null;
    const resizeDrag = state.resizeDrag;
    const opensDown = typeof panelDrag?.lockedOpensDown === "boolean"
      ? panelDrag.lockedOpensDown
      : typeof resizeDrag?.lockedOpensDown === "boolean"
        ? resizeDrag.lockedOpensDown
      : roomBelow >= minimumPanelHeight || roomBelow >= roomAbove;
    const availableHeight = opensDown ? roomBelow : roomAbove;
    const requestedHeight = Number.isFinite(panelDrag?.panelHeight)
      ? panelDrag.panelHeight
      : state.activeTab === "settings"
        ? clampPanelHeight(SETTINGS_PANEL_HEIGHT)
        : state.height;
    const height = Math.max(CHIP_HEIGHT, Math.min(requestedHeight, bounds.height, availableHeight));
    const compressionProgress = state.activeTab === "settings"
      ? 0
      : clamp(
        (requestedHeight - height) / Math.max(1, requestedHeight - chipHeight),
        0,
        1,
      );
    const desiredLeft = anchor.x - (width - chipWidth) / 2;
    const left = clamp(desiredLeft, bounds.left, Math.max(bounds.left, bounds.right - width));
    const desiredTop = opensDown ? anchor.y : anchor.y + chipHeight - height;
    const top = clamp(desiredTop, bounds.top, Math.max(bounds.top, bounds.bottom - height));
    const chipLeft = clamp(anchor.x - left, 0, Math.max(0, width - chipWidth));
    const chipTop = clamp(anchor.y - top, 0, Math.max(0, height - chipHeight));
    const collapsedShell = {
      left: chipLeft,
      top: chipTop,
      width: chipWidth,
      height: chipHeight,
      radius: CHIP_RADIUS,
    };
    const horizontalShell = {
      left: 0,
      top: chipTop,
      width,
      height: chipHeight,
      radius: CHIP_RADIUS,
    };
    const expandedShell = {
      left: 0,
      top: 0,
      width,
      height,
      radius: PANEL_RADIUS,
    };
    const distX = Math.max(1, expandedShell.width - collapsedShell.width);
    const distY = Math.max(1, expandedShell.height - collapsedShell.height);
    const stageMs = Math.max(
      Math.max(MIN_PHASE_MS, distX / MORPH_EDGE_SPEED),
      Math.max(MIN_PHASE_MS, distY / MORPH_EDGE_SPEED)
    );
    return {
      left,
      top,
      width,
      height,
      requestedHeight,
      availableHeight,
      compressionProgress,
      bounds,
      anchor,
      chip: {
        left: chipLeft,
        top: chipTop,
        width: chipWidth,
        height: chipHeight,
        radius: CHIP_RADIUS,
      },
      collapsedShell,
      horizontalShell,
      expandedShell,
      distX,
      distY,
      opensDown,
      phaseSplit: HORIZONTAL_PHASE,
      morphDurationMs: clamp(Math.round(stageMs * 2), MIN_MORPH_MS, MAX_MORPH_MS),
    };
  }

  function phaseSplitOf(geometry) {
    const split = Number(geometry?.phaseSplit);
    if (Number.isFinite(split) && split > 0.05 && split < 0.95) return split;
    return HORIZONTAL_PHASE;
  }

  // Canceling a morph invalidates its callbacks before stopping animations or clearing state.
  function cancelMorphAnimations() {
    const transition = state.morphTransition;
    if (transition) {
      transition.cancelled = true;
      if (transition.fallbackTimer) window.clearTimeout(transition.fallbackTimer);
    }
    state.morphTransition = null;
    state.morphGeneration += 1;
    const animations = [
      state.morphAnimation,
      state.rimMorphAnimation,
      state.displacementMorphAnimation,
      state.panelMorphAnimation,
      state.fabMorphAnimation,
      ...(transition?.animations || []),
    ];
    [...new Set(animations)].forEach((animation) => animation?.cancel?.());
    state.morphAnimation = null;
    state.rimMorphAnimation = null;
    state.displacementMorphAnimation = null;
    state.panelMorphAnimation = null;
    state.fabMorphAnimation = null;
  }

  function unfoldAxes(progress, collapsing = false, split = HORIZONTAL_PHASE) {
    const value = clamp(progress, 0, 1);
    const elapsed = collapsing ? 1 - value : value;
    const phase = clamp(split, 0.05, 0.95);
    let x;
    let y;
    if (elapsed <= phase) {
      x = axisEase(phase < 0.001 ? 1 : elapsed / phase);
      y = 0;
    } else {
      x = 1;
      y = axisEase((elapsed - phase) / Math.max(0.001, 1 - phase));
    }
    return collapsing ? { x: 1 - x, y: 1 - y } : { x, y };
  }

  function unfoldShell(geometry, progress, collapsing = false) {
    const { x, y } = unfoldAxes(progress, collapsing, phaseSplitOf(geometry));
    const collapsed = geometry.collapsedShell;
    const expanded = geometry.expandedShell;
    return {
      left: lerp(collapsed.left, expanded.left, x),
      top: lerp(collapsed.top, expanded.top, y),
      width: lerp(collapsed.width, expanded.width, x),
      height: lerp(collapsed.height, expanded.height, y),
      radius: lerp(collapsed.radius, expanded.radius, Math.max(x, y)),
    };
  }

  function morphPathProgress(shell, geometry) {
    const split = phaseSplitOf(geometry);
    const collapsed = geometry.collapsedShell;
    const expanded = geometry.expandedShell;
    const widthProgress = clamp(
      (shell.width - collapsed.width) / Math.max(1, expanded.width - collapsed.width),
      0,
      1
    );
    const heightProgress = clamp(
      (shell.height - collapsed.height) / Math.max(1, expanded.height - collapsed.height),
      0,
      1
    );
    if (heightProgress > 0.002 || widthProgress >= 0.998) {
      return split + heightProgress * (1 - split);
    }
    return widthProgress * split;
  }

  function readGlassGeometry(geometry) {
    const fallback = unfoldShell(geometry, state.open ? 1 : 0);
    if (!state.glass) return fallback;
    const computed = getComputedStyle(state.glass);
    const number = (value, fallbackValue) => {
      const parsed = Number.parseFloat(String(value || ""));
      return Number.isFinite(parsed) ? parsed : fallbackValue;
    };
    return {
      left: number(computed.left, fallback.left),
      top: number(computed.top, fallback.top),
      width: Math.max(1, number(computed.width, fallback.width)),
      height: Math.max(1, number(computed.height, fallback.height)),
      radius: Math.max(0, number(computed.borderTopLeftRadius, fallback.radius)),
    };
  }

  function morphPx(value) {
    return `${Number(value.toFixed(3))}px`;
  }

  function glassFrame(shell, offset) {
    return {
      left: morphPx(shell.left),
      top: morphPx(shell.top),
      width: morphPx(shell.width),
      height: morphPx(shell.height),
      borderRadius: morphPx(shell.radius),
      offset: Number(offset.toFixed(4)),
    };
  }

  function panelClipPath(shell, geometry) {
    const top = Math.max(0, shell.top);
    const right = Math.max(0, geometry.width - shell.left - shell.width);
    const bottom = Math.max(0, geometry.height - shell.top - shell.height);
    const left = Math.max(0, shell.left);
    return `inset(${morphPx(top)} ${morphPx(right)} ${morphPx(bottom)} ${morphPx(left)} round ${morphPx(shell.radius)})`;
  }

  function panelFrame(shell, geometry, offset) {
    return {
      clipPath: panelClipPath(shell, geometry),
      offset: Number(offset.toFixed(4)),
    };
  }

  function fabFrame(shell, offset) {
    const headerHeight = Math.min(CHIP_HEIGHT + 8, shell.height);
    return {
      left: morphPx(shell.left + (shell.width - CHIP_WIDTH) / 2),
      top: morphPx(shell.top + Math.max(0, (headerHeight - CHIP_HEIGHT) / 2)),
      offset: Number(offset.toFixed(4)),
    };
  }

  function buildMorphPath(currentShell, expanded, geometry) {
    const startProgress = morphPathProgress(currentShell, geometry);
    const targetProgress = expanded ? 1 : 0;
    const remaining = Math.abs(targetProgress - startProgress);
    const baseDuration = clamp(
      Number(geometry.morphDurationMs) || MIN_MORPH_MS,
      MIN_MORPH_MS,
      MAX_MORPH_MS
    );
    const duration = remaining < 0.002
      ? 0
      : clamp(Math.round(baseDuration * remaining), MIN_REVERSE_MS, MAX_MORPH_MS);
    const samples = [{ shell: currentShell, offset: 0 }];
    const steps = UNFOLD_SAMPLES + 1;
    const progressDelta = targetProgress - startProgress;
    const stageProgress = phaseSplitOf(geometry);
    const stageTimeline = Math.abs(progressDelta) < 0.000001
      ? -1
      : (stageProgress - startProgress) / progressDelta;
    const timelines = [];
    for (let index = 1; index <= steps; index += 1) {
      timelines.push(index / steps);
    }
    if (stageTimeline > 0.000001 && stageTimeline < 0.999999) {
      timelines.push(stageTimeline);
    }
    timelines.sort((left, right) => left - right);
    let previousTimeline = -1;
    for (const timeline of timelines) {
      if (Math.abs(timeline - previousTimeline) < 0.000001) continue;
      const motion = expanded ? expandMotionU(timeline) : timeline;
      const sampledProgress = startProgress + progressDelta * motion;
      const progress = Math.abs(timeline - stageTimeline) < 0.000001
        ? stageProgress
        : sampledProgress;
      samples.push({ shell: unfoldShell(geometry, progress, false), offset: timeline });
      previousTimeline = timeline;
    }
    const targetShell = expanded ? geometry.expandedShell : geometry.collapsedShell;
    samples[samples.length - 1] = { shell: targetShell, offset: 1 };
    return {
      duration,
      frames: samples.map(({ shell, offset }) => glassFrame(shell, offset)),
      panelFrames: samples.map(({ shell, offset }) => panelFrame(shell, geometry, offset)),
      fabFrames: samples.map(({ shell, offset }) => fabFrame(shell, offset)),
      startProgress,
      targetProgress,
      targetShell,
    };
  }

  function applyMorphShell(shell, geometry) {
    [state.glass, state.rim, state.displacementTexture, state.completionBeam].forEach((surface) => {
      if (!surface) return;
      surface.style.left = `${shell.left}px`;
      surface.style.top = `${shell.top}px`;
      surface.style.width = `${shell.width}px`;
      surface.style.height = `${shell.height}px`;
      surface.style.borderRadius = `${shell.radius}px`;
    });
    if (state.panel) {
      state.panel.style.clipPath = panelClipPath(shell, geometry);
    }
    if (state.fab) {
      const frame = fabFrame(shell, 0);
      state.fab.style.left = frame.left;
      state.fab.style.top = frame.top;
    }
  }

  function applyMorphProgress(progress) {
    if (!state.glass && !state.rim && !state.panel && !state.fab) return;
    const geometry = state.layout || shellLayout();
    const shell = unfoldShell(geometry, progress, false);
    applyMorphShell(shell, geometry);
  }

  function settleMorph(progress, focusTarget = "") {
    if (!isCurrentRuntime()) return;
    cancelMorphAnimations();
    resetEyePointer();
    const expanded = progress >= 0.999;
    state.open = expanded;
    state.popover.dataset.open = String(expanded);
    state.popover.dataset.morphing = "false";
    state.panel.inert = !expanded;
    state.panel.setAttribute("aria-hidden", String(!expanded));
    state.fab.setAttribute("aria-expanded", String(expanded));
    applyMorphProgress(expanded ? 1 : 0);
    resetGlassPointer();
    const runtimeGeneration = state.runtimeGeneration;
    if (focusTarget === "panel" && expanded) {
      window.requestAnimationFrame(() => {
        if (isCurrentRuntime(runtimeGeneration)) {
          state.panel?.querySelector("[data-action='collapse']")?.focus({ preventScroll: true });
        }
      });
    }
    if (focusTarget === "chip" && !expanded) {
      window.requestAnimationFrame(() => {
        if (isCurrentRuntime(runtimeGeneration)) state.fab?.focus({ preventScroll: true });
      });
    }
    if (!flushDeferredRender()) syncEyeTracking();
  }

  function startMorph(expanded, focusTarget = "") {
    if (!state.glass || !state.rim || !state.fab || !state.panel || !state.popover) return;
    resetEyePointer();
    const geometry = state.layout || shellLayout();
    const currentShell = readGlassGeometry(geometry);
    cancelMorphAnimations();
    state.open = expanded;
    state.focusAfterMorph = focusTarget;
    state.popover.dataset.open = String(expanded);
    state.popover.dataset.morphing = "true";
    resetGlassPointer();
    state.panel.inert = true;
    state.panel.setAttribute("aria-hidden", "true");
    state.fab.setAttribute("aria-expanded", String(expanded));
    const path = buildMorphPath(currentShell, expanded, geometry);
    applyMorphShell(currentShell, geometry);

    if (prefersReducedMotion() || path.duration === 0) {
      settleMorph(path.targetProgress, focusTarget);
      return;
    }

    const generation = state.morphGeneration;
    const runtimeGeneration = state.runtimeGeneration;
    const timing = {
      duration: path.duration,
      easing: "cubic-bezier(.2, .72, .2, 1)",
      fill: "forwards",
    };
    const animation = state.glass.animate(path.frames, timing);
    state.rimMorphAnimation = state.rim.animate(path.frames, timing);
    state.displacementMorphAnimation = state.displacementTexture?.animate(path.frames, timing) || null;
    state.panelMorphAnimation = state.panel.animate(path.panelFrames, timing);
    state.fabMorphAnimation = state.fab.animate(path.fabFrames, timing);
    state.morphAnimation = animation;
    const animations = [
      animation,
      state.rimMorphAnimation,
      state.displacementMorphAnimation,
      state.panelMorphAnimation,
      state.fabMorphAnimation,
    ].filter(Boolean);
    let settled = false;
    const transition = {
      animations,
      cancelled: false,
      fallbackTimer: 0,
      finish: () => {
        if (transition.cancelled || settled) return;
        settled = true;
        if (transition.fallbackTimer) window.clearTimeout(transition.fallbackTimer);
        if (state.morphTransition === transition) state.morphTransition = null;
        if (!isCurrentRuntime(runtimeGeneration) || generation !== state.morphGeneration) return;
        settleMorph(path.targetProgress, focusTarget);
      },
    };
    state.morphTransition = transition;
    transition.fallbackTimer = window.setTimeout(() => {
      transition.animations.forEach((item) => {
        if (item.playState !== "finished") item.cancel();
      });
      transition.finish();
    }, path.duration + MORPH_FALLBACK_BUFFER_MS);
    void Promise.all(transition.animations.map((item) => item.finished.catch(() => null)))
      .then(() => transition.finish());
  }

  function setOpen(expanded, focusTarget = "") {
    if (!isCurrentRuntime()) return;
    resetEyePointer();
    const target = Boolean(expanded);
    if (target === state.open) return;
    clearCompletionBeam();
    renderFloat({ preserveMorph: true });
    startMorph(target, focusTarget);
  }

  function panelDragPosition(drag, dx, dy) {
    const geometry = drag.originLayout;
    const bounds = contentSafeBounds();
    const maxLeft = Math.max(bounds.left, bounds.right - geometry.width);
    const maxTop = Math.max(bounds.top, bounds.bottom - geometry.height);
    const left = clamp(drag.originPanelLeft + dx, bounds.left, maxLeft);
    const top = clamp(drag.originPanelTop + dy, bounds.top, maxTop);
    return {
      x: left + (geometry.width - geometry.chip.width) / 2,
      y: drag.lockedOpensDown
        ? top
        : top + geometry.height - geometry.chip.height,
    };
  }

  function applyPosition() {
    if (!state.popover || !state.fab || !state.position) return;
    state.position = clampPosition(state.position);
    state.layout = shellLayout();
    state.popover.style.left = `${state.layout.left}px`;
    state.popover.style.top = `${state.layout.top}px`;
    state.popover.style.width = `${state.layout.width}px`;
    state.popover.style.height = `${state.layout.height}px`;
    state.root.style.setProperty("--csw-panel-width", `${state.layout.width}px`);
    state.root.style.setProperty("--csw-panel-height", `${state.layout.height}px`);
    state.popover.style.setProperty("--csw-chip-left", `${state.layout.chip.left}px`);
    const compressionProgress = state.layout.compressionProgress || 0;
    const compressed = state.activeTab !== "settings" && compressionProgress > 0.001;
    state.popover.dataset.compressed = String(compressed);
    state.popover.style.setProperty(
      "--csw-content-fade-size",
      `${compressed ? Math.min(48, 14 + compressionProgress * 34) : 0}px`,
    );
    syncContentFade();
    state.fab.style.left = `${state.layout.chip.left}px`;
    state.fab.style.top = `${state.layout.chip.top}px`;
    if (!state.morphAnimation) applyMorphProgress(state.open ? 1 : 0);
  }

  // SVG filters provide material-specific backdrop treatment without adding third-party runtime code.
