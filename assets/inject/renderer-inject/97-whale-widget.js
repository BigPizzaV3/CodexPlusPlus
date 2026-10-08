  // 内置小鲸鱼：只通过 launcher 读取脱敏数据，角色和个人显示偏好保存在当前窗口的本地存储。
  window.__codexPlusWhaleWidgetRuntime?.dispose?.();
  const codexPlusWhaleStorageKey = "codexPlus.whaleWidget.v1";
  const codexPlusWhaleMaxImageBytes = 1024 * 1024;
  const codexPlusWhaleState = {
    disposed: false, mounted: false, root: null, elements: {}, timer: null, generation: 0,
    sessionId: "", profileId: "", balance: null, session: null, balanceDue: 0, sessionDue: 0,
    balancePending: null, sessionPending: null, previousTurn: null, drag: null, suppressClick: false,
    listeners: [], audio: null, bubbleOpen: false, settingsOpen: false, message: "", storageError: "",
    prefs: null, imageRevision: 0, settingsCurrencies: "", pendingCancels: new Set(),
    dispatcher: null, subscriptions: [], liveTurn: null, completedTurns: new Set(),
  };

  function codexPlusWhaleNumber(value) {
    if (value === null || value === undefined || value === "") return null;
    const number = Number(value);
    return Number.isFinite(number) && number >= 0 ? number : null;
  }

  function codexPlusWhaleImageSource(value) {
    return typeof value === "string" && value.length <= 1400000
      && /^data:image\/(?:png|jpeg|webp|gif);base64,[A-Za-z0-9+/]+={0,2}$/.test(value) ? value : "";
  }

  function codexPlusWhaleNormalizePrefs(value) {
    const raw = value && typeof value === "object" && !Array.isArray(value) ? value : {};
    const amount = (value) => {
      const number = codexPlusWhaleNumber(value);
      return number !== null && number > 0 && number <= 1000000000 ? number : null;
    };
    const thresholds = {};
    for (const [currency, limits] of Object.entries(raw.thresholds || {}).slice(0, 20)) {
      if (!/^[A-Z]{3,8}$/.test(currency) || !limits || typeof limits !== "object") continue;
      thresholds[currency] = { low: amount(limits.low), budget: amount(limits.budget) };
    }
    const notices = {};
    for (const [key, date] of Object.entries(raw.notices || {}).slice(-120)) {
      if (key.length <= 240 && /^\d{4}-\d{2}-\d{2}$/.test(String(date))) notices[key] = date;
    }
    // 旧版占位鲸鱼的默认 88px 随原版角色迁移；用户上传的角色尺寸保持。
    const defaultSize = Math.max(122, Math.min(250, Math.min(window.innerWidth || 1024, window.innerHeight || 768) * 0.28)) * 0.5945;
    const savedSize = raw.version !== 2 && !raw.image && raw.size === 88 ? defaultSize : Number(raw.size);
    return {
      version: 2, image: codexPlusWhaleImageSource(raw.image), size: Math.max(72.529, Math.min(371.5625, savedSize || defaultSize)),
      x: codexPlusWhaleNumber(raw.x), y: codexPlusWhaleNumber(raw.y), snap: raw.snap !== false,
      sound: raw.sound === true, completion: raw.completion !== false,
      phrase: typeof raw.phrase === "string" ? raw.phrase.slice(0, 120) : "慢慢来，我陪你一起完成。",
      thresholds, notices,
    };
  }

  function codexPlusWhaleLoadPrefs() {
    try { return codexPlusWhaleNormalizePrefs(JSON.parse(localStorage.getItem(codexPlusWhaleStorageKey) || "{}")); }
    catch { return codexPlusWhaleNormalizePrefs({}); }
  }
  codexPlusWhaleState.prefs = codexPlusWhaleLoadPrefs();

  function codexPlusWhaleSavePrefs() {
    const state = codexPlusWhaleState;
    try {
      localStorage.setItem(codexPlusWhaleStorageKey, JSON.stringify(state.prefs));
      state.storageError = "";
      codexPlusWhaleRender();
      return true;
    } catch {
      state.storageError = "本地存储已满或不可用，修改只在本次有效；可恢复默认角色后重试。";
      codexPlusWhaleRender();
      return false;
    }
  }

  function codexPlusWhaleDay() {
    const date = new Date();
    return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
  }

  function codexPlusWhaleAmount(value) {
    const number = codexPlusWhaleNumber(value);
    return number === null ? "—" : number.toLocaleString(undefined, { maximumFractionDigits: 4 });
  }

  function codexPlusWhaleEnabled() {
    return !codexPlusWhaleState.disposed && codexPlusBackendSettingsLoaded && codexPlusSettings().whaleWidget === true;
  }

  function codexPlusWhaleListen(target, name, callback, options) {
    target.addEventListener(name, callback, options);
    codexPlusWhaleState.listeners.push(() => target.removeEventListener(name, callback, options));
  }

  function codexPlusWhaleSay(message, sound = false) {
    const state = codexPlusWhaleState;
    state.message = String(message || "").slice(0, 300);
    state.bubbleOpen = true;
    codexPlusWhaleRender();
    if (sound) codexPlusWhalePlaySound();
  }

  function codexPlusWhalePlaySound() {
    const state = codexPlusWhaleState;
    if (!state.prefs.sound || document.hidden || !state.mounted) return;
    // 只有用户在设置里打开声音后才创建音频上下文；系统不允许自动播放时保持静音。
    const audio = state.audio;
    if (!audio || audio.state !== "running") return;
    try {
      const oscillator = audio.createOscillator();
      const gain = audio.createGain();
      oscillator.type = "sine";
      oscillator.frequency.setValueAtTime(660, audio.currentTime);
      oscillator.frequency.exponentialRampToValueAtTime(880, audio.currentTime + 0.16);
      gain.gain.setValueAtTime(0.035, audio.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.001, audio.currentTime + 0.3);
      oscillator.connect(gain); gain.connect(audio.destination);
      oscillator.onended = () => { oscillator.disconnect(); gain.disconnect(); };
      oscillator.start(); oscillator.stop(audio.currentTime + 0.3);
    } catch {}
  }

  function codexPlusWhaleCheckAlerts(balance) {
    const state = codexPlusWhaleState;
    if (balance?.status !== "ok" || balance.stale || !balance.provider?.id) return;
    const day = codexPlusWhaleDay();
    const messages = [];
    for (const item of Array.isArray(balance.balances) ? balance.balances : []) {
      const limits = state.prefs.thresholds[item.currency];
      if (!limits) continue;
      for (const [kind, threshold, actual, reached, label] of [
        ["low", limits.low, codexPlusWhaleNumber(item.total), (a, b) => a <= b, "账户余额低于提醒阈值"],
        ["budget", limits.budget, codexPlusWhaleNumber(item.observedToday), (a, b) => a >= b, "账户今日观测消费已达到预算"],
      ]) {
        const key = `${balance.provider.accountId || balance.provider.id}|${item.currency}|${kind}`;
        if (threshold === null || threshold === undefined || actual === null || !reached(actual, threshold) || state.prefs.notices[key] === day) continue;
        state.prefs.notices[key] = day;
        messages.push(`${label}：${codexPlusWhaleAmount(actual)} ${item.currency}`);
      }
    }
    if (messages.length) {
      // 每个账户、币种和提醒类型每天最多提示一次，不因重复轮询持续发声。
      state.prefs.notices = Object.fromEntries(Object.entries(state.prefs.notices).filter(([, date]) => date === day).slice(-120));
      codexPlusWhaleSavePrefs();
      codexPlusWhaleSay(messages.join("；"), true);
    }
  }

  function codexPlusWhaleObserveTurn(session) {
    const state = codexPlusWhaleState;
    const turn = session?.status === "ok" ? session.lastTurn : null;
    const previous = state.previousTurn;
    if (turn?.id && previous?.id === turn.id && previous.status === "running" && turn.status === "completed" && !state.completedTurns.has(turn.id) && state.prefs.completion) {
      codexPlusWhaleSay(`这轮任务完成了。${state.prefs.phrase}`, true);
    }
    if (turn?.id && turn.status === "completed") {
      state.completedTurns.add(turn.id);
      if (state.completedTurns.size > 64) state.completedTurns.delete(state.completedTurns.values().next().value);
    }
    state.previousTurn = turn?.id ? { id: turn.id, status: turn.status } : null;
  }

  function codexPlusWhaleUnsubscribe() {
    const state = codexPlusWhaleState;
    for (const unsubscribe of state.subscriptions.splice(0)) { try { unsubscribe?.(); } catch {} }
    state.dispatcher = null;
  }

  function codexPlusWhaleSubscribe() {
    const state = codexPlusWhaleState;
    const dispatcher = window.__codexPlusRemoteSessionRecoveryDispatcher;
    if (!dispatcher || typeof dispatcher.subscribe !== "function" || state.dispatcher === dispatcher) return;
    codexPlusWhaleUnsubscribe();
    const observe = (payload, fallback) => {
      if (!state.mounted || !codexPlusWhaleEnabled() || document.hidden) return;
      const params = payload?.params || payload || {}, turn = params.turn || params;
      const threadId = String(params.threadId || params.thread_id || params.conversationId || "").replace(/^local:/, "");
      const currentId = String(currentSessionRef()?.session_id || "").replace(/^local:/, "");
      const id = String(turn.id || turn.turnId || params.turnId || "");
      if (!threadId || threadId !== currentId || !id) return;
      codexPlusWhaleContext();
      const status = fallback === "running" ? "running" : ({ failed: "failed", interrupted: "aborted", aborted: "aborted" })[turn.status] || "completed";
      state.liveTurn = { id, status, observedAt: Date.now() };
      codexPlusWhaleObserveTurn({ status: "ok", lastTurn: state.liveTurn });
      codexPlusWhaleRender();
    };
    try {
      for (const [method, status] of [["turn/started", "running"], ["turn/completed", "completed"]]) {
        const unsubscribe = dispatcher.subscribe(method, (payload) => observe(payload, status));
        if (typeof unsubscribe === "function") state.subscriptions.push(unsubscribe);
      }
      state.dispatcher = dispatcher;
    } catch { codexPlusWhaleUnsubscribe(); }
  }

  function codexPlusWhalePosition(save = false) {
    const state = codexPlusWhaleState;
    if (!state.root) return;
    const width = Math.max(64, window.innerWidth || 1024), height = Math.max(64, window.innerHeight || 768);
    const size = Math.min(state.prefs.size, (width - 16) * 0.5945, (height - 16) * 0.5945);
    const x = Math.max(8, Math.min(width - size - 8, state.prefs.x ?? width - size - 8));
    const y = Math.max(8, Math.min(height - size - 8, state.prefs.y ?? height - size - 8));
    Object.assign(state.root.style, { left: `${x}px`, top: `${y}px`, width: `${size}px`, height: `${size}px` });
    state.root.dataset.side = x + size / 2 < width / 2 ? "left" : "right";
    const below = height - y - size;
    state.root.dataset.vertical = below > y ? "below" : "above";
    state.root.style.setProperty("--whale-room", `${Math.max(40, Math.max(y, below) - 20)}px`);
    if (state.elements.bubble) {
      const panelWidth = Math.min(330, width - 32);
      const preferredLeft = state.root.dataset.side === "left" ? x : x + size - panelWidth;
      const panelLeft = Math.max(8, Math.min(width - panelWidth - 8, preferredLeft));
      Object.assign(state.elements.bubble.style, { left: `${panelLeft - x}px`, right: "auto" });
    }
    if (state.elements.pop) {
      // 与原版的 59.45% 角色/1026×700 气泡比例一致，靠左时气泡和角色一起镜像。
      const base = size / 0.5945;
      const preferredLeft = state.root.dataset.side === "left" ? x : x + size - base;
      const left = Math.max(8, Math.min(width - base - 8, preferredLeft));
      const top = Math.max(8, y + size - base);
      Object.assign(state.elements.pop.style, { width: `${base}px`, height: `${base * 700 / 1026}px`, left: `${left - x}px`, top: `${top - y}px` });
      state.root.style.setProperty("--whale-unit", `${base / 1026}px`);
    }
    if (save) { state.prefs.x = x; state.prefs.y = y; codexPlusWhaleSavePrefs(); }
  }

  function codexPlusWhaleElement(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  }

  function codexPlusWhaleRender() {
    const state = codexPlusWhaleState, elements = state.elements;
    if (!state.root) return;
    elements.bubble.hidden = !state.settingsOpen;
    elements.pop.hidden = !state.bubbleOpen || state.settingsOpen;
    elements.pop.dataset.open = String(state.bubbleOpen && !state.settingsOpen);
    elements.settings.hidden = !state.settingsOpen;
    elements.pet.setAttribute("aria-expanded", String(state.bubbleOpen));
    elements.message.textContent = state.message || state.prefs.phrase;
    elements.notice.textContent = state.storageError;
    elements.notice.hidden = !state.storageError;
    const balance = state.balance;
    const balanceLines = [];
    if (balance?.status === "ok" || (balance?.stale === true && Array.isArray(balance.balances) && balance.balances.length)) {
      balanceLines.push(`账户：${balance.provider?.name || "当前服务商"}${balance.stale ? "（缓存，刷新失败）" : ""}`);
      for (const item of Array.isArray(balance.balances) ? balance.balances : []) {
        balanceLines.push(`余额 ${codexPlusWhaleAmount(item.total)} ${item.currency || ""} · 今日观测消费 ${codexPlusWhaleAmount(item.observedToday)} ${item.currency || ""}`);
      }
      if (!balance.balances?.length) balanceLines.push("暂无可用余额数据");
      if (balance.stale && balance.message) balanceLines.push(String(balance.message).slice(0, 160));
    } else {
      balanceLines.push(({ unsupported: "当前供应商未提供余额数据", disabled: "余额查询未启用", unavailable: "余额暂不可用" })[balance?.status] || "正在读取余额…");
      if (balance?.message && balance.status !== "unsupported") balanceLines.push(String(balance.message).slice(0, 160));
    }
    elements.balance.textContent = balanceLines.join("\n");
    const session = state.session;
    const currentTurn = state.liveTurn || session?.lastTurn;
    const status = { running: "运行中", completed: "已完成", failed: "失败", aborted: "已停止", unknown: "未知" };
    elements.session.textContent = !state.sessionId ? "打开一个会话即可查看用量。" : session?.status === "ok"
      ? `当前会话${session.model ? ` · ${session.model}` : ""}\n今日 ${codexPlusWhaleAmount(session.today?.totalTokens)} tokens · 累计 ${codexPlusWhaleAmount(session.total?.totalTokens)} tokens\n输入 ${codexPlusWhaleAmount(session.total?.inputTokens)} · 缓存 ${codexPlusWhaleAmount(session.total?.cachedInputTokens)} · 输出 ${codexPlusWhaleAmount(session.total?.outputTokens)}\n任务：${status[currentTurn?.status] || "暂无状态"}`
      : session ? "当前会话用量暂不可用，稍后自动重试。" : "正在读取会话用量…";
    if (state.liveTurn && session?.status !== "ok") elements.session.textContent += `\n任务：${status[state.liveTurn.status] || "未知"}`;
    elements.limits.textContent = (Array.isArray(session?.rateLimits) ? session.rateLimits : []).map((item) => {
      const percent = codexPlusWhaleNumber(item.usedPercent);
      const reset = codexPlusWhaleNumber(item.resetAt);
      const observed = codexPlusWhaleNumber(item.observedAt);
      const expired = reset !== null && reset * 1000 <= Date.now();
      return `${item.label || "订阅额度"}额度快照${expired ? "（已过期）" : ""}：已用 ${percent === null ? "—" : `${Math.min(100, percent)}%`}${reset === null ? "" : `，${new Date(reset * 1000).toLocaleString()} 重置`}${observed === null ? "" : `\n记录于 ${new Date(observed * 1000).toLocaleString()}`}`;
    }).join("\n");
    elements.limits.hidden = !elements.limits.textContent;
    const used = session?.status === "ok" ? session.today?.totalTokens : null;
    elements.popLabel.textContent = state.message ? "Codex" : "今日用量";
    elements.popAmount.textContent = state.message ? String(state.message).slice(0, 64) : codexPlusWhaleAmount(used);
    elements.popAmount.dataset.message = String(Boolean(state.message));
    elements.popHint.textContent = state.message ? "点击气泡收起" : `tokens · ${status[currentTurn?.status] || "等待会话"}`;
    elements.pop.setAttribute("aria-label", state.message || `Codex 今日用量 ${codexPlusWhaleAmount(used)} tokens，${status[currentTurn?.status] || "等待会话"}`);
    state.root.dataset.running = String(currentTurn?.status === "running");
    codexPlusWhalePosition();
  }

  function codexPlusWhaleRenderCharacter() {
    const pet = codexPlusWhaleState.elements.pet;
    if (!pet) return;
    pet.replaceChildren();
    const imageSource = codexPlusWhaleImageSource(codexPlusWhaleState.prefs.image);
    if (imageSource) {
      const image = codexPlusWhaleElement("img");
      image.src = imageSource; image.alt = "自定义挂件角色"; image.draggable = false;
      pet.appendChild(image);
      return;
    }
    // 原版 DSniang1.png 由启动器内嵌，无运行时下载。来源和原始声明随素材保存。
    const image = codexPlusWhaleElement("img");
    image.src = codexPlusWhaleImageSource(window.__CODEX_PLUS_WHALE_IMAGE__);
    image.alt = "小鲸鱼娘"; image.draggable = false;
    pet.appendChild(image);
  }

  function codexPlusWhaleSettingRow(label, input) {
    const row = codexPlusWhaleElement("label", "whale-setting");
    row.append(codexPlusWhaleElement("span", "", label), input);
    return row;
  }

  function codexPlusWhaleSettings() {
    const state = codexPlusWhaleState, panel = state.elements.settings;
    panel.replaceChildren();
    panel.appendChild(codexPlusWhaleElement("strong", "", "挂件设置"));
    const checkbox = (label, key, after) => {
      const input = codexPlusWhaleElement("input"); input.type = "checkbox"; input.checked = state.prefs[key];
      input.addEventListener("change", () => { state.prefs[key] = input.checked; codexPlusWhaleSavePrefs(); after?.(input.checked); });
      panel.appendChild(codexPlusWhaleSettingRow(label, input));
    };
    checkbox("边缘吸附", "snap");
    checkbox("完成时显示气泡", "completion");
    checkbox("提示音（默认静音）", "sound", async (enabled) => {
      if (!enabled) { void state.audio?.close?.(); state.audio = null; return; }
      try {
        const Audio = window.AudioContext || window.webkitAudioContext;
        if (Audio && !state.audio) state.audio = new Audio();
        await state.audio?.resume?.(); codexPlusWhalePlaySound();
      } catch { codexPlusWhaleSay("当前系统暂不允许播放提示音。"); }
    });
    const size = codexPlusWhaleElement("input"); size.type = "range"; size.min = "73"; size.max = "372"; size.value = String(state.prefs.size);
    size.addEventListener("input", () => { state.prefs.size = Number(size.value); codexPlusWhalePosition(); });
    size.addEventListener("change", () => codexPlusWhalePosition(true));
    panel.appendChild(codexPlusWhaleSettingRow("角色大小", size));
    const phrase = codexPlusWhaleElement("textarea"); phrase.maxLength = 120; phrase.rows = 2; phrase.value = state.prefs.phrase;
    phrase.addEventListener("change", () => { state.prefs.phrase = phrase.value.slice(0, 120); state.message = ""; codexPlusWhaleSavePrefs(); codexPlusWhaleRender(); });
    panel.appendChild(codexPlusWhaleSettingRow("陪伴台词（纯文本）", phrase));
    const upload = codexPlusWhaleElement("input"); upload.type = "file"; upload.accept = "image/png,image/jpeg,image/webp,image/gif";
    upload.addEventListener("change", () => { const file = upload.files?.[0]; if (file) void codexPlusWhaleUpload(file); upload.value = ""; });
    panel.appendChild(codexPlusWhaleSettingRow("本地角色图片（最大 1 MiB）", upload));
    const restore = codexPlusWhaleElement("button", "", "恢复默认角色"); restore.type = "button";
    restore.addEventListener("click", () => { state.imageRevision += 1; state.prefs.image = ""; codexPlusWhaleSavePrefs(); codexPlusWhaleRenderCharacter(); });
    panel.appendChild(restore);
    const currencies = [...new Set((Array.isArray(state.balance?.balances) ? state.balance.balances : []).map((item) => item.currency).filter((currency) => /^[A-Z]{3,8}$/.test(currency)))];
    state.settingsCurrencies = currencies.join(",");
    if (!currencies.length) panel.appendChild(codexPlusWhaleElement("p", "whale-muted", "读取到余额币种后可设置阈值与每日预算。"));
    for (const currency of currencies) {
      panel.appendChild(codexPlusWhaleElement("strong", "", `${currency} 提醒（留空关闭）`));
      for (const [key, label] of [["low", "低余额阈值"], ["budget", "账户每日观测消费预算"]]) {
        const input = codexPlusWhaleElement("input"); input.type = "number"; input.min = "0"; input.max = "1000000000"; input.step = "any";
        input.value = String(state.prefs.thresholds[currency]?.[key] ?? "");
        input.addEventListener("change", () => {
          const value = codexPlusWhaleNumber(input.value);
          state.prefs.thresholds[currency] = { ...state.prefs.thresholds[currency], [key]: value !== null && value > 0 && value <= 1000000000 ? value : null };
          codexPlusWhaleSavePrefs(); codexPlusWhaleCheckAlerts(state.balance);
        });
        panel.appendChild(codexPlusWhaleSettingRow(`${label}（${currency}）`, input));
      }
    }
    panel.appendChild(codexPlusWhaleElement("p", "whale-muted", "消费由同一账户的余额变化观测，充值、其他设备和其他应用都可能影响结果。不会把账户扣费标作本轮费用。"));
  }

  async function codexPlusWhaleUpload(file) {
    const state = codexPlusWhaleState, revision = ++state.imageRevision, generation = state.generation;
    try {
      if (!/^image\/(png|jpeg|webp|gif)$/.test(file.type) || !file.size || file.size > codexPlusWhaleMaxImageBytes) throw new Error("仅支持不超过 1 MiB 的 PNG、JPEG、WebP 或 GIF 图片。");
      const bytes = new Uint8Array(await file.slice(0, 12).arrayBuffer());
      const ascii = (start, length) => String.fromCharCode(...bytes.slice(start, start + length));
      const valid = file.type === "image/png" ? bytes[0] === 137 && ascii(1, 3) === "PNG" && bytes[4] === 13 && bytes[5] === 10 && bytes[6] === 26 && bytes[7] === 10
        : file.type === "image/jpeg" ? bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255
        : file.type === "image/gif" ? ["GIF87a", "GIF89a"].includes(ascii(0, 6))
        : ascii(0, 4) === "RIFF" && ascii(8, 4) === "WEBP";
      if (!valid) throw new Error("图片内容与格式不符，请选择有效的本地图片。");
      const source = await new Promise((resolve, reject) => { const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.onerror = () => reject(new Error("读取图片失败。")); reader.readAsDataURL(file); });
      if (!codexPlusWhaleImageSource(source)) throw new Error("图片格式不受支持。");
      await new Promise((resolve, reject) => {
        const image = new Image();
        image.onload = () => image.naturalWidth > 0 && image.naturalHeight > 0 && image.naturalWidth <= 4096 && image.naturalHeight <= 4096 ? resolve() : reject(new Error("图片边长不能超过 4096 像素。"));
        image.onerror = () => reject(new Error("无法解码这张图片。")); image.src = source;
      });
      if (state.disposed || !state.mounted || state.imageRevision !== revision || state.generation !== generation) return;
      state.prefs.image = source; codexPlusWhaleSavePrefs(); codexPlusWhaleRenderCharacter();
      codexPlusWhaleSay("新角色已换好。");
    } catch (error) {
      if (state.mounted && state.imageRevision === revision) codexPlusWhaleSay(error?.message || "读取图片失败。");
    }
  }

  function codexPlusWhaleMount() {
    const state = codexPlusWhaleState;
    const root = codexPlusWhaleElement("aside"); root.id = "codex-plus-whale-widget";
    root.setAttribute("data-codex-plus-ext", "builtin-whale"); root.setAttribute("aria-label", "Codex 用量挂件");
    const style = codexPlusWhaleElement("style");
    style.textContent = `
      #codex-plus-whale-widget{position:fixed;z-index:2147482500;color:#203170;font:12px/1.55 system-ui,sans-serif;user-select:none;isolation:isolate;pointer-events:none;background:transparent!important;border:0!important;box-shadow:none!important;backdrop-filter:none!important}
      #codex-plus-whale-widget *{box-sizing:border-box}#codex-plus-whale-widget [hidden]{display:none!important}
      #codex-plus-whale-widget button,#codex-plus-whale-widget input,#codex-plus-whale-widget textarea{font:inherit;color:inherit}
      #codex-plus-whale-widget button{cursor:pointer;border:1px solid #8193a644;border-radius:8px;background:#7d98af12;padding:5px 8px}
      #codex-plus-whale-widget button:focus-visible,#codex-plus-whale-widget input:focus-visible,#codex-plus-whale-widget textarea:focus-visible{outline:2px solid #55b8df;outline-offset:3px}
      #codex-plus-whale-widget .whale-pet{display:block;width:100%;height:100%;padding:0;border:0;background:none;touch-action:none;cursor:grab;pointer-events:auto;transform-origin:50% 100%;transition:transform .22s cubic-bezier(.34,1.56,.64,1);-webkit-tap-highlight-color:transparent}
      #codex-plus-whale-widget .whale-pet:active{cursor:grabbing}#codex-plus-whale-widget .whale-pet svg,#codex-plus-whale-widget .whale-pet img{width:100%;height:100%;object-fit:contain;pointer-events:none}
      #codex-plus-whale-widget .whale-pet img{object-position:right bottom;transition:transform .3s ease}
      #codex-plus-whale-widget[data-side=left] .whale-pet img{transform:scaleX(-1)}
      #codex-plus-whale-widget[data-pressed=true] .whale-pet{transform:scaleY(.88) scaleX(1.05)}
      #codex-plus-whale-widget .whale-pop{position:absolute;padding:0;border:0;background:transparent;pointer-events:none;z-index:1}
      #codex-plus-whale-widget .whale-pop svg{display:block;width:100%;height:100%;background:transparent!important;border:0!important;overflow:visible}
      #codex-plus-whale-widget .whale-pop path,#codex-plus-whale-widget .whale-pop ellipse{pointer-events:visiblePainted;cursor:pointer;transform-box:fill-box;transform-origin:50% 50%;animation:codex-plus-whale-bubble .2s ease backwards}
      #codex-plus-whale-widget .whale-pop .whale-b1{animation-delay:.13s}#codex-plus-whale-widget .whale-pop .whale-bshape{animation-delay:.26s}
      #codex-plus-whale-widget[data-side=left] .whale-pop{transform:scaleX(-1)}
      #codex-plus-whale-widget .whale-pop-text{position:absolute;left:44.25%;top:36%;width:66%;height:64%;transform:translate(-50%,-50%);display:flex;flex-direction:column;align-items:center;justify-content:center;text-align:center;line-height:1.15;color:#536ba9;pointer-events:none;animation:codex-plus-whale-text .16s ease .36s backwards}
      #codex-plus-whale-widget[data-side=left] .whale-pop-text{transform:translate(-50%,-50%) scaleX(-1)}
      #codex-plus-whale-widget .whale-pop-label{font-size:calc(var(--whale-unit) * 66);font-weight:600;letter-spacing:.06em}
      #codex-plus-whale-widget .whale-pop-amount{font-size:calc(var(--whale-unit) * 128);font-weight:800;line-height:1.05}
      #codex-plus-whale-widget .whale-pop-amount[data-message=true]{font-size:calc(var(--whale-unit) * 58);line-height:1.2;white-space:normal;overflow-wrap:anywhere}
      #codex-plus-whale-widget .whale-pop-hint{font-size:calc(var(--whale-unit) * 56);color:#9fb0d9;letter-spacing:.02em;margin-top:calc(var(--whale-unit) * 9)}
      #codex-plus-whale-widget .whale-menu-button{position:absolute;top:4px;right:4px;width:26px;height:26px;border:0;border-radius:6px;background:rgba(32,49,112,.85);padding:0;color:white;pointer-events:auto;z-index:2;font-size:18px;line-height:26px;opacity:0;transition:opacity .15s ease}
      #codex-plus-whale-widget:hover .whale-menu-button,#codex-plus-whale-widget:focus-within .whale-menu-button{opacity:1}
      #codex-plus-whale-widget[data-side=left] .whale-menu-button{right:auto;left:4px}
      @media(hover:none){#codex-plus-whale-widget .whale-menu-button{opacity:1}}
      #codex-plus-whale-widget .whale-bubble{position:absolute;bottom:calc(100% + 8px);right:0;width:min(330px,calc(100vw - 32px));max-height:min(640px,var(--whale-room,70vh));overflow:auto;border:1px solid rgba(32,49,112,.35);border-radius:10px;background:rgba(255,255,255,.96);box-shadow:0 6px 18px rgba(0,0,0,.18);padding:10px 12px;user-select:text;overflow-wrap:anywhere;pointer-events:auto;z-index:3;color-scheme:light}
      #codex-plus-whale-widget[data-side=left] .whale-bubble{right:auto;left:0}#codex-plus-whale-widget[data-vertical=below] .whale-bubble{bottom:auto;top:calc(100% + 8px)}
      #codex-plus-whale-widget .whale-toolbar{display:flex;align-items:center;gap:6px;margin-bottom:10px}#codex-plus-whale-widget .whale-toolbar strong{flex:1;font-size:14px}
      #codex-plus-whale-widget p{margin:9px 0;white-space:pre-line}#codex-plus-whale-widget .whale-muted{opacity:.72;font-size:11px}#codex-plus-whale-widget .whale-notice{color:#e9ae56}
      #codex-plus-whale-widget .whale-settings{border-top:1px solid #8193a644;margin-top:12px;padding-top:12px}#codex-plus-whale-widget .whale-settings>strong{display:block;margin:8px 0}
      #codex-plus-whale-widget .whale-setting{display:flex;align-items:center;justify-content:space-between;gap:10px;margin:9px 0}#codex-plus-whale-widget .whale-setting:has(textarea),#codex-plus-whale-widget .whale-setting:has([type=file]){display:block}
      #codex-plus-whale-widget input[type=number]{width:100px}#codex-plus-whale-widget input[type=range]{width:120px}#codex-plus-whale-widget input[type=file]{display:block;width:100%;margin-top:6px;font-size:11px}
      #codex-plus-whale-widget input[type=number],#codex-plus-whale-widget textarea{background:#8193a614;border:1px solid #8193a655;border-radius:6px;padding:5px}#codex-plus-whale-widget textarea{display:block;width:100%;resize:vertical;margin-top:5px}
      @keyframes codex-plus-whale-bubble{from{opacity:0;transform:scale(.7)}to{opacity:1;transform:scale(1)}}@keyframes codex-plus-whale-text{from{opacity:0}to{opacity:1}}@media(prefers-reduced-motion:reduce){#codex-plus-whale-widget *{animation:none!important;transition:none!important}}
    `;
    const pet = codexPlusWhaleElement("button", "whale-pet"); pet.type = "button"; pet.setAttribute("aria-label", "Codex 用量挂件：查看用量与余额；拖动可移动"); pet.setAttribute("aria-controls", "codex-plus-whale-bubble");
    // 气泡路径与原版保持一致；仅文字数据换为 Codex 会话用量。
    const pop = codexPlusWhaleElement("div", "whale-pop"); pop.setAttribute("role", "button"); pop.setAttribute("tabindex", "0");
    pop.innerHTML = '<svg viewBox="0 0 1026 700" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><path class="whale-bshape" fill="#FFFFFF" stroke="#203170" stroke-width="18" stroke-linejoin="round" stroke-linecap="round" d="M 827 248 A 373 232 0 1 0 81 246 A 373 232 0 0 0 301 465 A 57 32 10 0 0 413 484 A 373 232 0 0 0 827 248 Z"/><ellipse class="whale-b1" cx="352" cy="561" rx="37.5" ry="26" fill="#FFFFFF" stroke="#203170" stroke-width="18"/><ellipse class="whale-b2" cx="442" cy="646" rx="24.5" ry="18" fill="#FFFFFF" stroke="#203170" stroke-width="18"/></svg>';
    const popText = codexPlusWhaleElement("div", "whale-pop-text");
    const popLabel = codexPlusWhaleElement("span", "whale-pop-label"), popAmount = codexPlusWhaleElement("span", "whale-pop-amount"), popHint = codexPlusWhaleElement("span", "whale-pop-hint");
    popText.append(popLabel, popAmount, popHint); pop.appendChild(popText);
    const dismissPop = () => { state.bubbleOpen = false; codexPlusWhaleRender(); };
    pop.addEventListener("click", dismissPop);
    pop.addEventListener("keydown", (event) => { if (["Enter", " "].includes(event.key)) { event.preventDefault(); dismissPop(); } });
    const menu = codexPlusWhaleElement("button", "whale-menu-button", "☰"); menu.type = "button"; menu.setAttribute("aria-label", "用量详情与挂件设置");
    const bubble = codexPlusWhaleElement("section", "whale-bubble"); bubble.id = "codex-plus-whale-bubble";
    const toolbar = codexPlusWhaleElement("div", "whale-toolbar");
    const settingsButton = codexPlusWhaleElement("button", "", "设置"); settingsButton.type = "button";
    const close = codexPlusWhaleElement("button", "", "收起"); close.type = "button";
    toolbar.append(codexPlusWhaleElement("strong", "", "Codex 用量挂件"), settingsButton, close);
    const message = codexPlusWhaleElement("p"); message.setAttribute("role", "status"); message.setAttribute("aria-live", "polite");
    const balance = codexPlusWhaleElement("p", "whale-muted"), session = codexPlusWhaleElement("p"), limits = codexPlusWhaleElement("p", "whale-muted");
    const notice = codexPlusWhaleElement("p", "whale-notice"); notice.setAttribute("role", "status");
    const settings = codexPlusWhaleElement("div", "whale-settings");
    bubble.append(toolbar, message, session, limits, balance, codexPlusWhaleElement("p", "whale-muted", "余额约每 60 秒更新；会话约每 10 秒更新。今日消费是账户观测值，非本轮费用。"), notice, settings);
    root.append(style, pop, bubble, pet, menu);
    state.root = root; state.elements = { pet, pop, popLabel, popAmount, popHint, bubble, message, balance, session, limits, settings, notice }; state.mounted = true;
    document.body.appendChild(root);
    registerCodexPlusExtensionSelector('[data-codex-plus-ext="builtin-whale"]');
    codexPlusWhaleRenderCharacter();
    pet.addEventListener("click", (event) => { if (state.suppressClick && event.detail !== 0) { state.suppressClick = false; return; } state.settingsOpen = false; state.bubbleOpen = !state.bubbleOpen; codexPlusWhaleRender(); });
    root.addEventListener("click", () => {
      if (!state.prefs.sound || state.audio) return;
      try {
        const Audio = window.AudioContext || window.webkitAudioContext;
        if (Audio) { state.audio = new Audio(); void state.audio.resume?.().catch?.(() => {}); }
      } catch {}
    });
    const toggleMenu = () => { state.settingsOpen = !state.settingsOpen; if (state.settingsOpen) codexPlusWhaleSettings(); codexPlusWhaleRender(); };
    menu.addEventListener("click", toggleMenu);
    pet.addEventListener("contextmenu", (event) => { event.preventDefault(); toggleMenu(); });
    settingsButton.addEventListener("click", () => { state.settingsOpen = !state.settingsOpen; if (state.settingsOpen) codexPlusWhaleSettings(); codexPlusWhaleRender(); });
    close.addEventListener("click", () => { state.settingsOpen = false; state.bubbleOpen = false; codexPlusWhaleRender(); pet.focus(); });
    root.addEventListener("keydown", (event) => {
      if (event.key === "Escape") { state.settingsOpen = false; state.bubbleOpen = false; codexPlusWhaleRender(); pet.focus(); }
      if (event.target !== pet || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
      event.preventDefault(); const delta = event.shiftKey ? 30 : 10;
      state.prefs.x = parseFloat(root.style.left) + (event.key === "ArrowLeft" ? -delta : event.key === "ArrowRight" ? delta : 0);
      state.prefs.y = parseFloat(root.style.top) + (event.key === "ArrowUp" ? -delta : event.key === "ArrowDown" ? delta : 0);
      codexPlusWhalePosition(true);
    });
    pet.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      state.suppressClick = false; root.dataset.pressed = "true";
      state.drag = { id: event.pointerId, x: event.clientX, y: event.clientY, left: parseFloat(root.style.left), top: parseFloat(root.style.top), moved: false };
      pet.setPointerCapture?.(event.pointerId);
    });
    pet.addEventListener("pointermove", (event) => {
      const drag = state.drag; if (!drag || event.pointerId !== drag.id) return;
      const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
      if (!drag.moved && Math.hypot(dx, dy) < 5) return;
      drag.moved = true; state.prefs.x = drag.left + dx; state.prefs.y = drag.top + dy; codexPlusWhalePosition();
    });
    const finish = (event) => {
      const drag = state.drag; if (!drag || event.pointerId !== drag.id) return;
      root.dataset.pressed = "false"; state.drag = null; state.suppressClick = drag.moved;
      if (drag.moved) {
        if (state.prefs.snap && event.type !== "pointercancel") state.prefs.x = parseFloat(root.style.left) + parseFloat(root.style.width) / 2 < window.innerWidth / 2 ? 8 : window.innerWidth - parseFloat(root.style.width) - 8;
        codexPlusWhalePosition(true);
      }
    };
    pet.addEventListener("pointerup", finish); pet.addEventListener("pointercancel", finish); pet.addEventListener("lostpointercapture", finish);
    codexPlusWhaleListen(document, "visibilitychange", () => {
      state.generation += 1; codexPlusWhaleCancelRequests(); state.balancePending = null; state.sessionPending = null; state.previousTurn = null; state.liveTurn = null;
      clearTimeout(state.timer); state.timer = null;
      if (!document.hidden) { state.sessionDue = 0; codexPlusWhalePoll(); }
    });
    codexPlusWhaleListen(window, "resize", () => codexPlusWhalePosition());
    codexPlusWhaleRender();
  }

  function codexPlusWhaleContext() {
    const state = codexPlusWhaleState;
    let sessionId = "", profileId = "";
    try { sessionId = String(currentSessionRef()?.session_id || ""); } catch {}
    try {
      const profile = codexRemoteSessionActiveProfile();
      const settings = typeof codexPlusBackendSettings === "object" ? codexPlusBackendSettings : {};
      // 只比较显示协议与公开字段，不读取或保存供应商密钥。
      profileId = JSON.stringify([profile?.id || "", profile?.baseUrl || "", profile?.relayMode || "",
        settings.codexAppWhaleBalanceProtocol, settings.codexAppWhaleBalancePath,
        settings.codexAppWhaleBalanceField, settings.codexAppWhaleBalanceCurrency, settings.codexAppWhaleBalanceScale]);
    } catch {}
    if (profileId !== state.profileId) {
      codexPlusWhaleCancelRequests();
      state.completedTurns.clear(); state.liveTurn = null;
      state.profileId = profileId; state.balance = null; state.balanceDue = 0; state.session = null; state.sessionDue = 0;
      state.generation += 1; state.balancePending = null; state.sessionPending = null; state.previousTurn = null;
    }
    if (sessionId !== state.sessionId) {
      if (state.balancePending) state.balanceDue = 0;
      codexPlusWhaleCancelRequests();
      state.completedTurns.clear(); state.liveTurn = null;
      state.sessionId = sessionId; state.session = null; state.sessionDue = 0; state.previousTurn = null; state.message = "";
      state.generation += 1; state.balancePending = null; state.sessionPending = null;
    }
  }

  function codexPlusWhaleCancelRequests() {
    for (const cancel of [...codexPlusWhaleState.pendingCancels]) cancel();
  }

  function codexPlusWhaleBridge(path, payload) {
    const state = codexPlusWhaleState;
    return new Promise((resolve) => {
      let finished = false;
      const finish = (value) => {
        if (finished) return;
        finished = true; clearTimeout(timer); state.pendingCancels.delete(cancel); resolve(value);
      };
      const cancel = () => finish(null);
      const timer = setTimeout(() => finish({ status: "unavailable" }), 15000);
      state.pendingCancels.add(cancel);
      try { Promise.resolve(postJson(path, payload)).then(finish, () => finish({ status: "unavailable" })); }
      catch { finish({ status: "unavailable" }); }
    });
  }

  async function codexPlusWhaleRequest(kind) {
    const state = codexPlusWhaleState;
    const generation = state.generation, sessionId = state.sessionId;
    const marker = {}; state[`${kind}Pending`] = marker;
    state[`${kind}Due`] = Date.now() + (kind === "balance" ? 60000 : 10000);
    try {
      const response = await codexPlusWhaleBridge(`/whale/${kind}`, kind === "session" ? { session_id: sessionId } : {});
      if (!state.mounted || !codexPlusWhaleEnabled() || document.hidden || state.generation !== generation || state[`${kind}Pending`] !== marker) return;
      if (kind === "session" && sessionId !== state.sessionId) return;
      state[kind] = response && typeof response === "object" ? response : { status: "unavailable" };
      if (kind === "balance") {
        codexPlusWhaleCheckAlerts(state.balance);
        const currencies = (Array.isArray(state.balance?.balances) ? state.balance.balances : []).map((item) => item.currency).filter((currency) => /^[A-Z]{3,8}$/.test(currency)).join(",");
        if (state.settingsOpen && currencies !== state.settingsCurrencies) codexPlusWhaleSettings();
      }
      else {
        // 原生事件先于日志落盘；旧日志不能把刚完成的任务重新显示成运行中。
        const loggedTurn = state.session?.lastTurn;
        const live = state.liveTurn;
        const loggedAt = Date.parse(loggedTurn?.updatedAt || "");
        const sameTurnCaughtUp = loggedTurn?.id === live?.id
          && (loggedTurn?.status === live?.status || (live?.status === "running" && ["completed", "failed", "aborted"].includes(loggedTurn?.status)));
        if (!live || sameTurnCaughtUp || (Number.isFinite(loggedAt) && loggedAt >= live.observedAt) || Date.now() - live.observedAt > 60000) {
          state.liveTurn = null; codexPlusWhaleObserveTurn(state.session);
        }
      }
      codexPlusWhaleRender();
    } catch {
      if (state.mounted && state.generation === generation && state[`${kind}Pending`] === marker) {
        state[kind] = { status: "unavailable" }; if (kind === "session") state.previousTurn = null; codexPlusWhaleRender();
      }
    } finally {
      if (state[`${kind}Pending`] === marker) state[`${kind}Pending`] = null;
    }
  }

  function codexPlusWhalePoll() {
    const state = codexPlusWhaleState;
    clearTimeout(state.timer); state.timer = null;
    if (!state.mounted || !codexPlusWhaleEnabled() || document.hidden) return;
    codexPlusWhaleContext();
    if (!state.balancePending && Date.now() >= state.balanceDue) void codexPlusWhaleRequest("balance");
    if (state.sessionId && !state.sessionPending && Date.now() >= state.sessionDue) void codexPlusWhaleRequest("session");
    state.timer = setTimeout(codexPlusWhalePoll, 10000);
  }

  function codexPlusWhaleStop() {
    const state = codexPlusWhaleState;
    state.generation += 1; state.imageRevision += 1; state.mounted = false;
    codexPlusWhaleCancelRequests();
    codexPlusWhaleUnsubscribe(); state.liveTurn = null; state.completedTurns.clear();
    clearTimeout(state.timer); state.timer = null;
    for (const cleanup of state.listeners.splice(0)) cleanup();
    state.root?.remove(); state.root = null; state.elements = {}; state.drag = null;
    state.balancePending = null; state.sessionPending = null; state.previousTurn = null;
    state.balance = null; state.session = null; state.balanceDue = 0; state.sessionDue = 0;
    state.bubbleOpen = false; state.settingsOpen = false; state.message = "";
    void state.audio?.close?.(); state.audio = null;
  }

  function syncCodexPlusWhaleWidget() {
    const state = codexPlusWhaleState;
    if (!codexPlusWhaleEnabled()) { if (state.mounted) codexPlusWhaleStop(); return; }
    if (!document.body) return;
    if (state.root && !state.root.isConnected) codexPlusWhaleStop();
    if (!state.mounted) codexPlusWhaleMount();
    codexPlusWhaleSubscribe();
    const previousSession = state.sessionId, previousProfile = state.profileId;
    codexPlusWhaleContext();
    if (previousSession !== state.sessionId || previousProfile !== state.profileId) codexPlusWhaleRender();
    if (!state.timer) codexPlusWhalePoll();
    else if (state.sessionDue === 0 || state.balanceDue === 0) codexPlusWhalePoll();
  }

  window.__codexPlusWhaleWidgetRuntime = {
    dispose() { codexPlusWhaleStop(); codexPlusWhaleState.disposed = true; },
  };
