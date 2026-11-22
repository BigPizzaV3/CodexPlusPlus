  /**
   * Codex++ 设置页里的「皮肤」分区（内置拓展「梦皮主题」注册，见 96-builtin-extensions.js）。
   *
   * 皮肤只在 Codex App 里有意义，所以界面只放这里；管理器只留「恢复默认皮肤」急救入口。
   * 数据与动作经 bridge 路由 /dream-skin（管理器 dream_skin_bridge.rs），跑的是管理器原来那组命令函数：
   * 主题库、主题市场、DreamSkin 社区、激活（同「应用所选主题」：先存开关再激活）、恢复原始外观。
   *
   * 页面拿不到也不提交本地路径：预览图按 key 由服务端读成 data URL；新建主题、导入主题包、
   * 换素材都是把选中的文件内容传过去，由服务端落到自己的上传目录再交给核心函数校验。
   * 换成不同主题时皮肤样式要重启 Codex 才完整切换（与管理器一致），这时给出「重启 Codex」。
   *
   * 「壁纸透出」（#3233）是本机偏好，存在 Codex++ 设置里、不写进主题包；服务端注入时作为
   * userTransparency 并进主题配置交给上游渲染器，皮肤开着就当场实时重应用。
   */
  const codexPlusSkinState = {
    data: null,
    busy: "",
    status: "",
    tone: "",
    tab: "local",
    previews: {},
    market: null,
    community: null,
    editing: null,
    pendingRestart: false,
    checks: null,
    root: null,
    // 「壁纸透出」提交中 / 提交中又改了的下一个值（{ value }）：松手才提交，连按方向键只发最后一个。
    transparencyBusy: false,
    transparencyQueued: null,
  };
  /** 「跟随主题」时滑块停的位置：上游渲染器给没写透明度的主题的默认透出。 */
  const codexPlusSkinTransparencyFallback = 30;

  const codexPlusSkinColorFields = [
    ["background", "背景色"], ["panel", "面板色"], ["panelAlt", "次级面板色"], ["accent", "强调色"], ["accentAlt", "次级强调色"],
    ["secondary", "辅助色"], ["highlight", "高亮色"], ["text", "文字色"], ["muted", "弱化文字色"], ["line", "边线色"],
  ];
  const codexPlusSkinTextFields = [
    ["name", "主题名称"], ["brandSubtitle", "品牌副标题"], ["tagline", "主题标语"], ["projectPrefix", "项目前缀"],
    ["projectLabel", "项目按钮文字"], ["statusText", "状态文字"], ["quote", "引用文字"],
  ];
  const codexPlusSkinCompanionLimit = 240000;
  const codexPlusSkinMediaAccept = codexPlusDreamSkinPlatform === "windows"
    ? ".png,.jpg,.jpeg,.webp,.gif,.bmp,.mp4,.webm"
    : ".png,.jpg,.jpeg,.heic,.tif,.tiff,.webp,.mp4,.webm";

  function codexPlusSkinRequest(action, payload = {}) {
    return postJson("/dream-skin", { action, ...payload });
  }

  function codexPlusSkinSetStatus(message, tone = "") {
    codexPlusSkinState.status = String(message || "");
    codexPlusSkinState.tone = tone;
  }

  function codexPlusSkinTone(result) {
    if (result?.status === "ok") return "ok";
    if (result?.status === "pending_restart" || result?.status === "not_checked") return "warn";
    return "error";
  }

  /** 命令结果里带回的开关与主题库直接并进状态，页面一次刷新完。 */
  function codexPlusSkinMerge(result) {
    if (!result || !Array.isArray(result.themes)) return;
    codexPlusSkinState.data = { ...(codexPlusSkinState.data || {}), ...result };
  }

  async function codexPlusSkinLoad() {
    const result = await codexPlusSkinRequest("overview");
    if (result?.status !== "ok" && !Array.isArray(result?.themes)) {
      codexPlusSkinSetStatus(result?.message || "读取皮肤失败", "error");
    } else {
      codexPlusSkinMerge(result);
      if (result.message) codexPlusSkinSetStatus(result.message, "error");
    }
    codexPlusSkinRender();
    void codexPlusSkinLoadPreviews();
  }

  /** 预览图逐个取（有的主题图很大，不并发）；取到就地换进卡片，不重绘整页。 */
  async function codexPlusSkinLoadPreviews() {
    const state = codexPlusSkinState;
    for (const theme of state.data?.themes || []) {
      if (!theme.hasPreview || theme.key in state.previews) continue;
      state.previews[theme.key] = "";
      const result = await codexPlusSkinRequest("preview", { key: theme.key });
      state.previews[theme.key] = result?.url || "";
      if (!state.previews[theme.key]) continue;
      state.root?.querySelectorAll(`[data-skin-preview="${CSS.escape(theme.key)}"]`).forEach((node) => {
        node.style.backgroundImage = `url("${state.previews[theme.key]}")`;
        node.dataset.loaded = "true";
      });
    }
  }

  async function codexPlusSkinRun(busy, action, payload, { reloadPreviews = false } = {}) {
    const state = codexPlusSkinState;
    state.busy = busy;
    codexPlusSkinRender();
    const result = await codexPlusSkinRequest(action, payload);
    state.busy = "";
    codexPlusSkinMerge(result);
    codexPlusSkinSetStatus(result?.message || (result?.status === "ok" ? "" : "操作失败"), codexPlusSkinTone(result));
    if (reloadPreviews) state.previews = {};
    codexPlusSkinRender();
    void codexPlusSkinLoadPreviews();
    return result;
  }

  async function codexPlusSkinActivate(key) {
    const result = await codexPlusSkinRun(`activate:${key}`, "activate", { key });
    if (result?.status === "ok") {
      codexPlusSkinState.pendingRestart = result.savedForNextLaunch === true;
      codexPlusSkinRender();
    }
  }

  async function codexPlusSkinRestore() {
    const result = await codexPlusSkinRun("restore", "restore", {});
    if (result?.status === "pending_restart" || result?.status === "ok") {
      codexPlusSkinState.pendingRestart = true;
      codexPlusSkinRender();
    }
  }

  /** 设置里的「壁纸透出」：0–100 的整数，其它（含未设）都是跟随主题（null）。 */
  function codexPlusSkinTransparencyValue(value) {
    return typeof value === "number" && Number.isFinite(value) ? Math.max(0, Math.min(100, Math.round(value))) : null;
  }

  function codexPlusSkinTransparencyText(value) {
    return value === null ? "跟随主题" : `${value}%`;
  }

  /** 提交「壁纸透出」（null = 跟随主题）。提交中再改只记下最新值，这一次回来后接着发。 */
  async function codexPlusSkinSetTransparency(value) {
    const state = codexPlusSkinState;
    state.transparencyQueued = { value: codexPlusSkinTransparencyValue(value) };
    if (state.transparencyBusy) return;
    state.transparencyBusy = true;
    let result = null;
    while (state.transparencyQueued) {
      const next = state.transparencyQueued.value;
      state.transparencyQueued = null;
      result = await codexPlusSkinRequest("transparency", { value: next });
    }
    state.transparencyBusy = false;
    codexPlusSkinMerge(result);
    if (result?.status === "pending_restart") state.pendingRestart = true;
    codexPlusSkinSetStatus(result?.message || (result?.status === "ok" ? "" : "保存壁纸透出失败"), codexPlusSkinTone(result));
    codexPlusSkinRender();
  }

  async function codexPlusSkinLoadCatalog(kind) {
    const state = codexPlusSkinState;
    state.busy = `catalog:${kind}`;
    codexPlusSkinRender();
    const result = await codexPlusSkinRequest(kind, {});
    state.busy = "";
    state[kind] = result || { status: "failed", message: "读取失败" };
    if (result?.status !== "ok") codexPlusSkinSetStatus(result?.message || "读取失败", "error");
    codexPlusSkinRender();
  }

  async function codexPlusSkinInstall(kind, id) {
    const state = codexPlusSkinState;
    state.busy = `install:${id}`;
    codexPlusSkinRender();
    const result = await codexPlusSkinRequest(kind === "market" ? "installMarket" : "installCommunity", { id });
    state.busy = "";
    if (result && (Array.isArray(result.themes) || Array.isArray(result.items))) state[kind] = result;
    codexPlusSkinSetStatus(result?.message || "安装失败", codexPlusSkinTone(result));
    await codexPlusSkinLoad();
  }

  /** 读选中的文件成 { name, data(base64) }。 */
  function codexPlusSkinReadFile(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve({ name: file.name, data: String(reader.result || "").replace(/^data:[^,]*,/, "") });
      reader.onerror = () => reject(new Error("读取文件失败"));
      reader.readAsDataURL(file);
    });
  }

  async function codexPlusSkinUpload(kind, file) {
    if (!file) return;
    if (file.size > 50 * 1024 * 1024) {
      codexPlusSkinSetStatus("文件超过 50 MiB。", "error");
      return codexPlusSkinRender();
    }
    const media = await codexPlusSkinReadFile(file).catch(() => null);
    if (!media) {
      codexPlusSkinSetStatus("读取文件失败，请重新选择。", "error");
      return codexPlusSkinRender();
    }
    if (kind === "media") {
      codexPlusSkinState.editing.media = media;
      return codexPlusSkinRender();
    }
    await codexPlusSkinRun(kind, kind, { media }, { reloadPreviews: true });
  }

  function codexPlusSkinChooseCompanion(file) {
    const editing = codexPlusSkinState.editing;
    if (!file || !editing) return;
    if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(file.type)) {
      editing.companionError = "仅支持 PNG、JPEG、WebP 或 GIF 图片";
      return codexPlusSkinRender();
    }
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = typeof reader.result === "string" ? reader.result : "";
      if (!dataUrl || dataUrl.length > codexPlusSkinCompanionLimit) {
        editing.companionError = "图片过大，请选择 180 KB 以内的图片";
      } else {
        editing.companionError = "";
        codexPlusSkinUpdateCompanion({ dataUrl });
      }
      codexPlusSkinRender();
    };
    reader.onerror = () => {
      editing.companionError = "读取图片失败，请重新选择";
      codexPlusSkinRender();
    };
    reader.readAsDataURL(file);
  }

  /** 与管理器 updateCompanion 一致：缺省宽 96、靠右、偏移 (0, 4)。 */
  function codexPlusSkinUpdateCompanion(patch) {
    const config = codexPlusSkinState.editing.config;
    const companion = config.companion || {};
    config.companion = {
      dataUrl: companion.dataUrl || "",
      enabled: companion.enabled ?? true,
      width: companion.width ?? 96,
      side: companion.side ?? "right",
      offsetX: companion.offsetX ?? 0,
      offsetY: companion.offsetY ?? 4,
      ...patch,
    };
    codexPlusSkinState.editing.dirty = true;
  }

  async function codexPlusSkinEdit(theme) {
    const result = await codexPlusSkinRequest("load", { key: theme.key });
    if (result?.status !== "ok" || !result.config) {
      codexPlusSkinSetStatus(result?.message || "读取主题失败", "error");
      return codexPlusSkinRender();
    }
    const config = JSON.parse(JSON.stringify(result.config));
    // 与管理器「保存」一致：内置主题另存成新主题，名字是默认名时改成「副本」。
    if (result.builtin) {
      config.id = `theme-${Date.now()}`;
      if (config.name === "Dream Skin") config.name = "Dream Skin 副本";
    }
    codexPlusSkinState.editing = { key: theme.key, config, media: null, dirty: result.builtin === true, companionError: "", builtin: result.builtin === true };
    codexPlusSkinRender();
    codexPlusSkinState.root?.querySelector(".codex-plus-skin-editor")?.scrollIntoView({ block: "start", behavior: "smooth" });
  }

  async function codexPlusSkinSave(apply) {
    const editing = codexPlusSkinState.editing;
    if (!editing) return;
    if (!String(editing.config.name || "").trim()) {
      codexPlusSkinSetStatus("请填写主题名称。", "error");
      return codexPlusSkinRender();
    }
    const result = await codexPlusSkinRun("save", "save", { config: editing.config, media: editing.media }, { reloadPreviews: true });
    if (result?.status !== "ok") return;
    const key = `stored:${editing.config.id}`;
    codexPlusSkinState.editing = null;
    codexPlusSkinRender();
    if (apply) await codexPlusSkinActivate(key);
  }

  async function codexPlusSkinRename(theme) {
    const answer = await codexPlusAsk({ title: "重命名主题", input: { default: theme.name, placeholder: "输入新的主题名称" }, options: [{ value: "ok", label: "重命名" }] });
    const name = String(answer?.text || "").trim();
    if (!answer || !name || name === theme.name) return;
    await codexPlusSkinRun(`rename:${theme.key}`, "rename", { id: theme.id, name });
  }

  async function codexPlusSkinDelete(theme) {
    const answer = await codexPlusAsk({ title: "删除主题", message: `删除主题「${theme.name}」？此操作无法撤销。`, options: [{ value: "delete", label: "删除" }] });
    if (!answer) return;
    if (codexPlusSkinState.editing?.key === theme.key) codexPlusSkinState.editing = null;
    await codexPlusSkinRun(`delete:${theme.key}`, "delete", { id: theme.id });
  }

  async function codexPlusSkinCheck() {
    const state = codexPlusSkinState;
    state.busy = "status";
    codexPlusSkinRender();
    const result = await codexPlusSkinRequest("status", {});
    state.busy = "";
    state.checks = result;
    codexPlusSkinRender();
  }

  function codexPlusSkinThemeSwatch(theme, label) {
    const url = codexPlusSkinState.previews[theme.key];
    const initial = escapeHtml(String(label || "?").trim().slice(0, 1) || "?");
    return `<div class="codex-plus-skin-thumb" data-skin-preview="${escapeHtml(theme.key)}" data-loaded="${String(Boolean(url))}"${url ? ` style="background-image:url(&quot;${url}&quot;)"` : ""}><span>${initial}</span></div>`;
  }

  function codexPlusSkinRemoteThumb(url, label) {
    const initial = escapeHtml(String(label || "?").trim().slice(0, 1) || "?");
    // 只认不带引号、括号、空白的 https 地址：它要放进 style 的 url("…") 里。
    const safe = /^https:\/\/[^\s"'()\\]+$/i.test(String(url || "")) ? String(url) : "";
    return `<div class="codex-plus-skin-thumb" data-loaded="${String(Boolean(safe))}"${safe ? ` style="background-image:url(&quot;${escapeHtml(safe)}&quot;)"` : ""}><span>${initial}</span></div>`;
  }

  function codexPlusSkinButton(action, label, attrs = "", busyKey = "") {
    const state = codexPlusSkinState;
    const busy = busyKey && state.busy === busyKey;
    return `<button type="button" class="codex-plus-action-button" data-skin-action="${action}" ${attrs} ${state.busy ? "disabled" : ""}>${escapeHtml(busy ? `${label}中…` : label)}</button>`;
  }

  function codexPlusSkinLocalCards() {
    const state = codexPlusSkinState;
    const data = state.data || {};
    const enabled = data.enabled === true && data.paused !== true;
    const themes = data.themes || [];
    if (!themes.length) return '<div class="codex-plus-skin-empty">主题库是空的。</div>';
    return `<div class="codex-plus-skin-grid">${themes.map((theme) => {
      const using = theme.active && enabled;
      const badges = [
        using ? '<span class="codex-plus-skin-badge" data-tone="active">使用中</span>' : theme.active ? '<span class="codex-plus-skin-badge">当前主题</span>' : "",
        theme.builtin ? '<span class="codex-plus-skin-badge">内置</span>' : "",
        theme.kind === "activeUnsaved" ? '<span class="codex-plus-skin-badge" data-tone="warn">未存进主题库</span>' : "",
        theme.modified ? '<span class="codex-plus-skin-badge" data-tone="warn">已修改</span>' : "",
      ].join("");
      const stored = theme.kind === "stored";
      return `<div class="codex-plus-skin-card" data-active="${String(using)}">
        ${codexPlusSkinThemeSwatch(theme, theme.name)}
        <div class="codex-plus-skin-card-body">
          <div class="codex-plus-skin-card-title" title="${escapeHtml(theme.name)}">${escapeHtml(theme.name)}</div>
          <div class="codex-plus-skin-card-badges">${badges}</div>
          <div class="codex-plus-skin-card-actions">
            ${using ? "" : codexPlusSkinButton("activate", "应用", `data-key="${escapeHtml(theme.key)}" data-primary="true"`, `activate:${theme.key}`)}
            ${codexPlusSkinButton("edit", "编辑", `data-key="${escapeHtml(theme.key)}"`)}
            ${stored ? codexPlusSkinButton("rename", "重命名", `data-key="${escapeHtml(theme.key)}"`) : ""}
            ${stored && !theme.active ? codexPlusSkinButton("delete", "删除", `data-key="${escapeHtml(theme.key)}" data-danger="true"`) : ""}
          </div>
        </div>
      </div>`;
    }).join("")}</div>`;
  }

  function codexPlusSkinCatalogCards(kind) {
    const state = codexPlusSkinState;
    const catalog = state[kind];
    if (!catalog) return `<div class="codex-plus-skin-empty">${state.busy === `catalog:${kind}` ? "正在读取…" : "还没有读取。"}</div>`;
    const items = kind === "market" ? catalog.themes || [] : catalog.items || [];
    const warning = catalog.warning ? `<div class="codex-plus-skin-note">${escapeHtml(catalog.warning)}</div>` : "";
    if (!items.length) return `${warning}<div class="codex-plus-skin-empty">${escapeHtml(catalog.status === "ok" ? "这里还没有主题。" : catalog.message || "读取失败")}</div>`;
    return `${warning}<div class="codex-plus-skin-grid">${items.map((item) => {
      const author = kind === "market" ? item.author : item.authorDisplayName;
      const meta = [author ? `作者 ${author}` : "", item.version ? `v${item.version}` : "", kind === "community" && item.downloadCount ? `${item.downloadCount} 次下载` : ""].filter(Boolean).join(" · ");
      const blocked = kind === "community" && item.applyCompatible === false;
      const action = item.updateAvailable ? "更新" : item.installed ? "" : "安装";
      return `<div class="codex-plus-skin-card">
        ${codexPlusSkinRemoteThumb(item.previewUrl, item.name)}
        <div class="codex-plus-skin-card-body">
          <div class="codex-plus-skin-card-title" title="${escapeHtml(item.name)}">${escapeHtml(item.name)}</div>
          <div class="codex-plus-skin-card-meta">${escapeHtml(meta)}</div>
          ${kind === "market" && item.description ? `<div class="codex-plus-skin-card-meta" title="${escapeHtml(item.description)}">${escapeHtml(item.description)}</div>` : ""}
          <div class="codex-plus-skin-card-actions">
            ${item.installed && !item.updateAvailable ? '<span class="codex-plus-skin-badge" data-tone="active">已安装</span>' : ""}
            ${blocked ? '<span class="codex-plus-skin-badge" data-tone="warn">当前版本不兼容</span>'
              : action ? codexPlusSkinButton("install", action, `data-kind="${kind}" data-id="${escapeHtml(item.id)}" data-primary="true"`, `install:${item.id}`) : ""}
          </div>
        </div>
      </div>`;
    }).join("")}</div>`;
  }

  function codexPlusSkinRow(title, description, control) {
    return `<div class="codex-plus-row"><div><div class="codex-plus-row-title">${escapeHtml(title)}</div>${description ? `<div class="codex-plus-row-description">${description}</div>` : ""}</div>${control || ""}</div>`;
  }

  function codexPlusSkinSegmented(field, value, options, label) {
    return `<div class="codex-plus-provider-segmented" role="radiogroup" aria-label="${escapeHtml(label)}">${options.map(([id, text]) =>
      `<button type="button" role="radio" aria-checked="${String(id === value)}" data-skin-choice="${field}" data-value="${escapeHtml(id)}">${escapeHtml(text)}</button>`).join("")}</div>`;
  }

  /** 「壁纸透出」行：滑块 + 当前值 + 「跟随主题」。当前主题的引擎不认这个偏好时禁用。 */
  function codexPlusSkinTransparencyRow() {
    const data = codexPlusSkinState.data || {};
    const value = codexPlusSkinTransparencyValue(data.userTransparency);
    const follow = value === null;
    const supported = data.transparencySupported !== false;
    const text = codexPlusSkinTransparencyText(value);
    const description = supported
      ? "侧栏、主区和输入框的底色透出多少壁纸（0% 不透，100% 全透）。「跟随主题」用主题配色自带的透明度，没写的透出 30%。只存在本机设置里，不写进主题包。"
      : "当前主题使用的皮肤引擎不支持调节壁纸透出。";
    return codexPlusSkinRow("壁纸透出", escapeHtml(description), `<span class="codex-plus-skin-inline">
        <input type="range" class="codex-plus-skin-range" min="0" max="100" step="1" value="${follow ? codexPlusSkinTransparencyFallback : value}" data-skin-transparency data-follow="${String(follow)}" aria-label="壁纸透出" aria-valuetext="${escapeHtml(text)}" ${supported ? "" : "disabled"}>
        <span class="codex-plus-skin-range-value" data-skin-transparency-value>${escapeHtml(text)}</span>
        ${follow || !supported ? "" : '<button type="button" class="codex-plus-action-button" data-skin-action="transparency-follow">跟随主题</button>'}
      </span>`);
  }

  function codexPlusSkinEditor() {
    const editing = codexPlusSkinState.editing;
    if (!editing) return "";
    const config = editing.config;
    const colors = config.colors || {};
    const companion = config.companion || null;
    const windows = codexPlusDreamSkinPlatform === "windows";
    const input = (field, value, attrs = "") => `<input type="text" class="codex-plus-skin-input" data-skin-field="${field}" value="${escapeHtml(value ?? "")}" spellcheck="false" ${attrs}>`;
    const textRows = codexPlusSkinTextFields.map(([field, label]) => codexPlusSkinRow(label, "", input(field, config[field]))).join("");
    const colorRows = windows
      ? codexPlusSkinRow("外观模式", "亮暗模式直接控制 Codex 外观。", codexPlusSkinSegmented("appearance", ["light", "dark"].includes(config.appearance) ? config.appearance : "auto", [["auto", "自动"], ["light", "亮色"], ["dark", "暗色"]], "外观模式"))
        + codexPlusSkinRow("强调色", "留空时自动从主题图片提取。", `<span class="codex-plus-skin-color"><span class="codex-plus-skin-swatch" style="background:${escapeHtml(config.palette?.accent || "transparent")}"></span>${input("palette.accent", config.palette?.accent || "", 'placeholder="跟随图片配色"')}</span>`)
      : codexPlusSkinColorFields.map(([key, label]) => codexPlusSkinRow(label, "",
        `<span class="codex-plus-skin-color"><span class="codex-plus-skin-swatch" style="background:${escapeHtml(colors[key] || "transparent")}"></span>${input(`colors.${key}`, colors[key] || "", 'placeholder="#RRGGBB"')}</span>`)).join("");
    const mediaName = editing.media ? editing.media.name : "沿用主题现在的图片或视频";
    const companionRows = [
      codexPlusSkinRow("输入框旁照片", editing.companionError ? `<span class="codex-plus-skin-error">${escapeHtml(editing.companionError)}</span>` : "显示在 Codex 输入框的左侧或右侧；180 KB 以内的 PNG、JPEG、WebP 或 GIF。",
        `<span class="codex-plus-skin-inline">${companion?.dataUrl ? `<img class="codex-plus-skin-companion" alt="" src="${escapeHtml(companion.dataUrl)}">` : ""}
          <label class="codex-plus-action-button codex-plus-skin-file">${companion?.dataUrl ? "更换照片" : "选择照片"}<input type="file" accept="image/png,image/jpeg,image/webp,image/gif" data-skin-file="companion" hidden></label>
          ${companion?.dataUrl ? '<button type="button" class="codex-plus-action-button" data-skin-action="companion-clear">清除</button>' : ""}</span>`),
      companion?.dataUrl ? codexPlusSkinRow("显示照片", "", `<button type="button" class="codex-plus-toggle" role="switch" aria-label="显示照片" aria-checked="${String(companion.enabled !== false)}" data-enabled="${String(companion.enabled !== false)}" data-skin-action="companion-toggle"><span></span></button>`) : "",
      companion?.dataUrl ? codexPlusSkinRow("显示位置", "", codexPlusSkinSegmented("companion.side", companion.side || "right", [["auto", "自动"], ["left", "左侧"], ["right", "右侧"]], "显示位置")) : "",
      companion?.dataUrl ? codexPlusSkinRow("照片宽度 / 水平偏移 / 垂直偏移", "单位像素。", `<span class="codex-plus-skin-inline">${["width", "offsetX", "offsetY"].map((key) =>
        `<input type="text" inputmode="numeric" class="codex-plus-skin-input codex-plus-skin-number" data-skin-field="companion.${key}" value="${escapeHtml(String(companion[key] ?? (key === "width" ? 96 : key === "offsetY" ? 4 : 0)))}" aria-label="${key === "width" ? "照片宽度" : key === "offsetX" ? "水平偏移" : "垂直偏移"}">`).join("")}</span>`) : "",
    ].join("");
    return `<section class="codex-plus-skin-editor">
      <div class="codex-plus-skin-section-head"><span>编辑主题：${escapeHtml(config.name || config.id)}</span>${editing.dirty ? '<span class="codex-plus-skin-badge" data-tone="warn">有未保存修改</span>' : ""}</div>
      <div class="codex-plus-settings-card">
        ${codexPlusSkinRow("图片或视频", escapeHtml(mediaName), `<label class="codex-plus-action-button codex-plus-skin-file">选择文件<input type="file" accept="${codexPlusSkinMediaAccept}" data-skin-file="media" hidden></label>`)}
        ${textRows}
      </div>
      <div class="codex-plus-skin-section-head"><span>${windows ? "外观" : "主题配色"}</span></div>
      <div class="codex-plus-settings-card">${colorRows}</div>
      <div class="codex-plus-skin-section-head"><span>输入框旁照片</span></div>
      <div class="codex-plus-settings-card">${companionRows}</div>
      <div class="codex-plus-skin-editor-actions">
        ${codexPlusSkinButton("save", "保存", 'data-primary="true"', "save")}
        ${codexPlusSkinButton("save-apply", "保存并应用")}
        <button type="button" class="codex-plus-action-button" data-skin-action="edit-cancel">取消</button>
      </div>
    </section>`;
  }

  function codexPlusSkinRender() {
    const state = codexPlusSkinState;
    const root = state.root;
    if (!root?.isConnected) return;
    const data = state.data;
    if (!data) {
      root.innerHTML = `<div class="codex-plus-settings-card">${codexPlusSkinRow(state.status || "正在读取皮肤…", "", state.status ? '<button type="button" class="codex-plus-action-button" data-skin-action="reload">重试</button>' : "")}</div>`;
      return;
    }
    const enabled = data.enabled === true && data.paused !== true;
    const activeTheme = (data.themes || []).find((theme) => theme.active);
    const stateText = data.enhancementsEnabled === false
      ? "Codex 增强总开关已关闭，皮肤不会生效；先在上面的「增强」里打开。"
      : enabled ? `正在使用「${escapeHtml(activeTheme?.name || data.activeName || "未命名主题")}」。` : "皮肤已关闭，Codex 显示原始外观。";
    const banner = state.status || state.pendingRestart
      ? `<div class="codex-plus-skin-banner" data-tone="${escapeHtml(state.pendingRestart && !state.status ? "warn" : state.tone || "info")}" role="${state.tone === "error" ? "alert" : "status"}">
          <span>${escapeHtml(state.status || "皮肤设置已保存，重启 Codex 后完整生效。")}</span>
          ${state.pendingRestart ? codexPlusSkinButton("restart", "重启 Codex", "", "restart") : ""}
        </div>`
      : "";
    const checks = state.checks
      ? (state.checks.checks || []).map((check) => `<div class="codex-plus-skin-check" data-level="${escapeHtml(check.level)}"><strong>${escapeHtml(check.label)}</strong>${escapeHtml(check.message)}</div>`).join("") || escapeHtml(state.checks.message || "")
      : "确认 Codex 里的皮肤有没有装上、素材有没有加载。";
    const counts = { local: (data.themes || []).length, community: state.community?.items?.length, market: state.market?.themes?.length };
    const tabs = `<div class="codex-plus-skin-toolbar">
      <div class="codex-plus-provider-segmented" role="tablist" aria-label="主题来源">${[["local", "我的主题"], ["community", "DreamSkin 社区"], ["market", "主题市场"]].map(([id, text]) =>
        `<button type="button" role="tab" aria-checked="${String(state.tab === id)}" aria-selected="${String(state.tab === id)}" data-skin-tab="${id}">${text}${counts[id] != null ? ` · ${counts[id]}` : ""}</button>`).join("")}</div>
      <div class="codex-plus-skin-toolbar-actions">${state.tab === "local"
        ? `<label class="codex-plus-action-button codex-plus-skin-file">从图片或视频新建<input type="file" accept="${codexPlusSkinMediaAccept}" data-skin-file="create" hidden></label>
           <label class="codex-plus-action-button codex-plus-skin-file">导入主题包<input type="file" accept=".zip" data-skin-file="import" hidden></label>`
        : codexPlusSkinButton("catalog-refresh", "刷新", `data-kind="${state.tab}"`, `catalog:${state.tab}`)}</div>
    </div>`;
    const body = state.tab === "local" ? codexPlusSkinLocalCards() : codexPlusSkinCatalogCards(state.tab);
    const scroll = root.closest(".codex-plus-modal-body")?.scrollTop;
    root.innerHTML = `<div class="codex-plus-skin">
      ${banner}
      <div class="codex-plus-settings-card">
        ${codexPlusSkinRow("启用皮肤", stateText, `<button type="button" class="codex-plus-toggle" role="switch" aria-label="启用皮肤" aria-checked="${String(enabled)}" data-enabled="${String(enabled)}" data-pending="${String(Boolean(state.busy))}" data-skin-action="toggle" ${state.busy ? "disabled" : ""}><span></span></button>`)}
        ${codexPlusSkinTransparencyRow()}
        ${codexPlusSkinRow("恢复 Codex 原始外观", "关掉皮肤并清掉当前窗口里的皮肤，主题库不受影响。", codexPlusSkinButton("restore", "恢复", "", "restore"))}
        ${codexPlusSkinRow("检查运行状态", checks, codexPlusSkinButton("check", "检查", "", "status"))}
      </div>
      ${tabs}
      ${body}
      ${codexPlusSkinEditor()}
    </div>`;
    if (scroll != null) root.closest(".codex-plus-modal-body").scrollTop = scroll;
  }

  /** 编辑区的输入：就地写回草稿不重绘（重绘会丢焦点），色块跟着变。 */
  function codexPlusSkinHandleInput(event) {
    const target = event.target;
    // 拖动「壁纸透出」只改旁边的数字，不重绘（重绘会打断拖动）；松手的 change 才提交。
    if (target?.dataset && "skinTransparency" in target.dataset) {
      const text = codexPlusSkinTransparencyText(codexPlusSkinTransparencyValue(Number(target.value)));
      target.dataset.follow = "false";
      target.setAttribute("aria-valuetext", text);
      const label = target.parentElement?.querySelector("[data-skin-transparency-value]");
      if (label) label.textContent = text;
      return;
    }
    const editing = codexPlusSkinState.editing;
    const field = target?.dataset?.skinField;
    if (!editing || !field) return;
    const value = target.value;
    const config = editing.config;
    if (field.startsWith("colors.")) {
      config.colors = { ...(config.colors || {}), [field.slice(7)]: value };
    } else if (field === "palette.accent") {
      const palette = { ...(config.palette || {}) };
      if (value.trim()) palette.accent = value;
      else delete palette.accent;
      if (Object.keys(palette).length) config.palette = palette;
      else delete config.palette;
    } else if (field.startsWith("companion.")) {
      const number = Number.parseInt(value, 10);
      if (!Number.isFinite(number)) return;
      // 范围与皮肤运行时一致：宽 48–160，水平偏移 ±48，垂直偏移 ±160。
      const key = field.slice(10);
      const [min, max] = { width: [48, 160], offsetX: [-48, 48], offsetY: [-160, 160] }[key] || [number, number];
      codexPlusSkinUpdateCompanion({ [key]: Math.max(min, Math.min(max, number)) });
    } else {
      config[field] = value;
    }
    editing.dirty = true;
    const swatch = target.closest(".codex-plus-skin-color")?.querySelector(".codex-plus-skin-swatch");
    if (swatch) swatch.style.background = value || "transparent";
  }

  async function codexPlusSkinHandleChange(event) {
    const target = event.target;
    if (target?.dataset && "skinTransparency" in target.dataset) return codexPlusSkinSetTransparency(Number(target.value));
    const kind = target?.dataset?.skinFile;
    if (!kind) return;
    const file = target.files?.[0];
    target.value = "";
    if (kind === "companion") return codexPlusSkinChooseCompanion(file);
    await codexPlusSkinUpload(kind, file);
  }

  async function codexPlusSkinHandleClick(event) {
    const target = event.target instanceof Element ? event.target : null;
    const state = codexPlusSkinState;
    if (!target) return;
    const tab = target.closest("[data-skin-tab]");
    if (tab) {
      state.tab = tab.dataset.skinTab;
      codexPlusSkinRender();
      if (state.tab !== "local" && !state[state.tab] && !state.busy) void codexPlusSkinLoadCatalog(state.tab);
      return;
    }
    const choice = target.closest("[data-skin-choice]");
    if (choice && state.editing) {
      const field = choice.dataset.skinChoice;
      if (field === "appearance") state.editing.config.appearance = choice.dataset.value;
      else if (field === "companion.side") codexPlusSkinUpdateCompanion({ side: choice.dataset.value });
      state.editing.dirty = true;
      return codexPlusSkinRender();
    }
    const button = target.closest("[data-skin-action]");
    if (!button || button.disabled) return;
    const theme = (state.data?.themes || []).find((item) => item.key === button.dataset.key);
    switch (button.dataset.skinAction) {
      case "reload": return codexPlusSkinLoad();
      case "toggle": {
        if (state.data?.enabled === true && state.data?.paused !== true) return codexPlusSkinRestore();
        const active = (state.data?.themes || []).find((item) => item.active) || state.data?.themes?.[0];
        return active ? codexPlusSkinActivate(active.key) : undefined;
      }
      case "restore": return codexPlusSkinRestore();
      case "check": return codexPlusSkinCheck();
      case "transparency-follow": return codexPlusSkinSetTransparency(null);
      case "restart": {
        const result = await codexPlusSkinRun("restart", "restart", {});
        if (result?.status === "accepted" || result?.status === "ok") codexPlusSkinSetStatus("正在重启 Codex…", "ok");
        return codexPlusSkinRender();
      }
      case "activate": return theme ? codexPlusSkinActivate(theme.key) : undefined;
      case "edit": return theme ? codexPlusSkinEdit(theme) : undefined;
      case "rename": return theme ? codexPlusSkinRename(theme) : undefined;
      case "delete": return theme ? codexPlusSkinDelete(theme) : undefined;
      case "install": return codexPlusSkinInstall(button.dataset.kind, button.dataset.id);
      case "catalog-refresh": return codexPlusSkinLoadCatalog(button.dataset.kind);
      case "save": return codexPlusSkinSave(false);
      case "save-apply": return codexPlusSkinSave(true);
      case "edit-cancel":
        state.editing = null;
        return codexPlusSkinRender();
      case "companion-clear":
        delete state.editing.config.companion;
        state.editing.dirty = true;
        return codexPlusSkinRender();
      case "companion-toggle":
        codexPlusSkinUpdateCompanion({ enabled: state.editing.config.companion?.enabled === false });
        return codexPlusSkinRender();
      default:
        return undefined;
    }
  }

  function codexPlusDreamSkinPageRender({ container }) {
    const state = codexPlusSkinState;
    state.root = container;
    state.editing = null;
    const onClick = (event) => { void codexPlusSkinHandleClick(event); };
    const onChange = (event) => { void codexPlusSkinHandleChange(event); };
    container.addEventListener("click", onClick);
    container.addEventListener("input", codexPlusSkinHandleInput);
    container.addEventListener("change", onChange);
    codexPlusSkinRender();
    void codexPlusSkinLoad();
    return () => {
      container.removeEventListener("click", onClick);
      container.removeEventListener("input", codexPlusSkinHandleInput);
      container.removeEventListener("change", onChange);
      if (state.root === container) state.root = null;
    };
  }
