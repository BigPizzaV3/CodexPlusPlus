  /**
   * Codex 左侧图标栏的「远程控制」页：不离开 Codex 就能开关远程控制、开局域网访问、给手机配对、管理设备。
   *
   * 分工：
   *   - 服务本体在 core `remote_control`，由管理器进程托管；本页经 bridge 路由 `/remote-control`
   *     `{ action, ...args }` 调用与管理器同一份 `remote_control::dispatch`，两边看到的是同一个状态。
   *   - 「钉钉与飞书通知」经 `/remote-control/robots` `{ action, provider, update }` 调与管理器同一份
   *     `chat_notifications::dispatch`；Webhook 和签名密钥只能写，读回来只有「有没有」。
   *   - 两条路由都不在拓展白名单里（能开关服务、发配对码、移除设备、改机器人）。
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
    robots: null,
    robotError: "",
    // 正在配置的机器人：{ provider, webhook, secret, clearSecret, network }，同一时间只配一个；关页即丢掉（含密钥草稿）。
    robotEdit: null,
  };
  const codexPlusRemoteRobotLabels = { dingtalk: "钉钉", feishu: "飞书" };
  const codexPlusRemoteRobotNetworks = [["lan", "局域网"], ["tailscale", "Tailscale"], ["none", "仅通知"]];

  async function codexPlusRemoteRequest(action, args = {}) {
    const result = await postJson("/remote-control", { action, ...args });
    if (result?.status !== "ok") throw new Error(result?.message || "远程控制操作失败");
    return result;
  }

  async function codexPlusRemoteRobotRequest(action, provider = null, update = null) {
    const result = await postJson("/remote-control/robots", { action, provider, update });
    if (result?.status !== "ok") throw new Error(result?.message || "机器人操作失败");
    return result;
  }

  async function codexPlusRemoteLoadRobots() {
    const state = codexPlusRemotePageState;
    try {
      state.robots = await codexPlusRemoteRobotRequest("status");
      state.robotError = "";
    } catch (error) {
      state.robotError = error?.message || String(error);
    }
    renderCodexPlusRemotePage();
  }

  async function codexPlusRemoteRobotRun(action, provider, update = null) {
    const state = codexPlusRemotePageState;
    state.busy = "robot";
    state.robotError = "";
    renderCodexPlusRemotePage();
    try {
      state.robots = await codexPlusRemoteRobotRequest(action, provider, update);
      return true;
    } catch (error) {
      state.robotError = error?.message || String(error);
      return false;
    } finally {
      state.busy = "";
      renderCodexPlusRemotePage();
    }
  }

  /** 保存正在配置的机器人：Webhook、密钥留空表示保留已保存的（同管理器）。 */
  async function codexPlusRemoteRobotSave() {
    const state = codexPlusRemotePageState;
    const edit = state.robotEdit;
    if (!edit || state.busy) return;
    const webhook = edit.webhook.trim();
    const secret = edit.secret.trim();
    if (!state.robots?.[edit.provider]?.hasWebhook && !webhook) {
      state.robotError = "请填写机器人 Webhook。";
      renderCodexPlusRemotePage();
      return;
    }
    const update = { ...(webhook ? { webhook } : {}), ...(secret ? { secret } : {}), clearSecret: edit.clearSecret, linkNetwork: edit.network };
    if (await codexPlusRemoteRobotRun("save", edit.provider, update)) {
      state.robotEdit = null;
      renderCodexPlusRemotePage();
    }
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

  /** 一个机器人：标题行（配置 / 测试 / 开关），配置中时展开 Webhook、签名密钥、会话链接与保存。 */
  function codexPlusRemoteRobotRows(provider, robot) {
    const state = codexPlusRemotePageState;
    const busy = Boolean(state.busy);
    const name = codexPlusRemoteRobotLabels[provider];
    const edit = state.robotEdit?.provider === provider ? state.robotEdit : null;
    const attrs = `data-codex-remote-robot="${provider}"`;
    const toggleOff = busy || !robot.hasWebhook || Boolean(edit);
    const head = codexPlusRemoteRow(name, escapeHtml(robot.lastResult || (robot.hasWebhook ? "已保存机器人" : "尚未配置机器人")),
      `<button type="button" class="codex-plus-action-button" data-codex-remote-action="robotEdit" ${attrs} ${busy ? "disabled" : ""}>${edit ? "取消" : "配置机器人"}</button>${robot.hasWebhook ? `<button type="button" class="codex-plus-action-button" data-codex-remote-action="robotTest" ${attrs} ${busy || edit ? "disabled" : ""}>测试通知</button>` : ""}<button type="button" class="codex-plus-toggle" role="switch" aria-label="${name}通知" aria-checked="${String(Boolean(robot.enabled))}" data-enabled="${String(Boolean(robot.enabled))}" data-pending="${String(busy)}" data-codex-remote-action="robotToggle" ${attrs} ${toggleOff ? "disabled" : ""}><span></span></button>`);
    if (!edit) return head;
    const input = (field, label, placeholder, disabled = false) => `<input class="codex-plus-provider-input codex-plus-service-input codex-plus-remote-robot-input" type="password" data-codex-remote-robot-field="${field}" value="${escapeHtml(edit[field])}" placeholder="${escapeHtml(placeholder)}" aria-label="${escapeHtml(label)}" autocomplete="off" spellcheck="false" ${busy || disabled ? "disabled" : ""}>`;
    const clearSecret = robot.hasSecret
      ? `<button type="button" class="codex-plus-toggle" role="switch" aria-label="清除签名" aria-checked="${String(edit.clearSecret)}" data-enabled="${String(edit.clearSecret)}" data-codex-remote-action="robotClearSecret" ${busy ? "disabled" : ""}><span></span></button><span class="codex-plus-row-description">清除签名</span>`
      : "";
    const networks = `<div class="codex-plus-remote-segmented" role="radiogroup" aria-label="消息中的会话链接">${codexPlusRemoteRobotNetworks
      .map(([id, label]) => `<button type="button" role="radio" aria-checked="${String(id === edit.network)}" data-codex-remote-robot-network="${id}" ${busy ? "disabled" : ""}>${label}</button>`)
      .join("")}</div>`;
    return `${head}
      ${codexPlusRemoteRow("机器人 Webhook", "在群里添加自定义机器人，复制它的 Webhook 地址。如果启用关键词校验，请添加 Codex++。",
        input("webhook", `${name} Webhook`, robot.hasWebhook ? "已保存，留空保留" : "https://…"))}
      ${codexPlusRemoteRow("机器人签名密钥", "使用加签的机器人请填写密钥。更换 Webhook 时同时填写新机器人的密钥。",
        `${input("secret", `${name} 机器人签名密钥`, robot.hasSecret ? "已保存，留空保留" : "可选", edit.clearSecret)}${clearSecret}`)}
      ${codexPlusRemoteRow("消息中的会话链接", "手机需能访问所选网络，审批使用已配对设备的完全控制权限。", networks)}
      ${codexPlusRemoteRow("机器人配置", "",
        `${robot.hasWebhook ? `<button type="button" class="codex-plus-action-button" data-codex-remote-action="robotClear" ${attrs} ${busy ? "disabled" : ""}>清除</button>` : ""}<button type="button" class="codex-plus-action-button" data-primary="true" data-codex-remote-action="robotSave" ${busy ? "disabled" : ""}>${state.busy === "robot" ? "正在保存…" : "保存"}</button>`)}`;
  }

  function codexPlusRemoteRobotSection(data) {
    const state = codexPlusRemotePageState;
    const robots = state.robots;
    const note = !data.running ? "打开远程控制后开始监听任务通知。" : !data.lan?.running ? "要在手机上打开会话，请启用局域网访问并配对设备。" : "";
    const body = robots
      ? ["dingtalk", "feishu"].map((provider) => codexPlusRemoteRobotRows(provider, robots[provider] || {})).join("")
      : codexPlusRemoteRow("机器人", state.robotError ? "读取失败。" : "正在读取…",
        state.robotError ? '<button type="button" class="codex-plus-action-button" data-codex-remote-action="robotReload">重试</button>' : "");
    return `<section class="codex-plus-settings-section" aria-label="钉钉与飞书通知">
      <h2 class="codex-plus-settings-section-title">钉钉与飞书通知</h2>
      ${state.robotError ? `<div class="codex-plus-remote-banner codex-plus-remote-robot-error" data-tone="warn" role="alert">${escapeHtml(state.robotError)}</div>` : ""}
      <div class="codex-plus-settings-card">
        ${codexPlusRemoteRow("任务通知", `任务完成、出错或等待审批时，向选定的群发送状态和会话短编号。启用局域网访问后，可从通知打开会话处理审批。${note ? `<br>${note}` : ""}`)}
        ${body}
      </div>
    </section>`;
  }

  function renderCodexPlusRemotePage() {
    const panel = document.querySelector("[data-codex-remote-panel]");
    if (!panel) return;
    const state = codexPlusRemotePageState;
    const data = state.data;
    const busy = Boolean(state.busy);
    // 页面名和「实验」标在左栏顶上（installCodexPlusSectionNav），右边每组第一节的标题就是大标题。
    const banner = [
      state.error || data?.error || data?.lan?.error ? `<div class="codex-plus-remote-banner" data-tone="warn" role="alert">${escapeHtml(state.error || data?.error || data?.lan?.error)}</div>` : "",
      state.notice ? `<div class="codex-plus-remote-banner" data-tone="ok" role="status">${escapeHtml(state.notice)}</div>` : "",
    ].join("");
    if (!data) {
      panel.innerHTML = `${banner}<div class="codex-plus-provider-empty">${state.busy ? "正在读取…" : ""}
        ${state.busy ? "" : '<button type="button" class="codex-plus-action-button" data-codex-remote-action="reload">重试</button>'}</div>`;
      return;
    }
    const lan = data.lan || {};
    const devices = Array.isArray(data.devices) ? data.devices : [];
    const url = data.url || "";
    const main = `<section class="codex-plus-settings-section" aria-label="远程控制">
      <h2 class="codex-plus-settings-section-title">远程控制</h2>
      <div class="codex-plus-settings-card">
        ${codexPlusRemoteRow("启用远程控制", `在浏览器或手机上查看 Codex 的会话、发消息、停止，处理命令审批和模型提问，切换模型、权限和计划模式。${data.weixinFocusEnabled && !data.running ? "开启远程控制会中断正在独立后台运行的微信专注轮次。" : ""}`,
          `${data.running ? '<span class="codex-plus-provider-badge" data-tone="active">运行中</span>' : ""}${codexPlusRemoteToggle(data.enabled ? "disable" : "enable", Boolean(data.enabled), "启用远程控制")}`)}
        ${codexPlusRemoteRow("允许微信专注操作桌面会话", "远程控制关闭时，仅在此项开启后，已扫码绑定的微信用户仍可进入桌面会话、发消息和处理审批。不会开启网页访问。",
          codexPlusRemoteToggle("setWeixinFocus", Boolean(data.weixinFocusEnabled), "允许微信专注操作桌面会话"))}
        ${url ? codexPlusRemoteRow("本机访问链接", `<code class="codex-plus-remote-url">${escapeHtml(url.replace(/#token=.*/, "#token=…"))}</code><br>链接里带着访问令牌，拿到链接就能操控 Codex，不要发给别人。`,
          `<button type="button" class="codex-plus-action-button" data-codex-remote-action="copy">${state.copied ? "已复制" : "复制"}</button>
           <button type="button" class="codex-plus-action-button" data-codex-remote-action="openBrowser">在浏览器打开</button>`) : ""}
        ${data.enabled ? codexPlusRemoteRow("更换令牌", "之前打开的本机页面会立刻失效；已配对的手机不受影响。",
          `<button type="button" class="codex-plus-action-button" data-codex-remote-action="resetToken" ${busy ? "disabled" : ""}>更换</button>`) : ""}
      </div>
    </section>`;
    const addresses = Array.isArray(lan.addresses) ? lan.addresses : [];
    const phones = `<section class="codex-plus-settings-section" aria-label="手机与其它设备">
      <h2 class="codex-plus-settings-section-title">手机与其它设备</h2>
      <div class="codex-plus-settings-card">
        ${!data.enabled ? codexPlusRemoteRow("先启用远程控制", "启用后，同一 Wi-Fi 或 Tailscale 网络里的手机才能配对访问。",
          `<button type="button" class="codex-plus-action-button" data-codex-remote-action="enable" ${busy ? "disabled" : ""}>启用远程控制</button>`) : `
        ${codexPlusRemoteRow("允许局域网访问", "同一 Wi-Fi 或同一 Tailscale 网络里的设备经 HTTPS 访问，只有配对过的设备能用。",
          `${lan.running ? `<span class="codex-plus-provider-badge" data-tone="active">端口 ${Number(lan.port)}</span>` : ""}${codexPlusRemoteToggle(lan.enabled ? "disableLan" : "enableLan", Boolean(lan.enabled), "允许局域网访问")}`)}
        ${lan.running ? `
          ${codexPlusRemoteRow("访问地址", addresses.length
            ? addresses.map((item) => `<span class="codex-plus-remote-address"><code>${escapeHtml(item.url)}</code><span class="codex-plus-provider-badge">${item.kind === "tailscale" ? "Tailscale" : "局域网"}</span></span>`).join("")
            : "没有找到可用的网络地址，检查一下是否连着 Wi-Fi。")}
          ${lan.fingerprint ? codexPlusRemoteRow("证书指纹", `<code class="codex-plus-remote-fingerprint">${escapeHtml(lan.fingerprint)}</code><br>证书是这台电脑自己签的，手机第一次打开会提示不受信任。可以在浏览器的证书详情里核对这串指纹，一致再继续访问。`) : ""}
          ${codexPlusRemoteRow("添加设备", "选好权限后生成一次性配对码（5 分钟内有效），用手机相机扫码打开，确认后即配对完成。",
            `${codexPlusRemoteRoleSwitch(state.pairRole)}<button type="button" class="codex-plus-action-button" data-codex-remote-action="startPairing" ${busy ? "disabled" : ""}>${data.pairing ? "重新生成" : "生成配对码"}</button>`)}
          ${renderCodexPlusRemotePairing(data.pairing)}` : ""}`}
      </div>
    </section>`;
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
    // 轮询重绘时正在填机器人 Webhook / 密钥：草稿在 robotEdit 里，重绘后把焦点和光标放回去。
    const active = document.activeElement;
    const field = active instanceof Element && panel.contains(active) ? active.getAttribute("data-codex-remote-robot-field") : null;
    const selection = field ? [active.selectionStart, active.selectionEnd] : null;
    const scroll = panel.closest(".codex-plus-modal-body")?.scrollTop || 0;
    panel.innerHTML = `${banner}${codexPlusSectionGroupHtml("remote", main)}${codexPlusSectionGroupHtml("devices", `${phones}${deviceList}`)}${codexPlusSectionGroupHtml("notify", codexPlusRemoteRobotSection(data))}${codexPlusSectionGroupHtml("help", help)}`;
    const body = panel.closest(".codex-plus-modal-body");
    if (body) body.scrollTop = scroll;
    const next = field ? panel.querySelector(`[data-codex-remote-robot-field="${field}"]`) : null;
    if (next && !next.disabled) {
      next.focus({ preventScroll: true });
      try {
        next.setSelectionRange(...selection);
      } catch {}
    }
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
    const networkButton = target?.closest("[data-codex-remote-robot-network]");
    if (networkButton && !networkButton.disabled && state.robotEdit) {
      state.robotEdit.network = networkButton.getAttribute("data-codex-remote-robot-network");
      renderCodexPlusRemotePage();
      return;
    }
    const button = target?.closest("[data-codex-remote-action]");
    if (!button || button.disabled) return;
    const action = button.getAttribute("data-codex-remote-action");
    const provider = button.getAttribute("data-codex-remote-robot") || "";
    const robot = state.robots?.[provider] || {};
    switch (action) {
      case "robotReload":
        await codexPlusRemoteLoadRobots();
        return;
      case "robotEdit":
        state.robotError = "";
        state.robotEdit = state.robotEdit?.provider === provider
          ? null
          : { provider, webhook: "", secret: "", clearSecret: false, network: robot.linkNetwork || "lan" };
        renderCodexPlusRemotePage();
        document.querySelector('[data-codex-remote-robot-field="webhook"]')?.focus({ preventScroll: true });
        return;
      case "robotClearSecret":
        if (!state.robotEdit) return;
        state.robotEdit.clearSecret = !state.robotEdit.clearSecret;
        if (state.robotEdit.clearSecret) state.robotEdit.secret = "";
        renderCodexPlusRemotePage();
        return;
      case "robotSave":
        await codexPlusRemoteRobotSave();
        return;
      case "robotTest":
        await codexPlusRemoteRobotRun("test", provider);
        return;
      case "robotToggle":
        await codexPlusRemoteRobotRun("save", provider, { enabled: !robot.enabled });
        return;
      case "robotClear": {
        const answer = await codexPlusAsk({
          title: `清除${codexPlusRemoteRobotLabels[provider] || ""}机器人配置？`,
          message: "清除后将停止向这个机器人推送。",
          options: [{ value: "clear", label: "清除" }],
        });
        if (answer && await codexPlusRemoteRobotRun("clear", provider)) {
          state.robotEdit = null;
          renderCodexPlusRemotePage();
        }
        return;
      }
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
      case "setWeixinFocus":
        await codexPlusRemoteRun("setWeixinFocus", { enabled: !state.data?.weixinFocusEnabled });
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

  /** 机器人输入框的草稿记进 robotEdit，轮询重绘不丢。 */
  function codexPlusRemoteHandleInput(event) {
    const input = event.target instanceof Element ? event.target.closest("[data-codex-remote-robot-field]") : null;
    const edit = codexPlusRemotePageState.robotEdit;
    if (input && edit) edit[input.getAttribute("data-codex-remote-robot-field")] = input.value;
  }

  /**
   * Codex 在 document 捕获阶段把按键送进它的输入框，所以在 window 捕获阶段先接住机器人输入框里的按键；
   * 回车保存，Esc 取消配置。
   */
  function codexPlusRemoteCaptureKeydown(event) {
    const target = event.target instanceof Element ? event.target : null;
    if (!target?.matches("[data-codex-remote-robot-field]") || !target.closest(".codex-plus-remote-page")) return;
    event.stopPropagation();
    if (event.key === "Enter") {
      event.preventDefault();
      void codexPlusRemoteRobotSave();
    } else if (event.key === "Escape") {
      event.preventDefault();
      codexPlusRemotePageState.robotEdit = null;
      renderCodexPlusRemotePage();
    }
  }

  /** 左栏目录：页面里的左栏和图标栏悬浮卡片共用。 */
  const codexPlusRemoteSectionNav = {
    title: "远程控制",
    titleHtml: '远程控制<em class="codex-plus-feature-tag" data-tone="experimental">实验</em>',
    groups: [
      { id: "remote", label: "远程控制", icon: "core" },
      { id: "devices", label: "手机与其它设备", icon: "standalone" },
      { id: "notify", label: "钉钉与飞书通知", icon: "chat" },
      { id: "help", label: "使用说明", icon: "tools" },
    ],
  };

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
      <div class="codex-plus-modal-body"><div class="codex-plus-panel codex-plus-settings" data-codex-plus-settings-paged="true" data-codex-remote-panel="true"></div></div></div>`;
    overlay.addEventListener("click", (event) => { void codexPlusRemoteHandleClick(event); });
    overlay.addEventListener("input", codexPlusRemoteHandleInput);
    installCodexPlusSectionNav(overlay, codexPlusRemoteSectionNav);
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
    window.removeEventListener("keydown", codexPlusRemoteCaptureKeydown, true);
    window.addEventListener("keydown", codexPlusRemoteCaptureKeydown, true);
    // 关页时由 closeCodexPlusPage 调用（92-extension-host.js）。
    window.__codexPlusExtensionPageCleanup = () => {
      clearInterval(state.pollTimer);
      clearInterval(state.tickTimer);
      state.pollTimer = null;
      state.tickTimer = null;
      state.robotEdit = null;
      window.removeEventListener("keydown", codexPlusRemoteCaptureKeydown, true);
    };
    Object.assign(state, { error: "", notice: "", copied: false, deviceCount: null, robotError: "", robotEdit: null });
    renderCodexPlusRemotePage();
    void codexPlusRemoteRun("status", {}, "load");
    void codexPlusRemoteLoadRobots();
  }
