  // 只在原生 Resume 可用时读取最新轮次；不根据历史横幅或普通对话中的 429 触发。
  const rateLimitAutoResumeDelayMs = 30000;
  const rateLimitAutoResumeMaxAttempts = 5;
  const rateLimitAutoResumeState = {
    timer: null, busy: false, generation: 0, identity: "", attempts: 0,
    pendingTurn: "", nextAttemptAt: 0, nextReadAt: 0, attemptedTurns: new Set(), stopped: false,
  };

  function rateLimitAutoResumeEnabled() {
    return codexPlusSettings().rateLimitAutoResume === true;
  }

  function rateLimitAutoResumeRef() {
    const id = locationThreadId();
    if (!id) return null;
    const refs = sessionRows().map((row) => ({ row, ref: sessionRefFromRow(row) }))
      .filter(({ row, ref }) => ref.session_id === id && ref.host_id && isCurrentSessionRow(row, ref));
    if (!refs.length) return null;
    const ref = refs[0].ref;
    return refs.every((candidate) => candidate.ref.host_id === ref.host_id) ? ref : null;
  }

  function rateLimitAutoResumeIdentity(ref) {
    return ref ? `${ref.host_id}:${ref.session_id}` : "";
  }

  function findRateLimitAutoResumeButton() {
    // 使用语义标签而不是三角形 SVG 或 hash 类名；多输入框/弹窗时保守退出。
    const roots = Array.from(document.querySelectorAll("[data-codex-composer-root]"))
      .filter((node) => visibleElement(node) && !node.closest('[role="dialog"], [aria-modal="true"], [data-codex-plus-ext]'));
    if (roots.length !== 1) return null;
    const root = roots[0];
    const editors = Array.from(root.querySelectorAll('textarea, [contenteditable="true"]'));
    if (!editors.length || editors.some((node) => String(node.value ?? node.textContent ?? "").trim())) return null;
    const buttons = Array.from(root.querySelectorAll("button"))
      .filter((node) => /^(?:Resume|继续|恢复|继续执行)$/i.test(node.getAttribute("aria-label") || "")
        && visibleElement(node) && !node.disabled && node.getAttribute("aria-disabled") !== "true");
    return buttons.length === 1 ? buttons[0] : null;
  }

  function rateLimitAutoResumeIs429(error) {
    if (!error || typeof error !== "object") return false;
    const info = error.codexErrorInfo;
    for (const key of ["httpConnectionFailed", "responseStreamConnectionFailed", "responseStreamDisconnected", "responseTooManyFailedAttempts"]) {
      const status = info?.[key]?.httpStatusCode;
      if (status != null) return status === 429;
    }
    // 没有结构化状态码时仅匹配明确的 HTTP/状态码文案，不匹配任意裸数字。
    return /\b(?:HTTP(?:\/\d(?:\.\d)?)?\s*(?:status\s*)?[:=]?\s*429|(?:status(?:\s+code)?|unexpected\s+status)\s*[:=]?\s*429|429\s+Too Many Requests)\b/i.test(error.message || "");
  }

  function rateLimitAutoResumeReset(identity = "") {
    const state = rateLimitAutoResumeState;
    state.generation += 1;
    state.identity = identity;
    state.attempts = 0;
    state.pendingTurn = "";
    state.nextAttemptAt = 0;
    state.nextReadAt = 0;
    state.attemptedTurns.clear();
    state.stopped = false;
    rateLimitAutoResumeStatus("");
  }

  function rateLimitAutoResumeStatus(message) {
    let chip = document.getElementById("codex-plus-rate-limit-auto-resume");
    if (!message) {
      if (chip) chip.style.display = "none";
      return;
    }
    if (!chip && document.body) {
      chip = document.createElement("button");
      chip.id = "codex-plus-rate-limit-auto-resume";
      chip.type = "button";
      chip.setAttribute("data-codex-plus-ext", "rate-limit-auto-resume");
      chip.style.cssText = "position:fixed;z-index:2147483600;right:24px;bottom:188px;padding:6px 10px;border-radius:16px;background:var(--color-token-bg-primary,#202020);color:var(--color-token-text-primary,#eee);font:12px/16px sans-serif;cursor:pointer";
      chip.addEventListener("click", () => {
        rateLimitAutoResumeState.stopped = true;
        rateLimitAutoResumeState.generation += 1;
        rateLimitAutoResumeStatus("");
        setCodexPlusSetting("rateLimitAutoResume", false);
      });
      document.body.appendChild(chip);
    }
    if (chip) {
      chip.textContent = `${message} · 点击停止`;
      chip.style.display = "block";
    }
  }

  async function rateLimitAutoResumeTick() {
    const state = rateLimitAutoResumeState;
    if (!rateLimitAutoResumeEnabled()) {
      if (state.identity) rateLimitAutoResumeReset();
      return;
    }
    const ref = rateLimitAutoResumeRef();
    const identity = rateLimitAutoResumeIdentity(ref);
    if (identity !== state.identity) rateLimitAutoResumeReset(identity);
    if (!ref || state.stopped || state.busy) return;
    if (state.attempts >= rateLimitAutoResumeMaxAttempts) {
      rateLimitAutoResumeStatus(`429 自动继续已达上限 ${rateLimitAutoResumeMaxAttempts} 次`);
      return;
    }
    if (!findRateLimitAutoResumeButton()) {
      // 输入内容/原生自动重试/新轮次运行时取消待点击计划，但保留本次访问的总次数。
      state.pendingTurn = "";
      state.nextAttemptAt = 0;
      state.nextReadAt = 0;
      rateLimitAutoResumeStatus("");
      return;
    }
    if (state.pendingTurn && Date.now() < state.nextAttemptAt) {
      rateLimitAutoResumeStatus(`429 · ${Math.ceil((state.nextAttemptAt - Date.now()) / 1000)} 秒后自动继续（${state.attempts}/${rateLimitAutoResumeMaxAttempts}）`);
      return;
    }
    if (Date.now() < state.nextReadAt) return;
    const generation = state.generation;
    const href = window.location.href;
    const current = () => generation === state.generation && href === window.location.href
      && rateLimitAutoResumeEnabled() && !state.stopped
      && rateLimitAutoResumeIdentity(rateLimitAutoResumeRef()) === identity;
    state.busy = true;
    state.nextReadAt = Date.now() + 5000;
    try {
      // 复用现有 host-aware RPC 发现逻辑；读不到最新状态时不点击。
      const client = await discoverNativeHostClient(ref) || nativeHostClient(ref.host_id);
      if (!client || !current()) return;
      const result = await client.sendRequest("thread/read", { threadId: ref.session_id, includeTurns: true }, { timeoutMs: 10000 });
      if (!current()) return;
      const thread = result?.thread;
      const turn = thread?.turns?.at(-1);
      const button = findRateLimitAutoResumeButton();
      if (thread?.id !== ref.session_id || !turn?.id || turn.status !== "failed"
          || !rateLimitAutoResumeIs429(turn.error) || !button || state.attemptedTurns.has(turn.id)) {
        state.pendingTurn = "";
        state.nextAttemptAt = 0;
        // 同一个终止轮次可能长时间显示 Resume；限制完整历史读取频率。
        state.nextReadAt = Date.now() + rateLimitAutoResumeDelayMs;
        rateLimitAutoResumeStatus("");
        return;
      }
      if (state.pendingTurn !== turn.id) {
        state.pendingTurn = turn.id;
        state.nextAttemptAt = Date.now() + rateLimitAutoResumeDelayMs;
        rateLimitAutoResumeStatus("429 · 30 秒后自动继续");
        return;
      }
      // 倒计时结束后再次读最新轮次；一个失败 turn 只点一次，防止未生效时反复点击。
      state.attemptedTurns.add(turn.id);
      state.attempts += 1;
      state.pendingTurn = "";
      state.nextAttemptAt = 0;
      // 快速失败时，1 秒轮询可能看不到 Resume -> Stop -> Resume 的中间态。
      // 不依赖按钮变化解锁，稍后重新确认 turnId；attemptedTurns 防止重复点击。
      state.nextReadAt = Date.now() + rateLimitAutoResumeDelayMs;
      button.click();
      rateLimitAutoResumeStatus(`429 · 已自动继续 ${state.attempts}/${rateLimitAutoResumeMaxAttempts} 次`);
    } catch {
      // RPC/DOM 不兼容时保留有限频率的只读检查，不绕过原生按钮发送消息。
    } finally {
      state.busy = false;
    }
  }

  function syncCodexPlusRateLimitAutoResume() {
    const state = rateLimitAutoResumeState;
    if (!rateLimitAutoResumeEnabled()) {
      window.clearInterval(state.timer);
      state.timer = null;
      rateLimitAutoResumeReset();
      return;
    }
    if (!state.timer) state.timer = window.setInterval(() => void rateLimitAutoResumeTick(), 1000);
  }

  syncCodexPlusRateLimitAutoResume();
