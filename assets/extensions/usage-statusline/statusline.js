  /**
   * 输入框下方用量状态栏：解码速度 / 首 token 延迟 / 缓存命中率 / 上下文占用，
   * 以及已完成消息的末次模型调用 token 用量。
   *
   * 缓存和上下文来自 Codex 自己写的 rollout 记录（`event_msg.token_count`），解码速度
   * 来自 Codex++ 的请求日志。首 token 延迟也来自同一份请求日志，按输出 token 加权。
   * 缓存命中 = cached_input_tokens / input_tokens，上下文占用 =
   * last_token_usage.total_tokens / model_context_window。这里的 model_context_window
   * 正是 Codex++ 注入 model_catalog_json 后 Codex 认到的每模型窗口，所以按模型配置
   * 窗口的功能会直接反映在这一项上。
   *
   * 状态栏的实时数字走 `/thread-usage-live`，宿主只读文件尾部。消息 token 用量需要
   * 完整 turn 记录，只在进入会话和收到 `turn/completed` 时读取一次 `/thread-usage-history`，
   * 不参与秒级轮询。
   *
   * 节点放进公开插槽 composer.below 给的容器（输入框下方、卡片外）；没有插槽容器时紧跟在
   * 输入卡片（[data-composer-surface-variant]）之后。旧版 Codex 没有这个属性，退回旧类名或输入框所在的 form，
   * 会话 id 退回 Codex++ 认的当前会话，挂不上时写诊断日志（issue #2753）。
   * 节点带 data-codex-plus-ext，扫描调度器会把它排除在
   * 自喂循环之外（issue #1960）。
   *
   * 这是官方拓展包「用量状态栏」的引擎：只用公开接口，后端调用、诊断、当前会话都由 main.js 经
   * codexPlusUsageHost 传进来（ctx.call / codexPlus.currentSession）。
   */
  const codexPlusUsageOwner = "usage-statusline";
  /** main.js 在拓展装上时填好：调清单里声明的路由、写诊断日志、取 Codex++ 认的当前会话 id。 */
  const codexPlusUsageHost = {
    call: () => Promise.reject(new Error("用量状态栏还没装上")),
    diagnose: () => {},
    legacySessionId: () => "",
  };

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
  /** 解码速度取最近多少次已完成请求的加权平均，避免单次短回答放大速度。 */
  const codexPlusUsageDecodeSampleLimit = 10;
  /** 首 token 延迟与解码速度共用最近成功请求窗口。 */
  const codexPlusUsageFirstTokenSampleLimit = codexPlusUsageDecodeSampleLimit;
  /** 完整 turn 用量只在进入会话和完成后刷新；短时间内虚拟滚动重挂载不必整读 rollout。 */
  const codexPlusUsageTurnUsageCacheMs = 300000;
  /** 连续失败到这个次数后停掉轮询，避免桥接不可用时每秒写诊断日志。 */
  const codexPlusUsageFailureLimit = 5;

  const codexPlusUsageState = {
    /** 拓展装着时为 true：观察与轮询只在这期间跑。 */
    running: false,
    timer: 0,
    node: null,
    parts: null,
    threadId: "",
    failures: 0,
    inFlight: false,
    lastFingerprint: "",
    cache: new Map(),
    turnUsage: new Map(),
    turnUsageInFlight: new Map(),
    turnUsageLiveInFlight: new Map(),
    observer: null,
    /** 页面里出现过新版 composer 标记后就不再走旧版兜底。 */
    modernDom: false,
    /** 已经写过的诊断原因，每个页面生命周期一条只记一次。 */
    diagnosed: new Set(),
    /** composer.below 插槽给的容器（每个输入框一个），由 main.js 增删。 */
    slotHosts: new Set(),
  };

  function codexPlusUsageVisible(node) {
    if (!node || typeof node.getBoundingClientRect !== "function") return false;
    const rect = node.getBoundingClientRect?.();
    return !!rect && rect.width > 0 && rect.height > 0;
  }

  const codexPlusUsageSvgNs = "http://www.w3.org/2000/svg";

  /**
   * 速度项只显示数字，名称和口径放在悬停提示里。主值是首 token 之后的纯解码速度；
   * rollout 还能算出含首 token 等待和工具执行的输出吞吐，只放在提示里作对比。
   */
  function codexPlusUsageText() {
    const language = (document.documentElement?.lang || navigator.language || "").toLowerCase();
    return language.startsWith("zh")
      ? {
        speed: (value) => `${value} tok/s`,
        ttft: (value) => `首 token ${value}`,
        cache: (value) => `缓存命中 ${value}`,
        context: (value) => value,
        turnUsage: (value) => `${value} tokens`,
        aria: "用量状态栏",
        separator: "：",
        comma: "，",
        speedTitle: (value) => `解码速度 ${value} tok/s（最近最多 10 次已完成请求的加权平均，按输出 token ÷ 首 token 后的生成耗时，不含首 token 等待和工具执行）`,
        ttftTitle: (value) => `首 token 延迟 ${value}s（最近最多 10 次成功请求的加权平均，按输出 token 加权，包含排队与提示词处理）`,
        throughputTitle: (value) => `输出吞吐 ${value} tok/s（包含首 token 等待和工具执行的整请求口径，仅作对比）`,
        cacheTitle: (value) => `缓存命中 ${value}`,
        contextTitle: (value) => `上下文占用 ${value}`,
        windowTitle: (value) => `窗口 ${value}`,
        turnUsageTitle: (usage) => [
          `本条消息 ${codexPlusUsageFormatNumber(usage.total)} tokens`,
          `输入 ${codexPlusUsageFormatNumber(usage.input)}`,
          `缓存输入 ${codexPlusUsageFormatNumber(usage.cached)}`,
          `输出 ${codexPlusUsageFormatNumber(usage.output)}`,
          usage.reasoning > 0 ? `推理 ${codexPlusUsageFormatNumber(usage.reasoning)}` : "",
          usage.contextLimit > 0 ? `上下文 ${codexPlusUsageFormatNumber(usage.total)} / ${codexPlusUsageFormatNumber(usage.contextLimit)}` : "",
        ].filter(Boolean).join(" · "),
      }
      : {
        speed: (value) => `${value} tok/s`,
        ttft: (value) => `first token ${value}`,
        cache: (value) => `cache hit ${value}`,
        context: (value) => value,
        turnUsage: (value) => `${value} tokens`,
        aria: "Usage status line",
        separator: ": ",
        comma: ", ",
        speedTitle: (value) => `Decode speed ${value} tok/s (token-weighted average of up to 10 completed requests, using output tokens ÷ generation time after the first token; excludes first-token wait and tool runs)`,
        ttftTitle: (value) => `First-token latency ${value}s (token-weighted average of up to 10 successful requests, including queueing and prompt processing)`,
        throughputTitle: (value) => `Output throughput ${value} tok/s (whole-request timing including first-token wait and tool runs; shown only for comparison)`,
        cacheTitle: (value) => `Cache hit ${value}`,
        contextTitle: (value) => `Context used ${value}`,
        windowTitle: (value) => `window ${value}`,
        turnUsageTitle: (usage) => [
          `This message used ${codexPlusUsageFormatNumber(usage.total)} tokens`,
          `input ${codexPlusUsageFormatNumber(usage.input)}`,
          `cached input ${codexPlusUsageFormatNumber(usage.cached)}`,
          `output ${codexPlusUsageFormatNumber(usage.output)}`,
          usage.reasoning > 0 ? `reasoning ${codexPlusUsageFormatNumber(usage.reasoning)}` : "",
          usage.contextLimit > 0 ? `context ${codexPlusUsageFormatNumber(usage.total)} / ${codexPlusUsageFormatNumber(usage.contextLimit)}` : "",
        ].filter(Boolean).join(" · "),
      };
  }

  function codexPlusUsageFormatNumber(value) {
    const number = Number(value);
    return Number.isFinite(number) && number >= 0
      ? Math.round(number).toLocaleString(document.documentElement?.lang || navigator.language || undefined)
      : "--";
  }

  function codexPlusUsageInstallStyle() {
    if (document.getElementById(codexPlusUsageStyleId)) return;
    const style = document.createElement("style");
    style.id = codexPlusUsageStyleId;
    style.dataset.codexPlusExt = codexPlusUsageOwner;
    style.textContent = `
      /* 颜色只用一层透明度：主题的 --color-token-text-secondary 本身就是 65% 透明，
         再叠容器 opacity 会掉到 0.3 出头，实测在 #141414 上几乎看不清。
         这里以主文字色取 82%，既够亮又略低于输入框文字。 */
      /* 三项作为一组居中，和 DSH 那条一致；两端对齐会把「上下文占用」甩到最右边，
         和速度、缓存拉得太开，读起来是两段信息。 */
      #${codexPlusUsageStatusLineId}{display:flex;align-items:center;justify-content:center;flex-wrap:wrap;gap:5px 18px;box-sizing:border-box;width:100%;margin:6px 0 0;padding:0 10px;font:inherit;font-size:12px;line-height:17px;color:var(--color-token-text-secondary,rgba(205,205,215,.95));color:color-mix(in srgb,var(--color-token-text-primary,#dfdfdf) 82%,transparent);user-select:none;cursor:default}
      #${codexPlusUsageStatusLineId}[data-hidden="true"]{display:none}
      #${codexPlusUsageStatusLineId} .codex-plus-usage-item{display:inline-flex;align-items:center;gap:5px;white-space:nowrap;font-variant-numeric:tabular-nums;transition:opacity .2s}
      #${codexPlusUsageStatusLineId} .codex-plus-usage-value{font-weight:500}
      #${codexPlusUsageStatusLineId} .codex-plus-usage-item svg{flex:0 0 auto;opacity:.8}
      .turn-action-controls>[data-codex-plus-turn-usage]{display:inline-flex;align-items:center;min-height:20px;margin-left:6px;color:var(--color-token-text-tertiary,rgba(180,180,190,.86));font-size:12px;line-height:17px;font-variant-numeric:tabular-nums;white-space:nowrap;user-select:none}
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
      ttft: codexPlusUsageItem(node, "ttft", "ttft", codexPlusUsageIcon([
        { tag: "circle", attrs: { cx: "8", cy: "8", r: "5.8" } },
        { tag: "path", attrs: { d: "M8 4.5V8l2.4 1.6" } },
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

  /** 当前版本 Codex（26.825 起）的输入卡片。 */
  const codexPlusUsageCardSelector = "[data-composer-surface-variant]";
  /**
   * 旧版 Codex 的输入卡片（issue #2753）：26.825 之前没有 data-composer-surface-variant，
   * 卡片是 `.composer-surface-chrome`，或带 CSS Modules 类名 `_ComposerLayoutRoot_` 的容器，
   * 和 Dream Skin 选择器契约里 composer-chrome 的几代写法一致。Intel Mac 拿不到新版
   * Codex（新版只发 arm64），只能停在旧版，所以这条兜底在 macOS 上最常用到。
   */
  const codexPlusUsageLegacyCardSelector = '.composer-surface-chrome, [class*="_ComposerLayoutRoot_"]';
  const codexPlusUsageEditorSelector = '.ProseMirror[contenteditable="true"], textarea:not(:disabled):not([readonly]), [role="textbox"][contenteditable="true"]';
  /** 同一页面生命周期内诊断事件的上限，避免 DOM 反复变化时刷屏。 */
  const codexPlusUsageDiagnosticLimit = 8;

  /**
   * 找输入卡片，返回 `{ card, source }`；source 记录走的是哪条路径，写进诊断日志。
   *
   * 旧版兜底只在页面从没出现过新版输入卡片标记时启用：新版 Codex 停在设置页等没有 composer 的
   * 页面时，不能把别处的 form 当成输入框。
   *
   * 「新版」只认 data-composer-surface-variant：部分旧版 Codex 已经有 data-codex-composer-root，
   * 却还没有新版卡片标记（#2753 的 Intel Mac 日志：composerRoot=1、surfaceVariant=0），
   * 把根标记也算作新版会让旧版永远走不到兜底。
   */
  function codexPlusUsageComposerCard() {
    let fallback = null;
    for (const card of document.querySelectorAll(codexPlusUsageCardSelector)) {
      if (!codexPlusUsageVisible(card) || card.closest('[data-codex-plus-ext]')) continue;
      fallback = card;
      // 首页可能同时挂着多个 composer 外壳；带真实输入框的那个才是当前会话。
      if (card.querySelector(codexPlusUsageEditorSelector)) return { card, source: "surface-variant" };
    }
    if (fallback) return { card: fallback, source: "surface-variant" };
    if (!codexPlusUsageState.modernDom && document.querySelector(codexPlusUsageCardSelector)) {
      codexPlusUsageState.modernDom = true;
    }
    if (codexPlusUsageState.modernDom) return null;
    for (const card of document.querySelectorAll(codexPlusUsageLegacyCardSelector)) {
      if (!codexPlusUsageVisible(card) || card.closest('[data-codex-plus-ext]')) continue;
      // 旧版类名也可能挂在装饰容器上，必须真的包着输入框才算。
      if (card.querySelector(codexPlusUsageEditorSelector)) return { card, source: "legacy-chrome" };
    }
    // 有 composer 根标记、没有旧类名的过渡版本：根节点包着输入框时就挂在根上。
    for (const card of document.querySelectorAll("[data-codex-composer-root]")) {
      if (!codexPlusUsageVisible(card) || card.closest('[data-codex-plus-ext]')) continue;
      if (card.querySelector(codexPlusUsageEditorSelector)) return { card, source: "composer-root" };
    }
    // 类名也对不上时按输入框本身找：取 ProseMirror 所在的 form。
    for (const editor of document.querySelectorAll('.ProseMirror[contenteditable="true"]')) {
      if (!codexPlusUsageVisible(editor) || editor.closest('[data-codex-plus-ext], [role="dialog"]')) continue;
      const form = editor.closest("form");
      if (form && codexPlusUsageVisible(form)) return { card: form, source: "editor-form" };
    }
    return null;
  }

  function codexPlusUsageValidThreadId(value) {
    const id = String(value || "").replace(/^local:/, "").trim();
    if (!id || id === "__proto__" || id === "prototype" || id === "constructor") return "";
    return /^[A-Za-z0-9_.-]{8,128}$/.test(id) ? id : "";
  }

  /**
   * 新版 Codex 的侧边栏当前行可能晚一拍更新，Codex++ 的当前会话也可能短暂保留旧值。
   * 可见 assistant 自己的 conversation id 是当前对话最直接的标识。
   */
  function codexPlusUsageVisibleConversationId() {
    const nodes = [...(document.querySelectorAll?.("[data-response-annotation-conversation]") || [])]
      .filter((node) => codexPlusUsageVisible(node))
      .reverse();
    for (const node of nodes) {
      const id = codexPlusUsageValidThreadId(node.getAttribute("data-response-annotation-conversation"));
      if (id) return id;
    }
    return "";
  }

  /**
   * 旧版 Codex 的当前会话：Codex++ 认的当前会话（侧边栏当前行 + 地址栏，删除 / 导出也靠它），
   * 由 main.js 经 codexPlus.currentSession() 提供。
   */
  function codexPlusUsageLegacyThreadId() {
    try {
      return String(codexPlusUsageHost.legacySessionId() || "");
    } catch (_) {
      return "";
    }
  }

  /**
   * 当前会话，返回 `{ id, source, reason, placement }`；拿不到 id 时 reason 说明原因。
   *
   * `[data-above-composer-conversation-id]` 是 Codex 自己挂在 composer 上的会话标识，
   * 比侧边栏选中行更贴近「这个输入框属于哪个会话」。侧边栏只作兜底，而且**只在
   * placement=thread 时用**：首页/新会话草稿的 composer 是 placement=home，此时侧边栏
   * 仍选中着上一个 thread，兜底会把上一个会话的读数挂到新会话上。
   * 新会话草稿的 id 形如 `local:client-new-thread:<uuid>`，还没有 rollout，直接当作无会话。
   *
   * 旧版 Codex 的 composer 上既没有 data-codex-composer-root / data-composer-placement，
   * 也没有会话标识（issue #2753），这时 placement 为空，退回侧边栏当前行 + 地址栏。
   */
  function codexPlusUsageResolveThread(card) {
    const none = (reason, placement = "") => ({ id: "", source: "", reason, placement });
    if (!card) return none("no-card");
    const composerRoot = card.closest?.("[data-codex-composer-root]") || null;
    const placement = String(composerRoot?.getAttribute("data-composer-placement") || "");
    const fromComposer = composerRoot
      ?.querySelector("[data-above-composer-conversation-id]")
      ?.getAttribute("data-above-composer-conversation-id");
    if (fromComposer) {
      const id = codexPlusUsageValidThreadId(fromComposer);
      if (id) return { id, source: "composer", reason: "", placement };
      return none(/client-new-thread:/i.test(fromComposer) ? "draft" : "invalid-id", placement);
    }
    if (!placement || placement === "thread") {
      const fromVisibleAssistant = codexPlusUsageVisibleConversationId();
      if (fromVisibleAssistant) {
        return { id: fromVisibleAssistant, source: "visible-assistant", reason: "", placement };
      }
    }
    if (placement === "home") return none("home", placement);
    if (placement === "thread") {
      const fromSidebar = codexPlusUsageValidThreadId(document
        .querySelector('[data-app-action-sidebar-thread-selected="true"][data-app-action-sidebar-thread-id]')
        ?.getAttribute("data-app-action-sidebar-thread-id"));
      if (fromSidebar) return { id: fromSidebar, source: "sidebar", reason: "", placement };
    } else if (placement) {
      // 其它显式 placement（侧边对话等）不认侧边栏，和原来的行为一致。
      return none("other-placement", placement.slice(0, 32));
    }
    const legacy = codexPlusUsageValidThreadId(codexPlusUsageLegacyThreadId());
    if (legacy) return { id: legacy, source: "legacy", reason: "", placement };
    return none("no-thread-id", placement);
  }

  function codexPlusUsageThreadId(card) {
    return codexPlusUsageResolveThread(card).id;
  }

  /** 同一原因每个页面生命周期只记一次。 */
  function codexPlusUsageDiagnoseOnce(event, key, detail) {
    const seen = codexPlusUsageState.diagnosed;
    const fullKey = `${event}:${key}`;
    if (seen.has(fullKey) || seen.size >= codexPlusUsageDiagnosticLimit) return;
    seen.add(fullKey);
    try {
      Promise.resolve(codexPlusUsageHost.diagnose(event, detail)).catch(() => {});
    } catch (_) {
      // 诊断失败不能影响状态栏本身。
    }
  }

  /** 挂载失败时的 DOM 快照：只记数量和布尔值，不记会话 id 与内容。 */
  function codexPlusUsageDomProbe() {
    const count = (selector) => {
      try {
        return document.querySelectorAll(selector).length;
      } catch (_) {
        return -1;
      }
    };
    return {
      surfaceVariant: count(codexPlusUsageCardSelector),
      composerRoot: count("[data-codex-composer-root]"),
      conversationMarker: count("[data-above-composer-conversation-id]"),
      legacyChrome: count(".composer-surface-chrome"),
      layoutRoot: count('[class*="_ComposerLayoutRoot_"]'),
      editors: count('.ProseMirror[contenteditable="true"]'),
      sidebarRows: count("[data-app-action-sidebar-thread-id]"),
      sidebarSelected: count('[data-app-action-sidebar-thread-selected="true"]'),
      currentSessionKnown: !!codexPlusUsageLegacyThreadId(),
    };
  }

  function codexPlusUsageVisibleEditor() {
    return [...document.querySelectorAll('.ProseMirror[contenteditable="true"]')]
      .some((editor) => codexPlusUsageVisible(editor) && !editor.closest("[data-codex-plus-ext]"));
  }

  /**
   * 把挂载结果写进诊断日志（issue #2753：同样设置 Windows 有、Mac 没有，用户侧无从排查）。
   *   - 页面上有输入框却找不到卡片、或找到卡片却认不出会话 → usage_statusline_mount_missing；
   *   - 走了旧版兜底才挂上 → usage_statusline_fallback_mount，方便确认用户是旧版 Codex。
   * 首页、新会话草稿、没有输入框的页面本来就不显示，不记。
   */
  function codexPlusUsageReportPlacement(found, thread) {
    let event = "";
    let detail = null;
    if (!found) {
      if (!codexPlusUsageVisibleEditor()) return;
      event = "usage_statusline_mount_missing";
      detail = { reason: "no-composer-card" };
    } else if (!thread?.id) {
      const reason = thread?.reason || "no-thread-id";
      if (reason === "home" || reason === "draft") return;
      event = "usage_statusline_mount_missing";
      detail = { reason, card: found.source, placement: thread?.placement || "" };
    } else if (found.source !== "surface-variant" || thread.source === "legacy") {
      event = "usage_statusline_fallback_mount";
      detail = { card: found.source, thread: thread.source };
    } else {
      return;
    }
    codexPlusUsageDiagnoseOnce(event, Object.values(detail).join("|"), { ...detail, dom: codexPlusUsageDomProbe() });
  }

  /** 旧版 Codex 的 rollout 可能缺窗口或轮 id：照样显示（对应项为 --），只记一次缺了什么。 */
  function codexPlusUsageNoteDataGaps(events) {
    if (!Array.isArray(events) || !events.length) return;
    const latest = events[events.length - 1] || {};
    const missing = [];
    if (!(Number(latest.context_window) > 0)) missing.push("context_window");
    if (!events.some((item) => typeof item?.turn_id === "string" && item.turn_id)) missing.push("turn_id");
    if (!events.some((item) => Number.isFinite(Date.parse(item?.t)))) missing.push("t");
    if (!missing.length) return;
    codexPlusUsageDiagnoseOnce("usage_statusline_data_partial", missing.join(","), { missing, events: events.length });
  }

  /**
   * 状态栏放在哪：main.js 经公开的 composer.below 插槽登记了容器时放进卡片所在输入框的那个容器，
   * 否则（插槽还没画出来、旧版页面）紧跟在输入卡片后面。
   */
  function codexPlusUsageSlotHost(card) {
    const root = card?.closest?.("[data-codex-composer-root]");
    if (!root) return null;
    for (const container of codexPlusUsageState.slotHosts || []) {
      if (container.isConnected && root.contains(container)) return container;
    }
    return null;
  }

  function codexPlusUsageNodePlaced(node, card) {
    if (!node?.isConnected) return false;
    const host = codexPlusUsageSlotHost(card);
    return host ? node.parentElement === host : node.previousElementSibling === card;
  }

  function codexPlusUsageEnsureNode(card) {
    const host = card?.parentElement;
    if (!host) return null;
    let node = codexPlusUsageState.node;
    if (codexPlusUsageNodePlaced(node, card)) return node;
    if (!node) {
      node = document.createElement("div");
      node.id = codexPlusUsageStatusLineId;
      node.setAttribute("data-codex-plus-ext", codexPlusUsageOwner);
      node.setAttribute("role", "status");
      codexPlusUsageState.node = node;
    }
    const slotHost = codexPlusUsageSlotHost(card);
    if (slotHost) slotHost.appendChild(node);
    else card.insertAdjacentElement("afterend", node);
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

  /**
   * 解码速度：请求日志里 `duration_ms - first_token_ms` 是首 token 后的生成时间。
   * 对最近最多 10 次成功请求做 token 加权，避免很短的请求把平均值放大。缺少首 token
   * 时间、输出为空或线程不匹配的记录不进入样本；没有合格样本时返回 null。
   */
  function codexPlusUsageDecodeRate(requests, threadId) {
    if (!Array.isArray(requests)) return null;
    let tokens = 0;
    let seconds = 0;
    let samples = 0;
    for (const request of requests) {
      if (samples >= codexPlusUsageDecodeSampleLimit) break;
      const requestThreadId = String(request?.threadId || "");
      if (threadId && requestThreadId !== threadId) continue;
      if (request?.success !== true) continue;
      const outputTokens = Number(request?.usage?.outputTokens) || 0;
      const durationMs = Number(request?.durationMs) || 0;
      const firstTokenMs = request?.firstTokenMs == null
        ? Number.NaN
        : Number(request.firstTokenMs);
      if (outputTokens <= 0 || !Number.isFinite(firstTokenMs) || firstTokenMs < 0
        || durationMs <= firstTokenMs) continue;
      tokens += outputTokens;
      seconds += (durationMs - firstTokenMs) / 1000;
      samples += 1;
    }
    return seconds > 0 ? tokens / seconds : null;
  }

  /**
   * 首 token 延迟：与解码速度共用最近 10 次成功请求，按输出 token 加权。
   * 短请求不应和长回答等权，否则一个很小的请求会把真实等待感冲淡。
   */
  function codexPlusUsageFirstTokenMs(requests, threadId) {
    if (!Array.isArray(requests)) return null;
    let weightedMs = 0;
    let tokens = 0;
    let samples = 0;
    for (const request of requests) {
      if (samples >= codexPlusUsageFirstTokenSampleLimit) break;
      const requestThreadId = String(request?.threadId || "");
      if (threadId && requestThreadId !== threadId) continue;
      if (request?.success !== true) continue;
      const outputTokens = Number(request?.usage?.outputTokens) || 0;
      const firstTokenMs = request?.firstTokenMs == null
        ? Number.NaN
        : Number(request.firstTokenMs);
      if (outputTokens <= 0 || !Number.isFinite(firstTokenMs) || firstTokenMs < 0) continue;
      weightedMs += firstTokenMs * outputTokens;
      tokens += outputTokens;
      samples += 1;
    }
    return tokens > 0 ? weightedMs / tokens : null;
  }

  /**
   * `/thread-usage-history` 会保留同一轮里的每次模型调用。消息角标只使用该 turn 的
   * 最后一条 usage，不能把工具步骤累加成整轮总量。turn_id 缺失的旧记录直接忽略。
   */
  function codexPlusUsageFinalTurnUsage(history) {
    const result = new Map();
    if (!Array.isArray(history)) return result;
    history.forEach((entry, index) => {
      const turnId = String(entry?.turn_id || "").trim();
      if (!turnId) return;
      const usage = codexPlusUsageMessageUsage(entry?.usage);
      if (!usage || usage.total <= 0) return;
      const observedAt = Date.parse(entry?.observed_at);
      const previous = result.get(turnId);
      if (previous && Number.isFinite(previous.observedAt) && Number.isFinite(observedAt)
        && observedAt < previous.observedAt) return;
      if (previous && !Number.isFinite(observedAt) && previous.index > index) return;
      result.set(turnId, { usage, observedAt, index });
    });
    return result;
  }

  /** 从既有的轻量尾部事件提取每轮最后一次模型调用，供新消息角标增量更新。 */
  function codexPlusUsageLiveTurnUsage(events) {
    const result = new Map();
    if (!Array.isArray(events)) return result;
    events.forEach((event, index) => {
      const turnId = String(event?.turn_id || "").trim();
      if (!turnId) return;
      const usage = codexPlusUsageMessageUsage({
        inputTokens: event?.input_tokens,
        cachedTokens: event?.cached_input_tokens,
        outputTokens: event?.output_tokens,
        reasoningTokens: event?.reasoning_output_tokens,
        totalTokens: event?.total_tokens,
        contextLimit: event?.context_window,
      });
      if (!usage || usage.total <= 0) return;
      result.set(turnId, { usage, observedAt: Date.parse(event?.t), index });
    });
    return result;
  }

  function codexPlusUsageMessageUsage(value) {
    if (!value || typeof value !== "object") return null;
    const input = Math.max(0, Number(value.inputTokens) || 0);
    const cached = Math.max(0, Number(value.cachedTokens) || 0);
    const output = Math.max(0, Number(value.outputTokens) || 0);
    const reasoning = Math.max(0, Number(value.reasoningTokens) || 0);
    const reportedTotal = Math.max(0, Number(value.totalTokens) || 0);
    const total = reportedTotal || input + cached + output;
    if (total <= 0 && input <= 0 && cached <= 0 && output <= 0) return null;
    return {
      total,
      input,
      cached,
      output,
      reasoning,
      contextLimit: Math.max(0, Number(value.contextLimit) || 0),
    };
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

  /** 单个区间超过这个时长就当作长停顿（等审批、挂起、断线重连），不计入吞吐。 */
  const codexPlusUsageMaxIntervalSeconds = 120;

  /**
   * 输出吞吐（token/s），issue #2636。
   *
   * Codex 只在每次模型调用结束时写一条 token_count，拿不到首个 token 的时间，事件
   * 间隔里同时包含首 token 等待、解码和工具执行，所以这里只能给「吞吐」，不能叫
   * 生成速度 / 解码速度，界面上也按这个口径命名。
   *
   * 口径：本轮各区间的输出 token 求和 ÷ 对应时长求和。不能对各区间速率等权平均：
   * 工具步骤输出少、耗时长，最终回答输出多、耗时短，等权平均会让几十 token 的工具
   * 步骤和上千 token 的回答同样有分量（issue 的算例：10.46 / 14.11 / 109.83 等权得
   * 44.8，按总量是 1781 / 54.293 ≈ 32.8）。
   *
   * 不合格的区间整段剔除（分子分母一起剔）：累计输出没增长（重复记录 / 非单调）、
   * 间隔过短（同一批写入）、间隔过长（中间停了很久）。没有合格区间时返回 null，
   * 界面显示 `--`。turn_output_tokens 是历史字段名，实际为会话累计输出，需用真实
   * 轮 id 切段。
   */
  function codexPlusUsageRate(events) {
    const segment = codexPlusUsageLatestTurn(events);
    let tokens = 0;
    let seconds = 0;
    for (let index = 1; index < segment.length; index += 1) {
      const previous = segment[index - 1];
      const current = segment[index];
      const previousOutput = Number(previous.turn_output_tokens) || 0;
      const currentOutput = Number(current.turn_output_tokens) || 0;
      if (currentOutput <= previousOutput) continue;
      const startedAt = Date.parse(previous.t);
      const endedAt = Date.parse(current.t);
      const interval = (endedAt - startedAt) / 1000;
      if (!(interval >= 0.5) || interval > codexPlusUsageMaxIntervalSeconds) continue;
      tokens += currentOutput - previousOutput;
      seconds += interval;
    }
    return seconds > 0 ? tokens / seconds : null;
  }

  function codexPlusUsageSnapshot(source) {
    const bundle = Array.isArray(source) ? { events: source } : source && typeof source === "object" ? source : {};
    const events = Array.isArray(bundle.events) ? bundle.events : [];
    if (!Array.isArray(events) || !events.length) return null;
    const latest = events[events.length - 1];
    const observedAt = Date.parse(latest.t);
    const outputThroughput = codexPlusUsageRate(events);
    return {
      speed: outputThroughput,
      decodeSpeed: codexPlusUsageDecodeRate(bundle.requests, String(bundle.threadId || latest.thread_id || "")),
      firstTokenMs: codexPlusUsageFirstTokenMs(bundle.requests, String(bundle.threadId || latest.thread_id || "")),
      outputThroughput,
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
    const speed = snapshot?.decodeSpeed == null ? "--" : String(Math.max(0, Math.round(snapshot.decodeSpeed)));
    const firstToken = snapshot?.firstTokenMs == null
      ? "--"
      : `${(Math.max(0, snapshot.firstTokenMs) / 1000).toFixed(1)}s`;
    const outputThroughput = snapshot?.outputThroughput == null
      ? null
      : String(Math.max(0, Math.round(snapshot.outputThroughput)));
    const cache = snapshot?.cache == null ? "--" : `${snapshot.cache.toFixed(1)}%`;
    const context = snapshot?.context == null ? "--" : `${Math.round(snapshot.context)}%`;
    const stale = snapshot?.stale === true;
    const contextWindow = snapshot?.modelContextWindow || 0;
    // 指纹一样就不写 DOM：每秒一次无意义赋值会让 Codex 的 MutationObserver 空转。
    const fingerprint = [speed, firstToken, outputThroughput, cache, context, stale, contextWindow].join("|");
    if (fingerprint === codexPlusUsageState.lastFingerprint) return;
    codexPlusUsageState.lastFingerprint = fingerprint;

    parts.speed.value.textContent = text.speed(speed);
    parts.ttft.value.textContent = text.ttft(firstToken);
    parts.cache.value.textContent = text.cache(cache);
    parts.context.value.textContent = text.context(context);
    if (parts.ring) {
      const filled = snapshot?.context == null ? 0 : (snapshot.context / 100) * codexPlusUsageRingCircumference;
      parts.ring.setAttribute("stroke-dasharray", `${filled.toFixed(2)} ${codexPlusUsageRingCircumference}`);
    }
    node.dataset.stale = String(stale);
    node.setAttribute("title", [
      text.speedTitle(format(speed)),
      text.ttftTitle(firstToken === "--" ? "--" : firstToken.replace(/s$/, "")),
      outputThroughput == null ? "" : text.throughputTitle(format(outputThroughput)),
      text.cacheTitle(format(cache)),
      text.contextTitle(format(context)),
      contextWindow > 0 ? text.windowTitle(codexPlusUsageFormatNumber(contextWindow)) : "",
    ].filter(Boolean).join(" · "));
    node.setAttribute("aria-label", `${text.aria}${text.separator}${[text.speedTitle(speed), text.ttft(firstToken), text.cache(cache), context].join(text.comma)}`);
  }

  /** 把 token 角标放到本轮最后一个已完成 assistant 动作栏，和原生时间提示同一行。 */
  function codexPlusUsageRenderTurnUsage(threadId) {
    if (!threadId) return;
    const bundle = codexPlusUsageState.turnUsage.get(threadId);
    const byTurn = bundle?.byTurn instanceof Map ? bundle.byTurn : new Map();
    const rows = [...document.querySelectorAll(".turn-action-controls")]
      .filter((row) => row.querySelector("[data-assistant-message-sent-time]"));
    const rowTurns = new Map();
    const latestByTurn = new Map();
    for (const row of rows) {
      const turnId = row.closest("[data-content-search-turn-key]")
        ?.getAttribute("data-content-search-turn-key") || "";
      rowTurns.set(row, turnId);
      if (turnId) latestByTurn.set(turnId, row);
    }
    const text = codexPlusUsageText();
    for (const row of rows) {
      const turnId = rowTurns.get(row) || "";
      const isLatestRow = turnId ? latestByTurn.get(turnId) === row : false;
      const usage = isLatestRow ? byTurn.get(turnId)?.usage : null;
      const badges = [...row.querySelectorAll("[data-codex-plus-turn-usage]")];
      if (!usage) {
        badges.forEach((badge) => badge.remove());
        continue;
      }
      let badge = badges.shift() || null;
      badges.forEach((extra) => extra.remove());
      if (!badge) {
        badge = document.createElement("span");
        badge.dataset.codexPlusExt = codexPlusUsageOwner;
        badge.dataset.codexPlusTurnUsage = turnId;
        badge.setAttribute("role", "note");
      }
      const time = row.querySelector("[data-assistant-message-sent-time]");
      if (badge.parentElement !== row || badge.nextElementSibling !== time) {
        row.insertBefore(badge, time || null);
      }
      const total = text.turnUsage(codexPlusUsageFormatNumber(usage.total));
      badge.textContent = total;
      badge.setAttribute("aria-label", total);
      badge.setAttribute("title", text.turnUsageTitle(usage));
    }
  }

  function codexPlusUsageTurnUsageCurrent(threadId) {
    const bundle = codexPlusUsageState.turnUsage.get(threadId);
    return bundle && Date.now() - Number(bundle.loadedAt || 0) < codexPlusUsageTurnUsageCacheMs
      ? bundle
      : null;
  }

  function codexPlusUsageRememberTurnUsage(threadId, bundle) {
    codexPlusUsageState.turnUsage.delete(threadId);
    codexPlusUsageState.turnUsage.set(threadId, bundle);
    if (codexPlusUsageState.turnUsage.size > 32) {
      codexPlusUsageState.turnUsage.delete(codexPlusUsageState.turnUsage.keys().next().value);
    }
  }

  /** 用尾部事件补齐或覆盖当前轮，不整读 rollout，也不延长完整历史的缓存期限。 */
  function codexPlusUsageMergeLiveTurnUsage(threadId, events) {
    const id = codexPlusUsageValidThreadId(threadId);
    if (!id) return null;
    const current = codexPlusUsageState.turnUsage.get(id);
    const byTurn = new Map(current?.byTurn instanceof Map ? current.byTurn : []);
    let changed = false;
    for (const [turnId, value] of codexPlusUsageLiveTurnUsage(events)) {
      byTurn.set(turnId, value);
      changed = true;
    }
    if (!changed) return current || null;
    const bundle = {
      byTurn,
      loadedAt: Number(current?.loadedAt || 0),
      liveUpdatedAt: Date.now(),
      failed: false,
    };
    codexPlusUsageRememberTurnUsage(id, bundle);
    if (codexPlusUsageState.threadId === id) codexPlusUsageRenderTurnUsage(id);
    return bundle;
  }

  /** 轮次完成时只补一次轻量尾部；秒级轮询本身也会增量更新同一份角标缓存。 */
  function codexPlusUsageRefreshLiveTurnUsage(threadId) {
    const id = codexPlusUsageValidThreadId(threadId);
    if (!id || !codexPlusUsageState.running) return Promise.resolve(null);
    const pending = codexPlusUsageState.turnUsageLiveInFlight.get(id);
    if (pending) return pending;
    const task = (async () => {
      try {
        const result = await codexPlusUsageHost.call("/thread-usage-live", { session_id: id });
        if (!codexPlusUsageState.running || result?.status !== "ok" || !Array.isArray(result.events)) {
          return null;
        }
        return codexPlusUsageMergeLiveTurnUsage(id, result.events);
      } catch {
        // 常规轮询会继续补数；完成事件上的尾读失败不单独刷诊断日志。
        return null;
      } finally {
        codexPlusUsageState.turnUsageLiveInFlight.delete(id);
      }
    })();
    codexPlusUsageState.turnUsageLiveInFlight.set(id, task);
    return task;
  }

  /**
   * 完整历史只在进入会话或缓存缺失时取。并发请求按 thread id 去重；
   * 新完成的轮次由 `codexPlusUsageMergeLiveTurnUsage` 从轻量尾部增量补齐。
   */
  function codexPlusUsageRefreshTurnUsage(threadId) {
    const id = codexPlusUsageValidThreadId(threadId);
    if (!id || !codexPlusUsageState.running) return Promise.resolve(null);
    const cached = codexPlusUsageState.turnUsage.get(id);
    if (cached && Date.now() - Number(cached.loadedAt || 0) < codexPlusUsageTurnUsageCacheMs) {
      if (codexPlusUsageState.threadId === id) codexPlusUsageRenderTurnUsage(id);
      return Promise.resolve(cached);
    }
    const pending = codexPlusUsageState.turnUsageInFlight.get(id);
    if (pending) return pending;
    const task = (async () => {
      try {
        const result = await codexPlusUsageHost.call("/thread-usage-history", { session_id: id });
        if (!codexPlusUsageState.running) return null;
        if (result?.status !== "ok" || !Array.isArray(result.history)) {
          const status = String(result?.status || "invalid").slice(0, 32);
          codexPlusUsageDiagnoseOnce("usage_statusline_turn_usage_unavailable", status, {
            status,
            message: String(result?.message || "").slice(0, 200),
          });
          if (!cached) {
            codexPlusUsageState.turnUsage.set(id, {
              byTurn: new Map(),
              loadedAt: Date.now(),
              failed: true,
            });
          }
          return null;
        }
        const bundle = {
          byTurn: codexPlusUsageFinalTurnUsage(result.history),
          loadedAt: Date.now(),
          failed: false,
        };
        codexPlusUsageRememberTurnUsage(id, bundle);
        if (codexPlusUsageState.threadId === id) codexPlusUsageRenderTurnUsage(id);
        return bundle;
      } catch (error) {
        if (codexPlusUsageState.running) {
          codexPlusUsageHost.diagnose("usage_statusline_turn_usage_failed", {
            errorName: error?.name || "",
            errorMessage: error?.message || String(error),
          });
          if (!cached) {
            codexPlusUsageState.turnUsage.set(id, {
              byTurn: new Map(),
              loadedAt: Date.now(),
              failed: true,
            });
          }
        }
        return null;
      } finally {
        codexPlusUsageState.turnUsageInFlight.delete(id);
      }
    })();
    codexPlusUsageState.turnUsageInFlight.set(id, task);
    return task;
  }

  function codexPlusUsageSchedule(delay) {
    if (!codexPlusUsageState.running) return;
    clearTimeout(codexPlusUsageState.timer);
    codexPlusUsageState.timer = window.setTimeout(codexPlusUsageTick, delay);
  }

  /** 开关就是拓展本身：拓展启用（装上）时显示，停用 / 卸下时撤掉。 */
  function codexPlusUsageEnabled() {
    return codexPlusUsageState.running;
  }

  /** 挂载与取数分开：切换时先恢复栏位，网络请求只更新数值。 */
  function codexPlusUsageSyncPlacement() {
    const enabled = codexPlusUsageEnabled();
    const found = enabled ? codexPlusUsageComposerCard() : null;
    const card = found?.card || null;
    const thread = codexPlusUsageResolveThread(card);
    if (enabled) codexPlusUsageReportPlacement(found, thread);
    const threadId = thread.id;
    if (!card || !threadId) {
      codexPlusUsageHide();
      codexPlusUsageState.threadId = "";
      return null;
    }
    const changed = codexPlusUsageState.threadId !== threadId
      || !codexPlusUsageState.node?.isConnected
      || !codexPlusUsageNodePlaced(codexPlusUsageState.node, card);
    const node = codexPlusUsageEnsureNode(card);
    if (!node) return null;
    if (changed) {
      codexPlusUsageState.threadId = threadId;
      codexPlusUsageState.failures = 0;
      codexPlusUsageState.lastFingerprint = "";
    }
    codexPlusUsageShow(node);
    codexPlusUsageRender(codexPlusUsageSnapshot(codexPlusUsageState.cache.get(threadId)));
    codexPlusUsageRenderTurnUsage(threadId);
    if (changed) void codexPlusUsageRefreshTurnUsage(threadId);
    return { card, threadId, changed };
  }

  /**
   * Codex 会随滚动重建消息节点。这里只同步轻量 DOM 并复用历史缓存；公开的
   * ctx.onScan 调它，不能把整份 `/thread-usage-history` 带进高频扫描。
   */
  function codexPlusUsageSyncDom() {
    if (!codexPlusUsageState.running) return null;
    const placement = codexPlusUsageSyncPlacement();
    const threadId = placement?.threadId || codexPlusUsageState.threadId;
    if (!threadId) return placement;
    codexPlusUsageRenderTurnUsage(threadId);
    if (!codexPlusUsageTurnUsageCurrent(threadId)) {
      void codexPlusUsageRefreshTurnUsage(threadId);
    }
    return placement;
  }

  /** 轮次结束后只补轻量尾部；当前会话仍以 composer / 可见 assistant 为准。 */
  function codexPlusUsageNoteTurnCompleted(threadId) {
    if (!codexPlusUsageState.running) return;
    const placement = codexPlusUsageSyncPlacement();
    const current = placement?.threadId || codexPlusUsageState.threadId;
    if (!current) return;
    const completed = codexPlusUsageValidThreadId(threadId);
    if (completed && completed !== current) return;
    void codexPlusUsageRefreshLiveTurnUsage(current);
  }

  function codexPlusUsageRemember(threadId, update) {
    const bundle = Array.isArray(update) ? { events: update } : update;
    if (!bundle?.events?.length) return;
    codexPlusUsageState.cache.delete(threadId);
    codexPlusUsageState.cache.set(threadId, { ...bundle, events: bundle.events.slice(-24) });
    if (codexPlusUsageState.cache.size > 32) {
      codexPlusUsageState.cache.delete(codexPlusUsageState.cache.keys().next().value);
    }
  }

  /** 只观察输入框结构和会话标识，忽略状态栏自身的写入，避免自喂循环。 */
  function codexPlusUsageObserve() {
    if (!document.body || typeof MutationObserver !== "function") return;
    // 旧版 Codex 没有前三个标记，输入框结构变化只能靠旧类名感知（issue #2753）。
    const selector = `[data-codex-composer-root], ${codexPlusUsageCardSelector}, [data-above-composer-conversation-id], ${codexPlusUsageLegacyCardSelector}`;
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
        // 旧版侧边栏用 active / aria-current 标当前行。
        "data-app-action-sidebar-thread-active", "aria-current",
      ],
    });
  }

  async function codexPlusUsageRefresh(threadId) {
    if (codexPlusUsageState.inFlight) return null;
    codexPlusUsageState.inFlight = true;
    try {
      // ctx.call 遇到 status: "failed" 会 reject，和桥接出错一样按失败计数。
      const [result, requestLog] = await Promise.all([
        codexPlusUsageHost.call("/thread-usage-live", { session_id: threadId }),
        codexPlusUsageHost.call("/request-log/recent", { limit: 100 }).catch(() => null),
      ]);
      codexPlusUsageState.failures = 0;
      if (result?.status !== "ok" || !Array.isArray(result.events)) {
        // 栏位照样显示占位符；这里只把宿主给的原因记一次，便于区分「没挂上」和「没数据」。
        const status = String(result?.status || "invalid").slice(0, 32);
        codexPlusUsageDiagnoseOnce("usage_statusline_data_unavailable", status, {
          status,
          message: String(result?.message || "").slice(0, 200),
        });
        return null;
      }
      const requests = requestLog?.status === "ok" && Array.isArray(requestLog.entries)
        ? requestLog.entries
        : [];
      codexPlusUsageNoteDataGaps(result.events);
      codexPlusUsageMergeLiveTurnUsage(threadId, result.events);
      return { threadId, events: result.events, requests };
    } finally {
      codexPlusUsageState.inFlight = false;
    }
  }

  async function codexPlusUsageTick() {
    if (!codexPlusUsageState.running) return;
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
    let update = null;
    try {
      update = await codexPlusUsageRefresh(threadId);
    } catch (error) {
      if (codexPlusUsageState.threadId === threadId) codexPlusUsageState.failures += 1;
      if (codexPlusUsageState.failures === codexPlusUsageFailureLimit) {
        codexPlusUsageHost.diagnose("usage_statusline_bridge_failed", {
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
      }
    }
    if (!codexPlusUsageState.running) return;
    codexPlusUsageRemember(threadId, update);
    const current = codexPlusUsageSyncPlacement();
    if (!current || current.threadId !== threadId) {
      codexPlusUsageSchedule(current ? 0 : codexPlusUsageIdleIntervalMs);
      return;
    }
    const snapshot = codexPlusUsageSnapshot(codexPlusUsageState.cache.get(threadId));
    if (snapshot) {
      codexPlusUsageRender(snapshot);
      codexPlusUsageRenderTurnUsage(threadId);
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
    codexPlusUsageRenderTurnUsage(threadId);
    codexPlusUsageSchedule(codexPlusUsageWarmupIntervalMs);
  }

  /**
   * 观察与轮询随拓展装卸（main.js 的 ctx.effect）：装上时开始，停用 / 热重载时停下并摘掉状态栏与样式，
   * 没装着时什么也不跑。热重载会重新执行本文件，旧实例的状态随旧作用域丢掉。
   */
  function startCodexPlusUsageStatusLine() {
    if (codexPlusUsageState.running) return;
    codexPlusUsageState.running = true;
    codexPlusUsageInstallStyle();
    if (!codexPlusUsageState.observer) codexPlusUsageObserve();
    codexPlusUsageSyncDom();
    codexPlusUsageSchedule(600);
  }

  /** 立即重新挂载、取一次数（插槽容器变了时由 main.js 调）。 */
  function syncCodexPlusUsageStatusLine() {
    codexPlusUsageSyncDom();
    codexPlusUsageSchedule(0);
  }

  function stopCodexPlusUsageStatusLine() {
    codexPlusUsageState.running = false;
    clearTimeout(codexPlusUsageState.timer);
    codexPlusUsageState.timer = 0;
    codexPlusUsageState.observer?.disconnect();
    codexPlusUsageState.observer = null;
    // 把节点整个摘掉：留一个隐藏节点等于每次重载多挂一个。
    codexPlusUsageState.node?.remove();
    codexPlusUsageState.node = null;
    codexPlusUsageState.parts = null;
    codexPlusUsageState.lastFingerprint = "";
    codexPlusUsageState.cache.clear();
    codexPlusUsageState.turnUsage.clear();
    codexPlusUsageState.turnUsageInFlight.clear();
    codexPlusUsageState.turnUsageLiveInFlight.clear();
    document.querySelectorAll("[data-codex-plus-turn-usage]").forEach((node) => node.remove());
    document.getElementById(codexPlusUsageStyleId)?.remove();
  }
