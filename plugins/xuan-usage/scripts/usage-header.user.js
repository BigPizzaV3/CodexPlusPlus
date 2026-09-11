(() => {
  const globalKey = "__xuanUsageHeaderUi";
  const buttonMarker = "data-xuan-usage-button";
  const panelMarker = "data-xuan-usage-panel";
  const styleMarker = "data-xuan-usage-style";
  const bridgeUrl = window.__XUAN_BRIDGE_URL__ || "http://127.0.0.1:57324";
  const bridgeToken = window.__XUAN_BRIDGE_TOKEN__ || "";

  window[globalKey]?.destroy?.();

  const state = {
    button: null,
    panel: null,
    subtitle: null,
    content: null,
    refreshButton: null,
    loading: false,
    lastLoadedAt: 0,
    errorKind: "",
    payload: null,
    metrics: [],
  };

  const createElement = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  };

  const ensureStyles = () => {
    if (document.querySelector(`[${styleMarker}]`)) return;
    const style = document.createElement("style");
    style.setAttribute(styleMarker, "true");
    style.textContent = `
      [${buttonMarker}] { min-width: 54px; max-width: 150px; justify-content: center; overflow: hidden; font-variant-numeric: tabular-nums; text-overflow: ellipsis; }
      [${panelMarker}] { position: fixed; z-index: 2147483000; overflow: hidden; border: 1px solid var(--color-token-border-default, rgba(0,0,0,.12)); border-radius: 8px; background: var(--color-background-elevated-primary-opaque, #fff); color: var(--color-text-secondary-solid, #222); box-shadow: var(--shadow-2xl, 0 16px 32px rgba(0,0,0,.18)); font-family: var(--font-sans-default, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif); font-size: 14px; }
      [${panelMarker}] *, [${panelMarker}] *::before, [${panelMarker}] *::after { box-sizing: border-box; letter-spacing: 0; }
      [${panelMarker}][hidden] { display: none !important; }
      .xuan-usage-header { display: flex; min-width: 0; align-items: center; gap: 8px; padding: 10px 12px; border-bottom: 1px solid var(--color-token-border-default, rgba(0,0,0,.12)); }
      .xuan-usage-heading { min-width: 0; flex: 1; }
      .xuan-usage-title { font-size: 14px; font-weight: 600; line-height: 20px; }
      .xuan-usage-subtitle { overflow: hidden; color: var(--color-text-tertiary, #777); font-size: 12px; line-height: 16px; text-overflow: ellipsis; white-space: nowrap; }
      .xuan-usage-header-button { height: 28px; border: 0; border-radius: 7px; background: transparent; color: var(--color-text-tertiary, #777); padding: 0 7px; font: inherit; font-size: 12px; cursor: pointer; }
      .xuan-usage-header-button:hover, .xuan-usage-header-button:focus-visible { background: var(--color-background-primary-ghost-focus, rgba(0,0,0,.06)); color: inherit; outline: none; }
      .xuan-usage-header-button:disabled { cursor: default; opacity: .55; }
      .xuan-usage-close { width: 28px; padding: 0; font-size: 16px; }
      .xuan-usage-content { min-height: 88px; padding: 4px 12px 8px; }
      .xuan-usage-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 16px; align-items: baseline; padding: 8px 0; border-bottom: 1px solid var(--color-token-border-default, rgba(0,0,0,.12)); }
      .xuan-usage-row:last-child { border-bottom: 0; }
      .xuan-usage-label { min-width: 0; color: var(--color-text-tertiary, #777); }
      .xuan-usage-value { color: var(--color-text-secondary-solid, #333); font-weight: 600; font-variant-numeric: tabular-nums; text-align: right; }
      .xuan-usage-state { padding: 18px 0 14px; color: var(--color-text-tertiary, #777); line-height: 20px; }
      .xuan-usage-state[data-kind="error"] { color: var(--color-text-error, #c33); }
      .xuan-usage-hint { margin-top: 5px; color: var(--color-text-tertiary, #777); font-size: 12px; line-height: 18px; }
      .xuan-usage-footer { padding: 7px 12px 8px; border-top: 1px solid var(--color-token-border-default, rgba(0,0,0,.12)); color: var(--color-text-tertiary, #777); font-size: 11px; line-height: 15px; }
    `;
    document.head.appendChild(style);
  };

  const shareButton = () => document.querySelector(
    "button[aria-label='分享当前会话'], button[aria-label='Share current conversation']"
  );

  const sidePanelButton = () => [...document.querySelectorAll("button")].find((button) => {
    const label = button.getAttribute("aria-label") || "";
    const rect = button.getBoundingClientRect();
    return /显示\/隐藏侧边面板|Show\/hide side panel/i.test(label)
      && rect.y >= 35
      && rect.y < 85
      && rect.x > window.innerWidth / 2;
  });

  const bridgeRequest = async () => {
    let body;
    if (typeof window.__codexSessionDeleteBridge === "function") {
      body = await window.__codexSessionDeleteBridge("/v1/usage", {});
    } else {
      const headers = { "content-type": "application/json" };
      if (bridgeToken) headers["x-xuan-bridge-token"] = bridgeToken;
      const response = await fetch(`${bridgeUrl}/v1/usage`, {
        method: "POST",
        headers,
        body: "{}",
      });
      body = await response.json().catch(() => null);
      if (!response.ok) body = body || { status: "failed" };
    }
    if (body?.status === "failed" || body?.error) {
      const error = new Error("bridge request failed");
      error.code = body?.error?.code || "request_failed";
      throw error;
    }
    return body;
  };

  const numberAt = (source, path) => {
    const value = path.split(".").reduce((current, key) => current?.[key], source);
    const number = typeof value === "string" && value.trim() ? Number(value) : value;
    return Number.isFinite(number) && number >= 0 ? number : null;
  };

  const stringAt = (source, paths) => {
    for (const path of paths) {
      const value = path.split(".").reduce((current, key) => current?.[key], source);
      if (typeof value === "string" && value.trim()) return value.trim();
    }
    return "";
  };

  const firstNumber = (source, paths) => {
    for (const path of paths) {
      const value = numberAt(source, path);
      if (value != null) return { value, path };
    }
    return null;
  };

  const extractMetrics = (data) => {
    const source = data && typeof data === "object" ? data : {};
    const unit = stringAt(source, ["unit", "currency", "usage.unit", "usage.currency"]);
    const definitions = [
      ["todayUsed", "今日用量", ["todayUsed", "today.used", "today.cost", "usage.today.actual_cost", "usage.today.cost"]],
      ["used", "已用额度", ["used", "totalUsed", "usage.used", "usage.total", "cost", "spend"]],
      ["remaining", "剩余额度", ["remaining", "balance", "usage.remaining", "quota.remaining"]],
      ["limit", "总额度", ["limit", "quota", "usage.limit", "quota.total"]],
      ["requests", "请求数", ["requests", "requestCount", "usage.requests"]],
      ["tokens", "Token", ["tokens", "totalTokens", "usage.tokens", "usage.total_tokens"]],
      ["inputTokens", "输入 Token", ["inputTokens", "input_tokens", "usage.input_tokens"]],
      ["outputTokens", "输出 Token", ["outputTokens", "output_tokens", "usage.output_tokens"]],
    ];
    const usedPaths = new Set();
    const metrics = [];
    definitions.forEach(([key, label, paths]) => {
      const found = firstNumber(source, paths);
      if (!found || usedPaths.has(found.path)) return;
      usedPaths.add(found.path);
      metrics.push({ key, label, value: found.value, unit: ["requests", "tokens", "inputTokens", "outputTokens"].includes(key) ? "" : unit });
    });
    return metrics;
  };

  const formatValue = (metric, compact = false) => {
    const value = metric?.value;
    if (!Number.isFinite(value)) return "—";
    if (/^[A-Z]{3}$/.test(metric.unit || "")) {
      try {
        return new Intl.NumberFormat("zh-CN", {
          style: "currency",
          currency: metric.unit,
          currencyDisplay: "narrowSymbol",
          maximumFractionDigits: 2,
        }).format(value);
      } catch {
        return `${value.toFixed(2)} ${metric.unit}`;
      }
    }
    const formatted = new Intl.NumberFormat("zh-CN", {
      notation: compact ? "compact" : "standard",
      maximumFractionDigits: value < 10 ? 2 : 1,
    }).format(value);
    return metric.unit ? `${formatted} ${metric.unit}` : formatted;
  };

  const messageForError = (error) => {
    const messages = {
      configuration_error: "尚未配置用量查询",
      transport_error: "暂时无法连接用量服务",
      remote_error: "用量服务暂时不可用",
      invalid_response: "用量服务返回了无法识别的数据",
    };
    return messages[error?.code] || "暂时无法获取用量";
  };

  const updateButton = () => {
    if (!state.button) return;
    const primary = state.metrics.find((metric) => metric.key === "todayUsed") || state.metrics[0];
    const label = primary ? `${primary.key === "todayUsed" ? "今日" : "用量"} ${formatValue(primary, true)}` : "用量";
    if (state.button.textContent !== label) state.button.textContent = label;
    const status = state.errorKind ? "，当前不可用" : primary ? `，${primary.label} ${formatValue(primary)}` : "";
    state.button.title = `查看用量${status}`;
    state.button.setAttribute("aria-label", `查看用量${status}`);
  };

  const renderPanel = () => {
    if (!state.content || !state.subtitle) return;
    state.content.replaceChildren();
    if (state.loading) {
      state.subtitle.textContent = "正在更新";
      state.content.append(createElement("div", "xuan-usage-state", "正在获取用量…"));
      return;
    }
    if (state.errorKind) {
      state.subtitle.textContent = "当前配置";
      const message = createElement("div", "xuan-usage-state", messageForError({ code: state.errorKind }));
      message.dataset.kind = "error";
      message.append(createElement("div", "xuan-usage-hint", "请在 Xuan 功能配置中完成供应商地址和密钥环境变量设置。"));
      state.content.append(message);
      return;
    }
    state.subtitle.textContent = state.payload?.profileName || "当前配置";
    if (!state.metrics.length) {
      state.content.append(createElement("div", "xuan-usage-state", "暂时没有可展示的用量汇总"));
      return;
    }
    state.metrics.forEach((metric) => {
      const row = createElement("div", "xuan-usage-row");
      row.append(
        createElement("span", "xuan-usage-label", metric.label),
        createElement("span", "xuan-usage-value", formatValue(metric))
      );
      state.content.append(row);
    });
  };

  const updateFooter = () => {
    const footer = state.panel?.querySelector(".xuan-usage-footer");
    if (!footer) return;
    footer.textContent = state.lastLoadedAt
      ? `更新于 ${new Date(state.lastLoadedAt).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}`
      : "等待更新";
  };

  const loadUsage = async (force = false) => {
    if (state.loading) return;
    if (!force && state.lastLoadedAt && Date.now() - state.lastLoadedAt < 60_000) return;
    state.loading = true;
    state.refreshButton && (state.refreshButton.disabled = true);
    renderPanel();
    try {
      state.payload = await bridgeRequest();
      state.metrics = extractMetrics(state.payload?.data);
      state.errorKind = "";
    } catch (error) {
      state.payload = null;
      state.metrics = [];
      state.errorKind = error?.code || "request_failed";
    } finally {
      state.loading = false;
      state.lastLoadedAt = Date.now();
      state.refreshButton && (state.refreshButton.disabled = false);
      updateButton();
      renderPanel();
      updateFooter();
    }
  };

  const ensurePanel = () => {
    if (state.panel?.isConnected) return state.panel;
    ensureStyles();
    const panel = createElement("section");
    panel.id = "xuan-usage-panel";
    panel.setAttribute(panelMarker, "true");
    panel.setAttribute("role", "dialog");
    panel.setAttribute("aria-label", "用量");
    panel.hidden = true;

    const header = createElement("div", "xuan-usage-header");
    const heading = createElement("div", "xuan-usage-heading");
    const title = createElement("div", "xuan-usage-title", "用量");
    const subtitle = createElement("div", "xuan-usage-subtitle", "当前配置");
    const refresh = createElement("button", "xuan-usage-header-button", "刷新");
    refresh.type = "button";
    refresh.addEventListener("click", () => loadUsage(true));
    const close = createElement("button", "xuan-usage-header-button xuan-usage-close", "×");
    close.type = "button";
    close.title = "关闭用量";
    close.setAttribute("aria-label", "关闭用量");
    close.addEventListener("click", () => setOpen(false));
    heading.append(title, subtitle);
    header.append(heading, refresh, close);

    const content = createElement("div", "xuan-usage-content");
    const footer = createElement("div", "xuan-usage-footer", "等待更新");
    panel.append(header, content, footer);
    document.body.append(panel);
    state.panel = panel;
    state.subtitle = subtitle;
    state.content = content;
    state.refreshButton = refresh;
    renderPanel();
    return panel;
  };

  const positionPanel = () => {
    if (!state.button || !state.panel || state.panel.hidden) return;
    const rect = state.button.getBoundingClientRect();
    const width = Math.min(320, Math.max(260, window.innerWidth - 16));
    const left = Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8));
    state.panel.style.width = `${width}px`;
    state.panel.style.left = `${left}px`;
    state.panel.style.top = `${rect.bottom + 6}px`;
  };

  const setOpen = (open) => {
    const panel = ensurePanel();
    panel.hidden = !open;
    state.button?.setAttribute("aria-expanded", String(open));
    if (!open) return;
    positionPanel();
    renderPanel();
    updateFooter();
    loadUsage();
  };

  const mountButton = () => {
    const share = shareButton();
    const anchor = share || sidePanelButton();
    if (!anchor?.parentElement) return;
    ensureStyles();
    let button = document.querySelector(`[${buttonMarker}]`);
    if (!button) {
      button = document.createElement("button");
      button.type = "button";
      button.setAttribute(buttonMarker, "true");
      button.setAttribute("aria-haspopup", "dialog");
      button.setAttribute("aria-controls", "xuan-usage-panel");
      button.addEventListener("click", (event) => {
        event.stopPropagation();
        setOpen(state.panel?.hidden !== false);
      });
    }
    button.className = anchor.className;
    if (share) {
      for (const property of ["width", "min-width", "padding-inline", "aspect-ratio"]) {
        button.style.removeProperty(property);
      }
    } else {
      button.style.setProperty("width", "auto", "important");
      button.style.setProperty("min-width", "54px", "important");
      button.style.setProperty("padding-inline", "8px", "important");
      button.style.setProperty("aspect-ratio", "auto", "important");
    }
    state.button = button;
    updateButton();
    if (anchor.previousElementSibling !== button) anchor.insertAdjacentElement("beforebegin", button);
    if (!state.lastLoadedAt) loadUsage();
  };

  const onPointerDown = (event) => {
    if (!state.panel || state.panel.hidden) return;
    if (state.panel.contains(event.target) || state.button?.contains(event.target)) return;
    setOpen(false);
  };

  const onKeyDown = (event) => {
    if (event.key !== "Escape" || !state.panel || state.panel.hidden) return;
    setOpen(false);
    state.button?.focus();
  };

  const observer = new MutationObserver(mountButton);
  observer.observe(document.documentElement, { childList: true, subtree: true });
  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("resize", positionPanel);
  mountButton();

  window[globalKey] = {
    destroy() {
      observer.disconnect();
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", positionPanel);
      document.querySelector(`[${buttonMarker}]`)?.remove();
      document.querySelector(`[${panelMarker}]`)?.remove();
      document.querySelector(`[${styleMarker}]`)?.remove();
    },
  };
})();
