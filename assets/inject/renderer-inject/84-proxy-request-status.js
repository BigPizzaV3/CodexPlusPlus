  /**
   * 输入框上方的「请求避震」状态条：直接显示当前代理请求走到了哪一步。
   *
   * 状态由代理侧 `/proxy-status` 路由提供（POST），阶段包括：
   *   requesting（正在请求）、waiting（等待上游返回）、
   *   retrying（正在重试 ×N）、generating（正在生成，可带输出 token 估算）。
   *
   * 节点挂在输入卡片之前、同一父容器内（输入框上方），空闲时 display:none，
   * 不占位；带 data-codex-plus-ext 让扫描调度器排除自喂循环。
   */
  registerCodexPlusExtensionSelector('[data-codex-plus-ext="builtin-proxy-request-status"]');
  window.__codexPlusProxyRequestStatusRuntime?.dispose?.();

  const codexPlusProxyStatusId = "codex-plus-proxy-request-status";
  const codexPlusProxyStatusStyleId = "codex-plus-proxy-request-status-style";
  const codexPlusProxyStatusPollMs = 1200;
  const codexPlusProxyStatusTickMs = 500;

  const codexPlusProxyStatusState = {
    disposed: false,
    pollTimer: 0,
    tickTimer: 0,
    node: null,
    requests: [],
    nowMs: Date.now(),
    inFlight: false,
    failures: 0,
  };

  function codexPlusProxyStatusVisibleElement(element) {
    if (!element?.isConnected) return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }

  function codexPlusProxyStatusComposerCard() {
    const cards = [...document.querySelectorAll("[data-composer-surface-variant]")];
    let fallback = null;
    for (const card of cards) {
      if (!codexPlusProxyStatusVisibleElement(card) || card.closest("[data-codex-plus-ext]")) continue;
      fallback = card;
      if (card.querySelector('.ProseMirror[contenteditable="true"], textarea:not(:disabled):not([readonly]), [role="textbox"][contenteditable="true"]')) {
        return card;
      }
    }
    return fallback;
  }

  /** 当前输入框所属会话 id，用于只显示本会话的请求。 */
  function codexPlusProxyStatusThreadId(card) {
    const composerRoot = card?.closest("[data-codex-composer-root]");
    const placement = composerRoot?.getAttribute("data-composer-placement");
    // 首页/新会话的输入框也是 composer，但没有会话归属；必须返回空让状态条隐藏，
    // 不能退化成“显示所有运行中的请求”，否则别的会话的请求会串到这里。
    if (placement && placement !== "thread") return "";
    const fromComposer = composerRoot
      ?.querySelector("[data-above-composer-conversation-id]")
      ?.getAttribute("data-above-composer-conversation-id");
    const inThread = composerRoot?.getAttribute("data-composer-placement") === "thread";
    const fromSidebar = inThread
      ? document
        .querySelector('[data-app-action-sidebar-thread-selected="true"][data-app-action-sidebar-thread-id]')
        ?.getAttribute("data-app-action-sidebar-thread-id")
      : null;
    return String(fromComposer || fromSidebar || "").replace(/^local:/, "").trim();
  }

  function codexPlusProxyStatusNormalizeThreadId(value) {
    return String(value || "").replace(/^local:/, "").trim();
  }

  function codexPlusProxyStatusText() {
    const language = (document.documentElement?.lang || navigator.language || "").toLowerCase();
    const zh = language.startsWith("zh");
    return zh
      ? {
          requesting: "正在请求",
          waiting: "等待上游返回",
          retrying: (count) => `正在重试 ×${count}`,
          generating: "正在生成",
          fallback: "请求进行中",
          lastError: "上次",
          aria: "请求避震状态",
        }
      : {
          requesting: "Requesting",
          waiting: "Waiting for upstream",
          retrying: (count) => `Retrying ×${count}`,
          generating: "Generating",
          fallback: "Request in progress",
          lastError: "Last",
          aria: "Request shock absorption status",
        };
  }

  function codexPlusProxyStatusInstallStyle() {
    if (document.getElementById(codexPlusProxyStatusStyleId)) return;
    const style = document.createElement("style");
    style.id = codexPlusProxyStatusStyleId;
    style.dataset.codexPlusExt = "builtin-proxy-request-status";
    style.textContent = `
      #${codexPlusProxyStatusId}{display:flex;align-items:center;justify-content:flex-end;gap:8px;box-sizing:border-box;width:100%;margin:0 0 6px;padding:0 12px;font-size:12px;line-height:17px;color:var(--color-token-text-secondary,rgba(205,205,215,.95));color:color-mix(in srgb,var(--color-token-text-primary,#dfdfdf) 82%,transparent);user-select:none;cursor:default}
      #${codexPlusProxyStatusId}[data-hidden="true"]{display:none}
      #${codexPlusProxyStatusId} .codex-plus-proxy-dot{width:7px;height:7px;border-radius:50%;flex:0 0 auto;background:#f59e0b;box-shadow:0 0 0 0 rgba(245,158,11,.35);animation:codex-plus-proxy-pulse 1.4s ease-out infinite}
      #${codexPlusProxyStatusId}[data-phase="retrying"] .codex-plus-proxy-dot{background:#f97316}
      #${codexPlusProxyStatusId}[data-phase="generating"] .codex-plus-proxy-dot{background:#22c55e}
      #${codexPlusProxyStatusId}[data-phase="waiting"] .codex-plus-proxy-dot{background:#60a5fa}
      #${codexPlusProxyStatusId} .codex-plus-proxy-phase{font-weight:500;white-space:nowrap}
      #${codexPlusProxyStatusId} .codex-plus-proxy-meta{opacity:.65;white-space:nowrap;font-variant-numeric:tabular-nums}
      #${codexPlusProxyStatusId} .codex-plus-proxy-error{max-width:280px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#f87171;opacity:.85}
      @keyframes codex-plus-proxy-pulse{0%{box-shadow:0 0 0 0 rgba(245,158,11,.35)}70%{box-shadow:0 0 0 5px rgba(245,158,11,0)}100%{box-shadow:0 0 0 0 rgba(245,158,11,0)}}
    `;
    document.head?.appendChild(style);
  }

  function codexPlusProxyStatusEnsureNode(card) {
    // 挂在 composer root 之前，而不是输入卡片之前：切会话时卡片会被 React
    // 放进只包住输入框的 relative 容器里，插在卡片前会跑到输入框内部。
    const anchor = card?.closest("[data-codex-composer-root]") || card;
    if (!anchor?.parentElement) return null;
    let node = codexPlusProxyStatusState.node;
    if (!node) {
      node = document.createElement("div");
      node.id = codexPlusProxyStatusId;
      node.setAttribute("data-codex-plus-ext", "builtin-proxy-request-status");
      node.setAttribute("role", "status");
      codexPlusProxyStatusState.node = node;
    }
    if (!node.isConnected || node.nextElementSibling !== anchor) {
      anchor.insertAdjacentElement("beforebegin", node);
    }
    return node;
  }

  function codexPlusProxyStatusHide() {
    const node = codexPlusProxyStatusState.node;
    if (node) node.dataset.hidden = "true";
  }

  function codexPlusProxyStatusFormatDuration(ms) {
    const totalSeconds = Math.max(0, Math.floor(ms / 1000));
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return minutes > 0 ? `${minutes}m${String(seconds).padStart(2, "0")}s` : `${seconds}s`;
  }

  function codexPlusProxyStatusRender() {
    const card = codexPlusProxyStatusComposerCard();
    const threadId = card ? codexPlusProxyStatusThreadId(card) : "";
    // 首页/新会话没有当前会话 id：不能把“没有会话”当成“不过滤”，
    // 否则会把别的会话里正在运行的请求显示到首页（串会话）。
    if (!card || !threadId) {
      codexPlusProxyStatusHide();
      return;
    }
    const visibleRequests = codexPlusProxyStatusState.requests.filter(
      (item) => codexPlusProxyStatusNormalizeThreadId(item?.threadId) === threadId,
    );
    const request = visibleRequests[0];
    if (!request) {
      codexPlusProxyStatusHide();
      return;
    }
    const node = codexPlusProxyStatusEnsureNode(card);
    if (!node) return;
    const text = codexPlusProxyStatusText();
    const phase = String(request.phase || "");
    const retries = Number(request.autoRetries) || 0;
    const attempts = Number(request.attempts) || 0;
    let label = text.fallback;
    if (phase === "requesting") label = text.requesting;
    else if (phase === "waiting") label = text.waiting;
    else if (phase === "retrying") label = text.retrying(Math.max(1, retries));
    else if (phase === "generating") label = text.generating;
    else if (retries > 0) label = text.retrying(retries);

    const attemptDetails = Array.isArray(request.attemptDetails) ? request.attemptDetails : [];
    const lastAttempt = attemptDetails.length ? attemptDetails[attemptDetails.length - 1] : null;
    const lastErrorCode = String(lastAttempt?.code || "").trim();
    const lastErrorMessage = String(lastAttempt?.message || "").trim();
    const lastError = lastErrorCode || lastErrorMessage.slice(0, 80);

    const meta = [];
    if (request.model) meta.push(String(request.model));
    const startedAtMs = Number(request.startedAtMs) || 0;
    if (startedAtMs > 0) meta.push(codexPlusProxyStatusFormatDuration(codexPlusProxyStatusState.nowMs - startedAtMs));
    const estimated = Number(request.estimatedOutputTokens) || 0;
    if (phase === "generating" && estimated > 0) meta.push(`~${estimated > 999 ? `${(estimated / 1000).toFixed(1)}k` : estimated} tok`);
    else if (attempts > 1) meta.push(`第 ${attempts} 次`);
    if (visibleRequests.length > 1) meta.push(`+${visibleRequests.length - 1}`);

    node.dataset.hidden = "false";
    node.dataset.phase = phase || "requesting";
    node.setAttribute("aria-label", text.aria);
    node.replaceChildren();
    const dot = document.createElement("span");
    dot.className = "codex-plus-proxy-dot";
    const phaseNode = document.createElement("span");
    phaseNode.className = "codex-plus-proxy-phase";
    phaseNode.textContent = label;
    node.append(dot, phaseNode);
    if (meta.length) {
      const metaNode = document.createElement("span");
      metaNode.className = "codex-plus-proxy-meta";
      metaNode.textContent = meta.join(" · ");
      node.appendChild(metaNode);
    }
    if (lastError) {
      const errorNode = document.createElement("span");
      errorNode.className = "codex-plus-proxy-error";
      errorNode.textContent = `${text.lastError} ${lastError}`;
      errorNode.title = lastErrorMessage || lastErrorCode;
      node.appendChild(errorNode);
    }
  }

  async function codexPlusProxyStatusPostWithTimeout() {
    // 桥接挂死时 __codexSessionDeleteBridge 的 Promise 可能永远不 settle。
    // 不设超时会让 inFlight 永久为 true，状态条从此不再刷新（只能重启恢复）。
    let timer = 0;
    try {
      return await Promise.race([
        postJson("/proxy-status", {}),
        new Promise((resolve) => {
          timer = window.setTimeout(
            () => resolve({ status: "failed", message: "桥接请求超时", timeout: true }),
            2500,
          );
        }),
      ]);
    } finally {
      window.clearTimeout(timer);
    }
  }

  async function codexPlusProxyStatusRefresh() {
    if (codexPlusProxyStatusState.disposed || codexPlusProxyStatusState.inFlight) return;
    if (document.hidden) return;
    codexPlusProxyStatusState.inFlight = true;
    try {
      const result = await codexPlusProxyStatusPostWithTimeout();
      if (result?.status === "ok" && Array.isArray(result.requests)) {
        codexPlusProxyStatusState.requests = result.requests;
        codexPlusProxyStatusState.failures = 0;
      } else if (result?.timeout) {
        // 让看门狗把桥接判定为失效并重注入；桥恢复后下一轮轮询自动接上。
        recordCodexPlusBridgeFailure();
        throw new Error("proxy-status bridge timeout");
      }
    } catch {
      codexPlusProxyStatusState.failures += 1;
    } finally {
      codexPlusProxyStatusState.inFlight = false;
    }
    if (!codexPlusProxyStatusState.disposed) codexPlusProxyStatusRender();
  }

  codexPlusProxyStatusInstallStyle();
  codexPlusProxyStatusRender();
  codexPlusProxyStatusState.pollTimer = window.setInterval(
    () => void codexPlusProxyStatusRefresh(),
    codexPlusProxyStatusPollMs,
  );
  codexPlusProxyStatusState.tickTimer = window.setInterval(() => {
    codexPlusProxyStatusState.nowMs = Date.now();
    codexPlusProxyStatusRender();
  }, codexPlusProxyStatusTickMs);
  void codexPlusProxyStatusRefresh();

  window.__codexPlusProxyRequestStatusRuntime = {
    refresh: codexPlusProxyStatusRefresh,
    dispose() {
      codexPlusProxyStatusState.disposed = true;
      window.clearInterval(codexPlusProxyStatusState.pollTimer);
      window.clearInterval(codexPlusProxyStatusState.tickTimer);
      codexPlusProxyStatusState.node?.remove();
      codexPlusProxyStatusState.node = null;
      document.getElementById(codexPlusProxyStatusStyleId)?.remove();
    },
  };
