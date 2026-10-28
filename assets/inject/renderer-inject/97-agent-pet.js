  // Agent 桌宠：Codex 窗口里一个可拖动的小角色，按下以底边为支点压扁、松开回弹，并播放按压 / 松开音。
  // 图片不随应用打包：经桥接 `/agent-pet/image` 由后端联网下载、缓存后以 data URI 返回。
  // 内嵌的两组音效随启动器注入（window.__CODEX_PLUS_AGENT_PET_SOUNDS__）；下载音效组和用户导入的自定义音效
  // 经桥接 `/agent-pet/sound` 取 data URI。一律直接从字节解码，不依赖 fetch(data:)。
  // 设置读后端的 `agentPet`，与「Codex 增强」总开关无关；目前只有 Codex 一个 Agent。
  window.__codexPlusAgentPetRuntime?.dispose?.();
  const codexPlusAgentPetPositionKey = "codexPlus.agentPet.position.v1";
  const codexPlusAgentPetSquash = "scaleY(0.88) scaleX(1.05)";
  // 点按时松开音提前进场的毫秒数，与按压音末尾轻微交叠，听感更「黏」。
  const codexPlusAgentPetReleaseLeadMs = 40;
  const codexPlusAgentPet = {
    disposed: false,
    root: null,
    body: null,
    image: null,
    imageKey: "",
    imageRequest: 0,
    retryAt: 0,
    failureLogged: false,
    audio: null,
    buffers: new Map(),
    soundKey: "",
    soundRequest: 0,
    soundRetryAt: 0,
    soundFailureLogged: false,
    press: null,
    drag: null,
    cleanup: [],
  };

  function codexPlusAgentPetSettings() {
    if (!codexPlusBackendSettingsLoaded) return null;
    const raw = codexPlusBackendSettings?.agentPet;
    if (!raw || typeof raw !== "object") return null;
    const size = Number(raw.size);
    const volume = Number(raw.volume);
    return {
      enabled: raw.enabled === true,
      agents: Array.isArray(raw.agents) ? raw.agents : ["codex"],
      image: typeof raw.image === "string" ? raw.image : "whale",
      customImageUrl: typeof raw.customImageUrl === "string" ? raw.customImageUrl : "",
      size: Number.isFinite(size) ? Math.min(240, Math.max(64, size)) : 112,
      soundEnabled: raw.soundEnabled !== false,
      soundSet: typeof raw.soundSet === "string" && raw.soundSet ? raw.soundSet : "duck",
      customSoundPress: typeof raw.customSoundPress === "string" ? raw.customSoundPress : "",
      customSoundRelease: typeof raw.customSoundRelease === "string" ? raw.customSoundRelease : "",
      volume: Number.isFinite(volume) ? Math.min(1, Math.max(0, volume)) : 0.6,
    };
  }

  function codexPlusAgentPetActive(settings) {
    return !codexPlusAgentPet.disposed && Boolean(settings?.enabled) && settings.agents.includes("codex");
  }

  function readCodexPlusAgentPetPosition() {
    try {
      const value = JSON.parse(localStorage.getItem(codexPlusAgentPetPositionKey) || "null");
      if (value && Number.isFinite(value.right) && Number.isFinite(value.bottom)) return value;
    } catch {}
    // 默认在右下角、输入框上方。
    return { right: 28, bottom: 132 };
  }

  function writeCodexPlusAgentPetPosition(position) {
    try { localStorage.setItem(codexPlusAgentPetPositionKey, JSON.stringify(position)); } catch {}
  }

  // 以右下角为基准记位置：窗口缩放时桌宠跟着右下角走，再夹回可视区。
  function placeCodexPlusAgentPet(position) {
    const root = codexPlusAgentPet.root;
    if (!root) return;
    const size = root.offsetWidth || 112;
    const maxRight = Math.max(0, window.innerWidth - size);
    const maxBottom = Math.max(0, window.innerHeight - size);
    const right = Math.min(maxRight, Math.max(0, position.right));
    const bottom = Math.min(maxBottom, Math.max(0, position.bottom));
    root.style.right = `${right}px`;
    root.style.bottom = `${bottom}px`;
    return { right, bottom };
  }

  function codexPlusAgentPetAudioContext() {
    if (codexPlusAgentPet.audio) return codexPlusAgentPet.audio;
    const AudioContextClass = window.AudioContext || window.webkitAudioContext;
    if (typeof AudioContextClass !== "function") return null;
    try { codexPlusAgentPet.audio = new AudioContextClass(); } catch { return null; }
    return codexPlusAgentPet.audio;
  }

  function decodeCodexPlusAgentPetSound(context, dataUri) {
    const comma = typeof dataUri === "string" ? dataUri.indexOf(",") : -1;
    if (comma < 0) return Promise.resolve(null);
    try {
      const binary = atob(dataUri.slice(comma + 1));
      const bytes = new Uint8Array(binary.length);
      for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
      return context.decodeAudioData(bytes.buffer).catch(() => null);
    } catch {
      return Promise.resolve(null);
    }
  }

  // 与后端 `agent_pet::load_sound` 返回的 key 同一规则：自定义音效带文件名，换了文件就重新取。
  function codexPlusAgentPetSoundKey(settings) {
    return settings.soundSet === "custom"
      ? `custom:${settings.customSoundPress}:${settings.customSoundRelease}`
      : settings.soundSet;
  }

  function decodeCodexPlusAgentPetSounds(context, key, sounds) {
    for (const slot of ["press", "release"]) {
      const bufferKey = `${key}:${slot}`;
      if (codexPlusAgentPet.buffers.has(bufferKey) || !sounds?.[slot]) continue;
      codexPlusAgentPet.buffers.set(bufferKey, null);
      void decodeCodexPlusAgentPetSound(context, sounds[slot]).then((buffer) => {
        if (buffer) codexPlusAgentPet.buffers.set(bufferKey, buffer);
        else codexPlusAgentPet.buffers.delete(bufferKey);
      });
    }
  }

  // 下载 / 自定义音效：向后端要一次，失败 30 秒后再试。
  function requestCodexPlusAgentPetSounds(context, key) {
    if (codexPlusAgentPet.soundKey === key) return;
    if (Date.now() < codexPlusAgentPet.soundRetryAt) return;
    codexPlusAgentPet.soundKey = key;
    const request = ++codexPlusAgentPet.soundRequest;
    void Promise.resolve(postJson("/agent-pet/sound", {})).then((result) => {
      if (request !== codexPlusAgentPet.soundRequest || codexPlusAgentPet.audio !== context) return;
      if (result?.status !== "ok" || typeof result.key !== "string") {
        throw new Error(result?.message || result?.status || "音效不可用");
      }
      codexPlusAgentPet.soundRetryAt = 0;
      codexPlusAgentPet.soundFailureLogged = false;
      decodeCodexPlusAgentPetSounds(context, result.key, result);
    }).catch((error) => {
      if (request !== codexPlusAgentPet.soundRequest) return;
      codexPlusAgentPet.soundKey = "";
      codexPlusAgentPet.soundRetryAt = Date.now() + 30000;
      if (!codexPlusAgentPet.soundFailureLogged) {
        codexPlusAgentPet.soundFailureLogged = true;
        void sendCodexPlusDiagnostic("agent_pet_sound_failed", { message: String(error?.message || error) });
      }
    });
  }

  // 预解码一组按压 / 松开音；起播走同步路径，手感才贴手。
  function warmCodexPlusAgentPetSounds(settings) {
    const context = codexPlusAgentPetAudioContext();
    if (!context) return;
    const embedded = window.__CODEX_PLUS_AGENT_PET_SOUNDS__?.[settings.soundSet];
    if (embedded) decodeCodexPlusAgentPetSounds(context, settings.soundSet, embedded);
    else requestCodexPlusAgentPetSounds(context, codexPlusAgentPetSoundKey(settings));
  }

  function playCodexPlusAgentPetSound(settings, slot, delaySeconds = 0) {
    const context = codexPlusAgentPet.audio;
    const buffer = codexPlusAgentPet.buffers.get(`${codexPlusAgentPetSoundKey(settings)}:${slot}`);
    if (!context || !buffer || settings.volume <= 0) return null;
    try {
      if (context.state === "suspended") void context.resume();
      const source = context.createBufferSource();
      const gain = context.createGain();
      gain.gain.value = settings.volume;
      source.buffer = buffer;
      source.connect(gain).connect(context.destination);
      const startAt = context.currentTime + Math.max(0, delaySeconds);
      source.start(startAt);
      return { source, startAt, duration: buffer.duration };
    } catch {
      return null;
    }
  }

  function stopCodexPlusAgentPetSound(playing) {
    try { playing?.source?.stop(); } catch {}
  }

  function pressCodexPlusAgentPet() {
    const settings = codexPlusAgentPetSettings();
    codexPlusAgentPet.body.style.transform = codexPlusAgentPetSquash;
    stopCodexPlusAgentPetSound(codexPlusAgentPet.press?.release);
    codexPlusAgentPet.press = { sound: null, release: null };
    if (!settings?.soundEnabled) return;
    // 首次按下才建音频上下文（需要用户手势），这次按压的音若还没解码好就静音，下一次就有了。
    warmCodexPlusAgentPetSounds(settings);
    codexPlusAgentPet.press.sound = playCodexPlusAgentPetSound(settings, "press");
  }

  function releaseCodexPlusAgentPet() {
    const settings = codexPlusAgentPetSettings();
    codexPlusAgentPet.body.style.transform = "";
    const press = codexPlusAgentPet.press;
    if (!press || !settings?.soundEnabled) return;
    // 点按：松开音排到按压音结束前一点点；按住超过按压音时长：立即播放。
    let delay = 0;
    if (press.sound && codexPlusAgentPet.audio) {
      const end = press.sound.startAt + press.sound.duration - codexPlusAgentPetReleaseLeadMs / 1000;
      delay = Math.max(0, end - codexPlusAgentPet.audio.currentTime);
    }
    press.release = playCodexPlusAgentPetSound(settings, "release", delay);
  }

  function mountCodexPlusAgentPet(settings) {
    const root = document.createElement("div");
    root.setAttribute("data-codex-plus-ext", "agent-pet");
    root.setAttribute("role", "img");
    root.setAttribute("aria-label", "Agent 桌宠");
    root.style.cssText = "position:fixed;z-index:2147483000;display:none;touch-action:none;user-select:none;-webkit-user-select:none;cursor:grab;";
    const body = document.createElement("div");
    body.setAttribute("data-codex-plus-ext", "agent-pet");
    // 与管理器里的预览同一组参数：底边为支点，松开时带一点过冲回弹。
    body.style.cssText = "width:100%;height:100%;transform-origin:50% 100%;transition:transform .22s cubic-bezier(.34,1.56,.64,1);";
    const image = document.createElement("img");
    image.setAttribute("data-codex-plus-ext", "agent-pet");
    image.alt = "";
    image.draggable = false;
    image.style.cssText = "width:100%;height:100%;object-fit:contain;pointer-events:none;filter:drop-shadow(0 4px 8px rgba(0,0,0,.18));";
    body.appendChild(image);
    root.appendChild(body);
    document.body.appendChild(root);
    codexPlusAgentPet.root = root;
    codexPlusAgentPet.body = body;
    codexPlusAgentPet.image = image;
    registerCodexPlusExtensionSelector("[data-codex-plus-ext=agent-pet]");

    const onPointerDown = (event) => {
      if (event.button !== 0 && event.pointerType === "mouse") return;
      event.preventDefault();
      try { root.setPointerCapture(event.pointerId); } catch {}
      const current = readCodexPlusAgentPetPosition();
      codexPlusAgentPet.drag = { id: event.pointerId, x: event.clientX, y: event.clientY, start: current, moved: false };
      root.style.cursor = "grabbing";
      pressCodexPlusAgentPet();
    };
    const onPointerMove = (event) => {
      const drag = codexPlusAgentPet.drag;
      if (!drag || drag.id !== event.pointerId) return;
      const dx = event.clientX - drag.x;
      const dy = event.clientY - drag.y;
      if (!drag.moved && Math.hypot(dx, dy) < 4) return;
      drag.moved = true;
      drag.position = placeCodexPlusAgentPet({ right: drag.start.right - dx, bottom: drag.start.bottom - dy });
    };
    const onPointerEnd = (event) => {
      const drag = codexPlusAgentPet.drag;
      if (!drag || drag.id !== event.pointerId) return;
      codexPlusAgentPet.drag = null;
      root.style.cursor = "grab";
      if (drag.moved && drag.position) writeCodexPlusAgentPetPosition(drag.position);
      releaseCodexPlusAgentPet();
    };
    const onResize = () => placeCodexPlusAgentPet(readCodexPlusAgentPetPosition());
    root.addEventListener("pointerdown", onPointerDown);
    root.addEventListener("pointermove", onPointerMove);
    root.addEventListener("pointerup", onPointerEnd);
    root.addEventListener("pointercancel", onPointerEnd);
    root.addEventListener("contextmenu", (event) => event.preventDefault());
    window.addEventListener("resize", onResize);
    codexPlusAgentPet.cleanup.push(() => window.removeEventListener("resize", onResize));
    applyCodexPlusAgentPetSize(settings);
  }

  function applyCodexPlusAgentPetSize(settings) {
    const root = codexPlusAgentPet.root;
    if (!root) return;
    const size = `${settings.size}px`;
    if (root.style.width !== size) {
      root.style.width = size;
      root.style.height = size;
      placeCodexPlusAgentPet(readCodexPlusAgentPetPosition());
    }
  }

  function unmountCodexPlusAgentPet() {
    codexPlusAgentPet.cleanup.splice(0).forEach((cleanup) => cleanup());
    codexPlusAgentPet.root?.remove();
    codexPlusAgentPet.root = null;
    codexPlusAgentPet.body = null;
    codexPlusAgentPet.image = null;
    codexPlusAgentPet.imageKey = "";
    codexPlusAgentPet.imageRequest += 1;
    codexPlusAgentPet.drag = null;
    codexPlusAgentPet.press = null;
    try { void codexPlusAgentPet.audio?.close(); } catch {}
    codexPlusAgentPet.audio = null;
    codexPlusAgentPet.buffers.clear();
    codexPlusAgentPet.soundKey = "";
    codexPlusAgentPet.soundRequest += 1;
  }

  function loadCodexPlusAgentPetImage(settings) {
    const key = settings.image === "custom" ? `custom:${settings.customImageUrl}` : settings.image;
    if (codexPlusAgentPet.imageKey === key) return;
    if (Date.now() < codexPlusAgentPet.retryAt) return;
    codexPlusAgentPet.imageKey = key;
    const request = ++codexPlusAgentPet.imageRequest;
    void Promise.resolve(postJson("/agent-pet/image", {})).then((result) => {
      if (request !== codexPlusAgentPet.imageRequest || !codexPlusAgentPet.image) return;
      if (result?.status !== "ok" || typeof result.image !== "string" || !result.image.startsWith("data:image/")) {
        throw new Error(result?.message || result?.status || "图片不可用");
      }
      codexPlusAgentPet.image.src = result.image;
      codexPlusAgentPet.root.style.display = "block";
      codexPlusAgentPet.retryAt = 0;
      codexPlusAgentPet.failureLogged = false;
      placeCodexPlusAgentPet(readCodexPlusAgentPetPosition());
    }).catch((error) => {
      if (request !== codexPlusAgentPet.imageRequest) return;
      // 下载失败（离线、地址失效）：30 秒后再试，期间不显示空白框。
      codexPlusAgentPet.imageKey = "";
      codexPlusAgentPet.retryAt = Date.now() + 30000;
      if (!codexPlusAgentPet.failureLogged) {
        codexPlusAgentPet.failureLogged = true;
        void sendCodexPlusDiagnostic("agent_pet_image_failed", { message: String(error?.message || error) });
      }
    });
  }

  function syncCodexPlusAgentPet() {
    const settings = codexPlusAgentPetSettings();
    if (!codexPlusAgentPetActive(settings)) {
      if (codexPlusAgentPet.root) unmountCodexPlusAgentPet();
      codexPlusAgentPet.retryAt = 0;
      return;
    }
    if (!document.body) return;
    if (!codexPlusAgentPet.root || !codexPlusAgentPet.root.isConnected) {
      if (codexPlusAgentPet.root) unmountCodexPlusAgentPet();
      mountCodexPlusAgentPet(settings);
    }
    applyCodexPlusAgentPetSize(settings);
    // 挂载后就预解码当前音效组（上下文此时可能是 suspended，按下时再 resume），第一次按压也有声音。
    if (settings.soundEnabled) warmCodexPlusAgentPetSounds(settings);
    loadCodexPlusAgentPetImage(settings);
  }

  window.__codexPlusAgentPetRuntime = {
    version: 1,
    sync: syncCodexPlusAgentPet,
    dispose() { unmountCodexPlusAgentPet(); codexPlusAgentPet.disposed = true; },
    get state() { return { mounted: Boolean(codexPlusAgentPet.root), imageKey: codexPlusAgentPet.imageKey, dragging: Boolean(codexPlusAgentPet.drag) }; },
  };
