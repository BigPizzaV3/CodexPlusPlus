  /**
   * Codex 左侧图标栏的「远程控制」页：不离开 Codex 就能开关远程控制、开局域网访问、给手机配对、管理设备。
   *
   * 分工：
   *   - 服务本体在 core `remote_control`，由管理器进程托管；本页经 bridge 路由 `/remote-control`
   *     `{ action, ...args }` 调用与管理器同一份 `remote_control::dispatch`，两边看到的是同一个状态。
   *   - 路由不在拓展白名单里（能开关服务、发配对码、移除设备）。
   *   - 管理器没在跑时路由返回 failed + 提示，本页照实显示。
   *
   * 页面 DOM 每次打开重建，状态只放在 codexPlusRemotePageState 里（见 01-registry 的生命周期约定）。
   */
  const codexPlusRemoteRoleLabels = { view: "只能查看", message: "可发消息", full: "完全控制" };
  const codexPlusRemoteRoleHints = {
    view: "看会话和输出",
    message: "还能发消息、插话、停止、新建会话",
    full: "还能审批命令、回答提问、改模型和权限",
  };
  const codexPlusRemotePageState = {
    data: null,
    busy: "",
    error: "",
    notice: "",
    pairRole: "message",
    copied: false,
    pollTimer: null,
    tickTimer: null,
    deviceCount: null,
  };

  async function codexPlusRemoteRequest(action, args = {}) {
    const result = await postJson("/remote-control", { action, ...args });
    if (result?.status !== "ok") throw new Error(result?.message || "远程控制操作失败");
    return result;
  }

  function codexPlusRemoteApply(data) {
    const state = codexPlusRemotePageState;
    const count = Array.isArray(data?.devices) ? data.devices.length : 0;
    if (state.deviceCount != null && count > state.deviceCount) {
      state.notice = `已配对：${data.devices[count - 1]?.name || "新设备"}`;
      setTimeout(() => {
        if (state.notice.startsWith("已配对")) {
          state.notice = "";
          renderCodexPlusRemotePage();
        }
      }, 4000);
    }
    state.deviceCount = count;
    state.data = data;
  }

  async function codexPlusRemoteRun(action, args = {}, busy = action) {
    const state = codexPlusRemotePageState;
    state.busy = busy;
    state.error = "";
    renderCodexPlusRemotePage();
    try {
      codexPlusRemoteApply(await codexPlusRemoteRequest(action, args));
    } catch (error) {
      state.error = error?.message || String(error);
      try {
        codexPlusRemoteApply(await codexPlusRemoteRequest("status"));
      } catch {}
    } finally {
      state.busy = "";
      renderCodexPlusRemotePage();
    }
  }

  function codexPlusRemoteRelative(seconds) {
    if (!seconds) return "从未";
    const diff = Date.now() / 1000 - seconds;
    if (diff < 90) return "刚刚";
    if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
    if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
    return new Date(seconds * 1000).toLocaleDateString();
  }

  function codexPlusRemoteCountdown(expiresAt) {
    const left = Math.max(0, Math.round(Number(expiresAt || 0) - Date.now() / 1000));
    return { left, text: `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}` };
  }

  function codexPlusRemoteToggle(action, on, label) {
    const busy = Boolean(codexPlusRemotePageState.busy);
    return `<button type="button" class="codex-plus-toggle" role="switch" aria-label="${escapeHtml(label)}" aria-checked="${String(on)}" data-enabled="${String(on)}" data-pending="${String(busy)}" data-codex-remote-action="${action}" ${busy ? "disabled" : ""}><span></span></button>`;
  }

  function codexPlusRemoteRow(title, description, control = "", extra = "") {
    return `<div class="codex-plus-row"${extra}>
      <div><div class="codex-plus-row-title">${title}</div>${description ? `<div class="codex-plus-row-description">${description}</div>` : ""}</div>
      ${control ? `<div class="codex-plus-remote-controls">${control}</div>` : ""}
    </div>`;
  }

  /** 三档权限的分段按钮；`target` 是设备 ID，空串表示「添加设备」时选的权限。 */
  function codexPlusRemoteRoleSwitch(current, target = "") {
    const busy = Boolean(codexPlusRemotePageState.busy);
    return `<div class="codex-plus-remote-segmented" role="radiogroup" aria-label="权限">${Object.entries(codexPlusRemoteRoleLabels)
      .map(([role, label]) => `<button type="button" role="radio" aria-checked="${String(role === current)}" data-codex-remote-role="${role}" data-codex-remote-device="${escapeHtml(target)}" title="${escapeHtml(codexPlusRemoteRoleHints[role])}" ${busy ? "disabled" : ""}>${label}</button>`)
      .join("")}</div>`;
  }

  function renderCodexPlusRemotePairing(pairing) {
    if (!pairing) return "";
    const countdown = codexPlusRemoteCountdown(pairing.expiresAt);
    if (countdown.left <= 0) return "";
    const links = Array.isArray(pairing.links) ? pairing.links : [];
    return `<div class="codex-plus-remote-pairing">
      <div class="codex-plus-remote-pairing-head">
        <span>配对码 <strong class="codex-plus-remote-code">${escapeHtml(pairing.code)}</strong> · ${escapeHtml(codexPlusRemoteRoleLabels[pairing.role] || pairing.role)} · <span data-codex-remote-countdown="${Number(pairing.expiresAt)}">${countdown.text}</span></span>
        <button type="button" class="codex-plus-action-button" data-codex-remote-action="cancelPairing">取消</button>
      </div>
      <div class="codex-plus-remote-qrs">${links.map((link) => `<figure>
        ${link.qrSvg ? `<div class="codex-plus-remote-qr">${link.qrSvg}</div>` : ""}
        <figcaption>${link.kind === "tailscale" ? "Tailscale" : "局域网"} · <code>${escapeHtml(link.address)}</code></figcaption>
      </figure>`).join("")}</div>
      <div class="codex-plus-row-description">用手机相机扫码，在打开的页面里确认设备名称即可。扫不了码时，在手机浏览器打开上面的访问地址，再输入配对码。</div>
    </div>`;
  }

  function renderCodexPlusRemotePage() {
    const panel = document.querySelector("[data-codex-remote-panel]");
    if (!panel) return;
    const state = codexPlusRemotePageState;
    const data = state.data;
    const busy = Boolean(state.busy);
    const header = `<div class="codex-plus-modal-header"><div class="codex-plus-modal-title">远程控制<em class="codex-plus-feature-tag" data-tone="experimental">实验</em></div></div>`;
    const banner = [
      state.error || data?.error || data?.lan?.error ? `<div class="codex-plus-remote-banner" data-tone="warn" role="alert">${escapeHtml(state.error || data?.error || data?.lan?.error)}</div>` : "",
      state.notice ? `<div class="codex-plus-remote-banner" data-tone="ok" role="status">${escapeHtml(state.notice)}</div>` : "",
    ].join("");
    if (!data) {
      panel.innerHTML = `${header}${banner}<div class="codex-plus-provider-empty">${state.busy ? "正在读取…" : ""}
        ${state.busy ? "" : '<button type="button" class="codex-plus-action-button" data-codex-remote-action="reload">重试</button>'}</div>`;
      return;
    }
    const lan = data.lan || {};
    const devices = Array.isArray(data.devices) ? data.devices : [];
    const url = data.url || "";
    const main = `<section class="codex-plus-settings-section" aria-label="远程控制">
      <h2 class="codex-plus-settings-section-title">远程控制</h2>
      <div class="codex-plus-settings-card">
        ${codexPlusRemoteRow("启用远程控制", "在浏览器或手机上查看 Codex 的会话、发消息、停止，处理命令审批和模型提问，切换模型、权限和计划模式。",
          `${data.running ? '<span class="codex-plus-provider-badge" data-tone="active">运行中</span>' : ""}${codexPlusRemoteToggle(data.enabled ? "disable" : "enable", Boolean(data.enabled), "启用远程控制")}`)}
        ${url ? codexPlusRemoteRow("本机访问链接", `<code class="codex-plus-remote-url">${escapeHtml(url.replace(/#token=.*/, "#token=…"))}</code><br>链接里带着访问令牌，拿到链接就能操控 Codex，不要发给别人。`,
          `<button type="button" class="codex-plus-action-button" data-codex-remote-action="copy">${state.copied ? "已复制" : "复制"}</button>
           <button type="button" class="codex-plus-action-button" data-codex-remote-action="openBrowser">在浏览器打开</button>`) : ""}
        ${data.enabled ? codexPlusRemoteRow("更换令牌", "之前打开的本机页面会立刻失效；已配对的手机不受影响。",
          `<button type="button" class="codex-plus-action-button" data-codex-remote-action="resetToken" ${busy ? "disabled" : ""}>更换</button>`) : ""}
      </div>
    </section>`;
    const addresses = Array.isArray(lan.addresses) ? lan.addresses : [];
    const phones = data.enabled ? `<section class="codex-plus-settings-section" aria-label="手机与其它设备">
      <h2 class="codex-plus-settings-section-title">手机与其它设备</h2>
      <div class="codex-plus-settings-card">
        ${codexPlusRemoteRow("允许局域网访问", "同一 Wi-Fi 或同一 Tailscale 网络里的设备经 HTTPS 访问，只有配对过的设备能用。",
          `${lan.running ? `<span class="codex-plus-provider-badge" data-tone="active">端口 ${Number(lan.port)}</span>` : ""}${codexPlusRemoteToggle(lan.enabled ? "disableLan" : "enableLan", Boolean(lan.enabled), "允许局域网访问")}`)}
        ${lan.running ? `
          ${codexPlusRemoteRow("访问地址", addresses.length
            ? addresses.map((item) => `<span class="codex-plus-remote-address"><code>${escapeHtml(item.url)}</code><span class="codex-plus-provider-badge">${item.kind === "tailscale" ? "Tailscale" : "局域网"}</span></span>`).join("")
            : "没有找到可用的网络地址，检查一下是否连着 Wi-Fi。")}
          ${lan.fingerprint ? codexPlusRemoteRow("证书指纹", `<code class="codex-plus-remote-fingerprint">${escapeHtml(lan.fingerprint)}</code><br>证书是这台电脑自己签的，手机第一次打开会提示不受信任。可以在浏览器的证书详情里核对这串指纹，一致再继续访问。`) : ""}
          ${codexPlusRemoteRow("添加设备", "选好权限后生成一次性配对码（5 分钟内有效），用手机相机扫码打开，确认后即配对完成。",
            `${codexPlusRemoteRoleSwitch(state.pairRole)}<button type="button" class="codex-plus-action-button" data-codex-remote-action="startPairing" ${busy ? "disabled" : ""}>${data.pairing ? "重新生成" : "生成配对码"}</button>`)}
          ${renderCodexPlusRemotePairing(data.pairing)}` : ""}
      </div>
    </section>` : "";
    const deviceList = devices.length ? `<section class="codex-plus-settings-section" aria-label="已配对设备">
      <h2 class="codex-plus-settings-section-title">已配对设备（${devices.length}）</h2>
      <div class="codex-plus-settings-card">
        ${devices.map((device) => codexPlusRemoteRow(escapeHtml(device.name),
          `最近活动 ${codexPlusRemoteRelative(device.lastSeenAt)}${device.lastAddress ? ` · ${escapeHtml(device.lastAddress)}` : ""}`,
          `${codexPlusRemoteRoleSwitch(device.role, device.id)}<button type="button" class="codex-plus-action-button" data-codex-remote-action="revoke" data-codex-remote-device="${escapeHtml(device.id)}" data-codex-remote-name="${escapeHtml(device.name)}" ${busy ? "disabled" : ""}>移除</button>`)).join("")}
      </div>
    </section>` : "";
    const help = `<section class="codex-plus-settings-section" aria-label="使用说明">
      <h2 class="codex-plus-settings-section-title">使用说明</h2>
      <div class="codex-plus-settings-card">
        ${codexPlusRemoteRow("会话在这里执行", "手机上的操作都落到这个 Codex 里。没打开的会话，手机发消息时会自动在这里打开。")}
        ${codexPlusRemoteRow("审批与提问", "命令审批、模型提问会同时出现在这里和手机上，任一端处理即可。")}
        ${codexPlusRemoteRow("不在同一网络时", "在电脑和手机上都装 Tailscale 并登录同一账号，就能用 Tailscale 地址随时访问，不经过任何第三方服务器。")}
        ${codexPlusRemoteRow("提醒", "手机页面开着时，需要审批、模型提问或一轮跑完会用标题、振动和系统通知提醒；页面关掉后收不到。")}
      </div>
    </section>`;
    const scroll = panel.closest(".codex-plus-modal-body")?.scrollTop || 0;
    panel.innerHTML = `${header}${banner}${main}${phones}${deviceList}${help}`;
    const body = panel.closest(".codex-plus-modal-body");
    if (body) body.scrollTop = scroll;
  }

  async function codexPlusRemoteHandleClick(event) {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    const state = codexPlusRemotePageState;
    const roleButton = target?.closest("[data-codex-remote-role]");
    if (roleButton && !roleButton.disabled) {
      const role = roleButton.getAttribute("data-codex-remote-role");
      const deviceId = roleButton.getAttribute("data-codex-remote-device") || "";
      if (!deviceId) {
        state.pairRole = role;
        renderCodexPlusRemotePage();
        return;
      }
      const device = state.data?.devices?.find((item) => item.id === deviceId);
      if (device && device.role !== role) await codexPlusRemoteRun("setDeviceRole", { id: deviceId, role });
      return;
    }
    const button = target?.closest("[data-codex-remote-action]");
    if (!button || button.disabled) return;
    const action = button.getAttribute("data-codex-remote-action");
    switch (action) {
      case "reload":
        await codexPlusRemoteRun("status", {}, "load");
        return;
      case "copy": {
        if (!state.data?.url) return;
        try {
          await navigator.clipboard.writeText(state.data.url);
          state.copied = true;
          renderCodexPlusRemotePage();
          setTimeout(() => { state.copied = false; renderCodexPlusRemotePage(); }, 1500);
        } catch (error) {
          state.error = `复制失败：${error?.message || error}`;
          renderCodexPlusRemotePage();
        }
        return;
      }
      case "openBrowser":
        // Electron 里 <a target=_blank> 不会交给系统浏览器，统一走 window.open（同 40 分片的推荐卡片）。
        if (state.data?.url) window.open(state.data.url, "_blank", "noopener,noreferrer");
        return;
      case "startPairing":
        await codexPlusRemoteRun("startPairing", { role: state.pairRole });
        return;
      case "revoke": {
        const name = button.getAttribute("data-codex-remote-name") || "这台设备";
        const answer = await codexPlusAsk({
          title: `移除「${name}」？`,
          message: "移除后这台设备立刻无法访问，要再用需重新配对。",
          options: [{ value: "remove", label: "移除" }],
        });
        if (answer) await codexPlusRemoteRun("revokeDevice", { id: button.getAttribute("data-codex-remote-device") || "" });
        return;
      }
      case "enable":
      case "disable":
      case "enableLan":
      case "disableLan":
      case "resetToken":
      case "cancelPairing":
        await codexPlusRemoteRun(action);
        return;
      default:
    }
  }

  /** 手机配上、设备活动、别处（管理器）改了开关：静默刷新；倒计时每秒走。 */
  function codexPlusRemotePoll() {
    const state = codexPlusRemotePageState;
    if (!document.querySelector("[data-codex-remote-panel]") || state.busy || document.hidden) return;
    void codexPlusRemoteRequest("status").then((data) => {
      if (state.busy) return;
      const before = JSON.stringify(state.data);
      codexPlusRemoteApply(data);
      if (JSON.stringify(state.data) !== before) renderCodexPlusRemotePage();
    }).catch(() => {});
  }

  function codexPlusRemoteTick() {
    document.querySelectorAll("[data-codex-remote-countdown]").forEach((node) => {
      const countdown = codexPlusRemoteCountdown(node.getAttribute("data-codex-remote-countdown"));
      node.textContent = countdown.text;
      if (countdown.left <= 0) renderCodexPlusRemotePage();
    });
  }

  function openCodexPlusRemoteControlPage() {
    closeCodexPlusPage();
    document.querySelectorAll('.codex-plus-modal-overlay, [data-codex-plus-dialog="true"]').forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = `${codexPlusPageClass} codex-plus-remote-page`;
    overlay.dataset.codexPlusPage = "true";
    markCodexPlusExtensionNode(overlay, "builtin-remote-control");
    applyCodexPlusTheme(overlay);
    applyCodexPlusZoom(overlay);
    overlay.innerHTML = `<div class="codex-plus-modal-content" role="region" aria-label="远程控制" data-codex-plus-active-tab="${codexPlusRemoteControlTab}">
      <div class="codex-plus-modal-body"><div class="codex-plus-panel codex-plus-settings" data-codex-remote-panel="true"></div></div></div>`;
    overlay.addEventListener("click", (event) => { void codexPlusRemoteHandleClick(event); });
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
    setCodexPlusSidebarNavActive(true, "remote");
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    window.__codexPlusPageResizeHandler = () => positionCodexPlusPage(document.querySelector(`.${codexPlusPageClass}`));
    window.addEventListener("resize", window.__codexPlusPageResizeHandler);
    const state = codexPlusRemotePageState;
    clearInterval(state.pollTimer);
    clearInterval(state.tickTimer);
    state.pollTimer = setInterval(codexPlusRemotePoll, 2500);
    state.tickTimer = setInterval(codexPlusRemoteTick, 1000);
    // 关页时由 closeCodexPlusPage 调用（92-extension-host.js）。
    window.__codexPlusExtensionPageCleanup = () => {
      clearInterval(state.pollTimer);
      clearInterval(state.tickTimer);
      state.pollTimer = null;
      state.tickTimer = null;
    };
    Object.assign(state, { error: "", notice: "", copied: false, deviceCount: null });
    renderCodexPlusRemotePage();
    void codexPlusRemoteRun("status", {}, "load");
  }
