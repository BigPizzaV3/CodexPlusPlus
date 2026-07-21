(() => {
  const runtimeVersion = "7";
  const existing = window.__codexPlusPetRealMouseLook;
  if (existing?.version === runtimeVersion && existing?.isReady?.()) return;
  existing?.stop?.();

  const eventType = "avatar-overlay-computer-use-cursor-changed";
  const mascotSelector = '[data-avatar-mascot="true"]';
  const activationRadius = 480;
  const movementThreshold = 2;
  const movementHoldMs = 1400;
  const interactionOwner = "real-mouse";

  const interaction = (() => {
    const key = "__codexPlusPetInteraction";
    const current = window[key];
    if (current?.version === 1 && typeof current.begin === "function") return current;
    const state = { pointerId: null, owner: "", captureTarget: null };
    const next = {
      version: 1,
      begin(event, owner, captureTarget) {
        if (state.pointerId != null) return state.pointerId === event.pointerId && state.owner === owner;
        state.pointerId = event.pointerId;
        state.owner = owner;
        state.captureTarget = captureTarget || event.target;
        try {
          state.captureTarget?.setPointerCapture?.(event.pointerId);
        } catch {
        }
        return true;
      },
      owns(event, owner) {
        return state.pointerId === event.pointerId && state.owner === owner;
      },
      active() {
        return state.pointerId != null;
      },
      end(event, owner) {
        if (!next.owns(event, owner)) return false;
        try {
          state.captureTarget?.releasePointerCapture?.(event.pointerId);
        } catch {
        }
        state.pointerId = null;
        state.owner = "";
        state.captureTarget = null;
        return true;
      },
      cancel(owner) {
        if (state.owner !== owner) return false;
        const pointerId = state.pointerId;
        const captureTarget = state.captureTarget;
        try {
          if (pointerId != null) captureTarget?.releasePointerCapture?.(pointerId);
        } catch {
        }
        state.pointerId = null;
        state.owner = "";
        state.captureTarget = null;
        return true;
      },
    };
    window[key] = next;
    return next;
  })();

  let stopped = false;
  let acceptsUpdates = true;
  let updateInFlight = false;
  let dispatcher = null;
  let unsubscribe = null;
  let sendingSynthetic = false;
  let nativeCursorActive = false;
  let dragging = false;
  let syntheticActive = false;
  let lastPoint = null;
  let lastMoveAt = 0;
  let dispatcherPromise = null;

  function assetUrl(namePart) {
    const urls = [
      ...Array.from(document.scripts || []).map((script) => script.src),
      ...Array.from(document.querySelectorAll("link[href]") || []).map((link) => link.href),
      ...performance.getEntriesByType("resource").map((entry) => entry.name),
    ].filter(Boolean);
    return urls.find((url) => url.includes("/assets/") && url.includes(namePart) && url.split("?")[0].endsWith(".js")) || "";
  }

  async function assetUrlFromScriptText(namePart) {
    for (const src of Array.from(document.scripts || []).map((script) => script.src).filter(Boolean)) {
      if (!src.includes("/assets/") || !src.split("?")[0].endsWith(".js")) continue;
      try {
        const text = await fetch(src).then((response) => response.ok ? response.text() : "");
        const escaped = namePart.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const match = text.match(new RegExp(`["'](\\./(?:assets/)?${escaped}[^"']+\\.js)["']`));
        if (match) return new URL(match[1], src).href;
      } catch {
      }
    }
    return "";
  }

  function dispatcherFromModule(module) {
    for (const value of Object.values(module || {})) {
      if (value && typeof value.dispatchHostMessage === "function") return value;
      if (typeof value === "function" && typeof value.getInstance === "function") {
        try {
          const instance = value.getInstance();
          if (instance && typeof instance.dispatchHostMessage === "function") return instance;
        } catch {
        }
      }
    }
    return null;
  }

  function disableUpdates(error) {
    acceptsUpdates = false;
    runtime.lastError = String(error || "avatar overlay cursor capability became unavailable");
    syntheticActive = false;
  }

  async function resolveDispatcher() {
    if (dispatcher) return dispatcher;
    dispatcherPromise = dispatcherPromise || Promise.resolve().then(async () => {
      const url = assetUrl("vscode-api-") || await assetUrlFromScriptText("vscode-api-");
      if (!url) throw new Error("vscode-api asset unavailable");
      const module = await import(url);
      const resolved = dispatcherFromModule(module);
      if (!resolved || typeof resolved.subscribe !== "function") {
        throw new Error("V2 avatar overlay dispatcher unavailable");
      }
      if (stopped) throw new Error("pet real-mouse runtime was retired during dispatcher setup");
      const nextUnsubscribe = resolved.subscribe(eventType, (message) => {
        if (sendingSynthetic) return;
        nativeCursorActive = !!message?.point;
        if (nativeCursorActive) syntheticActive = false;
      });
      if (stopped) {
        nextUnsubscribe?.();
        throw new Error("pet real-mouse runtime was retired during dispatcher subscription");
      }
      dispatcher = resolved;
      unsubscribe = nextUnsubscribe;
      return dispatcher;
    }).catch((error) => {
      dispatcherPromise = null;
      disableUpdates(error);
      throw error;
    });
    return await dispatcherPromise;
  }

  async function sendPoint(point) {
    const target = await resolveDispatcher();
    if (stopped || !acceptsUpdates || nativeCursorActive) return;
    sendingSynthetic = true;
    try {
      await Promise.resolve(target.dispatchHostMessage({ type: eventType, point }));
    } catch (error) {
      disableUpdates(error);
      throw error;
    } finally {
      sendingSynthetic = false;
    }
  }

  function clearSyntheticPoint() {
    if (!syntheticActive) return;
    syntheticActive = false;
    if (!nativeCursorActive && acceptsUpdates) {
      void sendPoint(null).catch(disableUpdates);
    }
  }

  async function updateScreenPoint(screenPoint) {
    if (stopped || !acceptsUpdates || updateInFlight) return;
    const mascot = document.querySelector(mascotSelector);
    runtime.mascotHovered = false;
    if (!mascot || document.visibilityState !== "visible" || interaction.active() || nativeCursorActive) {
      if (!nativeCursorActive) clearSyntheticPoint();
      return;
    }
    if (!Number.isFinite(screenPoint?.x) || !Number.isFinite(screenPoint?.y)) {
      clearSyntheticPoint();
      return;
    }

    updateInFlight = true;
    try {
      const localPoint = { x: screenPoint.x - window.screenX, y: screenPoint.y - window.screenY };
      const bounds = mascot.getBoundingClientRect();
      const mascotHovered = localPoint.x >= bounds.left
        && localPoint.x <= bounds.right
        && localPoint.y >= bounds.top
        && localPoint.y <= bounds.bottom;
      runtime.mascotHovered = mascotHovered;
      if (mascotHovered) {
        clearSyntheticPoint();
        return;
      }
      const centerX = bounds.left + bounds.width / 2;
      const centerY = bounds.top + bounds.height / 2;
      const distance = Math.hypot(localPoint.x - centerX, localPoint.y - centerY);
      const movement = lastPoint == null ? Number.POSITIVE_INFINITY : Math.hypot(screenPoint.x - lastPoint.x, screenPoint.y - lastPoint.y);
      if (movement >= movementThreshold) {
        lastPoint = screenPoint;
        lastMoveAt = Date.now();
      }
      const active = distance <= activationRadius && Date.now() - lastMoveAt <= movementHoldMs;
      if (!active || nativeCursorActive || interaction.active()) {
        clearSyntheticPoint();
        return;
      }
      syntheticActive = true;
      await sendPoint(localPoint);
    } catch (error) {
      disableUpdates(error);
      clearSyntheticPoint();
    } finally {
      updateInFlight = false;
    }
  }

  function mascotAtPoint(event) {
    if (!Number.isFinite(event?.clientX) || !Number.isFinite(event?.clientY)) return null;
    const mascot = document.querySelector(mascotSelector);
    if (!mascot) return null;
    const bounds = mascot.getBoundingClientRect();
    return event.clientX >= bounds.left
      && event.clientX <= bounds.right
      && event.clientY >= bounds.top
      && event.clientY <= bounds.bottom
      ? mascot
      : null;
  }

  function onPointerDown(event) {
    const target = event.target instanceof Element ? event.target : null;
    const mascot = target?.closest(mascotSelector) || mascotAtPoint(event);
    if (!mascot) return;
    const ownsPointer = interaction.begin(event, interactionOwner, target || mascot);
    if (!ownsPointer) return;
    dragging = true;
    clearSyntheticPoint();
    event.preventDefault?.();
  }

  function onPointerMove(event) {
    if (!dragging || !interaction.owns(event, interactionOwner)) return;
    event.preventDefault?.();
  }

  function releaseDragging(event) {
    if (!dragging) return;
    if (event) {
      if (!interaction.owns(event, interactionOwner)) return;
      interaction.end(event, interactionOwner);
    } else {
      interaction.cancel(interactionOwner);
    }
    dragging = false;
  }

  function onPointerUp(event) {
    releaseDragging(event);
  }

  function onWindowBlur() {
    releaseDragging();
  }

  function stop() {
    if (stopped) return;
    stopped = true;
    acceptsUpdates = false;
    if (syntheticActive && !nativeCursorActive && dispatcher) {
      sendingSynthetic = true;
      try {
        const cleared = dispatcher.dispatchHostMessage({ type: eventType, point: null });
        if (cleared && typeof cleared.catch === "function") void cleared.catch(disableUpdates);
      } catch (error) {
        disableUpdates(error);
      } finally {
        sendingSynthetic = false;
      }
    }
    syntheticActive = false;
    document.removeEventListener("pointerdown", onPointerDown, true);
    document.removeEventListener("pointermove", onPointerMove, true);
    document.removeEventListener("pointerup", onPointerUp, true);
    document.removeEventListener("pointercancel", onPointerUp, true);
    document.removeEventListener("lostpointercapture", onPointerUp, true);
    window.removeEventListener("blur", onWindowBlur);
    interaction.cancel(interactionOwner);
    unsubscribe?.();
    if (window.__codexPlusPetRealMouseLook?.version === runtimeVersion) {
      delete window.__codexPlusPetRealMouseLook;
    }
  }

  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("pointermove", onPointerMove, true);
  document.addEventListener("pointerup", onPointerUp, true);
  document.addEventListener("pointercancel", onPointerUp, true);
  document.addEventListener("lostpointercapture", onPointerUp, true);
  window.addEventListener("blur", onWindowBlur);
  const runtime = {
    version: runtimeVersion,
    transport: "cdp-push",
    capabilities: { cdpPush: true, nativeCursorEvent: eventType },
    lastError: "",
    mascotHovered: false,
    isReady() {
      return !stopped && acceptsUpdates;
    },
    isVisualOverrideActive() {
      return nativeCursorActive || syntheticActive || interaction.active() || this.mascotHovered;
    },
    stop,
    updateScreenPoint(point) {
      if (!this.isReady()) return false;
      void updateScreenPoint(point);
      return true;
    },
  };
  window.__codexPlusPetRealMouseLook = runtime;
})();
