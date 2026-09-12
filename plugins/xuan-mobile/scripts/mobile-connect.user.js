(() => {
  const adapterVersion = "1.0.0";
  const requestTimeoutMs = 30000;
  const nameTimeoutMs = 8000;

  function requestError(method, error) {
    const message = String(error?.message || error || `Codex ${method} failed`);
    return new Error(`${method}: ${message}`);
  }

  function callCodex(method, params, timeoutMs = requestTimeoutMs) {
    const bridge = window.electronBridge;
    if (!bridge || typeof bridge.sendMessageFromView !== "function") {
      return Promise.reject(new Error("Codex renderer command bridge unavailable"));
    }
    const requestId = typeof crypto?.randomUUID === "function"
      ? crypto.randomUUID()
      : `xuan-mobile-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    return new Promise((resolve, reject) => {
      let timeout = 0;
      const cleanup = () => {
        window.clearTimeout(timeout);
        window.removeEventListener("message", onMessage);
      };
      const onMessage = (event) => {
        const message = event?.data;
        if (!message || message.type !== "mcp-response" || message.hostId !== "local"
            || message.message?.id !== requestId) return;
        cleanup();
        if (message.message?.error) {
          reject(requestError(method, message.message.error));
          return;
        }
        resolve(message.message?.result ?? null);
      };
      window.addEventListener("message", onMessage);
      timeout = window.setTimeout(() => {
        cleanup();
        reject(new Error(`Codex ${method} timed out`));
      }, timeoutMs);
      Promise.resolve(bridge.sendMessageFromView({
        type: "mcp-request",
        request: { id: requestId, method, params },
        hostId: "local",
        priority: "critical",
        source: "xuan_plus_remote",
        timeoutMs,
        expiresAtMs: Date.now() + timeoutMs,
      })).catch((error) => {
        cleanup();
        reject(requestError(method, error));
      });
    });
  }

  function install() {
    const bridge = window.electronBridge;
    if (!bridge || typeof bridge.sendMessageFromView !== "function") return false;
    window.__xuanPlusRemoteCommandAdapterVersion = adapterVersion;
    window.__codexPlusMobileRemoteCommand = async (request) => {
      const commandType = String(request?.commandType || "");
      const threadId = String(request?.threadId || "");
      const turnId = String(request?.turnId || "");
      const clientRequestId = String(request?.clientRequestId || "");
      const text = String(request?.text || "");
      if (commandType === "create_task") {
        const started = await callCodex("thread/start", {
          cwd: String(request?.cwd || ""),
          model: String(request?.model || ""),
          modelProvider: String(request?.provider || ""),
          approvalPolicy: "on-request",
          sandbox: "workspace-write",
          persistExtendedHistory: true,
        });
        const createdThreadId = String(started?.thread?.id || started?.threadId || started?.id || "");
        if (!createdThreadId) throw new Error("thread/start returned no thread id");
        const turn = await callCodex("turn/start", {
          threadId: createdThreadId,
          clientUserMessageId: `xuan-mobile-${clientRequestId}`,
          input: [{ type: "text", text, text_elements: [] }],
        });
        const createdTurnId = String(turn?.turn?.id || turn?.turnId || turn?.id || "");
        if (!createdTurnId) throw new Error("turn/start returned no turn id");
        const name = String(request?.name || "").trim();
        if (name) {
          try {
            await callCodex("thread/name/set", { threadId: createdThreadId, name }, nameTimeoutMs);
          } catch {}
        }
        return { status: "completed", threadId: createdThreadId, turnId: createdTurnId };
      }
      if (commandType === "start_task" || commandType === "send_input") {
        await callCodex("thread/resume", { threadId, persistExtendedHistory: true });
        const turn = await callCodex("turn/start", {
          threadId,
          clientUserMessageId: `xuan-mobile-${clientRequestId}`,
          input: [{ type: "text", text, text_elements: [] }],
        });
        const acceptedTurnId = String(turn?.turn?.id || turn?.turnId || turn?.id || "");
        if (!acceptedTurnId) throw new Error("turn/start returned no turn id");
        return { status: "completed", threadId, turnId: acceptedTurnId };
      }
      if (commandType === "stop_task") {
        if (!turnId) throw new Error("stop_task requires turnId");
        await callCodex("turn/interrupt", { threadId, turnId });
        return { status: "completed", threadId, turnId };
      }
      return { status: "rejected", errorCode: "unsupported_operation" };
    };
    return true;
  }

  if (install()) return;
  let attempts = 0;
  const retry = window.setInterval(() => {
    attempts += 1;
    if (install() || attempts >= 60) window.clearInterval(retry);
  }, 500);
})();

(() => {
  const API_KEY = "__xuanMobileConnectUi";
  const ROOT_ID = "xuan-mobile-connect";
  const PANEL_ID = "xuan-mobile-connect-panel";
  const STYLE_ID = "xuan-mobile-connect-style";
  const bridgeUrl = window.__XUAN_BRIDGE_URL__ || "http://127.0.0.1:57324";
  const bridgeToken = window.__XUAN_BRIDGE_TOKEN__ || "";

  window[API_KEY]?.destroy?.();
  const state = {
    status: null,
    tasks: [],
    query: "",
    busy: false,
    error: "",
    open: false,
    timer: 0,
    clock: Date.now(),
  };

  function escapeHtml(value) {
    return String(value ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#39;");
  }

  async function request(path, payload, method = "POST") {
    const headers = { "content-type": "application/json" };
    if (bridgeToken) headers["x-xuan-bridge-token"] = bridgeToken;
    const response = await fetch(`${bridgeUrl}${path}`, {
      method,
      headers,
      body: method === "GET" ? undefined : JSON.stringify(payload || {}),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(value?.error?.message || value?.message || "手机连接操作未完成");
    }
    return value;
  }

  function ensureStyle() {
    if (document.getElementById(STYLE_ID)) return;
    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      #${ROOT_ID}{position:fixed;z-index:2147482399;top:6px;right:268px;height:24px;padding:4px 10px;border:1px solid transparent;border-radius:6px;background:transparent;color:color-mix(in srgb,CanvasText 58%,transparent);font:400 14px/14px -apple-system,BlinkMacSystemFont,"Segoe UI Variable Text","Segoe UI","Microsoft YaHei UI",sans-serif;cursor:pointer;white-space:nowrap;-webkit-app-region:no-drag}
      #${ROOT_ID}:hover,#${ROOT_ID}[data-open="true"]{background:color-mix(in srgb,CanvasText 6%,transparent);color:CanvasText}
      #${ROOT_ID}[data-bound="true"]::before{content:"";display:inline-block;width:6px;height:6px;margin-right:6px;border-radius:50%;background:#16a34a;vertical-align:2px}
      #${PANEL_ID}{position:fixed;z-index:2147482402;top:50px;right:16px;width:min(760px,calc(100vw - 24px));max-height:calc(100vh - 64px);overflow:auto;border:1px solid color-mix(in srgb,currentColor 18%,transparent);border-radius:8px;background:Canvas;color:CanvasText;box-shadow:0 18px 54px rgba(0,0,0,.24);font:13px/1.45 -apple-system,BlinkMacSystemFont,"Segoe UI","Microsoft YaHei",sans-serif;-webkit-app-region:no-drag}
      #${PANEL_ID}[hidden]{display:none}#${PANEL_ID} *{box-sizing:border-box;letter-spacing:0}
      .xmc-head,.xmc-toolbar{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.xmc-head{position:sticky;top:0;z-index:2;justify-content:space-between;padding:14px 16px;border-bottom:1px solid color-mix(in srgb,currentColor 14%,transparent);background:Canvas}.xmc-title{font-size:15px;font-weight:700}.xmc-status{color:color-mix(in srgb,CanvasText 62%,transparent);font-size:12px}.xmc-close{width:30px;height:30px;border:0;border-radius:5px;background:transparent;color:inherit;font-size:20px;cursor:pointer}.xmc-close:hover{background:color-mix(in srgb,CanvasText 8%,Canvas)}
      .xmc-section{padding:14px 16px;border-bottom:1px solid color-mix(in srgb,currentColor 12%,transparent)}.xmc-section:last-child{border-bottom:0}.xmc-section h3{margin:0;font-size:14px}.xmc-button,.xmc-input{height:32px;border:1px solid color-mix(in srgb,currentColor 18%,transparent);border-radius:5px;background:Canvas;color:CanvasText;font:inherit}.xmc-button{display:inline-flex;align-items:center;justify-content:center;padding:0 11px;cursor:pointer}.xmc-button:hover{background:color-mix(in srgb,CanvasText 7%,Canvas)}.xmc-button:disabled{opacity:.5;cursor:not-allowed}.xmc-primary{background:CanvasText;color:Canvas;border-color:CanvasText}.xmc-input{width:100%;margin-top:10px;padding:0 9px}.xmc-toggle{display:inline-flex;align-items:center;gap:7px;cursor:pointer}.xmc-toggle input,.xmc-task input{width:15px;height:15px;margin:0;accent-color:#1677d2}
      .xmc-error{margin:10px 0 0;color:#dc2626}.xmc-pairing{display:grid;grid-template-columns:256px minmax(0,1fr);gap:18px;align-items:center;margin-top:14px}.xmc-qr{width:256px;height:256px;border:1px solid color-mix(in srgb,currentColor 14%,transparent);border-radius:6px;background:#fff;object-fit:contain}.xmc-pair-copy{display:grid;gap:8px}.xmc-phrase{font:700 22px/1.3 ui-monospace,SFMono-Regular,Consolas,monospace;overflow-wrap:anywhere}
      .xmc-task-list{margin-top:10px;border:1px solid color-mix(in srgb,currentColor 13%,transparent);border-radius:6px;overflow:hidden}.xmc-task{display:grid;grid-template-columns:18px minmax(0,1fr) auto;gap:8px;align-items:center;padding:9px 10px;border-bottom:1px solid color-mix(in srgb,currentColor 10%,transparent);cursor:pointer}.xmc-task:last-child{border-bottom:0}.xmc-task small{max-width:240px;overflow:hidden;color:color-mix(in srgb,CanvasText 56%,transparent);text-overflow:ellipsis;white-space:nowrap}.xmc-empty{padding:18px;color:color-mix(in srgb,CanvasText 58%,transparent);text-align:center}.xmc-issues{margin:10px 0 0;padding-left:20px;color:#b45309}
      @media(max-width:720px){#${ROOT_ID}{right:152px}#${PANEL_ID}{top:0;right:0;width:100vw;max-height:100vh;height:100vh;border:0;border-radius:0}.xmc-pairing{grid-template-columns:1fr}.xmc-qr{justify-self:center}.xmc-task{grid-template-columns:18px minmax(0,1fr)}.xmc-task small{grid-column:2;max-width:none}}
    `;
    document.head.appendChild(style);
  }

  function ensureRoot() {
    ensureStyle();
    let button = document.getElementById(ROOT_ID);
    if (!button) {
      button = document.createElement("button");
      button.id = ROOT_ID;
      button.type = "button";
      button.textContent = "手机连接";
      button.title = "连接轩++远程手机端";
      button.addEventListener("click", () => {
        state.open = !state.open;
        render();
        if (state.open) void refreshAll();
      });
      document.body.appendChild(button);
    }
    let panel = document.getElementById(PANEL_ID);
    if (!panel) {
      panel = document.createElement("section");
      panel.id = PANEL_ID;
      panel.setAttribute("role", "dialog");
      panel.setAttribute("aria-label", "手机连接");
      document.body.appendChild(panel);
    }
    return { button, panel };
  }

  function render() {
    const { button, panel } = ensureRoot();
    const status = state.status || {};
    const selected = new Set(Array.isArray(status.selected) ? status.selected : []);
    const query = state.query.trim().toLowerCase();
    const tasks = state.tasks.filter((task) => `${task.name || ""} ${task.workspaceName || ""}`.toLowerCase().includes(query));
    const qrValid = status.qrImage && status.qrExpiresAt && Date.parse(status.qrExpiresAt) > state.clock;
    const pending = status.pending;
    const confirmationValid = pending?.expiresAt && Date.parse(pending.expiresAt) > state.clock;
    button.dataset.open = String(state.open);
    button.dataset.bound = String(Boolean(status.bound));
    panel.hidden = !state.open;
    panel.innerHTML = `
      <div class="xmc-head"><div><div class="xmc-title">轩++远程</div><div class="xmc-status" role="status">${escapeHtml(status.message || "正在读取连接状态")}</div></div><button class="xmc-close" data-action="close" title="关闭" aria-label="关闭">×</button></div>
      <section class="xmc-section">
        <div class="xmc-toolbar"><label class="xmc-toggle"><input data-action="enable" type="checkbox" ${status.enabled ? "checked" : ""} ${state.busy || !state.status ? "disabled" : ""}>连接手机</label><button class="xmc-button xmc-primary" data-action="pair" ${state.busy || !state.status ? "disabled" : ""}>${status.bound ? "绑定其他手机" : "生成绑定二维码"}</button><span>${status.bound ? "已绑定" : "未绑定"}</span>${status.lastSyncedAt ? `<span>最近同步 ${escapeHtml(new Date(status.lastSyncedAt).toLocaleTimeString("zh-CN"))}</span>` : ""}</div>
        ${state.error || status.syncError ? `<p class="xmc-error" role="alert">${escapeHtml(state.error || status.syncError)}</p>` : ""}
        ${Array.isArray(status.syncIssues) && status.syncIssues.length ? `<ul class="xmc-issues">${status.syncIssues.map((issue) => `<li>${escapeHtml(issue.taskName || "未命名任务")}${issue.workspaceName ? `（${escapeHtml(issue.workspaceName)}）` : ""}：${escapeHtml(issue.reason || "暂不可读")}</li>`).join("")}</ul>` : ""}
        ${qrValid ? `<div class="xmc-pairing"><img class="xmc-qr" src="${escapeHtml(status.qrImage)}" alt="手机绑定二维码"><div class="xmc-pair-copy"><strong>等待手机扫码</strong><span>${escapeHtml(new Date(status.qrExpiresAt).toLocaleTimeString("zh-CN"))} 到期</span><span>在轩++远程手机端打开扫码绑定。</span></div></div>` : status.qrImage ? `<p>二维码已过期，请重新生成。</p>` : ""}
        ${pending ? `<div class="xmc-pairing"><div class="xmc-pair-copy"><strong>${escapeHtml(pending.phoneName || "待绑定手机")}</strong><span>核对短语</span><span class="xmc-phrase">${escapeHtml(pending.safetyPhrase || "")}</span><div class="xmc-toolbar"><button class="xmc-button xmc-primary" data-action="confirm" ${state.busy || !confirmationValid || !status.connected ? "disabled" : ""}>确认绑定</button><button class="xmc-button" data-action="reject" ${state.busy || !confirmationValid || !status.connected ? "disabled" : ""}>拒绝</button>${!confirmationValid ? "<span>确认已过期</span>" : ""}</div></div></div>` : ""}
      </section>
      <section class="xmc-section"><div class="xmc-toolbar"><h3>官方任务</h3><span>已选择 ${selected.size} 项</span><button class="xmc-button" data-action="refresh-tasks" ${state.busy ? "disabled" : ""}>刷新</button><label class="xmc-toggle"><input data-action="auto-sync" type="checkbox" ${status.autoSync ? "checked" : ""} ${state.busy || !state.status ? "disabled" : ""}>自动同步最近 20 个任务</label></div><input class="xmc-input" data-action="query" aria-label="搜索任务" placeholder="搜索任务" value="${escapeHtml(state.query)}"><div class="xmc-task-list">${tasks.length ? tasks.map((task) => `<label class="xmc-task"><input data-action="select-task" data-task-id="${escapeHtml(task.id)}" type="checkbox" ${selected.has(task.id) ? "checked" : ""} ${state.busy || status.autoSync ? "disabled" : ""}><span>${escapeHtml(task.name || "未命名任务")}</span><small>${escapeHtml(task.workspaceName || "")}</small></label>`).join("") : `<div class="xmc-empty">${state.query ? "没有匹配的任务" : "暂无可用任务"}</div>`}</div></section>`;
    bindPanel(panel, selected, pending);
  }

  function bindPanel(panel, selected, pending) {
    panel.querySelector('[data-action="close"]')?.addEventListener("click", () => { state.open = false; render(); });
    panel.querySelector('[data-action="pair"]')?.addEventListener("click", () => void action("/v1/mobile/pair", {}));
    panel.querySelector('[data-action="enable"]')?.addEventListener("change", (event) => void action("/v1/mobile/enable", { enabled: event.target.checked }));
    panel.querySelector('[data-action="auto-sync"]')?.addEventListener("change", (event) => void action("/v1/mobile/auto-sync", { enabled: event.target.checked }));
    panel.querySelector('[data-action="confirm"]')?.addEventListener("click", () => void action("/v1/mobile/confirm", { requestId: pending?.requestId || "", confirmed: true }));
    panel.querySelector('[data-action="reject"]')?.addEventListener("click", () => void action("/v1/mobile/confirm", { requestId: pending?.requestId || "", confirmed: false }));
    panel.querySelector('[data-action="refresh-tasks"]')?.addEventListener("click", () => void refreshTasks());
    panel.querySelector('[data-action="query"]')?.addEventListener("input", (event) => { state.query = event.target.value; render(); panel.querySelector('[data-action="query"]')?.focus(); });
    panel.querySelectorAll('[data-action="select-task"]').forEach((input) => input.addEventListener("change", () => {
      const next = new Set(selected);
      if (input.checked) next.add(input.dataset.taskId); else next.delete(input.dataset.taskId);
      void action("/v1/mobile/select", { selected: [...next] });
    }));
  }

  async function action(path, payload) {
    if (state.busy) return;
    state.busy = true;
    state.error = "";
    render();
    try {
      state.status = await request(path, payload);
      if (path === "/v1/mobile/pair") state.open = true;
    } catch (error) {
      state.error = error?.message || "手机连接操作未完成";
    } finally {
      state.busy = false;
      state.clock = Date.now();
      render();
    }
  }

  async function refreshStatus() {
    try {
      state.status = await request("/v1/mobile/status", null, "GET");
      state.error = "";
    } catch (error) {
      state.error = error?.message || "无法读取手机连接状态";
    }
    state.clock = Date.now();
    render();
  }

  async function refreshTasks() {
    try {
      const result = await request("/v1/mobile/tasks", {});
      state.tasks = Array.isArray(result.tasks) ? result.tasks : [];
    } catch (error) {
      state.error = error?.message || "任务列表暂不可用，请稍后刷新";
    }
    render();
  }

  async function refreshAll() {
    await Promise.all([refreshStatus(), refreshTasks()]);
  }

  function schedule() {
    window.clearTimeout(state.timer);
    state.timer = window.setTimeout(async () => {
      await refreshStatus();
      schedule();
    }, 1500);
  }

  function destroy() {
    window.clearTimeout(state.timer);
    document.getElementById(ROOT_ID)?.remove();
    document.getElementById(PANEL_ID)?.remove();
    document.getElementById(STYLE_ID)?.remove();
    if (window[API_KEY]?.destroy === destroy) window[API_KEY] = undefined;
  }

  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && state.open) {
      state.open = false;
      render();
    }
  });
  window[API_KEY] = { destroy, open: () => { state.open = true; render(); void refreshAll(); } };
  render();
  void refreshAll();
  schedule();
})();
