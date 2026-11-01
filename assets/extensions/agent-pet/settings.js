  // Agent 桌宠拓展的设置界面。Codex 里「拓展」详情页（ctx.registerSettingsPanel，见 main.js）和管理器里的设置页
  // （settings.html，见 settings-page.js）共用这一份；两边各自给一个 host：
  //   { config, save(values) → Promise<生效配置>, call(route, payload) → Promise<结果>, asset(path) → data URL 或 Promise }
  // 图片、音效的下载与导入走 Codex++ 的 /agent-pet/* 路由（清单 permissions.routes 声明过），内嵌音效取包里的 assets/。
  // 这里的名字都带 cppPetSettings 前缀：拓展包的页面脚本拼在同一个作用域里，不能和 pet.js / whale.js 重名。

  const cppPetSettingsSizeMin = 64;
  const cppPetSettingsSizeMax = 240;
  const cppPetSettingsSoundFileMaxBytes = 2 * 1024 * 1024;
  const cppPetSettingsPreviewConcurrency = 4;
  const cppPetSettingsSaveDelayMs = 600;
  /** 随拓展包内嵌、不用下载的音效组：[id, 名称, 按压音, 松开音]。 */
  const cppPetSettingsEmbeddedSounds = [
    ["duck", "小黄鸭", "assets/sounds/Ya1.mp3", "assets/sounds/Ya2.mp3"],
    ["fx1", "音效 1", "assets/sounds/D1.mp3", "assets/sounds/D2.mp3"],
  ];
  const cppPetSettingsIcons = {
    image: '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21"/></svg>',
    fish: '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6.5 12c.94-3.46 4.94-6 8.5-6 3.56 0 6.06 2.54 7 6-.94 3.47-3.44 6-7 6s-7.56-2.53-8.5-6Z"/><path d="M18 12v.5"/><path d="M16 17.93a9.77 9.77 0 0 1 0-11.86"/><path d="M7 10.67C7 8 5.58 5.97 2.73 5.5c-1 1.5-1 5 .23 6.5-1.24 1.5-1.24 5-.23 6.5C5.58 18.03 7 16 7 13.33"/></svg>',
    check: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>',
    volume: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M11 5 6 9H2v6h4l5 4V5Z"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/></svg>',
  };

  /** 拓展配置 → 设置界面用的值（与 codexpp.json 的 config 一致），数值夹在范围内。 */
  function cppPetSettingsNormalize(raw) {
    const value = raw && typeof raw === "object" ? raw : {};
    const num = (input, fallback) => (typeof input === "number" && Number.isFinite(input) ? input : fallback);
    const text = (input) => (typeof input === "string" ? input.trim() : "");
    const protocol = value.codexAppWhaleBalanceProtocol;
    return {
      kind: text(value.kind) === "whaleWidget" ? "whaleWidget" : "image",
      image: text(value.image) || "whale",
      customImageUrl: text(value.customImageUrl),
      size: Math.round(Math.min(cppPetSettingsSizeMax, Math.max(cppPetSettingsSizeMin, num(value.size, 112)))),
      soundEnabled: value.soundEnabled !== false,
      soundSet: text(value.soundSet) || "duck",
      customSoundPress: text(value.customSoundPress),
      customSoundRelease: text(value.customSoundRelease),
      volume: Math.min(1, Math.max(0, num(value.volume, 0.6))),
      codexAppWhaleBalanceProtocol: protocol === "custom" || protocol === "off" ? protocol : "auto",
      codexAppWhaleBalancePath: text(value.codexAppWhaleBalancePath),
      codexAppWhaleBalanceField: text(value.codexAppWhaleBalanceField),
      codexAppWhaleBalanceCurrency: (text(value.codexAppWhaleBalanceCurrency) || "USD").toUpperCase(),
      codexAppWhaleBalanceScale: num(value.codexAppWhaleBalanceScale, 1),
    };
  }

  /** 只接受不带账号信息的 https 地址。 */
  function cppPetSettingsHttpsUrl(value) {
    try {
      const url = new URL(String(value || "").trim());
      return url.protocol === "https:" && !url.username && !url.password && !!url.hostname;
    } catch {
      return false;
    }
  }

  /** 保存前的校验：图片桌宠选了自定义图片时校验地址；大肥鱼选了自定义余额接口时校验接口。返回提示文字，没问题时为空串。 */
  function cppPetSettingsIssue(config) {
    if (config.kind === "whaleWidget") {
      if (config.codexAppWhaleBalanceProtocol !== "custom") return "";
      const path = config.codexAppWhaleBalancePath;
      if (!path || path.length > 1024 || path.startsWith("//") || /[\\?#\s\x00-\x1f]/.test(path) || /:/.test(path)) {
        return "请填写供应商同域下的余额接口路径，不包含网址、查询参数或片段。";
      }
      const field = config.codexAppWhaleBalanceField;
      if (!field || field.length > 256 || !/^[A-Za-z0-9_-]+(?:\[\d+\])*(?:\.[A-Za-z0-9_-]+(?:\[\d+\])*)*$/.test(field)) {
        return "请填写余额字段路径，例如 data.balance 或 data.accounts[0].balance。";
      }
      if (!/^[A-Z]{3}$/.test(config.codexAppWhaleBalanceCurrency)) return "币种请使用三位代码，例如 USD 或 CNY。";
      const scale = config.codexAppWhaleBalanceScale;
      if (!Number.isFinite(scale) || scale <= 0 || scale > 1e12) return "金额倍率必须是大于 0 且不超过 1e12 的有限数值。";
      return "";
    }
    if (config.image === "custom" && !cppPetSettingsHttpsUrl(config.customImageUrl)) return "自定义图片地址无效，请填写 https:// 开头的图片地址。";
    return "";
  }

  /** 一组音效的缓存键，与后端 /agent-pet/sound 回的 key 同一规则：自定义音效带文件名。 */
  function cppPetSettingsSoundKey(config) {
    return config.soundSet === "custom" ? `custom:${config.customSoundPress}:${config.customSoundRelease}` : config.soundSet;
  }

  /** 点按时松开音距现在的延迟（秒）：按压音结束前 lead 毫秒进场，已经过了就立即播放（与 Codex 里的桌宠一致）。 */
  function cppPetSettingsReleaseDelay(pressStartAt, pressDuration, now, leadMs = 40) {
    return Math.max(0, pressStartAt + pressDuration - leadMs / 1000 - now);
  }

  function cppPetSettingsBase64(buffer) {
    const bytes = new Uint8Array(buffer);
    let binary = "";
    // 分块拼接，避免大文件一次展开参数撑爆调用栈。
    for (let index = 0; index < bytes.length; index += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    }
    return btoa(binary);
  }

  function cppPetSettingsErrorText(error) {
    return String(error?.message || error || "未知错误");
  }

  /** 试听播放器：按音效键缓存解码后的音频，按下 / 松开的衔接与 Codex 里的桌宠一致。 */
  function cppPetSettingsPlayer() {
    let context = null;
    const sources = new Map();
    const buffers = new Map();
    let press = null;
    let release = null;
    const audio = () => {
      if (context) return context;
      const AudioContextClass = window.AudioContext || window.webkitAudioContext;
      if (!AudioContextClass) return null;
      try {
        context = new AudioContextClass();
      } catch {
        return null;
      }
      return context;
    };
    const bytes = (uri) => {
      const comma = String(uri || "").indexOf(",");
      if (comma < 0) return null;
      try {
        const binary = atob(uri.slice(comma + 1));
        const array = new Uint8Array(binary.length);
        for (let index = 0; index < binary.length; index += 1) array[index] = binary.charCodeAt(index);
        return array.buffer;
      } catch {
        return null;
      }
    };
    const buffer = (key, slot) => {
      const id = `${key}:${slot}`;
      if (buffers.has(id)) return buffers.get(id);
      const ctx = audio();
      const data = bytes(sources.get(key)?.[slot]);
      const pending = ctx && data ? ctx.decodeAudioData(data).catch(() => null) : Promise.resolve(null);
      buffers.set(id, pending);
      return pending;
    };
    const play = async (key, slot, volume, delay = 0) => {
      const decoded = await buffer(key, slot);
      if (!decoded || !context || volume <= 0) return null;
      if (context.state === "suspended") await context.resume().catch(() => undefined);
      const source = context.createBufferSource();
      const gain = context.createGain();
      gain.gain.value = volume;
      source.buffer = decoded;
      source.connect(gain).connect(context.destination);
      const startAt = context.currentTime + Math.max(0, delay);
      source.start(startAt);
      return { source, startAt, duration: decoded.duration };
    };
    const stop = (playing) => {
      try {
        playing?.source.stop();
      } catch {}
    };
    return {
      set(key, source) {
        sources.set(key, source);
        buffers.delete(`${key}:press`);
        buffers.delete(`${key}:release`);
      },
      has(key) {
        const source = sources.get(key);
        return !!(source && (source.press || source.release));
      },
      async down(key, volume) {
        stop(release);
        release = null;
        press = play(key, "press", volume);
        await press;
      },
      async up(key, volume) {
        const pressed = await press;
        press = null;
        const delay = pressed && context ? cppPetSettingsReleaseDelay(pressed.startAt, pressed.duration, context.currentTime) : 0;
        release = await play(key, "release", volume, delay);
      },
      async preview(key, volume) {
        await this.down(key, volume);
        await this.up(key, volume);
      },
      dispose() {
        void press?.then((playing) => stop(playing));
        stop(release);
        void context?.close?.().catch(() => undefined);
        context = null;
        buffers.clear();
      },
    };
  }

  const cppPetSettingsStyle = `
    .cpps { --cpps-fg: var(--codex-plus-text, #171717); --cpps-muted: var(--codex-plus-text-secondary, #6b7280);
      --cpps-border: var(--codex-plus-border, rgba(0,0,0,.12)); --cpps-hover: var(--codex-plus-bg-hover, rgba(0,0,0,.05));
      --cpps-selected: var(--codex-plus-bg-selected, rgba(0,0,0,.08)); --cpps-surface: var(--codex-plus-bg-secondary, transparent);
      --cpps-accent: var(--codex-plus-success, #10a37f); --cpps-danger: var(--codex-plus-danger, #dc2626);
      display: grid; gap: 12px; color: var(--cpps-fg); font-size: 13px; line-height: 1.5; text-align: left; }
    .cpps * { box-sizing: border-box; }
    .cpps button, .cpps input, .cpps select { font: inherit; color: inherit; }
    .cpps-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
    .cpps-muted { color: var(--cpps-muted); font-size: 12px; }
    .cpps-status { font-size: 12px; color: var(--cpps-muted); white-space: nowrap; }
    .cpps-status[data-tone="ok"] { color: var(--cpps-accent); }
    .cpps-status[data-tone="error"] { color: var(--cpps-danger); white-space: normal; }
    .cpps-error { margin: 6px 0 0; color: var(--cpps-danger); font-size: 12px; }
    .cpps-layout { display: flex; flex-wrap: wrap; gap: 12px; align-items: flex-start; }
    .cpps-main { flex: 1 1 360px; min-width: 0; display: grid; gap: 12px; }
    .cpps-group { border: 1px solid var(--cpps-border); border-radius: 10px; padding: 12px; display: grid; gap: 10px; background: var(--cpps-surface); }
    .cpps-group-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
    .cpps-group-head strong { display: block; font-size: 13px; font-weight: 600; }
    .cpps-group-head small { display: block; color: var(--cpps-muted); font-size: 12px; margin-top: 2px; }
    .cpps-kinds { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 8px; }
    .cpps-kind { display: flex; align-items: flex-start; gap: 10px; padding: 10px; border: 1px solid var(--cpps-border); border-radius: 8px; background: transparent; cursor: pointer; text-align: left; }
    .cpps-kind:hover { background: var(--cpps-hover); }
    .cpps-kind.is-selected { border-color: var(--cpps-accent); background: var(--cpps-selected); }
    .cpps-kind-icon { flex: none; display: inline-flex; color: var(--cpps-muted); }
    .cpps-kind.is-selected .cpps-kind-icon, .cpps-kind-check { color: var(--cpps-accent); }
    .cpps-kind-text { flex: 1; min-width: 0; }
    .cpps-kind-text strong { display: block; font-weight: 600; }
    .cpps-kind-text small { display: block; color: var(--cpps-muted); font-size: 12px; }
    .cpps-gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(76px, 1fr)); gap: 8px; }
    .cpps-tile { display: grid; justify-items: center; gap: 4px; padding: 8px 4px 6px; border: 1px solid var(--cpps-border); border-radius: 8px; background: transparent; cursor: pointer; }
    .cpps-tile:hover { background: var(--cpps-hover); }
    .cpps-tile.is-selected { border-color: var(--cpps-accent); background: var(--cpps-selected); }
    .cpps-tile-image { width: 48px; height: 48px; display: grid; place-items: center; color: var(--cpps-muted); }
    .cpps-tile-image img { max-width: 48px; max-height: 48px; object-fit: contain; }
    .cpps-tile-name { font-size: 11.5px; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cpps-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .cpps-input { flex: 1 1 220px; min-width: 0; height: 30px; padding: 0 10px; border: 1px solid var(--cpps-border); border-radius: 8px; background: transparent; }
    .cpps-input:focus, .cpps-select:focus { outline: 2px solid var(--cpps-accent); outline-offset: -1px; }
    .cpps-select { height: 30px; padding: 0 8px; border: 1px solid var(--cpps-border); border-radius: 8px; background: transparent; min-width: 200px; }
    .cpps-button { height: 30px; padding: 0 12px; display: inline-flex; align-items: center; gap: 6px; border: 1px solid var(--cpps-border); border-radius: 8px; background: transparent; cursor: pointer; white-space: nowrap; }
    .cpps-button:hover:not(:disabled) { background: var(--cpps-hover); }
    .cpps-button:disabled, .cpps-segmented button:disabled { opacity: .5; cursor: default; }
    .cpps-range { display: grid; grid-template-columns: 44px minmax(0, 1fr) 48px; align-items: center; gap: 10px; }
    .cpps-range input { width: 100%; height: 4px; margin: 0; padding: 0; border: 0; border-radius: 999px; outline: none; cursor: pointer;
      -webkit-appearance: none; appearance: none; background: linear-gradient(to right, var(--cpps-accent) var(--cpps-fill, 0%), var(--cpps-border) var(--cpps-fill, 0%)); }
    .cpps-range input:disabled { cursor: default; opacity: .55; }
    .cpps-range input::-webkit-slider-thumb { -webkit-appearance: none; appearance: none; width: 14px; height: 14px; border: 0; border-radius: 999px; background: var(--cpps-accent); box-shadow: 0 1px 3px rgba(0,0,0,.3); }
    .cpps-range input::-moz-range-thumb { width: 14px; height: 14px; border: 0; border-radius: 999px; background: var(--cpps-accent); }
    .cpps-range input:focus-visible { outline: 2px solid var(--cpps-accent); outline-offset: 4px; }
    .cpps-range output { text-align: right; color: var(--cpps-muted); font-variant-numeric: tabular-nums; }
    .cpps-switch-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
    .cpps-switch-row strong { display: block; font-weight: 500; }
    .cpps-switch-row small { display: block; color: var(--cpps-muted); font-size: 12px; }
    .cpps-switch { flex: none; width: 36px; height: 20px; padding: 2px; border: 0; border-radius: 999px; background: var(--cpps-border); cursor: pointer; }
    .cpps-switch span { display: block; width: 16px; height: 16px; border-radius: 999px; background: #fff; box-shadow: 0 1px 2px rgba(0,0,0,.2); transition: transform .12s ease; }
    .cpps-switch[aria-checked="true"] { background: var(--cpps-accent); }
    .cpps-switch[aria-checked="true"] span { transform: translateX(16px); }
    .cpps-sound { display: grid; gap: 10px; }
    .cpps-sound.is-off { opacity: .55; }
    .cpps-segmented { display: inline-flex; flex-wrap: wrap; gap: 4px; padding: 3px; border: 1px solid var(--cpps-border); border-radius: 9px; }
    .cpps-segmented button { height: 26px; padding: 0 10px; border: 0; border-radius: 6px; background: transparent; cursor: pointer; }
    .cpps-segmented button:hover:not(:disabled) { background: var(--cpps-hover); }
    .cpps-segmented button[aria-checked="true"] { background: var(--cpps-selected); color: var(--cpps-accent); font-weight: 600; }
    .cpps-custom-sounds { display: grid; gap: 6px; }
    .cpps-custom-sound { display: grid; grid-template-columns: 56px minmax(0, 1fr) auto auto; align-items: center; gap: 8px; }
    .cpps-field { display: grid; grid-template-columns: 110px minmax(0, 1fr); align-items: center; gap: 10px; }
    .cpps-field-pair { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 10px; }
    .cpps-preview { flex: 0 0 220px; position: sticky; top: 12px; display: grid; justify-items: center; gap: 8px; padding: 12px; border: 1px solid var(--cpps-border); border-radius: 10px; background: var(--cpps-surface); }
    .cpps-preview-label { justify-self: start; font-size: 12px; color: var(--cpps-muted); }
    .cpps-stage { width: 100%; min-height: 200px; display: grid; place-items: center; text-align: center; }
    .cpps-figure { padding: 0; border: 0; background: transparent; cursor: pointer; touch-action: none; transform-origin: 50% 100%; transition: transform .16s cubic-bezier(.34,1.56,.64,1); }
    .cpps-figure.is-pressed { transform: scaleY(.88) scaleX(1.05); transition-duration: .06s; }
    .cpps-figure img { width: 100%; height: 100%; object-fit: contain; pointer-events: none; user-select: none; }
    .cpps-whale { max-width: 160px; max-height: 160px; object-fit: contain; }
    .cpps-spinner { width: 18px; height: 18px; border-radius: 999px; border: 2px solid var(--cpps-border); border-top-color: var(--cpps-accent); animation: cpps-spin .8s linear infinite; }
    @keyframes cpps-spin { to { transform: rotate(360deg); } }
    @media (prefers-reduced-motion: reduce) { .cpps-figure, .cpps-switch span { transition: none; } .cpps-spinner { animation: none; } }
  `;

  /**
   * 把设置界面画进 container，返回清理函数。改动防抖后经 host.save 保存，Codex 里的桌宠用 onConfigUpdate 原地生效。
   * 界面只在离散操作（点选、切换）时整块重画；滑块、输入框只改自己的值，不打断拖动和输入。
   */
  function createCodexPlusAgentPetSettings(container, host) {
    const doc = container.ownerDocument || document;
    let config = cppPetSettingsNormalize(host.config);
    let saved = JSON.stringify(config);
    let disposed = false;
    let saveTimer = 0;
    const state = {
      catalog: null,
      catalogError: "",
      sounds: [],
      whaleImage: "",
      previews: {},
      custom: null,
      customDraft: config.customImageUrl,
      soundStates: {},
      importing: "",
      importError: "",
    };
    const player = cppPetSettingsPlayer();

    const h = (tag, props = {}, ...children) => {
      const node = doc.createElement(tag);
      for (const [name, value] of Object.entries(props)) {
        if (value === undefined || value === null || value === false) continue;
        if (name === "class") node.className = value;
        else if (name === "text") node.textContent = value;
        else if (name === "html") node.innerHTML = value;
        else if (name.startsWith("on") && typeof value === "function") node.addEventListener(name.slice(2), value);
        else if (name === "value") node.value = value;
        else if (name === "checked" || name === "disabled" || name === "hidden") node[name] = !!value;
        else node.setAttribute(name, value === true ? "" : String(value));
      }
      for (const child of children.flat()) if (child !== null && child !== undefined && child !== false) node.append(child);
      return node;
    };
    const spinner = () => h("span", { class: "cpps-spinner", role: "status", "aria-label": "加载中" });

    const call = async (route, payload) => {
      const result = await host.call(route, payload);
      if (!result || result.status === "failed") throw new Error(result?.message || `${route} 调用失败`);
      if (result.status === "disabled") throw new Error("桌宠拓展没有启用，图片和音效暂时取不到");
      return result;
    };
    const asset = (path) => Promise.resolve(host.asset(path)).catch(() => "");

    // ------------------------------------------------------------ 保存
    const status = h("span", { class: "cpps-status", role: "status", "aria-live": "polite" });
    const setStatus = (tone, text) => {
      if (tone) status.setAttribute("data-tone", tone);
      else status.removeAttribute("data-tone");
      status.textContent = text;
    };
    const flush = async () => {
      saveTimer = 0;
      const snapshot = JSON.stringify(config);
      if (snapshot === saved || cppPetSettingsIssue(config)) return;
      if (!disposed) setStatus("", "正在保存…");
      try {
        await host.save({ ...config });
        saved = snapshot;
        if (!disposed) setStatus("ok", "已保存");
      } catch (error) {
        if (!disposed) setStatus("error", `没有保存：${cppPetSettingsErrorText(error)}`);
      }
    };
    const schedule = () => {
      clearTimeout(saveTimer);
      saveTimer = 0;
      const issue = cppPetSettingsIssue(config);
      if (issue) return setStatus("error", issue);
      if (JSON.stringify(config) === saved) return setStatus("", "");
      setStatus("", "待保存…");
      saveTimer = setTimeout(() => void flush(), cppPetSettingsSaveDelayMs);
    };
    const update = (patch, options = {}) => {
      config = { ...config, ...patch };
      if (options.render !== false) render();
      schedule();
    };

    // ------------------------------------------------------------ 音效
    const loadSound = async (next) => {
      const key = cppPetSettingsSoundKey(next);
      if (player.has(key)) return true;
      const embedded = cppPetSettingsEmbeddedSounds.find(([id]) => id === next.soundSet);
      if (embedded) {
        player.set(key, { press: await asset(embedded[2]), release: await asset(embedded[3]) });
        return player.has(key);
      }
      state.soundStates[next.soundSet] = { state: "loading" };
      render();
      try {
        const result = await call("/agent-pet/sound", {
          soundSet: next.soundSet,
          customSoundPress: next.customSoundPress,
          customSoundRelease: next.customSoundRelease,
        });
        player.set(result.key || key, { press: result.press, release: result.release });
        state.soundStates[next.soundSet] = { state: "ok" };
        return true;
      } catch (error) {
        state.soundStates[next.soundSet] = { state: "failed", message: cppPetSettingsErrorText(error) };
        return false;
      } finally {
        render();
      }
    };
    // 下载组只有取到了才切过去，免得 Codex 里的桌宠没声音。
    const selectSound = async (soundSet) => {
      const next = { ...config, soundSet };
      if (!(await loadSound(next))) return;
      update({ soundSet });
      void player.preview(cppPetSettingsSoundKey(next), config.volume);
    };
    const importSound = async (slot, file) => {
      if (!file) return;
      state.importError = "";
      if (file.size > cppPetSettingsSoundFileMaxBytes) {
        state.importError = "音频文件超过 2 MB，请换一个短一点的文件。";
        return render();
      }
      state.importing = slot;
      render();
      try {
        const result = await call("/agent-pet/import-sound", { slot, data: cppPetSettingsBase64(await file.arrayBuffer()) });
        const next = { ...config, soundSet: "custom", [slot === "press" ? "customSoundPress" : "customSoundRelease"]: result.file };
        if (await loadSound(next)) {
          update(next);
          void player.preview(cppPetSettingsSoundKey(next), config.volume);
        }
      } catch (error) {
        state.importError = cppPetSettingsErrorText(error);
      } finally {
        state.importing = "";
        render();
      }
    };
    const clearCustomSound = (slot) => {
      const next = { ...config, [slot === "press" ? "customSoundPress" : "customSoundRelease"]: "" };
      // 两个都清空了就回到默认音效组。
      if (!next.customSoundPress && !next.customSoundRelease && next.soundSet === "custom") next.soundSet = "duck";
      update(next);
      void loadSound(next);
    };

    // ------------------------------------------------------------ 图片
    const tileContent = (preview) => {
      if (preview?.state === "ok") return h("img", { src: preview.image, alt: "", draggable: "false" });
      if (preview?.state === "failed") return h("span", { text: "×", title: preview.message });
      return spinner();
    };
    const updateTile = (id) => {
      const tile = [...root.querySelectorAll("[data-cpps-tile]")].find((node) => node.getAttribute("data-cpps-tile") === id);
      tile?.querySelector(".cpps-tile-image")?.replaceChildren(tileContent(state.previews[id]));
    };
    const syncRetry = () => {
      const button = root.querySelector("[data-cpps-retry]");
      if (button) button.hidden = !Object.values(state.previews).some((item) => item.state === "failed");
    };
    const fetchPreviews = async (ids) => {
      const queue = [...ids];
      for (const id of ids) {
        state.previews[id] = { state: "loading" };
        updateTile(id);
      }
      syncRetry();
      const worker = async () => {
        for (let id = queue.shift(); id; id = queue.shift()) {
          if (disposed) return;
          try {
            const result = await call("/agent-pet/image", { image: id, customImageUrl: "" });
            state.previews[id] = { state: "ok", image: result.image };
          } catch (error) {
            state.previews[id] = { state: "failed", message: cppPetSettingsErrorText(error) };
          }
          if (disposed) return;
          updateTile(id);
          if (config.image === id) renderPreview();
        }
      };
      await Promise.all(Array.from({ length: cppPetSettingsPreviewConcurrency }, worker));
      if (!disposed) syncRetry();
    };
    const loadCustomPreview = async (url) => {
      state.custom = { state: "loading" };
      renderPreview();
      try {
        const result = await call("/agent-pet/image", { image: "custom", customImageUrl: url });
        state.custom = { state: "ok", image: result.image };
      } catch (error) {
        state.custom = { state: "failed", message: cppPetSettingsErrorText(error) };
      }
      return state.custom.state === "ok";
    };
    // 只有图片取得到才切过去，免得 Codex 里的桌宠变成空白。
    const applyCustomUrl = async () => {
      const url = state.customDraft.trim();
      if (!cppPetSettingsHttpsUrl(url)) {
        state.custom = { state: "failed", message: "请填写 https:// 开头的图片地址" };
        return render();
      }
      render();
      if (await loadCustomPreview(url)) update({ image: "custom", customImageUrl: url });
      else render();
    };

    // ------------------------------------------------------------ 界面
    const group = (title, detail, ...body) => h("section", { class: "cpps-group" },
      h("div", { class: "cpps-group-head" }, h("div", {}, h("strong", { text: title }), detail ? h("small", { text: detail }) : null)),
      ...body);
    const switchButton = (checked, label, onToggle) => h("button", {
      type: "button", class: "cpps-switch", role: "switch", "aria-checked": String(checked), "aria-label": label,
      onclick: () => onToggle(!checked),
    }, h("span"));

    const kindGroup = () => group("桌宠类型", "二选一，同一时间只显示一个。", h("div", { class: "cpps-kinds", role: "radiogroup", "aria-label": "桌宠类型" },
      [
        ["image", cppPetSettingsIcons.image, "简单桌宠", "从图库挑一张图或用自己的图片，点一下压扁回弹，带按压音效。"],
        ["whaleWidget", cppPetSettingsIcons.fish, "大肥鱼（DeepSeek 鲸鱼）", "原版鲸鱼用量挂件：余额、价格与用量统计、泡泡编辑器、音效库和提醒。"],
      ].map(([kind, icon, title, detail]) => h("button", {
        type: "button", role: "radio", "aria-checked": String(config.kind === kind),
        class: `cpps-kind${config.kind === kind ? " is-selected" : ""}`,
        onclick: () => update({ kind }),
      },
      h("span", { class: "cpps-kind-icon", html: icon }),
      h("span", { class: "cpps-kind-text" }, h("strong", { text: title }), h("small", { text: detail })),
      config.kind === kind ? h("span", { class: "cpps-kind-check", html: cppPetSettingsIcons.check }) : null)),
    ));

    const whaleGroup = () => {
      const field = (label, key, placeholder, type = "text") => h("label", { class: "cpps-field" }, h("span", { text: label }), h("input", {
        class: "cpps-input", type, value: String(config[key] ?? ""), placeholder, "data-cpps-focus": key,
        step: type === "number" ? "any" : null,
        oninput: (event) => update({ [key]: type === "number" ? event.target.valueAsNumber : event.target.value.trim() }, { render: false }),
      }));
      const custom = config.codexAppWhaleBalanceProtocol === "custom";
      return group("API 余额（可选）", "跟随 Codex++ 当前供应商查询余额；Codex 订阅额度和本地 token 无需配置此项。",
        h("label", { class: "cpps-field" }, h("span", { text: "余额查询方式" }), h("select", {
          class: "cpps-select", "aria-label": "余额查询方式",
          onchange: (event) => update({ codexAppWhaleBalanceProtocol: event.target.value }),
        }, [["auto", "自动识别已支持的接口"], ["off", "仅显示 Codex 用量"], ["custom", "自定义供应商余额接口"]].map(([value, label]) => {
          const option = h("option", { value, text: label });
          option.selected = config.codexAppWhaleBalanceProtocol === value;
          return option;
        }))),
        custom ? [
          h("small", { class: "cpps-muted", text: "按供应商文档填写 GET 接口和返回字段，复用当前供应商的 API Key。币种和倍率应与供应商账单一致。" }),
          field("余额接口路径", "codexAppWhaleBalancePath", "/api/balance"),
          field("余额字段路径", "codexAppWhaleBalanceField", "data.balance"),
          h("div", { class: "cpps-field-pair" }, field("币种", "codexAppWhaleBalanceCurrency", "USD"), field("金额倍率", "codexAppWhaleBalanceScale", "1", "number")),
          h("small", { class: "cpps-muted", text: "显示金额 = 接口数值 × 倍率；例如接口返回分时，倍率填 0.01。" }),
        ] : null,
        h("small", { class: "cpps-muted", text: "鲸鱼角色、完整界面引擎及内置音效 / 动图来自 MeteorNOX/DeepSeek-Balance-Whale-Widget（代码 MIT；素材按上游声明原样提供，不重新授权，也不声明为本项目原创）。" }),
      );
    };

    const imageGroup = () => {
      const failed = Object.values(state.previews).some((item) => item.state === "failed");
      const retry = h("button", {
        type: "button", class: "cpps-button", "data-cpps-retry": "", hidden: !failed,
        onclick: () => void fetchPreviews(Object.keys(state.previews).filter((id) => state.previews[id].state === "failed")),
      }, "重试下载");
      const head = h("div", { class: "cpps-group-head" },
        h("div", {}, h("strong", { text: "形象" }), h("small", { text: "图片在用到时联网下载并缓存在本机，不随 Codex++ 打包。内置图库来自 Microsoft Fluent Emoji（MIT），二次元形象来自 Wikimedia Commons（CC0）。" })),
        retry);
      const gallery = state.catalog
        ? h("div", { class: "cpps-gallery", role: "radiogroup", "aria-label": "形象" }, state.catalog.map((entry) => {
          const selected = config.image === entry.id;
          const preview = state.previews[entry.id];
          return h("button", {
            type: "button", role: "radio", "aria-checked": String(selected), "data-cpps-tile": entry.id,
            class: `cpps-tile${selected ? " is-selected" : ""}`,
            title: entry.credit ? `${entry.name} · ${entry.credit}` : entry.name,
            onclick: () => update({ image: entry.id }),
          }, h("span", { class: "cpps-tile-image" }, tileContent(preview)), h("span", { class: "cpps-tile-name", text: entry.name }));
        }))
        : state.catalogError ? null : h("p", { class: "cpps-muted" }, spinner(), " 正在加载图库…");
      const loading = state.custom?.state === "loading";
      return h("section", { class: "cpps-group" }, head,
        state.catalogError ? h("p", { class: "cpps-error", role: "alert", text: state.catalogError }) : null,
        gallery,
        h("div", { class: "cpps-row" },
          h("input", {
            class: "cpps-input", type: "url", "aria-label": "自定义图片地址", "data-cpps-focus": "customImageUrl",
            placeholder: "自定义图片：https://…/pet.png（PNG / JPEG / WebP / GIF，4 MB 以内）", value: state.customDraft,
            oninput: (event) => { state.customDraft = event.target.value; },
            onkeydown: (event) => { if (event.key === "Enter") void applyCustomUrl(); },
          }),
          h("button", { type: "button", class: "cpps-button", disabled: loading, onclick: () => void applyCustomUrl() }, loading ? spinner() : null, "使用这张"),
        ),
        state.custom?.state === "failed" ? h("p", { class: "cpps-error", role: "alert", text: state.custom.message }) : null,
      );
    };

    const soundGroup = () => {
      const on = config.soundEnabled;
      const hasCustom = !!(config.customSoundPress || config.customSoundRelease);
      const options = [
        ...cppPetSettingsEmbeddedSounds.map(([id, name]) => [id, name]),
        ...state.sounds.map((entry) => [entry.id, entry.name]),
      ];
      const selectedEntry = state.sounds.find((entry) => entry.id === config.soundSet);
      const soundState = state.soundStates[config.soundSet];
      const range = (label, min, max, step, value, format, onInput, disabled = false) => {
        const output = h("output", { text: format(value) });
        // 轨道的已填充部分：Codex 会把滑块的原生外观重置掉，这里自己画。
        const fill = (input, next) => input.style.setProperty("--cpps-fill", `${((next - min) / (max - min)) * 100}%`);
        const input = h("input", {
          type: "range", min, max, step, value: String(value), disabled,
          oninput: (event) => {
            const next = event.target.valueAsNumber;
            output.textContent = format(next);
            fill(event.target, next);
            onInput(next);
          },
        });
        fill(input, value);
        return h("label", { class: "cpps-range" }, h("span", { text: label }), input, output);
      };
      return group("大小与音效", "按下时播放按压音，松开后接上回弹音。",
        range("大小", cppPetSettingsSizeMin, cppPetSettingsSizeMax, 4, config.size, (value) => `${value}px`, (size) => {
          update({ size }, { render: false });
          renderPreview();
        }),
        h("div", { class: "cpps-switch-row" },
          h("span", {}, h("strong", { text: "按压音效" }), h("small", { text: "关闭后只保留压扁回弹的动画。" })),
          switchButton(on, "按压音效", (soundEnabled) => update({ soundEnabled }))),
        h("div", { class: `cpps-sound${on ? "" : " is-off"}` },
          h("div", { class: "cpps-row" },
            h("div", { class: "cpps-segmented", role: "radiogroup", "aria-label": "音效" },
              options.map(([id, name]) => {
                const item = state.soundStates[id];
                return h("button", {
                  type: "button", role: "radio", "aria-checked": String(config.soundSet === id),
                  disabled: !on || item?.state === "loading", title: item?.state === "failed" ? item.message : null,
                  onclick: () => void selectSound(id),
                }, item?.state === "loading" ? spinner() : null, name);
              }),
              h("button", {
                type: "button", role: "radio", "aria-checked": String(config.soundSet === "custom"),
                disabled: !on || !hasCustom, title: hasCustom ? null : "先在下面导入音频文件",
                onclick: () => void selectSound("custom"),
              }, "自定义")),
            h("button", {
              type: "button", class: "cpps-button", disabled: !on || config.volume <= 0,
              onclick: () => void player.preview(cppPetSettingsSoundKey(config), config.volume),
            }, h("span", { html: cppPetSettingsIcons.volume, style: "display:inline-flex" }), "试听")),
          selectedEntry ? h("small", { class: "cpps-muted", title: selectedEntry.sourceUrl, text: `用到时下载并缓存在本机。来源：${selectedEntry.author}（${selectedEntry.license}）` }) : null,
          soundState?.state === "failed" ? h("p", { class: "cpps-error", role: "alert", text: soundState.message }) : null,
          h("div", { class: "cpps-custom-sounds", "aria-label": "自定义音效" },
            [["press", "按压音", config.customSoundPress], ["release", "松开音", config.customSoundRelease]].map(([slot, label, file]) => {
              const input = h("input", {
                type: "file", hidden: true, accept: "audio/mpeg,audio/ogg,audio/wav,audio/mp4,.mp3,.ogg,.wav,.m4a",
                onchange: (event) => {
                  const target = event.target;
                  void importSound(slot, target.files?.[0]).finally(() => { target.value = ""; });
                },
              });
              return h("div", { class: "cpps-custom-sound" },
                h("span", { text: label }),
                h("span", { class: "cpps-muted", text: file ? "已导入" : "未设置" }),
                input,
                h("button", { type: "button", class: "cpps-button", disabled: !on || !!state.importing, onclick: () => input.click() },
                  state.importing === slot ? spinner() : null, file ? "更换" : "选择文件"),
                file ? h("button", { type: "button", class: "cpps-button", "aria-label": `清除${label}`, disabled: !on, onclick: () => clearCustomSound(slot) }, "×") : h("span"),
              );
            }),
            h("small", { class: "cpps-muted", text: "导入本地 MP3 / OGG / WAV / M4A（2 MB 以内），复制到 Codex++ 状态目录，可只设其中一个。" }),
            state.importError ? h("p", { class: "cpps-error", role: "alert", text: state.importError }) : null),
          range("音量", 0, 100, 5, Math.round(config.volume * 100), (value) => `${value}%`, (value) => update({ volume: value / 100 }, { render: false }), !on),
        ),
      );
    };

    const preview = h("aside", { class: "cpps-preview", "aria-label": "预览" });
    const figure = (image) => {
      let pressed = false;
      const button = h("button", { type: "button", class: "cpps-figure", "aria-label": "按住预览压扁效果", style: `width:${config.size}px;height:${config.size}px` },
        h("img", { src: image, alt: "", draggable: "false" }));
      const release = () => {
        if (!pressed) return;
        pressed = false;
        button.classList.remove("is-pressed");
        if (config.soundEnabled) void player.up(cppPetSettingsSoundKey(config), config.volume);
      };
      button.addEventListener("pointerdown", (event) => {
        try { button.setPointerCapture(event.pointerId); } catch {}
        pressed = true;
        button.classList.add("is-pressed");
        if (config.soundEnabled) void player.down(cppPetSettingsSoundKey(config), config.volume);
      });
      for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) button.addEventListener(name, release);
      return button;
    };
    const renderPreview = () => {
      if (disposed) return;
      const whale = config.kind === "whaleWidget";
      const selected = config.image === "custom" ? state.custom : state.previews[config.image];
      let stage;
      if (whale) stage = state.whaleImage ? h("img", { class: "cpps-whale", src: state.whaleImage, alt: "大肥鱼（DeepSeek 鲸鱼）", draggable: "false" }) : state.catalog || state.catalogError ? h("span", { class: "cpps-muted", text: "大肥鱼预览暂不可用" }) : spinner();
      else if (selected?.state === "ok") stage = figure(selected.image);
      else if (selected?.state === "failed") stage = h("span", { class: "cpps-muted", text: selected.message });
      else stage = spinner();
      preview.replaceChildren(
        h("span", { class: "cpps-preview-label", text: whale ? "预览" : "预览 · 按住试试" }),
        h("div", { class: "cpps-stage" }, stage),
        h("small", { class: "cpps-muted", text: whale ? "在 Codex 里可以拖动、缩放它，点角色旁的菜单配置泡泡、音效、提醒和用量统计。" : "在 Codex 里可以拖动它换位置，位置会记住。" }),
      );
    };

    const root = h("div", { class: "cpps" });
    const style = h("style", { text: cppPetSettingsStyle });
    const toolbar = h("div", { class: "cpps-toolbar" }, h("span", { class: "cpps-muted", text: "改动自动保存，几秒内同步到 Codex，不用重启。" }), status);
    function render() {
      if (disposed) return;
      // 重画前记下焦点所在的输入框，画完放回去（例如正在填余额接口时预览图到了）。
      const focusKey = doc.activeElement?.getAttribute?.("data-cpps-focus");
      const main = h("div", { class: "cpps-main" }, kindGroup(), config.kind === "whaleWidget" ? whaleGroup() : [imageGroup(), soundGroup()]);
      root.replaceChildren(style, toolbar, h("div", { class: "cpps-layout" }, main, preview));
      renderPreview();
      if (focusKey) root.querySelector(`[data-cpps-focus="${focusKey}"]`)?.focus({ preventScroll: true });
    }

    container.appendChild(root);
    render();
    const issue = cppPetSettingsIssue(config);
    if (issue) setStatus("error", issue);
    call("/agent-pet/catalog", {}).then((result) => {
      if (disposed) return;
      state.catalog = Array.isArray(result.images) ? result.images : [];
      state.sounds = Array.isArray(result.sounds) ? result.sounds : [];
      state.whaleImage = String(result.whaleImage || "");
      render();
      void fetchPreviews(state.catalog.map((entry) => entry.id));
    }).catch((error) => {
      if (disposed) return;
      state.catalogError = cppPetSettingsErrorText(error);
      render();
    });
    // 已保存的是下载组 / 自定义组或自定义图片：打开时取一次，预览按压也有声音、有图。
    void loadSound(config);
    if (config.image === "custom" && cppPetSettingsHttpsUrl(config.customImageUrl)) void loadCustomPreview(config.customImageUrl).then(renderPreview);

    return () => {
      // 关掉面板时还有没存的改动就立刻存掉。
      if (saveTimer) {
        clearTimeout(saveTimer);
        void flush();
      }
      disposed = true;
      player.dispose();
      root.remove();
    };
  }
