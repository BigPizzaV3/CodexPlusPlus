  /**
   * 模型容量错误自动重试。
   *
   * 背景（本功能要解决的问题）：
   *   Codex 应用把「模型容量不足」的失败轮次渲染成一条带「重试」按钮的横幅，
   *   文案由服务端下发（例如 Selected model is at capacity. Please try a different
   *   model.）。应用本身自带自动重试（app-primary 的 yGe + SGe），但有两个门槛：
   *     1. 需要特性开关 2899820207 打开；
   *     2. 需要 SGe 选出的 retryDelaySeconds 非空。
   *   而 SGe 的延迟阶梯是 `vL = [10, 30, 120, 300]`，循环守卫 `i < vL.length`
   *   让计数最多到 4：第 5 次连续容量失败时返回 null，倒计时和自动重试同时消失，
   *   只剩一个必须手点的「重试」按钮。用户看到的现象就是「重试很多次之后就不再
   *   自动重试了」。
   *
   *   本分片补上的就是这一段：横幅还在、原生又不再自动重试时，替用户点按钮。
   *   节奏默认与手动点击一致——上一轮失败后立刻重试（0 秒间隔，内部有 1 秒地板值
   *   防热循环），间隔和次数上限都可以在 Codex++ 设置里改。
   *
   *   原生那 4 档阶梯（10/30/120/300 秒）不是我们复刻的对象，也不等待：倒计时显示
   *   「10 秒后重试」时本功能同样在下一秒就把按钮点掉，等于把原生那 4 次也压成 0 秒。
   *   倒计时只用来在浮标上标注「已接管原生倒计时」。代价是原生定时器到点后可能再触发
   *   一次它自己的重试：两条路径走的是同一个处理函数（yGe 里的 Fv(...)），进入前都会
   *   检查 thread 是否 ready、是否已在重试中，所以重复触发最多被 Codex 自己拒一次。
   *
   * 选择器策略（刻意不依赖 Codex 的 hash 类名，只依赖语义结构）：
   *   1. 横幅是 <aside>——Callout 组件的根元素（app-initial 里的 qR），源码已确认
   *      所有变体都以 <aside> 收尾；
   *   2. 只有 errorInfo === 'serverOverloaded' 且最后一轮 failed 时才渲染重试按钮
   *      （app-primary 的 hGe → vGe），所以「aside 里出现重试按钮」本身就等价于
   *      「模型容量错误」，不需要匹配服务端文案——中转站/聚合模型的文案各不相同，
   *      按文案匹配一定漏；
   *   3. 按钮文案来自 localConversation.serverOverloaded.retry（重试 / Retry），
   *      倒计时变体是 localConversation.serverOverloaded.retryCountdown
   *      （{seconds} 秒后重试 / Retry in {seconds}s）。
   */
  const capacityAutoRetryChipId = "codex-plus-capacity-auto-retry";
  const capacityAutoRetryIdlePollMs = 2000;
  const capacityAutoRetryWaitingPollMs = 1000;
  const capacityAutoRetryAbsenceTicksBeforeReset = 3;
  // 默认「上一轮失败后立刻重试」：用户观察到的原生行为就是 0s 就重试。
  // 但 0 间隔会变成热循环（一轮失败→立刻再点→立刻再失败），所以内部留 1 秒地板值，
  // 既保持「点了就重试」的手感，又不会把请求打成连发。想要退避就在设置里把间隔调大。
  const capacityAutoRetryMinIntervalMs = 1000;
  const capacityAutoRetryCountdownPattern = /(?:\d+\s*(?:秒|s|S)\s*后\s*(?:重试|再试)|Retry\s+in\s+\d+\s*s)/;
  const capacityAutoRetryLabelPattern = /^(?:重试|再试一次|重新尝试|Retry|Try again)$/;
  const capacityAutoRetryState = {
    timer: null,
    attempts: 0,
    nextAttemptAt: 0,
    absenceTicks: 0,
    nativeCountdown: false,
    exhausted: false,
    userPaused: false,
    note: "",
  };

  function capacityAutoRetrySettings() {
    try {
      return codexPlusSettings() || {};
    } catch {
      return {};
    }
  }

  function capacityAutoRetryEnabled() {
    return capacityAutoRetrySettings().capacityAutoRetry === true;
  }

  function capacityAutoRetryMaxAttempts() {
    return normalizeCapacityAutoRetryMaxAttempts(capacityAutoRetrySettings().capacityAutoRetryMaxAttempts);
  }

  function normalizeCapacityAutoRetryMaxAttempts(value) {
    const raw = Number(value);
    if (!Number.isFinite(raw)) return capacityAutoRetryDefaultMaxAttempts;
    return Math.min(capacityAutoRetryMaxAttemptsCeiling, Math.max(1, Math.round(raw)));
  }

  function normalizeCapacityAutoRetryIntervalSeconds(value) {
    const raw = Number(value);
    if (!Number.isFinite(raw)) return capacityAutoRetryDefaultIntervalSeconds;
    return Math.min(capacityAutoRetryIntervalCeilingSeconds, Math.max(0, Math.round(raw)));
  }

  function capacityAutoRetryIntervalSeconds() {
    return normalizeCapacityAutoRetryIntervalSeconds(capacityAutoRetrySettings().capacityAutoRetryIntervalSeconds);
  }

  function setCapacityAutoRetryMaxAttempts(value) {
    const next = normalizeCapacityAutoRetryMaxAttempts(value);
    setCodexPlusSetting("capacityAutoRetryMaxAttempts", next);
    return next;
  }

  function setCapacityAutoRetryIntervalSeconds(value) {
    const next = normalizeCapacityAutoRetryIntervalSeconds(value);
    setCodexPlusSetting("capacityAutoRetryIntervalSeconds", next);
    return next;
  }

  /**
   * 两次重试之间的等待时间。
   *
   * 间隔设 0 表示「立刻重试」，但仍然套 1 秒地板值：按钮在重试请求期间本来就是
   * disabled，真正的热循环风险来自「点完发现没生效又马上再点」，所以下限必须是
   * 一个能容纳一次请求往返的值。
   */
  function capacityAutoRetryWaitMs() {
    return Math.max(capacityAutoRetryMinIntervalMs, capacityAutoRetryIntervalSeconds() * 1000);
  }

  /**
   * 找一个当前可点的容量错误重试按钮。
   *
   * 从后往前找：历史轮次的横幅没有重试按钮（vGe 在没有 retryTurnId 时只渲染文案），
   * 只有最新那一条才有，但反向遍历同时也避开了「旧横幅先被点到」的可能。
   */
  function findCapacityAutoRetryBanner() {
    const roots = [];
    const main = document.querySelector("main");
    if (main) roots.push(main);
    roots.push(document.body);
    for (const root of roots) {
      if (!root) continue;
      const asides = root.querySelectorAll("aside");
      for (let index = asides.length - 1; index >= 0; index -= 1) {
        const aside = asides[index];
        // 对话框/菜单里的 aside 不是对话横幅，别误点。
        if (aside.closest?.('[role="dialog"], [role="menu"], [aria-modal="true"]')) continue;
        const button = Array.from(aside.querySelectorAll("button"))
          .find((node) => capacityAutoRetryLabelPattern.test((node.textContent || "").trim()));
        if (button) return { aside, button };
      }
    }
    return null;
  }

  function capacityAutoRetryNativeCountdownRunning(aside) {
    return capacityAutoRetryCountdownPattern.test(aside.textContent || "");
  }

  function capacityAutoRetryChip() {
    let chip = document.getElementById(capacityAutoRetryChipId);
    if (chip) return chip;
    if (!document.body) return null;
    chip = document.createElement("div");
    chip.id = capacityAutoRetryChipId;
    // 归属标记：扫描调度器据此忽略我们自己插入的节点，避免自喂扫描循环（issue #1960）。
    chip.setAttribute("data-codex-plus-ext", "capacity-auto-retry");
    chip.setAttribute("role", "status");
    chip.style.cssText = [
      "position:fixed",
      "z-index:2147483600",
      "inset-inline-end:24px",
      "bottom:148px",
      "display:none",
      "align-items:center",
      "gap:8px",
      "max-width:min(60vw,420px)",
      "padding:6px 10px",
      "border:1px solid color-mix(in srgb,currentColor 22%,transparent)",
      "border-radius:999px",
      "background:color-mix(in srgb,var(--color-token-bg-primary,#202020) 92%,transparent)",
      "color:var(--color-token-text-secondary,rgba(210,210,220,.9))",
      "font:12px/16px ui-sans-serif,system-ui,-apple-system,'PingFang SC',sans-serif",
      "box-shadow:0 4px 16px rgba(0,0,0,.28)",
      "user-select:none",
    ].join(";");
    const text = document.createElement("span");
    text.setAttribute("data-codex-plus-capacity-retry-text", "true");
    const resume = document.createElement("button");
    resume.type = "button";
    resume.setAttribute("data-codex-plus-capacity-retry-resume", "true");
    resume.textContent = "继续";
    resume.style.cssText = "display:none;cursor:pointer;color:inherit;background:transparent;border:0;font:inherit;text-decoration:underline;padding:0";
    const stop = document.createElement("button");
    stop.type = "button";
    stop.setAttribute("data-codex-plus-capacity-retry-stop", "true");
    stop.textContent = "停止";
    stop.style.cssText = "display:none;cursor:pointer;color:inherit;background:transparent;border:0;font:inherit;text-decoration:underline;padding:0";
    resume.addEventListener("click", () => {
      capacityAutoRetryState.userPaused = false;
      capacityAutoRetryState.exhausted = false;
      capacityAutoRetryState.attempts = 0;
      capacityAutoRetryState.nextAttemptAt = 0;
      capacityAutoRetryTick();
    });
    stop.addEventListener("click", () => {
      capacityAutoRetryState.userPaused = true;
      capacityAutoRetryState.nextAttemptAt = 0;
      capacityAutoRetryRenderChip();
    });
    // 用 appendChild 而不是 append(...)：cdp_bridge.rs 的 Node 注入 harness 里
    // 元素 stub 只实现了 appendChild，用 append 会在那里直接崩。
    for (const child of [text, resume, stop]) chip.appendChild(child);
    document.body.appendChild(chip);
    return chip;
  }

  /**
   * 刷新状态浮标。
   *
   * `mode` 为空字符串表示本轮已结束——直接隐藏，不留常驻节点。
   */
  function capacityAutoRetryRenderChip(mode, detail) {
    const chip = capacityAutoRetryChip();
    if (!chip) return;
    if (!mode) {
      chip.style.display = "none";
      return;
    }
    const text = chip.querySelector("[data-codex-plus-capacity-retry-text]");
    const resume = chip.querySelector("[data-codex-plus-capacity-retry-resume]");
    const stop = chip.querySelector("[data-codex-plus-capacity-retry-stop]");
    // 取不到子节点说明浮标被 Codex 的重渲染搬走了；这种情况下宁可只更新容器，
    // 也不要抛错中断整条重试链（注入层任何未捕获异常都会记进 scan 失败列表）。
    if (!text || !resume || !stop) {
      chip.style.display = "inline-flex";
      return;
    }
    const max = capacityAutoRetryMaxAttempts();
    const remaining = capacityAutoRetryState.nextAttemptAt > Date.now()
      ? Math.max(0, Math.ceil((capacityAutoRetryState.nextAttemptAt - Date.now()) / 1000))
      : 0;
    let label = "";
    if (mode === "paused") {
      label = `自动重试已暂停（已试 ${capacityAutoRetryState.attempts}/${max} 次）`;
    } else if (mode === "exhausted") {
      label = `自动重试已达上限 ${max} 次`;
    } else if (mode === "waiting") {
      label = `模型容量不足 · 自动重试第 ${Math.min(capacityAutoRetryState.attempts + 1, max)}/${max} 次${remaining > 0 ? ` · ${remaining} 秒后` : ""}${capacityAutoRetryState.nativeCountdown ? " · 已接管原生倒计时" : ""}`;
    } else {
      label = detail || "模型容量不足 · 自动重试中";
    }
    text.textContent = label;
    resume.style.display = mode === "paused" || mode === "exhausted" ? "inline" : "none";
    stop.style.display = mode === "paused" || mode === "exhausted" ? "none" : "inline";
    chip.style.display = "inline-flex";
  }

  function capacityAutoRetrySchedule(delayMs) {
    clearTimeout(capacityAutoRetryState.timer);
    capacityAutoRetryState.timer = window.setTimeout(capacityAutoRetryTick, Math.max(200, delayMs));
  }

  function capacityAutoRetryResetRun() {
    capacityAutoRetryState.attempts = 0;
    capacityAutoRetryState.nextAttemptAt = 0;
    capacityAutoRetryState.exhausted = false;
    capacityAutoRetryState.userPaused = false;
  }

  function capacityAutoRetryTick() {
    capacityAutoRetryState.timer = null;
    if (!capacityAutoRetryEnabled()) {
      capacityAutoRetryResetRun();
      capacityAutoRetryState.absenceTicks = 0;
      capacityAutoRetryRenderChip("");
      return;
    }
    const banner = findCapacityAutoRetryBanner();
    if (!banner) {
      // 横幅消失 = 本轮容量错误结束（重试成功、或用户换了模型/新开了对话）。
      // 不立刻清零：点击到新横幅渲染之间会有短暂空档，按 tick 数确认后再复位。
      capacityAutoRetryState.absenceTicks += 1;
      if (capacityAutoRetryState.absenceTicks >= capacityAutoRetryAbsenceTicksBeforeReset) {
        capacityAutoRetryResetRun();
        capacityAutoRetryRenderChip("");
      }
      capacityAutoRetrySchedule(capacityAutoRetryIdlePollMs);
      return;
    }
    capacityAutoRetryState.absenceTicks = 0;
    // 原生倒计时（10/30/120/300 秒）在跑时也照样立刻接管——用户要的是「每次都是 0 秒
    // 就重试」，不是等原生那套阶梯。这里只把它记下来显示在浮标上，让用户知道
    // Codex 自己也排了一次重试。代价是原生定时器到点后可能再触发一次它自己的重试：
    // 两条路径走同一个处理函数、同一道 ready/重试中守卫，最多被 Codex 自己拒一次。
    capacityAutoRetryState.nativeCountdown = capacityAutoRetryNativeCountdownRunning(banner.aside);
    if (capacityAutoRetryState.userPaused) {
      capacityAutoRetryRenderChip("paused");
      capacityAutoRetrySchedule(capacityAutoRetryIdlePollMs);
      return;
    }
    if (capacityAutoRetryState.exhausted) {
      capacityAutoRetryRenderChip("exhausted");
      capacityAutoRetrySchedule(capacityAutoRetryIdlePollMs * 2);
      return;
    }
    if (banner.button.disabled) {
      // 重试请求在途（yGe 的 isRetrying 会禁用按钮），等它结束。
      capacityAutoRetryRenderChip("waiting");
      capacityAutoRetrySchedule(capacityAutoRetryIdlePollMs);
      return;
    }
    const max = capacityAutoRetryMaxAttempts();
    if (capacityAutoRetryState.attempts >= max) {
      capacityAutoRetryState.exhausted = true;
      // showToast 在 80-session-share.js；切片沙箱里可能没有它，缺了就静默跳过。
      if (typeof showToast === "function") {
        showToast(`自动重试已达上限 ${max} 次，仍未抢到模型容量；可点浮标上的「继续」再试。`, { type: "warn" });
      }
      capacityAutoRetryRenderChip("exhausted");
      capacityAutoRetrySchedule(capacityAutoRetryIdlePollMs * 2);
      return;
    }
    const now = Date.now();
    if (now < capacityAutoRetryState.nextAttemptAt) {
      capacityAutoRetryRenderChip("waiting");
      capacityAutoRetrySchedule(Math.min(capacityAutoRetryWaitingPollMs, capacityAutoRetryState.nextAttemptAt - now));
      return;
    }
    // 到点了：点按钮。Codex 的按钮是 React onClick，根容器上的委托监听会收到
    // 冒泡事件，所以原生 click() 与真实点击等价。
    try {
      banner.button.click();
    } catch {
      // 点击抛错不该打断循环，等下一轮再试。
    }
    capacityAutoRetryState.attempts += 1;
    capacityAutoRetryState.nextAttemptAt = Date.now() + capacityAutoRetryWaitMs();
    capacityAutoRetryRenderChip("waiting");
    capacityAutoRetrySchedule(capacityAutoRetryWaitingPollMs);
  }

  /**
   * 按当前设置启停自动重试。
   *
   * 幂等：开启时确保有一个定时器在跑，关闭时清干净（含浮标），可以随时重复调用。
   */
  function syncCodexPlusCapacityAutoRetry() {
    if (!capacityAutoRetryEnabled()) {
      clearTimeout(capacityAutoRetryState.timer);
      capacityAutoRetryState.timer = null;
      capacityAutoRetryResetRun();
      capacityAutoRetryState.absenceTicks = 0;
      capacityAutoRetryRenderChip("");
      return;
    }
    if (capacityAutoRetryState.timer) return;
    capacityAutoRetrySchedule(capacityAutoRetryIdlePollMs);
  }

  // 诊断/联调入口：出问题时可在控制台直接看状态或强制走一轮。
  window.__codexPlusCapacityAutoRetry = {
    state: capacityAutoRetryState,
    tick: capacityAutoRetryTick,
    sync: syncCodexPlusCapacityAutoRetry,
    find: findCapacityAutoRetryBanner,
    intervalSeconds: capacityAutoRetryIntervalSeconds,
    maxAttempts: capacityAutoRetryMaxAttempts,
  };

  syncCodexPlusCapacityAutoRetry();
