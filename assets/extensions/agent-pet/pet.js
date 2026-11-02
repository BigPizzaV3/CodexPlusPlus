  // Agent 桌宠（图片桌宠）：Codex 窗口里一个可拖动的小角色，按下以底边为支点压扁、松开回弹，并播放按压 / 松开音。
  // 只用公开拓展接口：容器来自 overlay 插槽，设置来自本拓展的 config（管理器「Agent 桌宠」页写入），
  // 图片与下载 / 自定义音效经路由 `/agent-pet/image`、`/agent-pet/sound` 由 Codex++ 联网下载、校验、缓存后以 data URI 返回，
  // 内嵌的两组音效随拓展包的 assets/ 下发。一律直接从字节解码，不依赖 fetch(data:)。
  // `kind` 为 whaleWidget（大肥鱼）时这里不挂载，由 whale.js 显示完整鲸鱼挂件。
  const codexPlusAgentPetPositionKey = "codexPlus.agentPet.position.v1";
  const codexPlusAgentPetSquash = "scaleY(0.88) scaleX(1.05)";
  // 点按时松开音提前进场的毫秒数，与按压音末尾轻微交叠，听感更「黏」。
  const codexPlusAgentPetReleaseLeadMs = 40;
  /** 随拓展包内嵌的音效组：组名 → 包内资源。 */
  const codexPlusAgentPetEmbeddedSounds = {
    duck: { press: "assets/sounds/Ya1.mp3", release: "assets/sounds/Ya2.mp3" },
    fx1: { press: "assets/sounds/D1.mp3", release: "assets/sounds/D2.mp3" },
  };

  /** 拓展配置 → 桌宠设置。值都经过管理器校验，这里只兜底范围。 */
  function codexPlusAgentPetSettingsFrom(config) {
    const raw = config && typeof config === "object" ? config : {};
    const size = Number(raw.size);
    const volume = Number(raw.volume);
    return {
      kind: raw.kind === "whaleWidget" ? "whaleWidget" : "image",
      image: typeof raw.image === "string" && raw.image ? raw.image : "whale",
      customImageUrl: typeof raw.customImageUrl === "string" ? raw.customImageUrl : "",
      size: Number.isFinite(size) ? Math.min(240, Math.max(64, size)) : 112,
      soundEnabled: raw.soundEnabled !== false,
      soundSet: typeof raw.soundSet === "string" && raw.soundSet ? raw.soundSet : "duck",
      customSoundPress: typeof raw.customSoundPress === "string" ? raw.customSoundPress : "",
      customSoundRelease: typeof raw.customSoundRelease === "string" ? raw.customSoundRelease : "",
      volume: Number.isFinite(volume) ? Math.min(1, Math.max(0, volume)) : 0.6,
    };
  }

  /**
   * 图片桌宠。`ctx` 只用公开接口：config、asset、call（清单声明了 /agent-pet/* 路由）。
   * 返回 { setHost, sync, dispose, state }：setHost 交给 overlay 插槽的容器，sync 按当前配置对齐，dispose 全部撤下。
   */
  function createCodexPlusAgentPet(ctx) {
    const pet = {
      disposed: false,
      root: null,
      /** overlay 插槽给的容器；mountedHost 是桌宠当前挂在的那个。 */
      host: null,
      mountedHost: null,
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
    const settings = () => codexPlusAgentPetSettingsFrom(ctx.config);
    const diagnostic = (event, detail) => {
      void Promise.resolve(ctx.call("/diagnostics/log", { event, detail })).catch(() => {});
    };

    function active(current) {
      return !pet.disposed && current.kind !== "whaleWidget";
    }

    function readPosition() {
      try {
        const value = JSON.parse(localStorage.getItem(codexPlusAgentPetPositionKey) || "null");
        if (value && Number.isFinite(value.right) && Number.isFinite(value.bottom)) return value;
      } catch {}
      // 默认在右下角、输入框上方。
      return { right: 28, bottom: 132 };
    }

    function writePosition(position) {
      try { localStorage.setItem(codexPlusAgentPetPositionKey, JSON.stringify(position)); } catch {}
    }

    // 以右下角为基准记位置：窗口缩放时桌宠跟着右下角走，再夹回可视区。
    function place(position) {
      const root = pet.root;
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

    function audioContext() {
      if (pet.audio) return pet.audio;
      const AudioContextClass = window.AudioContext || window.webkitAudioContext;
      if (typeof AudioContextClass !== "function") return null;
      try { pet.audio = new AudioContextClass(); } catch { return null; }
      return pet.audio;
    }

    function decodeSound(context, dataUri) {
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

    // 与 Codex++ `agent_pet::load_sound` 返回的 key 同一规则：自定义音效带文件名，换了文件就重新取。
    function soundKey(current) {
      return current.soundSet === "custom"
        ? `custom:${current.customSoundPress}:${current.customSoundRelease}`
        : current.soundSet;
    }

    function decodeSounds(context, key, sounds) {
      for (const slot of ["press", "release"]) {
        const bufferKey = `${key}:${slot}`;
        if (pet.buffers.has(bufferKey) || !sounds?.[slot]) continue;
        pet.buffers.set(bufferKey, null);
        void decodeSound(context, sounds[slot]).then((buffer) => {
          if (buffer) pet.buffers.set(bufferKey, buffer);
          else pet.buffers.delete(bufferKey);
        });
      }
    }

    // 下载 / 自定义音效：向 Codex++ 要一次，失败 30 秒后再试。
    function requestSounds(context, current) {
      const key = soundKey(current);
      if (pet.soundKey === key) return;
      if (Date.now() < pet.soundRetryAt) return;
      pet.soundKey = key;
      const request = ++pet.soundRequest;
      const payload = { soundSet: current.soundSet, customSoundPress: current.customSoundPress, customSoundRelease: current.customSoundRelease };
      void Promise.resolve(ctx.call("/agent-pet/sound", payload)).then((result) => {
        if (request !== pet.soundRequest || pet.audio !== context) return;
        if (result?.status !== "ok" || typeof result.key !== "string") {
          throw new Error(result?.message || result?.status || "音效不可用");
        }
        pet.soundRetryAt = 0;
        pet.soundFailureLogged = false;
        decodeSounds(context, result.key, result);
      }).catch((error) => {
        if (request !== pet.soundRequest) return;
        pet.soundKey = "";
        pet.soundRetryAt = Date.now() + 30000;
        if (!pet.soundFailureLogged) {
          pet.soundFailureLogged = true;
          diagnostic("agent_pet_sound_failed", { message: String(error?.message || error) });
        }
      });
    }

    // 预解码一组按压 / 松开音；起播走同步路径，手感才贴手。
    function warmSounds(current) {
      const context = audioContext();
      if (!context) return;
      const embedded = codexPlusAgentPetEmbeddedSounds[current.soundSet];
      if (embedded) decodeSounds(context, current.soundSet, { press: ctx.asset(embedded.press), release: ctx.asset(embedded.release) });
      else requestSounds(context, current);
    }

    function playSound(current, slot, delaySeconds = 0) {
      const context = pet.audio;
      const buffer = pet.buffers.get(`${soundKey(current)}:${slot}`);
      if (!context || !buffer || current.volume <= 0) return null;
      try {
        if (context.state === "suspended") void context.resume();
        const source = context.createBufferSource();
        const gain = context.createGain();
        gain.gain.value = current.volume;
        source.buffer = buffer;
        source.connect(gain).connect(context.destination);
        const startAt = context.currentTime + Math.max(0, delaySeconds);
        source.start(startAt);
        return { source, startAt, duration: buffer.duration };
      } catch {
        return null;
      }
    }

    function stopSound(playing) {
      try { playing?.source?.stop(); } catch {}
    }

    function press() {
      const current = settings();
      pet.body.style.transform = codexPlusAgentPetSquash;
      stopSound(pet.press?.release);
      pet.press = { sound: null, release: null };
      if (!current.soundEnabled) return;
      // 首次按下才建音频上下文（需要用户手势），这次按压的音若还没解码好就静音，下一次就有了。
      warmSounds(current);
      pet.press.sound = playSound(current, "press");
    }

    function release() {
      const current = settings();
      pet.body.style.transform = "";
      const pressed = pet.press;
      if (!pressed || !current.soundEnabled) return;
      // 点按：松开音排到按压音结束前一点点；按住超过按压音时长：立即播放。
      let delay = 0;
      if (pressed.sound && pet.audio) {
        const end = pressed.sound.startAt + pressed.sound.duration - codexPlusAgentPetReleaseLeadMs / 1000;
        delay = Math.max(0, end - pet.audio.currentTime);
      }
      pressed.release = playSound(current, "release", delay);
    }

    function mount(current) {
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
      // overlay 插槽给了容器就放进去，否则直接挂在 body 上。
      const slotHost = pet.host?.isConnected ? pet.host : null;
      (slotHost || document.body).appendChild(root);
      pet.mountedHost = slotHost;
      pet.root = root;
      pet.body = body;
      pet.image = image;

      const onPointerDown = (event) => {
        if (event.button !== 0 && event.pointerType === "mouse") return;
        event.preventDefault();
        try { root.setPointerCapture(event.pointerId); } catch {}
        const start = readPosition();
        pet.drag = { id: event.pointerId, x: event.clientX, y: event.clientY, start, moved: false };
        root.style.cursor = "grabbing";
        press();
      };
      const onPointerMove = (event) => {
        const drag = pet.drag;
        if (!drag || drag.id !== event.pointerId) return;
        const dx = event.clientX - drag.x;
        const dy = event.clientY - drag.y;
        if (!drag.moved && Math.hypot(dx, dy) < 4) return;
        drag.moved = true;
        drag.position = place({ right: drag.start.right - dx, bottom: drag.start.bottom - dy });
      };
      const onPointerEnd = (event) => {
        const drag = pet.drag;
        if (!drag || drag.id !== event.pointerId) return;
        pet.drag = null;
        root.style.cursor = "grab";
        if (drag.moved && drag.position) writePosition(drag.position);
        release();
      };
      const onResize = () => place(readPosition());
      root.addEventListener("pointerdown", onPointerDown);
      root.addEventListener("pointermove", onPointerMove);
      root.addEventListener("pointerup", onPointerEnd);
      root.addEventListener("pointercancel", onPointerEnd);
      root.addEventListener("contextmenu", (event) => event.preventDefault());
      window.addEventListener("resize", onResize);
      pet.cleanup.push(() => window.removeEventListener("resize", onResize));
      applySize(current);
    }

    function applySize(current) {
      const root = pet.root;
      if (!root) return;
      const size = `${current.size}px`;
      if (root.style.width !== size) {
        root.style.width = size;
        root.style.height = size;
        place(readPosition());
      }
    }

    function unmount() {
      pet.cleanup.splice(0).forEach((cleanup) => cleanup());
      pet.root?.remove();
      pet.root = null;
      pet.body = null;
      pet.image = null;
      pet.imageKey = "";
      pet.imageRequest += 1;
      pet.drag = null;
      pet.press = null;
      try { void pet.audio?.close(); } catch {}
      pet.audio = null;
      pet.buffers.clear();
      pet.soundKey = "";
      pet.soundRequest += 1;
    }

    function loadImage(current) {
      const key = current.image === "custom" ? `custom:${current.customImageUrl}` : current.image;
      if (pet.imageKey === key) return;
      if (Date.now() < pet.retryAt) return;
      pet.imageKey = key;
      const request = ++pet.imageRequest;
      void Promise.resolve(ctx.call("/agent-pet/image", { image: current.image, customImageUrl: current.customImageUrl })).then((result) => {
        if (request !== pet.imageRequest || !pet.image) return;
        if (result?.status !== "ok" || typeof result.image !== "string" || !result.image.startsWith("data:image/")) {
          throw new Error(result?.message || result?.status || "图片不可用");
        }
        pet.image.src = result.image;
        pet.root.style.display = "block";
        pet.retryAt = 0;
        pet.failureLogged = false;
        place(readPosition());
      }).catch((error) => {
        if (request !== pet.imageRequest) return;
        // 下载失败（离线、地址失效）：30 秒后再试，期间不显示空白框。
        pet.imageKey = "";
        pet.retryAt = Date.now() + 30000;
        if (!pet.failureLogged) {
          pet.failureLogged = true;
          diagnostic("agent_pet_image_failed", { message: String(error?.message || error) });
        }
      });
    }

    function sync() {
      const current = settings();
      if (!active(current)) {
        if (pet.root) unmount();
        pet.retryAt = 0;
        return;
      }
      if (!document.body) return;
      const wantedHost = pet.host?.isConnected ? pet.host : null;
      if (!pet.root || !pet.root.isConnected || pet.mountedHost !== wantedHost) {
        if (pet.root) unmount();
        mount(current);
      }
      applySize(current);
      // 挂载后就预解码当前音效组（上下文此时可能是 suspended，按下时再 resume），第一次按压也有声音。
      if (current.soundEnabled) warmSounds(current);
      loadImage(current);
    }

    return {
      setHost(container) {
        pet.host = container || null;
        sync();
      },
      sync,
      dispose() {
        unmount();
        pet.disposed = true;
      },
      get state() {
        return { mounted: Boolean(pet.root), imageKey: pet.imageKey, dragging: Boolean(pet.drag) };
      },
    };
  }
