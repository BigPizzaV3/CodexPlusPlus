  /**
   * 输入框下方用量状态栏：生成速度 / 缓存命中率 / 上下文占用。
   *
   * 数据全部来自 Codex 自己写的 rollout 记录（`event_msg.token_count`），中文档口径
   * 与 Codex 一致：缓存命中 = cached_input_tokens / input_tokens，上下文占用 =
   * last_token_usage.total_tokens / model_context_window。这里的 model_context_window
   * 正是 Codex++ 注入 model_catalog_json 后 Codex 认到的每模型窗口，所以按模型配置
   * 窗口的功能会直接反映在这一项上。
   *
   * 为什么不复用 `/thread-usage-history`：那条路由按会话整读 rollout 并解析全部行，
   * 真实会话的 rollout 可以到几百 MB，状态栏按秒级轮询扛不住。这里走
   * `/thread-usage-live`，宿主只读文件尾部。
   *
   * 节点挂在输入卡片（[data-composer-surface-variant]）之后、同一父容器内，也就是
   * 「输入框下方、卡片外」。节点带 data-codex-plus-ext，扫描调度器会把它排除在
   * 自喂循环之外（issue #1960）。
   */
  registerCodexPlusExtensionSelector('[data-codex-plus-ext="builtin-usage-statusline"]');
  window.__codexPlusUsageStatusLineRuntime?.dispose?.();

  const codexPlusUsageStatusLineId = "codex-plus-usage-statusline";
  const codexPlusUsageStyleId = "codex-plus-usage-statusline-style";
  /** 空闲轮询间隔；最近有 token 事件时切到活跃间隔。 */
  const codexPlusUsageIdleIntervalMs = 5000;
  /** 本轮在跑（最近有事件）时的间隔。 */
  const codexPlusUsageActiveIntervalMs = 1200;
  /** 已经拿到会话、但还没写出第一条 token 事件时的间隔。 */
  const codexPlusUsageWarmupIntervalMs = 2500;
  /** 最近一条 token 事件超过这个时间就认为本轮已经结束，速度值转为淡化显示。 */
  const codexPlusUsageStaleMs = 120000;
  /** 连续失败到这个次数后停掉轮询，避免桥接不可用时每秒写诊断日志。 */
  const codexPlusUsageFailureLimit = 5;

  const codexPlusUsageState = {
    disposed: false,
    timer: 0,
    node: null,
    parts: null,
    threadId: "",
    failures: 0,
    inFlight: false,
    lastFingerprint: "",
    cache: new Map(),
    observer: null,
  };

  const codexPlusUsageSvgNs = "http://www.w3.org/2000/svg";

  function codexPlusUsageText() {
    const language = (document.documentElement?.lang || navigator.language || "").toLowerCase();
    return language.startsWith("zh")
      ? { speed: (value) => `${value} tok/s`, cache: (value) => `缓存命中 ${value}`, context: (value) => value, aria: "用量状态栏" }
      : { speed: (value) => `${value} tok/s`, cache: (value) => `cache hit ${value}`, context: (value) => value, aria: "Usage status line" };
  }

  function codexPlusUsageInstallStyle() {
    if (document.getElementById(codexPlusUsageStyleId)) return;
    const style = document.createElement("style");
    style.id = codexPlusUsageStyleId;
    style.dataset.codexPlusExt = "builtin-usage-statusline";
    style.textContent = `
      /* 颜色只用一层透明度：主题的 --color-token-text-secondary 本身就是 65% 透明，
         再叠容器 opacity 会掉到 0.3 出头，实测在 #141414 上几乎看不清。
         这里以主文字色取 82%，既够亮又略低于输入框文字。 */
      /* 三项作为一组居中，和 DSH 那条一致；两端对齐会把「上下文占用」甩到最右边，
         和速度、缓存拉得太开，读起来是两段信息。 */
      #${codexPlusUsageStatusLineId}{display:flex;align-items:center;justify-content:center;gap:24px;box-sizing:border-box;width:100%;margin:6px 0 0;padding:0 10px;font:inherit;font-size:12px;line-height:17px;color:var(--color-token-text-secondary,rgba(205,205,215,.95));color:color-mix(in srgb,var(--color-token-text-primary,#dfdfdf) 82%,transparent);user-select:none;cursor:default}
      #${codexPlusUsageStatusLineId}[data-hidden="true"]{display:none}
      #${codexPlusUsageStatusLineId} .codex-plus-usage-item{display:inline-flex;align-items:center;gap:5px;white-space:nowrap;font-variant-numeric:tabular-nums;transition:opacity .2s}
      #${codexPlusUsageStatusLineId} .codex-plus-usage-value{font-weight:500}
      #${codexPlusUsageStatusLineId} .codex-plus-usage-item svg{flex:0 0 auto;opacity:.8}
      /* 本轮结束超过阈值只淡化速度：缓存命中与上下文占用仍然是当前值，不该跟着变灰。 */
      #${codexPlusUsageStatusLineId}[data-stale="true"] .codex-plus-usage-item[data-usage="speed"]{opacity:.55}
    `;
    document.head?.appendChild(style);
  }

  /** 描边图标，尺寸与颜色都跟随文字，避免在浅色主题下露白。 */
  function codexPlusUsageIcon(shapes) {
    const svg = document.createElementNS(codexPlusUsageSvgNs, "svg");
    svg.setAttribute("viewBox", "0 0 16 16");
    svg.setAttribute("width", "13");
    svg.setAttribute("height", "13");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "1.4");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    svg.setAttribute("aria-hidden", "true");
    const created = shapes.map((shape) => {
      const node = document.createElementNS(codexPlusUsageSvgNs, shape.tag);
      Object.entries(shape.attrs).forEach(([key, value]) => node.setAttribute(key, value));
      svg.appendChild(node);
      return node;
    });
    return { svg, created };
  }

  /** 上下文用进度环，环长 2πr；r=6 时周长约 37.7。 */
  const codexPlusUsageRingCircumference = 37.7;

  function codexPlusUsageRingIcon() {
    const icon = codexPlusUsageIcon([
      { tag: "circle", attrs: { cx: "8", cy: "8", r: "6", "stroke-opacity": ".3" } },
      { tag: "circle", attrs: { cx: "8", cy: "8", r: "6", "stroke-dasharray": `0 ${codexPlusUsageRingCircumference}`, transform: "rotate(-90 8 8)" } },
    ]);
    return { svg: icon.svg, ring: icon.created[1] };
  }

  function codexPlusUsageItem(node, key, label, icon) {
    const item = document.createElement("span");
    item.className = "codex-plus-usage-item";
    item.dataset.usage = key;
    item.appendChild(icon);
    const value = document.createElement("span");
    value.className = "codex-plus-usage-value";
    value.textContent = "--";
    item.appendChild(value);
    node.appendChild(item);
    return { item, value, label };
  }

  /** DOM 只建一次；之后每轮只改文本与环长，避免每秒重建节点触发扫描。 */
  function codexPlusUsageBuild(node) {
    node.textContent = "";
    const ring = codexPlusUsageRingIcon();
    const parts = {
      speed: codexPlusUsageItem(node, "speed", "speed", codexPlusUsageIcon([
        { tag: "path", attrs: { d: "M2 12.5a6 6 0 0 1 12 0" } },
        { tag: "path", attrs: { d: "M8 12.5 10.8 8.6" } },
      ]).svg),
      cache: codexPlusUsageItem(node, "cache", "cache", codexPlusUsageIcon([
        { tag: "path", attrs: { d: "M8 2.2c2.5 0 4.5.8 4.5 1.8S10.5 5.8 8 5.8 3.5 5 3.5 4 5.5 2.2 8 2.2Z" } },
        { tag: "path", attrs: { d: "M3.5 4v8c0 1 2 1.8 4.5 1.8s4.5-.8 4.5-1.8V4" } },
        { tag: "path", attrs: { d: "M3.5 8c0 1 2 1.8 4.5 1.8S12.5 9 12.5 8" } },
      ]).svg),
      context: codexPlusUsageItem(node, "context", "context", ring.svg),
      ring: ring.ring,
    };
    codexPlusUsageState.parts = parts;
    return parts;
  }

  function codexPlusUsageComposerCard() {
    const cards = [...document.querySelectorAll("[data-composer-surface-variant]")];
    let fallback = null;
    for (const card of cards) {
      if (!visibleElement(card) || card.closest('[data-codex-plus-ext]')) continue;
      fallback = card;
      // 首页可能同时挂着多个 composer 外壳；带真实输入框的那个才是当前会话。
      if (card.querySelector('.ProseMirror[contenteditable="true"], textarea:not(:disabled):not([readonly]), [role="textbox"][contenteditable="true"]')) {
        return card;
      }
    }
    return fallback;
  }

  /**
   * 当前会话 id。
   *
   * `[data-above-composer-conversation-id]` 是 Codex 自己挂在 composer 上的会话标识，
   * 比侧边栏选中行更贴近「这个输入框属于哪个会话」。侧边栏只作兜底，而且**只在
   * placement=thread 时用**：首页/新会话草稿的 composer 是 placement=home，此时侧边栏
   * 仍选中着上一个 thread，兜底会把上一个会话的读数挂到新会话上。
   * 新会话草稿的 id 形如 `local:client-new-thread:<uuid>`，还没有 rollout，直接当作无会话。
   */
  function codexPlusUsageThreadId(card) {
    const composerRoot = card?.closest("[data-codex-composer-root]");
    const fromComposer = composerRoot
      ?.querySelector("[data-above-composer-conversation-id]")
      ?.getAttribute("data-above-composer-conversation-id");
    const inThread = composerRoot?.getAttribute("data-composer-placement") === "thread";
    const fromSidebar = inThread
      ? document
        .querySelector('[data-app-action-sidebar-thread-selected="true"][data-app-action-sidebar-thread-id]')
        ?.getAttribute("data-app-action-sidebar-thread-id")
      : null;
    const id = String(fromComposer || fromSidebar || "").replace(/^local:/, "").trim();
    if (!id || id === "__proto__" || id === "prototype" || id === "constructor") return "";
    return /^[A-Za-z0-9_.-]{8,128}$/.test(id) ? id : "";
  }

  function codexPlusUsageEnsureNode(card) {
    const host = card?.parentElement;
    if (!host) return null;
    let node = codexPlusUsageState.node;
    if (node?.isConnected && node.previousElementSibling === card) return node;
    if (!node) {
      node = document.createElement("div");
      node.id = codexPlusUsageStatusLineId;
      node.setAttribute("data-codex-plus-ext", "builtin-usage-statusline");
      node.setAttribute("role", "status");
      codexPlusUsageState.node = node;
    }
    card.insertAdjacentElement("afterend", node);
    return node;
  }

  function codexPlusUsageHide() {
    const node = codexPlusUsageState.node;
    if (node) {
      // 只切属性、不摘节点：新会话在第一条 token 事件写出前会反复走空数据路径，
      // 每秒增删一次节点既浪费又会让 Codex 的 MutationObserver 空转。
      node.dataset.hidden = "true";
    }
    codexPlusUsageState.lastFingerprint = "";
  }

  function codexPlusUsageShow(node) {
    if (node.dataset.hidden !== "true") return;
    node.dataset.hidden = "false";
    codexPlusUsageState.lastFingerprint = "";
  }

  function codexPlusUsagePercent(numerator, denominator) {
    if (!(denominator > 0) || !(numerator >= 0)) return null;
    return Math.min(100, Math.max(0, (numerator / denominator) * 100));
  }

  /**
   * 只取真实 turn_id 对应的最近一轮。total_token_usage 是会话累计，不会在换轮时
   * 清零；旧记录或尾部截断缺少轮边界时只用最后一次调用，避免把前几轮混进来。
   */
  function codexPlusUsageLatestTurn(events) {
    if (!Array.isArray(events) || !events.length) return [];
    let start = events.length - 1;
    const turnId = events[start].turn_id;
    if (typeof turnId === "string" && turnId) {
      while (start > 0 && events[start - 1].turn_id === turnId) start -= 1;
    }
    return events.slice(start);
  }

  /** 同一轮的多次模型调用按输入 token 加权，不能直接平均各次百分比。 */
  function codexPlusUsageTurnCache(events) {
    const segment = codexPlusUsageLatestTurn(events);
    let input = 0;
    let cached = 0;
    for (const event of segment) {
      const currentInput = Number(event.input_tokens) || 0;
      const currentCached = Number(event.cached_input_tokens) || 0;
      if (currentInput > 0) {
        input += currentInput;
        cached += Math.max(0, Math.min(currentCached, currentInput));
      }
    }
    return input > 0 ? codexPlusUsagePercent(cached, input) : null;
  }

  /**
   * 生成速度（token/s）。
   *
   * Codex 只在每次模型调用结束时写一条 token_count，事件间隔里同时包含生成和工具
   * 执行时间，所以这是「吞吐」而不是纯生成速度，会比真实解码速度低。用同一轮内
   * 相邻事件的累计输出差除以时间差得到区间速度，再取最近几个区间平均，避免单次
   * 抖动。turn_output_tokens 是历史字段名，实际为会话累计输出，需用真实轮 id 切段。
   */
  function codexPlusUsageRate(events) {
    const segment = codexPlusUsageLatestTurn(events);
    const rates = [];
    for (let index = 1; index < segment.length; index += 1) {
      const previous = segment[index - 1];
      const current = segment[index];
      const previousOutput = Number(previous.turn_output_tokens) || 0;
      const currentOutput = Number(current.turn_output_tokens) || 0;
      if (currentOutput <= previousOutput) continue;
      const startedAt = Date.parse(previous.t);
      const endedAt = Date.parse(current.t);
      const seconds = (endedAt - startedAt) / 1000;
      // 间隔过短说明是同一批写入，过长说明中间停了很久，都不适合当速度样本。
      if (!(seconds >= 0.5) || seconds > 120) continue;
      rates.push((currentOutput - previousOutput) / seconds);
    }
    if (!rates.length) return null;
    const recent = rates.slice(-3);
    return recent.reduce((total, rate) => total + rate, 0) / recent.length;
  }

  function codexPlusUsageSnapshot(events) {
    if (!Array.isArray(events) || !events.length) return null;
    const latest = events[events.length - 1];
    const observedAt = Date.parse(latest.t);
    return {
      speed: codexPlusUsageRate(events),
      cache: codexPlusUsageTurnCache(events),
      context: codexPlusUsagePercent(Number(latest.total_tokens), Number(latest.context_window)),
      stale: Number.isFinite(observedAt) ? Date.now() - observedAt > codexPlusUsageStaleMs : false,
      modelContextWindow: Number(latest.context_window) || 0,
    };
  }

  /** `snapshot` 为 null 时渲染占位符：会话已确定但还没有第一条 token 事件。 */
  function codexPlusUsageRender(snapshot) {
    const node = codexPlusUsageState.node;
    if (!node?.isConnected) return;
    const parts = codexPlusUsageState.parts || codexPlusUsageBuild(node);
    const text = codexPlusUsageText();
    const format = (value) => (value === null || value === undefined || Number.isNaN(value) ? "--" : value);
    const speed = snapshot?.speed == null ? "--" : String(Math.max(0, Math.round(snapshot.speed)));
    const cache = snapshot?.cache == null ? "--" : `${snapshot.cache.toFixed(1)}%`;
    const context = snapshot?.context == null ? "--" : `${Math.round(snapshot.context)}%`;
    const stale = snapshot?.stale === true;
    const contextWindow = snapshot?.modelContextWindow || 0;
    // 指纹一样就不写 DOM：每秒一次无意义赋值会让 Codex 的 MutationObserver 空转。
    const fingerprint = [speed, cache, context, stale, contextWindow].join("|");
    if (fingerprint === codexPlusUsageState.lastFingerprint) return;
    codexPlusUsageState.lastFingerprint = fingerprint;

    parts.speed.value.textContent = text.speed(speed);
    parts.cache.value.textContent = text.cache(cache);
    parts.context.value.textContent = text.context(context);
    if (parts.ring) {
      const filled = snapshot?.context == null ? 0 : (snapshot.context / 100) * codexPlusUsageRingCircumference;
      parts.ring.setAttribute("stroke-dasharray", `${filled.toFixed(2)} ${codexPlusUsageRingCircumference}`);
    }
    node.dataset.stale = String(stale);
    node.setAttribute("title", [
      `生成速度 ${format(speed)} tok/s`,
      `缓存命中 ${format(cache)}`,
      `上下文占用 ${format(context)}`,
      contextWindow > 0 ? `窗口 ${contextWindow}` : "",
    ].filter(Boolean).join(" · "));
    node.setAttribute("aria-label", `${text.aria}：${text.speed(speed)}，${text.cache(cache)}，${context}`);
  }

  function codexPlusUsageSchedule(delay) {
    if (codexPlusUsageState.disposed) return;
    clearTimeout(codexPlusUsageState.timer);
    codexPlusUsageState.timer = window.setTimeout(codexPlusUsageTick, delay);
  }

  function codexPlusUsageEnabled() {
    return !codexPlusUsageState.disposed
      && codexPlusBackendSettingsLoaded
      && codexPlusSettings().usageStatusLine === true;
  }

  /** 挂载与取数分开：切换时先恢复栏位，网络请求只更新数值。 */
  function codexPlusUsageSyncPlacement() {
    const card = codexPlusUsageEnabled() ? codexPlusUsageComposerCard() : null;
    const threadId = codexPlusUsageThreadId(card);
    if (!card || !threadId) {
      codexPlusUsageHide();
      codexPlusUsageState.threadId = "";
      return null;
    }
    const changed = codexPlusUsageState.threadId !== threadId
      || !codexPlusUsageState.node?.isConnected
      || codexPlusUsageState.node.previousElementSibling !== card;
    const node = codexPlusUsageEnsureNode(card);
    if (!node) return null;
    if (changed) {
      codexPlusUsageState.threadId = threadId;
      codexPlusUsageState.failures = 0;
      codexPlusUsageState.lastFingerprint = "";
    }
    codexPlusUsageShow(node);
    codexPlusUsageRender(codexPlusUsageSnapshot(codexPlusUsageState.cache.get(threadId)));
    return { card, threadId, changed };
  }

  function codexPlusUsageRemember(threadId, events) {
    if (!events?.length) return;
    codexPlusUsageState.cache.delete(threadId);
    codexPlusUsageState.cache.set(threadId, events.slice(-24));
    if (codexPlusUsageState.cache.size > 32) {
      codexPlusUsageState.cache.delete(codexPlusUsageState.cache.keys().next().value);
    }
  }

  /** 只观察输入框结构和会话标识，忽略状态栏自身的写入，避免自喂循环。 */
  function codexPlusUsageObserve() {
    if (!document.body || typeof MutationObserver !== "function") return;
    const selector = "[data-codex-composer-root], [data-composer-surface-variant], [data-above-composer-conversation-id]";
    codexPlusUsageState.observer = new MutationObserver((records) => {
      const relevant = records.some((record) => {
        if (record.target?.closest?.("[data-codex-plus-ext]")) return false;
        if (record.type === "attributes") return true;
        return [...record.addedNodes, ...record.removedNodes].some((node) =>
          node.nodeType === 1 && !node.matches?.("[data-codex-plus-ext]")
          && (node.matches?.(selector) || node.querySelector?.(selector)));
      });
      if (!relevant) return;
      const placement = codexPlusUsageSyncPlacement();
      if (placement?.changed) codexPlusUsageSchedule(0);
    });
    codexPlusUsageState.observer.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: [
        "data-above-composer-conversation-id", "data-composer-placement",
        "data-app-action-sidebar-thread-selected", "data-app-action-sidebar-thread-id",
      ],
    });
  }

  async function codexPlusUsageRefresh(threadId) {
    if (codexPlusUsageState.inFlight) return null;
    codexPlusUsageState.inFlight = true;
    try {
      const result = await postJson("/thread-usage-live", { session_id: threadId });
      codexPlusUsageState.failures = 0;
      if (result?.status !== "ok" || !Array.isArray(result.events)) return null;
      return result.events;
    } finally {
      codexPlusUsageState.inFlight = false;
    }
  }

  async function codexPlusUsageTick() {
    if (codexPlusUsageState.disposed) return;
    if (!codexPlusUsageEnabled()) {
      codexPlusUsageHide();
      codexPlusUsageSchedule(codexPlusUsageIdleIntervalMs);
      return;
    }
    // 页面不可见时不拉数据，但保留轮询，切回来下一拍就会刷新。
    if (document.hidden) {
      codexPlusUsageSchedule(codexPlusUsageIdleIntervalMs);
      return;
    }
    const placement = codexPlusUsageSyncPlacement();
    if (!placement) {
      codexPlusUsageSchedule(codexPlusUsageIdleIntervalMs);
      return;
    }
    const { threadId } = placement;
    if (codexPlusUsageState.inFlight) {
      codexPlusUsageSchedule(codexPlusUsageActiveIntervalMs);
      return;
    }
    let events = null;
    try {
      events = await codexPlusUsageRefresh(threadId);
    } catch (error) {
      if (codexPlusUsageState.threadId === threadId) codexPlusUsageState.failures += 1;
      if (codexPlusUsageState.failures === codexPlusUsageFailureLimit) {
        sendCodexPlusDiagnostic("usage_statusline_bridge_failed", {
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
      }
    }
    if (codexPlusUsageState.disposed) return;
    codexPlusUsageRemember(threadId, events);
    const current = codexPlusUsageSyncPlacement();
    if (!current || current.threadId !== threadId) {
      codexPlusUsageSchedule(current ? 0 : codexPlusUsageIdleIntervalMs);
      return;
    }
    const snapshot = codexPlusUsageSnapshot(codexPlusUsageState.cache.get(threadId));
    if (snapshot) {
      codexPlusUsageRender(snapshot);
      // 刚有事件说明本轮在跑，加密轮询；本轮已经结束就拉长间隔。
      codexPlusUsageSchedule(snapshot.stale ? codexPlusUsageIdleIntervalMs : codexPlusUsageActiveIntervalMs);
      return;
    }
    if (codexPlusUsageState.failures >= codexPlusUsageFailureLimit) {
      // 桥接连续失败仍保留栏位与最近读数，只降低取数频率。
      codexPlusUsageSchedule(30000);
      return;
    }
    // 会话已经确定、只是还没写出 token 事件（新会话第一轮）：先显示占位符，
    // 用较慢的间隔等第一条数据，避免空转。
    codexPlusUsageRender(null);
    codexPlusUsageSchedule(codexPlusUsageWarmupIntervalMs);
  }

  codexPlusUsageInstallStyle();
  codexPlusUsageObserve();
  codexPlusUsageSyncPlacement();
  codexPlusUsageSchedule(600);
  window.__codexPlusUsageStatusLineRuntime = {
    version: 2,
    sync: () => codexPlusUsageSchedule(0),
    dispose() {
      codexPlusUsageState.disposed = true;
      clearTimeout(codexPlusUsageState.timer);
      codexPlusUsageState.timer = 0;
      codexPlusUsageState.observer?.disconnect();
      codexPlusUsageState.cache.clear();
      // 卸载时把节点整个摘掉：注入会被重跑，留一个隐藏节点等于每次重注入多挂一个。
      codexPlusUsageState.node?.remove();
      codexPlusUsageState.node = null;
      codexPlusUsageState.parts = null;
      codexPlusUsageState.lastFingerprint = "";
      globalThis.__CODEX_PLUS_USAGE_STATUSLINE_SYNC__ = undefined;
    },
  };
  globalThis.__CODEX_PLUS_USAGE_STATUSLINE_SYNC__ = () => codexPlusUsageSchedule(0);
