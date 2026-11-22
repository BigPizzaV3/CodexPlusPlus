  /**
   * Codex 左侧图标栏的「微信连接」页：不离开 Codex 就能扫码登录、启停微信连接、改常用设置。
   *
   * 分工：
   *   - 连接本体由管理器进程托管（扫码会话、运行中的连接都在它内存里），不需要 Codex 开着也能用。
   *     本页经 bridge 路由 `/weixin-connect` `{ action, ...args }` 调管理器登记的宿主路由，
   *     动作与管理器微信页共用同一组命令函数（`weixin_connect_bridge.rs`），两边看到的是同一个状态。
   *   - 路由不在拓展白名单里（能扫码登录、启停连接、改设置）。管理器没在跑时路由返回 failed + 提示，本页照实显示。
   *   - 只放每天会碰的设置；新会话项目、重置联系人会话、登录凭据、连接参数到管理器里改（页底有入口）。
   *
   * 页面 DOM 每次打开重建，状态只放在 codexPlusWeixinPageState 里（见 01-registry 的生命周期约定）。
   */
  const codexPlusWeixinStateLabels = {
    starting: "正在启动",
    running: "运行中",
    retrying: "正在重试",
    stopping: "正在停止",
    error: "异常",
    stopped: "已停止",
  };
  const codexPlusWeixinSandboxOptions = [["read-only", "只读"], ["workspace-write", "允许修改工作目录"], ["danger-full-access", "完全访问"]];
  const codexPlusWeixinModeOptions = [["jiajia", "加加"], ["direct", "直连"]];
  const codexPlusWeixinPageState = {
    data: null,
    catalog: null,
    catalogError: "",
    catalogLoading: false,
    qr: null,
    qrGeneration: 0,
    busy: "",
    error: "",
    notice: "",
    copied: false,
    renderPending: false,
    pollTimer: null,
  };

  async function codexPlusWeixinRequest(action, args = {}) {
    const result = await postJson("/weixin-connect", { action, ...args });
    if (result?.status !== "ok") throw new Error(result?.message || "微信连接操作失败");
    return result;
  }

  function codexPlusWeixinNotice(message) {
    const state = codexPlusWeixinPageState;
    state.notice = message;
    setTimeout(() => {
      if (state.notice !== message) return;
      state.notice = "";
      renderCodexPlusWeixinPage();
    }, 4000);
  }

  /** 跑一个会改状态的动作：期间按钮置灰，失败时显示原因并重读一次状态。 */
  async function codexPlusWeixinRun(action, args = {}, busy = action) {
    const state = codexPlusWeixinPageState;
    state.busy = busy;
    state.error = "";
    renderCodexPlusWeixinPage();
    try {
      const result = await codexPlusWeixinRequest(action, args);
      if (result.settings) state.data = result;
      // 二维码本身就是结果，不再弹「已生成」。
      if (result.message && action !== "state" && action !== "qrStart") codexPlusWeixinNotice(result.message);
      return result;
    } catch (error) {
      state.error = error?.message || String(error);
      try {
        state.data = await codexPlusWeixinRequest("state");
      } catch {}
      return null;
    } finally {
      state.busy = "";
      renderCodexPlusWeixinPage();
    }
  }

  /** 改一项设置（输入框失焦 / 回车、开关、下拉选中时）：只提交改动的那几项，后端按白名单合并。 */
  function codexPlusWeixinSave(patch) {
    return codexPlusWeixinRun("save", { settings: patch }, "save");
  }

  async function codexPlusWeixinLoadCatalog() {
    const state = codexPlusWeixinPageState;
    if (state.catalogLoading) return;
    state.catalogLoading = true;
    renderCodexPlusWeixinPage();
    try {
      const result = await codexPlusWeixinRequest("catalog");
      const catalog = result.catalog || {};
      if (catalog.status === "failed" || !Array.isArray(catalog.models)) throw new Error(catalog.message || "读取模型目录失败。");
      state.catalog = catalog;
      state.catalogError = "";
    } catch (error) {
      state.catalogError = error?.message || String(error);
    } finally {
      state.catalogLoading = false;
      renderCodexPlusWeixinPage();
    }
  }

  /** 扫码：生成二维码后一直查状态，直到确认、过期、出错或关页（qrGeneration 变了就停）。 */
  async function codexPlusWeixinStartQr() {
    const state = codexPlusWeixinPageState;
    const generation = ++state.qrGeneration;
    state.qr = null;
    const result = await codexPlusWeixinRun("qrStart", {}, "qr");
    if (!result || generation !== state.qrGeneration) return;
    state.qr = result;
    renderCodexPlusWeixinPage();
    while (generation === state.qrGeneration && ["", "wait", "scaned"].includes(state.qr?.qrStatus || "")) {
      await new Promise((resolve) => setTimeout(resolve, 1000));
      if (generation !== state.qrGeneration || !document.querySelector("[data-codex-weixin-panel]")) return;
      let next;
      try {
        next = await codexPlusWeixinRequest("qrStatus");
      } catch (error) {
        if (generation !== state.qrGeneration) return;
        state.qr = null;
        state.error = error?.message || String(error);
        renderCodexPlusWeixinPage();
        return;
      }
      if (generation !== state.qrGeneration) return;
      if (next.qrStatus === "confirmed") {
        state.qr = null;
        codexPlusWeixinNotice(next.message || "微信扫码登录成功，正在启动连接。");
        try {
          state.data = await codexPlusWeixinRequest("state");
        } catch {}
        renderCodexPlusWeixinPage();
        return;
      }
      if (next.qrStatus === "expired") {
        state.qr = null;
        state.error = "二维码已过期，请重新扫码登录。";
        renderCodexPlusWeixinPage();
        return;
      }
      const changed = next.qrStatus !== state.qr?.qrStatus;
      state.qr = next;
      if (changed) renderCodexPlusWeixinPage();
    }
  }

  function codexPlusWeixinCancelQr() {
    const state = codexPlusWeixinPageState;
    state.qrGeneration += 1;
    state.qr = null;
    renderCodexPlusWeixinPage();
  }

  function codexPlusWeixinRow(title, description, control = "", extra = "") {
    return `<div class="codex-plus-row"${extra}>
      <div><div class="codex-plus-row-title">${title}</div>${description ? `<div class="codex-plus-row-description">${description}</div>` : ""}</div>
      ${control ? `<div class="codex-plus-weixin-controls">${control}</div>` : ""}
    </div>`;
  }

  function codexPlusWeixinToggle(field, on, label, disabled = false) {
    const off = disabled || Boolean(codexPlusWeixinPageState.busy);
    return `<button type="button" class="codex-plus-toggle" role="switch" aria-label="${escapeHtml(label)}" aria-checked="${String(on)}" data-enabled="${String(on)}" data-pending="${String(Boolean(codexPlusWeixinPageState.busy))}" data-codex-weixin-toggle="${field}" ${off ? "disabled" : ""}><span></span></button>`;
  }

  function codexPlusWeixinInput(field, value, label, placeholder = "") {
    return `<input class="codex-plus-provider-input codex-plus-service-input codex-plus-weixin-input" type="text" data-codex-weixin-field="${field}" value="${escapeHtml(value || "")}" placeholder="${escapeHtml(placeholder)}" aria-label="${escapeHtml(label)}" title="${escapeHtml(value || "")}" spellcheck="false" autocomplete="off">`;
  }

  /**
   * 和 Codex 原生一样的下拉：胶囊按钮 + 自绘菜单（复用 10-style 的 .codex-plus-menu-*，与供应商页同一套）。
   * 不用原生 select：它的弹出层不跟主题。选中即保存（field 是设置键）。
   */
  function codexPlusWeixinMenu(field, value, options, label) {
    const current = options.find(([id]) => id === value) || options[0] || ["", ""];
    const busy = Boolean(codexPlusWeixinPageState.busy);
    return `<div class="codex-plus-weixin-menu"><button type="button" class="codex-plus-menu-trigger" data-codex-weixin-menu="${escapeHtml(field)}" data-value="${escapeHtml(current[0])}" data-options="${escapeHtml(JSON.stringify(options))}" data-menu-label="${escapeHtml(label)}" aria-haspopup="menu" aria-expanded="false" aria-label="${escapeHtml(label)}：${escapeHtml(current[1])}" title="${escapeHtml(current[1])}" ${busy ? "disabled" : ""}><span class="codex-plus-weixin-menu-label">${escapeHtml(current[1])}</span>${codexPlusMenuIcons.chevron}</button></div>`;
  }

  function codexPlusWeixinOpenMenu(trigger) {
    codexPlusWeixinCloseMenu();
    let options = [];
    try {
      options = JSON.parse(trigger.dataset.options || "[]");
    } catch {}
    const value = trigger.dataset.value || "";
    trigger.setAttribute("aria-expanded", "true");
    trigger.insertAdjacentHTML("afterend", `<div class="codex-plus-menu-popover codex-plus-weixin-menu-popover" role="menu" aria-label="${escapeHtml(trigger.dataset.menuLabel || "")}">${options
      .map(([id, text]) => `<button type="button" class="codex-plus-menu-item" role="menuitemradio" aria-checked="${String(id === value)}" data-codex-weixin-menu-value="${escapeHtml(id)}" title="${escapeHtml(text)}"><span class="codex-plus-menu-item-label">${escapeHtml(text)}</span>${id === value ? codexPlusMenuIcons.check : ""}</button>`)
      .join("")}</div>`);
    window.addEventListener("pointerdown", codexPlusWeixinMenuPointerdown, true);
    const menu = trigger.nextElementSibling;
    (menu?.querySelector('[aria-checked="true"]') || menu?.querySelector(".codex-plus-menu-item"))?.focus({ preventScroll: true });
  }

  function codexPlusWeixinCloseMenu(refocus = false) {
    window.removeEventListener("pointerdown", codexPlusWeixinMenuPointerdown, true);
    const menu = document.querySelector(".codex-plus-weixin-menu-popover");
    if (!menu) return;
    const trigger = menu.previousElementSibling;
    menu.remove();
    trigger?.setAttribute("aria-expanded", "false");
    if (refocus) trigger?.focus({ preventScroll: true });
    if (codexPlusWeixinPageState.renderPending) renderCodexPlusWeixinPage();
  }

  function codexPlusWeixinMenuPointerdown(event) {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest(".codex-plus-weixin-menu")) return;
    codexPlusWeixinCloseMenu();
  }

  function codexPlusWeixinChooseMenuItem(item) {
    const trigger = item.closest(".codex-plus-weixin-menu-popover")?.previousElementSibling;
    if (!trigger) return;
    const value = item.dataset.codexWeixinMenuValue || "";
    const changed = value !== (trigger.dataset.value || "");
    codexPlusWeixinCloseMenu(true);
    if (changed) void codexPlusWeixinSave({ [trigger.dataset.codexWeixinMenu]: value });
  }

  /** 菜单键盘操作：上下移动、Home / End、回车 / 空格选中、Esc / Tab 收起（与供应商页菜单一致）。 */
  function codexPlusWeixinMenuKeydown(event, menu) {
    const items = Array.from(menu.querySelectorAll(".codex-plus-menu-item"));
    const index = items.indexOf(document.activeElement);
    let next = null;
    if (event.key === "ArrowDown") next = items[(index + 1) % items.length];
    else if (event.key === "ArrowUp") next = items[(index - 1 + items.length) % items.length];
    else if (event.key === "Home") next = items[0];
    else if (event.key === "End") next = items[items.length - 1];
    else if (event.key === "Enter" || event.key === " ") {
      if (index < 0) return;
      event.preventDefault();
      codexPlusWeixinChooseMenuItem(items[index]);
      return;
    } else if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      codexPlusWeixinCloseMenu(event.key === "Escape");
      return;
    } else {
      return;
    }
    event.preventDefault();
    next?.focus({ preventScroll: true });
  }

  /**
   * Codex 在 document 捕获阶段把按键送进它的输入框（页面盖在上面，字会悄悄进 composer），
   * 所以按键在 window 捕获阶段先接：本页内或焦点在 body 时拦下，自己处理菜单导航和输入框回车。
   * 不在输入框里时带 ⌘ / Ctrl / Alt 的组合键放行给 Codex 快捷键。
   */
  function codexPlusWeixinCaptureKeydown(event) {
    const overlay = document.querySelector(".codex-plus-weixin-page");
    if (!overlay) return;
    const target = event.target instanceof Element ? event.target : null;
    const inside = Boolean(target && overlay.contains(target));
    const idle = event.target === document.body || event.target === document.documentElement;
    if (!inside && !idle) return;
    const typing = Boolean(target?.matches("input, textarea"));
    if (typing || !(event.metaKey || event.ctrlKey || event.altKey)) event.stopPropagation();
    const menu = document.querySelector(".codex-plus-weixin-menu-popover");
    if (menu) {
      codexPlusWeixinMenuKeydown(event, menu);
      return;
    }
    if (typing && event.key === "Enter") {
      event.preventDefault();
      target.blur();
    } else if (typing && event.key === "Escape") {
      // 放弃这次输入：恢复成已保存的值。
      const saved = codexPlusWeixinPageState.data?.settings?.[target.getAttribute("data-codex-weixin-field")];
      if (typeof saved === "string") target.value = saved;
      target.blur();
    }
  }

  /** 输入框改完（失焦、回车）才保存；没变就不提交。 */
  function codexPlusWeixinHandleChange(event) {
    const input = event.target instanceof Element ? event.target.closest("[data-codex-weixin-field]") : null;
    if (!input) return;
    const field = input.getAttribute("data-codex-weixin-field");
    const value = input.value.trim();
    const saved = codexPlusWeixinPageState.data?.settings?.[field] ?? "";
    if (value === saved) return;
    void codexPlusWeixinSave({ [field]: value });
  }

  function codexPlusWeixinModelOptions(state, settings) {
    const catalog = state.catalog;
    const models = Array.isArray(catalog?.models) ? catalog.models : [];
    const metadata = catalog?.modelMetadata || {};
    const inherited = catalog?.model || catalog?.default_model || "";
    const model = settings.weixinConnectModel || "";
    const display = (slug) => metadata[slug]?.displayName || slug;
    const modelOptions = [
      ["", `继承 Codex 默认模型${inherited ? ` (${display(inherited)})` : ""}`],
      ...(model && !models.includes(model) ? [[model, `${model} (${catalog && !state.catalogError ? "不可用" : "当前值"})`]] : []),
      ...models.map((slug) => [slug, display(slug)]),
    ];
    const effective = model || inherited;
    const efforts = Array.from(new Set((metadata[effective]?.supportedReasoningEfforts || [])
      .map((entry) => entry?.reasoningEffort)
      .filter(Boolean)));
    const effort = settings.weixinConnectReasoningEffort || "";
    const effortOptions = [
      ["", "继承推理强度"],
      ...(effort && !efforts.includes(effort) ? [[effort, `${effort} (${catalog && !state.catalogError ? "不可用" : "当前值"})`]] : []),
      ...efforts.map((value) => [value, value]),
    ];
    const hint = state.catalogError
      || (catalog && model && !models.includes(model) ? "当前模型已不在可用模型目录中，请重新选择。" : "")
      || (catalog && effort && !efforts.includes(effort) ? "当前模型不支持已选推理强度，请重新选择或继承。" : "")
      || (catalog && !models.length ? "当前没有可用模型，请检查供应商配置。" : "")
      || "微信里的新会话用这个模型；推理强度跟着模型走。";
    return { modelOptions, effortOptions, hint };
  }

  /** 整页 HTML（纯函数，便于测试）。 */
  function codexPlusWeixinPageHtml(state) {
    const data = state.data;
    const busy = Boolean(state.busy);
    const header = `<div class="codex-plus-modal-header"><div class="codex-plus-modal-title">微信连接</div></div>`;
    const bannerText = state.error || (data?.connection?.state === "error" ? data.connection.message : "");
    const banner = [
      bannerText ? `<div class="codex-plus-remote-banner" data-tone="warn" role="alert">${escapeHtml(bannerText)}</div>` : "",
      state.notice ? `<div class="codex-plus-remote-banner" data-tone="ok" role="status">${escapeHtml(state.notice)}</div>` : "",
    ].join("");
    if (!data) {
      return `${header}${banner}<div class="codex-plus-provider-empty">${busy ? "正在读取…" : '<button type="button" class="codex-plus-action-button" data-codex-weixin-action="reload">重试</button>'}</div>`;
    }
    const settings = data.settings || {};
    const connection = data.connection || {};
    const runtime = connection.state || "stopped";
    const running = ["starting", "running", "retrying"].includes(runtime);
    const stopping = runtime === "stopping";
    const loggedIn = Boolean(settings.loggedIn);
    const account = connection.accountId || settings.weixinConnectAccountId || "未登录";
    const peer = connection.lastPeerId || "暂无";
    const message = connection.message || "微信连接未启动。";
    const tone = runtime === "running" ? "active" : runtime === "error" ? "off" : "";
    const primary = !loggedIn
      ? `<button type="button" class="codex-plus-action-button" data-primary="true" data-codex-weixin-action="qrStart" ${busy ? "disabled" : ""}>${state.busy === "qr" ? "正在生成…" : "扫码登录"}</button>`
      : running || stopping
        ? `<button type="button" class="codex-plus-action-button" data-codex-weixin-action="stop" ${busy || stopping ? "disabled" : ""}>${stopping || state.busy === "stop" ? "正在停止" : "停止"}</button>`
        : `<button type="button" class="codex-plus-action-button" data-primary="true" data-codex-weixin-action="start" ${busy ? "disabled" : ""}>${state.busy === "start" ? "正在启动" : "启动"}</button>`;
    const relogin = loggedIn ? `<button type="button" class="codex-plus-action-button" data-codex-weixin-action="qrStart" ${busy ? "disabled" : ""}>重新登录</button>` : "";
    const ellipsis = (label, value) => `<span class="codex-plus-weixin-meta">${label} <code class="codex-plus-weixin-ellipsis" title="${escapeHtml(value)}">${escapeHtml(value)}</code></span>`;
    const statusSection = `<section class="codex-plus-settings-section" aria-label="连接">
      <h2 class="codex-plus-settings-section-title">连接</h2>
      <div class="codex-plus-settings-card">
        ${codexPlusWeixinRow("个人微信连接",
          `<span class="codex-plus-weixin-ellipsis codex-plus-weixin-status-message" title="${escapeHtml(message)}">${escapeHtml(message)}</span>
          <span class="codex-plus-weixin-meta-line">${ellipsis("账号", account)}<span class="codex-plus-weixin-meta">已处理消息 <b>${Number(connection.processedMessages) || 0}</b></span>${ellipsis("最近联系人", peer)}</span>`,
          `<span class="codex-plus-provider-badge" data-tone="${tone}" data-codex-weixin-state="${escapeHtml(runtime)}">${escapeHtml(codexPlusWeixinStateLabels[runtime] || runtime)}</span>${primary}${relogin}`)}
        ${codexPlusWeixinRow("在手机上用", "微信连接由 Codex++ 管理工具托管：Codex 没开着也能在微信里发消息给 Codex。")}
      </div>
    </section>`;
    const qr = state.qr;
    const qrSection = qr?.qrContent ? `<section class="codex-plus-settings-section" aria-label="微信扫码登录">
      <h2 class="codex-plus-settings-section-title">${qr.qrStatus === "scaned" ? "已扫码，请在手机上确认" : "微信扫码登录"}</h2>
      <div class="codex-plus-settings-card codex-plus-weixin-qr-card">
        ${qr.qrSvg ? `<div class="codex-plus-remote-qr codex-plus-weixin-qr" data-codex-weixin-qr="true">${qr.qrSvg}</div>` : ""}
        <div class="codex-plus-weixin-qr-side">
          <div class="codex-plus-row-description">用手机微信扫码，或在手机微信里打开登录链接完成确认。</div>
          <code class="codex-plus-weixin-qr-link" title="${escapeHtml(qr.qrContent)}">${escapeHtml(qr.qrContent)}</code>
          <div class="codex-plus-weixin-controls">
            <button type="button" class="codex-plus-action-button" data-codex-weixin-action="openQr">打开登录链接</button>
            <button type="button" class="codex-plus-action-button" data-codex-weixin-action="copyQr">${state.copied ? "已复制" : "复制链接"}</button>
            <button type="button" class="codex-plus-action-button" data-codex-weixin-action="cancelQr">取消</button>
          </div>
        </div>
      </div>
    </section>` : "";
    const { modelOptions, effortOptions, hint } = codexPlusWeixinModelOptions(state, settings);
    const sessionSection = `<section class="codex-plus-settings-section" aria-label="对话">
      <h2 class="codex-plus-settings-section-title">对话</h2>
      <div class="codex-plus-settings-card">
        ${codexPlusWeixinRow("允许的微信用户 ID", "每个微信联系人会映射到独立的 Codex 会话；多个用逗号分隔。",
          codexPlusWeixinInput("weixinConnectAllowFrom", settings.weixinConnectAllowFrom, "允许的微信用户 ID", "user@im.wechat"))}
        ${codexPlusWeixinRow("工作目录", "新会话在这个目录里工作。",
          codexPlusWeixinInput("weixinConnectWorkDir", settings.weixinConnectWorkDir, "工作目录", "输入工作目录的完整路径"))}
        ${codexPlusWeixinRow("模型", escapeHtml(hint),
          `${codexPlusWeixinMenu("weixinConnectModel", settings.weixinConnectModel || "", modelOptions, "模型")}${codexPlusWeixinMenu("weixinConnectReasoningEffort", settings.weixinConnectReasoningEffort || "", effortOptions, "推理强度")}<button type="button" class="codex-plus-action-button" data-codex-weixin-action="catalog" ${state.catalogLoading ? "disabled" : ""}>${state.catalogLoading ? "读取中…" : "刷新"}</button>`)}
      </div>
    </section>`;
    const remoteRunning = Boolean(data.remoteControlRunning);
    const jiajiaSection = `<section class="codex-plus-settings-section" aria-label="加加">
      <h2 class="codex-plus-settings-section-title">加加<span class="codex-plus-provider-badge codex-plus-weixin-title-badge" data-tone="${remoteRunning ? "active" : ""}" data-codex-weixin-remote="${String(remoteRunning)}">${remoteRunning ? "远程控制已开" : "远程控制未开"}</span></h2>
      <div class="codex-plus-settings-card">
        ${codexPlusWeixinRow("微信默认模式", remoteRunning
          ? "没单独选过的联系人用这个模式；在微信里发 /加加 或 /直连 可单独切换。"
          : "加加需要先打开远程控制；没开时消息先按直连处理。",
          `${remoteRunning ? "" : '<button type="button" class="codex-plus-action-button" data-codex-weixin-action="openRemote">打开远程控制</button>'}${codexPlusWeixinMenu("weixinConnectDefaultMode", settings.weixinConnectDefaultMode || "jiajia", codexPlusWeixinModeOptions, "微信默认模式")}`)}
        ${codexPlusWeixinRow("推送到微信", "没在对话时，加加的审批卡片和提醒也发到微信；回复数字即可审批。",
          codexPlusWeixinToggle("weixinConnectPushToWeixin", Boolean(settings.weixinConnectPushToWeixin), "推送到微信"))}
      </div>
    </section>`;
    const allowFrom = String(settings.weixinConnectAllowFrom || "").split(",").map((peerId) => peerId.trim());
    const outboundOn = Boolean(settings.weixinConnectAllowOutboundAttachments);
    const outboundBlocked = !outboundOn && (!allowFrom.some(Boolean) || allowFrom.includes("*") || !String(settings.weixinConnectWorkDir || "").trim());
    const permissionSection = `<section class="codex-plus-settings-section" aria-label="权限">
      <h2 class="codex-plus-settings-section-title">权限</h2>
      <div class="codex-plus-settings-card">
        ${codexPlusWeixinRow("沙箱权限", "Codex 在工作目录里能做什么。",
          codexPlusWeixinMenu("weixinConnectSandbox", settings.weixinConnectSandbox || "read-only", codexPlusWeixinSandboxOptions, "沙箱权限"))}
        ${codexPlusWeixinRow("工作区附件回传", outboundBlocked ? "先填允许的微信用户 ID（不能是 *）并填好工作目录，才能打开。" : "允许向指定微信用户回传工作区文件。",
          codexPlusWeixinToggle("weixinConnectAllowOutboundAttachments", outboundOn, "允许向指定微信用户回传工作区文件", outboundBlocked))}
      </div>
    </section>`;
    const moreSection = `<section class="codex-plus-settings-section" aria-label="更多设置">
      <h2 class="codex-plus-settings-section-title">更多设置</h2>
      <div class="codex-plus-settings-card">
        ${codexPlusWeixinRow("新会话项目、重置联系人会话、登录凭据、连接参数", "这些不常用的设置在 Codex++ 管理工具的微信连接页里改。",
          '<button type="button" class="codex-plus-action-button" data-codex-weixin-action="openManager">在 Codex++ 管理工具里打开更多设置</button>')}
      </div>
    </section>`;
    return `${header}${banner}${statusSection}${qrSection}${sessionSection}${jiajiaSection}${permissionSection}${moreSection}`;
  }

  /** 正在编辑输入框或开着菜单时不重绘（会丢输入、收掉菜单），等编辑完再补画。 */
  function codexPlusWeixinEditing(panel) {
    const active = document.activeElement;
    return Boolean(panel.querySelector(".codex-plus-weixin-menu-popover")
      || (active instanceof Element && panel.contains(active) && active.matches("[data-codex-weixin-field]")));
  }

  function renderCodexPlusWeixinPage(force = false) {
    const panel = document.querySelector("[data-codex-weixin-panel]");
    if (!panel) return;
    const state = codexPlusWeixinPageState;
    // 保存期间用户已经点进了另一个输入框：照常重绘，但把焦点、光标和没提交的内容放回去。
    const active = document.activeElement;
    const field = active instanceof Element && panel.contains(active) ? active.getAttribute("data-codex-weixin-field") : null;
    if (!force && panel.querySelector(".codex-plus-weixin-menu-popover")) {
      state.renderPending = true;
      return;
    }
    state.renderPending = false;
    const draft = field ? { value: active.value, start: active.selectionStart, end: active.selectionEnd } : null;
    const body = panel.closest(".codex-plus-modal-body");
    const scroll = body?.scrollTop || 0;
    panel.innerHTML = codexPlusWeixinPageHtml(state);
    if (body) body.scrollTop = scroll;
    if (field && draft) {
      const next = panel.querySelector(`[data-codex-weixin-field="${field}"]`);
      if (next) {
        next.value = draft.value;
        next.focus({ preventScroll: true });
        try {
          next.setSelectionRange(draft.start, draft.end);
        } catch {}
      }
    }
  }

  async function codexPlusWeixinHandleClick(event) {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    if (!target) return;
    const item = target.closest(".codex-plus-weixin-menu-popover .codex-plus-menu-item");
    if (item) {
      codexPlusWeixinChooseMenuItem(item);
      return;
    }
    const trigger = target.closest("[data-codex-weixin-menu]");
    if (trigger) {
      if (trigger.disabled) return;
      if (trigger.getAttribute("aria-expanded") === "true") codexPlusWeixinCloseMenu();
      else codexPlusWeixinOpenMenu(trigger);
      return;
    }
    const toggle = target.closest("[data-codex-weixin-toggle]");
    if (toggle) {
      if (toggle.disabled) return;
      const field = toggle.getAttribute("data-codex-weixin-toggle");
      await codexPlusWeixinSave({ [field]: toggle.getAttribute("aria-checked") !== "true" });
      return;
    }
    const button = target.closest("[data-codex-weixin-action]");
    if (!button || button.disabled) return;
    const state = codexPlusWeixinPageState;
    const action = button.getAttribute("data-codex-weixin-action");
    switch (action) {
      case "reload":
        await codexPlusWeixinRun("state", {}, "load");
        return;
      case "qrStart":
        await codexPlusWeixinStartQr();
        return;
      case "cancelQr":
        codexPlusWeixinCancelQr();
        return;
      case "openQr":
        // Electron 里 <a target=_blank> 不会交给系统浏览器，统一走 window.open（同远程控制页）。
        if (state.qr?.qrContent) window.open(state.qr.qrContent, "_blank", "noopener,noreferrer");
        return;
      case "copyQr":
        if (!state.qr?.qrContent) return;
        try {
          await navigator.clipboard.writeText(state.qr.qrContent);
          state.copied = true;
          renderCodexPlusWeixinPage();
          setTimeout(() => { state.copied = false; renderCodexPlusWeixinPage(); }, 1500);
        } catch (error) {
          state.error = `复制失败：${error?.message || error}`;
          renderCodexPlusWeixinPage();
        }
        return;
      case "start":
      case "stop":
        await codexPlusWeixinRun(action);
        return;
      case "catalog":
        await codexPlusWeixinLoadCatalog();
        return;
      case "openRemote":
        openCodexPlusRemoteControlPage();
        return;
      case "openManager": {
        const result = await postJson("/manager/open", { page: "weixin" });
        if (result?.status !== "ok") {
          state.error = result?.message || "打开 Codex++ 管理工具失败";
          renderCodexPlusWeixinPage();
        }
        return;
      }
      default:
    }
  }

  /** 连接状态（启动中 → 运行中、处理了几条消息）、别处（管理器）改了设置：静默刷新。 */
  function codexPlusWeixinPoll() {
    const state = codexPlusWeixinPageState;
    const panel = document.querySelector("[data-codex-weixin-panel]");
    if (!panel || state.busy || document.hidden) return;
    void codexPlusWeixinRequest("state").then((data) => {
      if (state.busy) return;
      const before = JSON.stringify(state.data);
      state.data = data;
      if (JSON.stringify(data) === before) return;
      if (codexPlusWeixinEditing(panel)) state.renderPending = true;
      else renderCodexPlusWeixinPage();
    }).catch(() => {});
  }

  function openCodexPlusWeixinConnectPage() {
    closeCodexPlusPage();
    document.querySelectorAll('.codex-plus-modal-overlay, [data-codex-plus-dialog="true"]').forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = `${codexPlusPageClass} codex-plus-weixin-page`;
    overlay.dataset.codexPlusPage = "true";
    markCodexPlusExtensionNode(overlay, "builtin-weixin-connect");
    applyCodexPlusTheme(overlay);
    applyCodexPlusZoom(overlay);
    overlay.innerHTML = `<div class="codex-plus-modal-content" role="region" aria-label="微信连接" data-codex-plus-active-tab="${codexPlusWeixinConnectTab}">
      <div class="codex-plus-modal-body"><div class="codex-plus-panel codex-plus-settings" data-codex-weixin-panel="true"></div></div></div>`;
    overlay.addEventListener("click", (event) => { void codexPlusWeixinHandleClick(event); });
    overlay.addEventListener("change", codexPlusWeixinHandleChange);
    // 输入框失焦后补画轮询期间攒下的变化（没改内容时不会有 change）。
    overlay.addEventListener("focusout", () => setTimeout(() => {
      const panel = document.querySelector("[data-codex-weixin-panel]");
      if (panel && codexPlusWeixinPageState.renderPending && !codexPlusWeixinEditing(panel)) renderCodexPlusWeixinPage();
    }, 0));
    document.body.appendChild(overlay);
    positionCodexPlusPage(overlay);
    observeCodexPlusPageLayout(overlay);
    document.querySelectorAll('[data-codex-plus-ext-rail-active="true"]').forEach((node) => {
      node.removeAttribute("data-codex-plus-ext-rail-active");
      const button = node.querySelector("button") || node;
      button.dataset.active = "false";
      button.removeAttribute("data-selected");
      button.removeAttribute("aria-current");
    });
    setCodexPlusSidebarNavActive(true, "weixin");
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    window.__codexPlusPageResizeHandler = () => positionCodexPlusPage(document.querySelector(`.${codexPlusPageClass}`));
    window.addEventListener("resize", window.__codexPlusPageResizeHandler);
    const state = codexPlusWeixinPageState;
    clearInterval(state.pollTimer);
    state.pollTimer = setInterval(codexPlusWeixinPoll, 2500);
    window.removeEventListener("keydown", codexPlusWeixinCaptureKeydown, true);
    window.addEventListener("keydown", codexPlusWeixinCaptureKeydown, true);
    // 关页时由 closeCodexPlusPage 调用（92-extension-host.js）。
    window.__codexPlusExtensionPageCleanup = () => {
      clearInterval(state.pollTimer);
      state.pollTimer = null;
      // 停掉扫码轮询；关页前把正在编辑的输入框存掉（节点马上被移除，不会再有 change）。
      state.qrGeneration += 1;
      state.qr = null;
      window.removeEventListener("keydown", codexPlusWeixinCaptureKeydown, true);
      window.removeEventListener("pointerdown", codexPlusWeixinMenuPointerdown, true);
      const editing = document.activeElement;
      if (editing instanceof Element && editing.matches("[data-codex-weixin-field]")) codexPlusWeixinHandleChange({ target: editing });
    };
    Object.assign(state, { error: "", notice: "", copied: false, renderPending: false, qr: null });
    renderCodexPlusWeixinPage();
    void codexPlusWeixinRun("state", {}, "load");
    void codexPlusWeixinLoadCatalog();
  }
