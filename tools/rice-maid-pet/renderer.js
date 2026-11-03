codexPlus.extension((ctx) => {
  const ID = "rice-maid-pet";
  const ROOT_ID = "codex-plus-ext-rice-maid-pet";
  const RATIO = 568 / 512;
  const GAP = 8;
  const TOP = 48;
  const FRAME_DURATIONS = [100, 100, 120, 100, 100, 180, 100, 100, 160, 100, 100];
  const configuredSize = Number(ctx.config.size);
  const size = Number.isFinite(configuredSize) ? Math.min(280, Math.max(112, configuredSize)) : 192;
  let visible = ctx.storage.get("visible", true) !== false;
  let position = ctx.storage.get("position", null);
  let bites = Math.max(0, Math.floor(Number(ctx.storage.get("bites", 0)) || 0));
  let epoch = null;
  const retiredEpochs = new Set();
  let seen = 0;
  let queued = 0;
  let busy = false;
  let disposed = false;
  let drag = null;
  let suppressClick = false;
  let stopFrame = null;
  let stopSync = null;
  let syncing = false;

  const node = (tag, className) => {
    const element = document.createElement(tag);
    element.setAttribute("data-codex-plus-ext", ID);
    if (className) element.className = className;
    return element;
  };
  const root = node("div", "rice-maid-pet");
  root.id = ROOT_ID;
  root.setAttribute("role", "group");
  root.setAttribute("aria-label", "米饭桌宠");
  root.dataset.phase = "idle";
  root.dataset.frame = "0";
  const pet = node("button", "rice-maid-pet-body");
  pet.type = "button";
  pet.title = "喂一口；拖动移动";
  const frame = node("span", "rice-maid-pet-frame");
  frame.setAttribute("aria-hidden", "true");
  const motion = node("span", "rice-maid-pet-motion");
  const image = node("img", "rice-maid-pet-sheet");
  image.alt = "";
  image.draggable = false;
  frame.append(image);
  motion.append(frame);
  pet.append(motion);
  const bubble = node("span", "rice-maid-pet-bubble");
  bubble.textContent = "啊呜";
  bubble.setAttribute("aria-hidden", "true");
  root.append(pet, bubble);
  const style = node("style");
  style.id = `${ROOT_ID}-style`;
  style.textContent = `
    #${ROOT_ID} {
      position: fixed; z-index: 9990; margin: 0; padding: 0;
      pointer-events: none; user-select: none; -webkit-user-select: none;
      -webkit-app-region: no-drag; isolation: isolate;
    }
    #${ROOT_ID}[hidden] { display: none !important; }
    #${ROOT_ID} .rice-maid-pet-body {
      display: block; position: relative; width: 100%; height: 100%;
      margin: 0; padding: 0; border: 0; border-radius: 8px; background: transparent;
      box-shadow: none; cursor: grab; pointer-events: auto; touch-action: none;
      -webkit-app-region: no-drag; outline-offset: -4px;
    }
    #${ROOT_ID} .rice-maid-pet-body:focus-visible {
      outline: 2px solid var(--codex-plus-text, #c9d9f9);
    }
    #${ROOT_ID}[data-dragging="true"] .rice-maid-pet-body { cursor: grabbing; }
    #${ROOT_ID} .rice-maid-pet-motion {
      display: block; position: absolute; inset: 0;
      pointer-events: none; transform-origin: 50% 94%;
      animation: rice-maid-pet-breathe 3.8s ease-in-out infinite;
    }
    #${ROOT_ID} .rice-maid-pet-frame {
      display: block; position: absolute; inset: 0; overflow: hidden;
      pointer-events: none; transform-origin: 50% 94%;
    }
    #${ROOT_ID} .rice-maid-pet-sheet {
      display: block; position: absolute; width: 400%; height: 300%;
      max-width: none; max-height: none; left: 0; top: 0;
      pointer-events: none; object-fit: fill; user-select: none;
      transform: translate3d(0, 0, 0); will-change: transform;
    }
    #${ROOT_ID}[data-phase="eating"] .rice-maid-pet-frame {
      animation: rice-maid-pet-eat 1.26s ease-in-out both;
    }
    #${ROOT_ID}[data-dragging="true"] .rice-maid-pet-motion {
      animation-play-state: paused;
    }
    #${ROOT_ID}[data-dragging="true"] .rice-maid-pet-frame {
      animation: none;
    }
    #${ROOT_ID} .rice-maid-pet-bubble {
      position: absolute; left: 50%; top: 6%; transform: translate(-50%, -50%);
      padding: 4px 9px; border: 1px solid var(--codex-plus-border, #64748b);
      border-radius: 8px; background: var(--codex-plus-bg-elevated, #fff);
      color: var(--codex-plus-text, #17202d); font: 500 12px/18px system-ui, sans-serif;
      letter-spacing: 0; white-space: nowrap; opacity: 0; pointer-events: none;
      transition: opacity 120ms ease-in-out;
    }
    #${ROOT_ID}[data-phase="eating"] .rice-maid-pet-bubble { opacity: 1; }
    @keyframes rice-maid-pet-breathe {
      0%, 100% { transform: translateY(0); }
      50% { transform: translateY(-2px); }
    }
    @keyframes rice-maid-pet-eat {
      0%, 100% { transform: translateY(0) rotate(0); }
      28% { transform: translateY(-1.5px) rotate(-0.35deg); }
      55% { transform: translateY(-0.5px) rotate(0.2deg); }
      76% { transform: translateY(-1px) rotate(-0.15deg); }
    }
    @media (prefers-reduced-motion: reduce) {
      #${ROOT_ID} .rice-maid-pet-motion,
      #${ROOT_ID} .rice-maid-pet-frame { animation: none !important; }
      #${ROOT_ID} .rice-maid-pet-bubble { transition: none !important; }
    }
  `;

  function viewportSize() {
    const width = Math.max(1, window.innerWidth);
    const height = Math.max(1, window.innerHeight);
    const roomWidth = width > GAP * 2 ? width - GAP * 2 : width;
    const roomHeight = height > TOP + GAP ? height - TOP - GAP : height;
    const petSize = Math.min(size, roomWidth, roomHeight / RATIO);
    return { width, height, petSize, petHeight: petSize * RATIO };
  }

  function place(next, persist = false) {
    const { width, height, petSize, petHeight } = viewportSize();
    const defaultPosition = { x: width - petSize - 28, y: height - petHeight - 164 };
    const candidate = next && Number.isFinite(next.x) && Number.isFinite(next.y) ? next : defaultPosition;
    const minX = Math.min(GAP, Math.max(0, width - petSize));
    const minY = Math.min(TOP, Math.max(0, height - petHeight));
    position = {
      x: Math.max(minX, Math.min(candidate.x, width - petSize - GAP)),
      y: Math.max(minY, Math.min(candidate.y, height - petHeight - GAP)),
    };
    root.style.width = `${petSize}px`;
    root.style.height = `${petHeight}px`;
    root.style.left = `${position.x}px`;
    root.style.top = `${position.y}px`;
    if (persist) ctx.storage.set("position", position);
  }

  function updateLabel() {
    root.dataset.bites = String(bites);
    pet.setAttribute("aria-label", `米饭桌宠，已吃 ${bites} 口`);
  }

  function showFrame(index) {
    root.dataset.frame = String(index);
    image.style.transform = `translate3d(-${(index % 4) * 25}%, -${Math.floor(index / 4) * 100 / 3}%, 0)`;
  }

  function clearAnimation() {
    stopFrame?.();
    stopFrame = null;
    queued = 0;
    busy = false;
    root.dataset.phase = "idle";
    showFrame(0);
  }

  function later(fn, delay) {
    stopFrame = ctx.setTimeout(() => {
      stopFrame = null;
      if (!disposed) fn();
    }, delay);
  }

  function playNext() {
    if (busy || !visible || !queued || document.hidden || disposed) return;
    queued = 0;
    busy = true;
    root.dataset.phase = "eating";
    let index = 0;
    const step = () => {
      if (index === FRAME_DURATIONS.length) {
        root.dataset.phase = "idle";
        showFrame(0);
        later(() => {
          busy = false;
          playNext();
        }, 100);
        return;
      }
      showFrame(++index);
      later(step, FRAME_DURATIONS[index - 1]);
    };
    step();
  }

  function feed(amount = 1) {
    if (disposed || !Number.isSafeInteger(amount) || amount <= 0) return;
    bites = Math.min(Number.MAX_SAFE_INTEGER, bites + amount);
    ctx.storage.set("bites", bites);
    updateLabel();
    if (visible && !document.hidden) {
      // 请求数完整保留，视觉上只补一口，避免突发请求积压成长队。
      queued = 1;
      playNext();
    }
  }

  function feedManual() {
    // 连点不打断当前动作，也不排队；下一口完成后即可再次响应。
    if (busy) return;
    feed();
  }

  // epoch 区分后台重启；计数差值补齐并发请求，也避免快照和推送重复喂饭。
  function receive(payload, baseline = false) {
    if (disposed || !payload || typeof payload.epoch !== "string" ||
        !Number.isSafeInteger(payload.count) || payload.count < 0) return;
    if (retiredEpochs.has(payload.epoch)) return;
    if (epoch !== payload.epoch) {
      if (baseline && epoch !== null) return;
      const firstEpoch = epoch === null;
      if (epoch !== null) retiredEpochs.add(epoch);
      epoch = payload.epoch;
      // 首条推送可能来自已经运行的后台，只从订阅后的这一口开始计数。
      seen = baseline ? payload.count : firstEpoch ? Math.max(0, payload.count - 1) : 0;
    }
    const delta = Math.max(0, payload.count - seen);
    seen = Math.max(seen, payload.count);
    root.dataset.requestCount = String(seen);
    if (!baseline && delta) feed(delta);
  }

  async function sync() {
    if (syncing || disposed) return;
    syncing = true;
    const epochBeforeCall = epoch;
    try {
      const snapshot = await ctx.backend.call("getState");
      if (disposed) return;
      // 请求发出后若已收到新后台事件，不能让迟到的旧快照切回旧 epoch。
      if (epoch === epochBeforeCall || snapshot?.epoch === epoch) receive(snapshot, epoch === null);
      root.dataset.connected = "true";
    } catch {
      if (!disposed) root.dataset.connected = "false";
    } finally {
      syncing = false;
      if (!disposed) stopSync = ctx.setTimeout(sync, 15000);
    }
  }

  function finishDrag(event) {
    if (!drag || (event && event.pointerId !== drag.pointerId)) return;
    const { pointerId, moved } = drag;
    drag = null;
    root.dataset.dragging = "false";
    suppressClick = moved;
    if (pet.hasPointerCapture?.(pointerId)) pet.releasePointerCapture(pointerId);
    if (moved) ctx.storage.set("position", position);
  }

  ctx.listen(pet, "pointerdown", (event) => {
    if (event.button !== 0 || event.isPrimary === false) return;
    suppressClick = false;
    drag = {
      pointerId: event.pointerId, x: event.clientX, y: event.clientY,
      originX: position.x, originY: position.y, moved: false,
    };
    pet.setPointerCapture?.(event.pointerId);
  });
  ctx.listen(pet, "pointermove", (event) => {
    if (!drag || drag.pointerId !== event.pointerId) return;
    const dx = event.clientX - drag.x;
    const dy = event.clientY - drag.y;
    if (!drag.moved && Math.hypot(dx, dy) < 4) return;
    drag.moved = true;
    root.dataset.dragging = "true";
    place({ x: drag.originX + dx, y: drag.originY + dy });
  });
  ctx.listen(pet, "pointerup", finishDrag);
  ctx.listen(pet, "pointercancel", finishDrag);
  ctx.listen(pet, "lostpointercapture", finishDrag);
  ctx.listen(window, "blur", () => finishDrag());
  ctx.listen(pet, "click", (event) => {
    if (suppressClick) {
      suppressClick = false;
      event.preventDefault();
      return;
    }
    feedManual();
  });
  ctx.listen(pet, "keydown", (event) => {
    const directions = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    const direction = directions[event.key];
    if (!direction) return;
    event.preventDefault();
    const step = event.shiftKey ? 24 : 8;
    place({ x: position.x + direction[0] * step, y: position.y + direction[1] * step }, true);
  });
  ctx.listen(image, "load", () => { root.dataset.imageReady = "true"; });
  ctx.listen(image, "error", () => {
    root.dataset.imageReady = "false";
    ctx.fail(new Error("米饭桌宠素材加载失败"));
  });
  image.src = ctx.asset("assets/rice-maid-sequence.webp");
  ctx.listen(window, "resize", () => {
    finishDrag();
    place(position, true);
  });
  ctx.listen(document, "visibilitychange", () => {
    if (document.hidden) {
      finishDrag();
      clearAnimation();
    }
    else playNext();
  });

  ctx.registerMenuItem({
    label: "米饭桌宠",
    toggleValue: () => visible,
    onChange: (next) => {
      visible = !!next;
      root.hidden = !visible;
      ctx.storage.set("visible", visible);
      if (!visible) {
        finishDrag();
        clearAnimation();
      }
    },
  });
  ctx.registerMenuItem({
    label: "喂一口米饭",
    buttonLabel: "喂一口",
    onActivate: ({ close }) => {
      visible = true;
      root.hidden = false;
      ctx.storage.set("visible", true);
      feedManual();
      close();
    },
  });
  ctx.registerMenuItem({
    label: "桌宠位置",
    buttonLabel: "归位",
    onActivate: ({ close }) => {
      finishDrag();
      place(null, true);
      close();
    },
  });

  function mount() {
    if (disposed) return;
    if (!style.isConnected) document.head?.append(style);
    if (!root.isConnected) document.body?.append(root);
  }
  ctx.effect(() => {
    place(position);
    root.hidden = !visible;
    updateLabel();
    mount();
    return () => {
      disposed = true;
      finishDrag();
      stopSync?.();
      clearAnimation();
      root.remove();
      style.remove();
    };
  });
  ctx.onScan(mount, { phase: "light" });
  ctx.backend.on("bite", (payload) => receive(payload));
  void sync();
});
