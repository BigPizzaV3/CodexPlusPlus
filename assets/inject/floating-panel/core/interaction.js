/* Floating-panel interaction: material surfaces, pointer input, drag, resize, and tracking. */

  function createDisplacementFilter(id, options) {
    document.getElementById(id)?.ownerSVGElement?.remove();
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("width", "0");
    svg.setAttribute("height", "0");
    svg.setAttribute("aria-hidden", "true");
    svg.style.cssText = "position:absolute;width:0;height:0;overflow:hidden;pointer-events:none";
    const stitchTiles = options.stitchTiles ? ` stitchTiles="${options.stitchTiles}"` : "";
    const blurNode = Number.isFinite(options.blur)
      ? `<feGaussianBlur in="noise" stdDeviation="${options.blur}" result="blurred"></feGaussianBlur>`
      : "";
    const displacementInput = blurNode ? "blurred" : "noise";
    svg.innerHTML = `
      <defs>
        <filter id="${id}" x="${options.x}" y="${options.y}" width="${options.width}" height="${options.height}" color-interpolation-filters="sRGB">
          <feTurbulence type="fractalNoise" baseFrequency="${options.baseFrequency}" numOctaves="${options.numOctaves}" seed="${options.seed}"${stitchTiles} result="noise"></feTurbulence>
          ${blurNode}
          <feDisplacementMap in="SourceGraphic" in2="${displacementInput}" scale="${options.scale}" xChannelSelector="R" yChannelSelector="G"></feDisplacementMap>
        </filter>
      </defs>
    `;
    return svg;
  }

  function createClearFilter() {
    const svg = createDisplacementFilter(CLEAR_FILTER_ID, {
      x: "-15%",
      y: "-15%",
      width: "130%",
      height: "130%",
      baseFrequency: "0.006 0.010",
      numOctaves: 1,
      seed: 92,
      stitchTiles: "stitch",
      blur: 7,
      scale: 3,
    });
    state.clearDisplacement = svg.querySelector("feDisplacementMap");
    return svg;
  }

  function createLiquidFilter() {
    return createDisplacementFilter(LIQUID_FILTER_ID, {
      x: "-45%",
      y: "-45%",
      width: "190%",
      height: "190%",
      baseFrequency: "0.012 0.012",
      numOctaves: 2,
      seed: 92,
      blur: 2,
      scale: 85,
    });
  }

  function createCrystalFilter() {
    return createDisplacementFilter(CRYSTAL_FILTER_ID, {
      x: "-60%",
      y: "-60%",
      width: "220%",
      height: "220%",
      baseFrequency: "0.03 0.03",
      numOctaves: 2,
      seed: 92,
      blur: 2,
      scale: 140,
    });
  }

  function updateClearDisplacement(expanded, active) {
    if (!state.clearDisplacement) return;
    const scale = active ? (expanded ? 6 : 6) : (expanded ? 3 : 2);
    state.clearDisplacement.setAttribute("scale", String(scale));
  }

  function updateMaterialDistortion(expanded, active) {
    updateClearDisplacement(expanded, active);
  }

  // The DOM shell is created once; later renders update its contents without stacking another overlay.
  function installFloat() {
    if (!isCurrentRuntime()) return;
    document.querySelectorAll?.(`[${ROOT_ATTR}="true"]`).forEach((node) => {
      if (node !== state.root) node.remove();
    });
    if (state.root && document.body.contains(state.root)) return;

    state.position = savedPosition();
    state.root = document.createElement("div");
    state.root.setAttribute(ROOT_ATTR, "true");

    state.fab = document.createElement("button");
    state.fab.className = "csw-fab";
    state.fab.type = "button";
    state.fab.title = "下一步";
    state.fab.setAttribute("aria-controls", POPOVER_ID);
    state.fab.innerHTML = `${statusStageHtml()}${sourceTrackHtml()}`;

    state.popover = document.createElement("div");
    state.popover.className = "csw-popover";
    state.popover.dataset.open = "false";
    state.popover.dataset.morphing = "false";
    state.popover.dataset.completionBeam = "false";

    state.glass = document.createElement("div");
    state.glass.className = "csw-glass";
    state.glass.setAttribute("aria-hidden", "true");

    state.rim = document.createElement("div");
    state.rim.className = "csw-rim";
    state.rim.setAttribute("aria-hidden", "true");

    state.completionBeam = document.createElement("div");
    state.completionBeam.className = "csw-completion-beam";
    state.completionBeam.setAttribute("aria-hidden", "true");

    state.clearFilter = createClearFilter();
    state.liquidFilter = createLiquidFilter();
    state.crystalFilter = createCrystalFilter();

    const clearTexture = document.createElement("div");
    clearTexture.className = "csw-clear-texture";
    clearTexture.setAttribute("aria-hidden", "true");
    state.clearDistortion = document.createElement("div");
    state.clearDistortion.className = "csw-clear-distortion";
    state.clearDistortion.setAttribute("aria-hidden", "true");
    state.glass.append(clearTexture, state.clearDistortion);

    state.displacementTexture = document.createElement("div");
    state.displacementTexture.className = "csw-displacement-texture";
    state.displacementTexture.setAttribute("aria-hidden", "true");

    const materialLayer = document.createElement("div");
    materialLayer.className = "csw-material-layer";
    materialLayer.setAttribute("aria-hidden", "true");
    materialLayer.append(state.displacementTexture, state.glass, state.rim);
    materialLayer.append(state.completionBeam);

    state.panel = document.createElement("section");
    state.panel.id = POPOVER_ID;
    state.panel.className = "csw-panel";
    state.panel.setAttribute("role", "dialog");
    state.panel.setAttribute("aria-label", "下一步建议与回答大纲");
    state.panel.setAttribute("aria-hidden", "true");
    state.panel.inert = true;

    const resizeBottomLeft = document.createElement("span");
    resizeBottomLeft.className = "csw-resize-handle";
    resizeBottomLeft.dataset.corner = "bl";
    resizeBottomLeft.setAttribute("aria-hidden", "true");
    const resizeBottomRight = document.createElement("span");
    resizeBottomRight.className = "csw-resize-handle";
    resizeBottomRight.dataset.corner = "br";
    resizeBottomRight.setAttribute("aria-hidden", "true");

    state.popover.append(materialLayer, state.fab, state.panel, resizeBottomLeft, resizeBottomRight);
    state.root.append(state.clearFilter, state.liquidFilter, state.crystalFilter, state.popover);
    document.body.appendChild(state.root);

    state.fab.addEventListener("pointerdown", onFabPointerDown);
    state.fab.addEventListener("click", onFabClick);
    bindGlassPointerSurface(state.fab);
    state.panel.addEventListener("wheel", onPanelWheel, { passive: false });
    state.glass.addEventListener("click", onGlassClick);
    resetGlassPointer();
    state.keyHandler = onKeyDown;
    document.addEventListener("keydown", state.keyHandler, true);
    window.addEventListener("resize", onResize);
    installEyeTracking();
    installThemeObserver();
    installTypographyObserver();
    syncTheme();
    applyMaterial();
    installResize();
    applyPosition();
    settleMorph(0);
  }

  function onResize() {
    if (!state.position) return;
    const target = state.open ? 1 : 0;
    cancelMorphAnimations();
    state.position = clampPosition(state.position);
    applyPosition();
    settleMorph(target);
    syncContentFade();
  }

  function onPanelWheel(event) {
    if (!state.open || (!event.altKey && !event.metaKey) || event.deltaY === 0) return;
    event.preventDefault();
    event.stopPropagation();
    bumpFontSize(event.deltaY > 0 ? -1 : 1);
  }

  function onFabPointerDown(event) {
    beginDrag(event, "fab");
  }

  function dragTargetBlocked(target) {
    if (!(target instanceof Element)) return false;
    if (target.closest(".csw-head-side")) return true;
    if (target.closest(".csw-head-face")) return false;
    return Boolean(target.closest("button,a,input,textarea,select,[role='button']"));
  }

  function beginDrag(event, source) {
    if (event.button !== 0 || state.morphAnimation || !state.position) return;
    if (source === "fab" && state.open) return;
    if (source === "panel" && (!state.open || dragTargetBlocked(event.target))) return;

    state.dragCleanup?.();
    const handle = event.currentTarget;
    const originLayout = state.layout || shellLayout();
    const drag = {
      pointerId: event.pointerId,
      source,
      startedOnHeadFace: source === "panel" && event.target instanceof Element && Boolean(event.target.closest(".csw-head-face")),
      startX: event.clientX,
      startY: event.clientY,
      originX: state.position.x,
      originY: state.position.y,
      originLayout,
      originPanelLeft: originLayout.left,
      originPanelTop: originLayout.top,
      lockedOpensDown: source === "panel" ? originLayout.opensDown : null,
      panelHeight: source === "panel" ? originLayout.height : null,
      moved: false,
    };
    state.drag = drag;
    state.suppressFabClick = false;
    state.suppressHeadFaceClick = false;
    resetEyePointer();

    const onPointerMove = (moveEvent) => {
      if (state.drag !== drag || moveEvent.pointerId !== drag.pointerId) return;
      const dx = moveEvent.clientX - drag.startX;
      const dy = moveEvent.clientY - drag.startY;
      if (!drag.moved && Math.hypot(dx, dy) < 3) return;
      if (!drag.moved) {
        drag.moved = true;
        handle?.setAttribute?.("data-dragging", "true");
        try { handle?.setPointerCapture?.(drag.pointerId); } catch {}
      }
      moveEvent.preventDefault();
      const nextPosition = source === "panel"
        ? panelDragPosition(drag, dx, dy)
        : { x: drag.originX + dx, y: drag.originY + dy };
      setPosition(nextPosition);
      snapRightIfNear();
    };

    const cleanup = () => {
      window.removeEventListener("pointermove", onPointerMove, true);
      window.removeEventListener("pointerup", onPointerEnd, true);
      window.removeEventListener("pointercancel", onPointerEnd, true);
      handle?.removeAttribute?.("data-dragging");
      try { handle?.releasePointerCapture?.(drag.pointerId); } catch {}
      if (state.dragCleanup === cleanup) state.dragCleanup = null;
    };

    const onPointerEnd = (endEvent) => {
      if (state.drag !== drag || endEvent.pointerId !== drag.pointerId) return;
      cleanup();
      if (!drag.moved) {
        state.drag = null;
        return;
      }
      const snapped = snapRightIfNear(true, true);
      state.drag = null;
      if (!snapped) persistPosition();
      if (source === "fab") {
        state.suppressFabClick = true;
        window.setTimeout(() => { state.suppressFabClick = false; }, 300);
      } else {
        state.suppressHeadFaceClick = true;
        window.setTimeout(() => { state.suppressHeadFaceClick = false; }, 300);
        if (drag.startedOnHeadFace && document.activeElement instanceof HTMLElement) {
          document.activeElement.blur();
        }
      }
      syncEyeTracking();
    };

    state.dragCleanup = cleanup;
    window.addEventListener("pointermove", onPointerMove, { capture: true, passive: false });
    window.addEventListener("pointerup", onPointerEnd, true);
    window.addEventListener("pointercancel", onPointerEnd, true);
    if (source === "panel" && !drag.startedOnHeadFace) event.preventDefault();
  }

  // Dragging, right docking, resizing, and persisted bounds share the same clamped position model.
  function installPanelDrag() {
    const head = state.panel?.querySelector(".csw-head");
    if (head && head.dataset.dragBound !== "1") {
      head.dataset.dragBound = "1";
      head.addEventListener("pointerdown", (event) => beginDrag(event, "panel"));
    }
  }

  function installViewTabReorder() {
    state.viewReorderCleanup?.();
    state.viewReorderCleanup = null;
    const tabs = state.panel?.querySelector(".csw-view-tabs");
    const buttons = Array.from(tabs?.querySelectorAll("[data-reorderable='true']") || []);
    if (buttons.length < 2) return;

    let drag = null;
    const clearButtonState = () => {
      buttons.forEach((button) => {
        button.removeAttribute("data-dragging");
        button.style.removeProperty("order");
      });
      tabs.removeAttribute("data-reordering");
    };
    const finish = (commit) => {
      buttons.forEach((button) => button.removeEventListener("pointerdown", onPointerDown));
      window.removeEventListener("pointermove", onPointerMove, true);
      window.removeEventListener("pointerup", onPointerEnd, true);
      window.removeEventListener("pointercancel", onPointerEnd, true);
      const activeDrag = drag;
      drag = null;
      clearButtonState();
      if (!activeDrag) {
        state.viewReorderCleanup = null;
        return;
      }
      try { activeDrag.button.releasePointerCapture(activeDrag.pointerId); } catch {}
      if (activeDrag.moved) state.suppressViewTabClickUntil = performance.now() + 320;
      const changed = activeDrag.pendingOrder.join("|") !== state.viewOrder.join("|");
      state.viewReorderCleanup = null;
      if (commit && activeDrag.moved && changed) {
        persistViewOrder(activeDrag.pendingOrder);
        renderFloat({ preserveMorph: true });
      }
    };
    const onPointerMove = (event) => {
      if (!drag || event.pointerId !== drag.pointerId) return;
      const dx = event.clientX - drag.startX;
      const dy = event.clientY - drag.startY;
      if (!drag.moved && Math.hypot(dx, dy) < 5) return;
      if (!drag.moved) {
        drag.moved = true;
        drag.button.setAttribute("data-dragging", "true");
        tabs.setAttribute("data-reordering", "true");
      }
      event.preventDefault();
      const currentIndex = drag.pendingOrder.indexOf(drag.view);
      const targetIndex = currentIndex === 0 ? 1 : 0;
      const otherView = drag.pendingOrder[targetIndex];
      const otherButton = buttons.find((button) => button.dataset.view === otherView);
      if (!otherButton) return;
      const rect = otherButton.getBoundingClientRect();
      const midpoint = rect.left + rect.width / 2;
      const crossed = currentIndex === 0 ? event.clientX > midpoint : event.clientX < midpoint;
      if (!crossed) return;
      const nextOrder = drag.pendingOrder.slice();
      [nextOrder[currentIndex], nextOrder[targetIndex]] = [nextOrder[targetIndex], nextOrder[currentIndex]];
      drag.pendingOrder = nextOrder;
      buttons.forEach((button) => {
        button.style.order = String(nextOrder.indexOf(button.dataset.view));
      });
      syncViewTabSelection(state.activeTab, true);
    };
    const onPointerEnd = (event) => {
      if (!drag || event.pointerId !== drag.pointerId) return;
      finish(true);
    };
    const onPointerDown = (event) => {
      if (event.button !== 0 || drag) return;
      const button = event.currentTarget;
      event.stopPropagation();
      drag = {
        button,
        view: button.dataset.view,
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        moved: false,
        pendingOrder: state.viewOrder.slice(),
      };
      try { button.setPointerCapture(event.pointerId); } catch {}
      window.addEventListener("pointermove", onPointerMove, true);
      window.addEventListener("pointerup", onPointerEnd, true);
      window.addEventListener("pointercancel", onPointerEnd, true);
    };
    buttons.forEach((button) => button.addEventListener("pointerdown", onPointerDown));
    state.viewReorderCleanup = () => finish(false);
  }

  function resizePositionFromFace(nextHeight, resize) {
    const panelTop = resize.faceCenterY - resize.faceOffsetY;
    return {
      x: resize.faceCenterX - resize.chipWidth / 2,
      y: resize.lockedOpensDown
        ? panelTop
        : panelTop + nextHeight - resize.chipHeight,
    };
  }

  function installResize() {
    if (!state.popover || state.popover.dataset.resizeBound === "1") return;
    state.popover.dataset.resizeBound = "1";
    state.popover.querySelectorAll(".csw-resize-handle").forEach((handle) => {
      handle.addEventListener("pointerdown", (event) => {
        if (event.button !== 0 || !state.open || state.activeTab === "settings" || state.morphAnimation || !state.layout) return;
        event.preventDefault();
        event.stopPropagation();
        state.resizeCleanup?.();
        const corner = handle.dataset.corner === "bl" ? "bl" : "br";
        const startRect = state.popover.getBoundingClientRect();
        const startWidth = state.layout.width;
        const startHeight = state.layout.height;
        const startX = event.clientX;
        const startY = event.clientY;
        const faceRect = state.panel?.querySelector(".csw-head-face")?.getBoundingClientRect();
        const faceCenterX = faceRect ? faceRect.left + faceRect.width / 2 : startRect.left + startRect.width / 2;
        const faceCenterY = faceRect ? faceRect.top + faceRect.height / 2 : startRect.top + CHIP_HEIGHT / 2;
        const resize = {
          pointerId: event.pointerId,
          corner,
          chipHeight: state.layout.chip.height,
          chipWidth: state.layout.chip.width,
          faceCenterX,
          faceCenterY,
          faceOffsetY: faceCenterY - startRect.top,
          lockedOpensDown: state.layout.opensDown,
        };
        state.resizeDrag = resize;
        state.popover.dataset.resizing = "true";

        const onMove = (moveEvent) => {
          if (state.resizeDrag !== resize || moveEvent.pointerId !== resize.pointerId) return;
          moveEvent.preventDefault();
          const dx = moveEvent.clientX - startX;
          const dy = moveEvent.clientY - startY;
          const nextWidth = clampPanelWidth(corner === "bl" ? startWidth - dx * 2 : startWidth + dx * 2);
          const nextHeight = clampPanelHeight(startHeight + dy);
          state.width = nextWidth;
          state.height = nextHeight;
          state.position = clampPosition(resizePositionFromFace(nextHeight, resize));
          applyPosition();
        };

        const cleanup = () => {
          window.removeEventListener("pointermove", onMove, true);
          window.removeEventListener("pointerup", endResize, true);
          window.removeEventListener("pointercancel", endResize, true);
          window.removeEventListener("blur", finishResize, true);
          document.removeEventListener("visibilitychange", onVisibilityChange, true);
          handle.removeEventListener("lostpointercapture", onLostPointerCapture, true);
          if (state.resizeCleanup === finishResize) state.resizeCleanup = null;
        };

        const finishResize = () => {
          cleanup();
          if (state.resizeDrag === resize) state.resizeDrag = null;
          state.popover?.removeAttribute("data-resizing");
          storage.set(WIDTH_KEY, String(state.width));
          storage.set(HEIGHT_KEY, String(state.height));
          applyPosition();
          try { handle.releasePointerCapture(resize.pointerId); } catch {}
        };

        const endResize = (endEvent) => {
          if (state.resizeDrag !== resize || endEvent.pointerId !== resize.pointerId) return;
          finishResize();
        };

        const onLostPointerCapture = (captureEvent) => {
          if (captureEvent.pointerId !== resize.pointerId) return;
          finishResize();
        };

        const onVisibilityChange = () => {
          if (document.visibilityState === "hidden") finishResize();
        };

        state.resizeCleanup = finishResize;
        try { handle.setPointerCapture?.(event.pointerId); } catch {}
        window.addEventListener("pointermove", onMove, { capture: true, passive: false });
        window.addEventListener("pointerup", endResize, true);
        window.addEventListener("pointercancel", endResize, true);
        window.addEventListener("blur", finishResize, true);
        document.addEventListener("visibilitychange", onVisibilityChange, true);
        handle.addEventListener("lostpointercapture", onLostPointerCapture, true);
      });

      handle.addEventListener("dblclick", (event) => {
        event.preventDefault();
        event.stopPropagation();
        state.resizeCleanup?.();
        state.width = PANEL_WIDTH;
        state.height = clampPanelHeight(PANEL_HEIGHT);
        storage.set(WIDTH_KEY, String(state.width));
        storage.set(HEIGHT_KEY, String(state.height));
        applyPosition();
      });
    });
  }

  function eyeTrackingActive() {
    return state.fabExpression === "answering"
      && !state.open
      && !state.morphAnimation
      && !state.drag
      && state.root?.dataset.hidden !== "true";
  }

  function curiousEyeTrackingActive() {
    return state.activeTab === "settings"
      && state.open
      && !state.morphAnimation
      && !state.drag
      && state.root?.dataset.hidden !== "true";
  }

  function eyeTrackingNeeded() {
    return eyeTrackingActive() || curiousEyeTrackingActive();
  }

  function applyEyeOffset(x = 0, y = 0) {
    state.root?.style.setProperty("--csw-eye-x", `${x.toFixed(2)}px`);
    state.root?.style.setProperty("--csw-eye-y", `${y.toFixed(2)}px`);
  }

  function applyCuriousEyeOffset(x = 0, y = 0) {
    state.root?.style.setProperty("--csw-curious-eye-x", `${x.toFixed(2)}px`);
    state.root?.style.setProperty("--csw-curious-eye-y", `${y.toFixed(2)}px`);
  }

  function pointerInsideRect(pointer, rect) {
    return pointer.x >= rect.left
      && pointer.x <= rect.right
      && pointer.y >= rect.top
      && pointer.y <= rect.bottom;
  }

  function eyeOffset(pointer, rect, maxX, maxY, reachDistance) {
    const dx = pointer.x - (rect.left + rect.width / 2);
    const dy = pointer.y - (rect.top + rect.height / 2);
    const distance = Math.hypot(dx, dy);
    const reach = clamp(distance / reachDistance, 0, 1);
    const angle = Math.atan2(dy, dx);
    return {
      x: Math.cos(angle) * maxX * reach,
      y: Math.sin(angle) * maxY * reach,
    };
  }

  function flushEyePointer(generation = state.runtimeGeneration) {
    if (!isCurrentRuntime(generation)) return;
    state.eyeRaf = 0;
    if (!state.eyePointer || !eyeTrackingNeeded()) {
      applyEyeOffset();
      applyCuriousEyeOffset();
      return;
    }

    if (eyeTrackingActive() && state.fab) {
      const rect = state.fab.getBoundingClientRect();
      if (rect.width && rect.height) {
        const offset = eyeOffset(state.eyePointer, rect, EYE_MAX_X, EYE_MAX_Y, 220);
        applyEyeOffset(offset.x, offset.y);
      } else {
        applyEyeOffset();
      }
    } else {
      applyEyeOffset();
    }

    if (!curiousEyeTrackingActive()) {
      applyCuriousEyeOffset();
      return;
    }
    const surface = state.panel?.querySelector('.csw-mouth-stage[data-mouth-stage="settings"]');
    const face = state.panel?.querySelector('.csw-head-face[data-expression="curious"]');
    const surfaceRect = surface?.getBoundingClientRect();
    const faceRect = face?.getBoundingClientRect();
    if (!surfaceRect?.width || !surfaceRect.height || !faceRect?.width || !faceRect.height
      || !pointerInsideRect(state.eyePointer, surfaceRect)) {
      applyCuriousEyeOffset();
      return;
    }
    const offset = eyeOffset(
      state.eyePointer,
      faceRect,
      CURIOUS_EYE_MAX_X,
      CURIOUS_EYE_MAX_Y,
      Math.max(120, surfaceRect.height)
    );
    applyCuriousEyeOffset(offset.x, offset.y);
  }

  function scheduleEyePointer() {
    if (!isCurrentRuntime() || state.eyeRaf) return;
    const generation = state.runtimeGeneration;
    state.eyeRaf = window.requestAnimationFrame(() => flushEyePointer(generation));
  }

  function resetEyePointer(clearPointer = false) {
    if (state.eyeRaf) window.cancelAnimationFrame(state.eyeRaf);
    state.eyeRaf = 0;
    if (clearPointer) state.eyePointer = null;
    applyEyeOffset();
    applyCuriousEyeOffset();
  }

  function syncEyeTracking() {
    if (!isCurrentRuntime()) return;
    if (!eyeTrackingNeeded()) {
      resetEyePointer();
      return;
    }
    scheduleEyePointer();
  }

  // Eye tracking is pointer-only decoration and is reset whenever the pointer leaves our surfaces.
  function installEyeTracking() {
    if (state.eyeCleanup) return;
    const onPointerMove = (event) => {
      state.eyePointer = { x: event.clientX, y: event.clientY };
      if (eyeTrackingNeeded()) scheduleEyePointer();
    };
    const onPointerLeave = () => resetEyePointer(true);
    window.addEventListener("pointermove", onPointerMove, { passive: true });
    window.addEventListener("blur", onPointerLeave);
    document.addEventListener("mouseleave", onPointerLeave);
    state.eyeCleanup = () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("blur", onPointerLeave);
      document.removeEventListener("mouseleave", onPointerLeave);
      resetEyePointer(true);
      state.eyeCleanup = null;
    };
  }

  function onFabClick(event) {
    if (state.suppressFabClick || state.drag?.moved) {
      state.suppressFabClick = false;
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    setOpen(!state.open, state.open ? "chip" : (event.detail === 0 ? "panel" : ""));
  }

  function onHeadFaceClick(event) {
    if (state.suppressHeadFaceClick || state.drag?.moved) {
      state.suppressHeadFaceClick = false;
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    setOpen(false, "chip");
  }

  function onGlassClick(event) {
    if (state.popover?.dataset.morphing !== "true") return;
    event.preventDefault();
    event.stopPropagation();
    const expanded = !state.open;
    startMorph(expanded, expanded ? "" : "chip");
  }

  function bindGlassPointerSurface(surface) {
    if (!(surface instanceof Element)) return;
    surface.addEventListener("pointerenter", onShellPointerMove);
    surface.addEventListener("pointermove", onShellPointerMove);
    surface.addEventListener("pointerleave", onShellPointerLeave);
    surface.addEventListener("pointercancel", resetGlassPointer);
  }

  function onShellPointerMove(event) {
    if (!state.glass || !state.popover) return;
    const expanded = state.open || state.popover.dataset.open === "true";
    const surface = event.currentTarget;
    const validSurface = expanded
      ? surface instanceof Element && surface.matches(".csw-head-face")
      : surface === state.fab;
    if (!validSurface || !(surface instanceof Element)) {
      resetGlassPointer();
      return;
    }
    const surfaceRect = surface.getBoundingClientRect();
    if (!surfaceRect.width || !surfaceRect.height) return;
    const rect = state.glass.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    state.popover.toggleAttribute("data-csw-hot-hover", true);
    const x = clamp(((event.clientX - rect.left) / rect.width) * 100, 0, 100);
    const y = clamp(((event.clientY - rect.top) / rect.height) * 100, 0, 100);
    const angle = Math.atan2(event.clientY - rect.top - rect.height / 2, event.clientX - rect.left - rect.width / 2) * 180 / Math.PI;
    const normalizedX = (event.clientX - surfaceRect.left) / surfaceRect.width - 0.5;
    const normalizedY = (event.clientY - surfaceRect.top) / surfaceRect.height - 0.5;
    const proximity = 1 - clamp(Math.hypot(normalizedX, normalizedY) / 0.72, 0, 1);
    updateMaterialDistortion(expanded, true);
    const strength = expanded ? 0.1 + proximity * 0.12 : 0.62 + proximity * 0.38;
    const parallaxX = expanded ? 1.6 : 1.8;
    const parallaxY = expanded ? 1.2 : 1.4;
    state.popover.style.setProperty("--csw-glass-x", `${x.toFixed(2)}%`);
    state.popover.style.setProperty("--csw-glass-y", `${y.toFixed(2)}%`);
    state.popover.style.setProperty("--csw-glass-px", `${(normalizedX * parallaxX).toFixed(2)}px`);
    state.popover.style.setProperty("--csw-glass-py", `${(normalizedY * parallaxY).toFixed(2)}px`);
    state.popover.style.setProperty("--csw-glass-strength", strength.toFixed(3));
    state.popover.style.setProperty("--csw-glass-angle", `${angle.toFixed(2)}deg`);
  }

  function onShellPointerLeave() {
    resetGlassPointer();
  }

  function resetGlassPointer() {
    const expanded = state.open || state.popover?.dataset.open === "true";
    state.popover?.removeAttribute("data-csw-hot-hover");
    updateMaterialDistortion(expanded, false);
    state.popover?.style.setProperty("--csw-glass-x", "28%");
    state.popover?.style.setProperty("--csw-glass-y", expanded ? "16%" : "22%");
    state.popover?.style.setProperty("--csw-glass-px", "0px");
    state.popover?.style.setProperty("--csw-glass-py", "0px");
    state.popover?.style.setProperty("--csw-glass-strength", "0");
    state.popover?.style.setProperty("--csw-glass-angle", "-40deg");
  }

  function onKeyDown(event) {
    if (event.key === "Escape" && state.open) {
      event.preventDefault();
      event.stopImmediatePropagation();
      setOpen(false, "chip");
      return;
    }
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const target = event.target;
    if (target instanceof Element && (
      target.closest("input, textarea, select, [contenteditable='true'], .ProseMirror") ||
      target.isContentEditable
    )) return;
    const isOutlineToggle = event.shiftKey && (
      event.code === "KeyO" || String(event.key || "").toUpperCase() === "O"
    );
    if (!isOutlineToggle || !state.panel || !outlineEnabled()) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (state.open && state.activeTab === "outline") {
      setOpen(false, "chip");
      return;
    }
