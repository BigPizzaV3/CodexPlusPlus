  async function loadBackendSettings() {
    const loaded = await loadBackendSettingsState();
    if (loaded && codexRemoteSessionProviderOverrideEnabled()) {
      void loadCodexModelCatalog();
    }
    refreshCodexPlusBackendToggles();
    if (loaded) syncOfficialUsagePolicy();
    if (loaded) runScanStep(syncCodexPlusTypingEffects);
    if (loaded && typeof installCodexPlusCustomLayout === "function") runScanStep(installCodexPlusCustomLayout);
    return loaded;
  }

  function loadBackendSettingsForStartup(attempt = 0) {
    loadBackendSettings().then((loaded) => {
      if (loaded) {
        scan();
        return;
      }
      if (attempt < 60) {
        setTimeout(() => loadBackendSettingsForStartup(attempt + 1), 250);
      }
    });
  }

  let syncBackendSettingsInFlight = false;
  async function syncBackendSettingsFromHeartbeat() {
    if (syncBackendSettingsInFlight) return;
    syncBackendSettingsInFlight = true;
    try {
      const previousConversationView = !!codexPlusSettings().conversationView;
      const loaded = await loadBackendSettingsState();
      if (loaded) {
        syncOfficialUsagePolicy();
        runScanStep(syncCodexPlusTypingEffects);
        if (typeof installCodexPlusCustomLayout === "function") runScanStep(installCodexPlusCustomLayout);
        renderCodexPlusMenu();
        if (previousConversationView !== !!codexPlusSettings().conversationView) {
          refreshConversationView();
        }
      }
    } finally {
      syncBackendSettingsInFlight = false;
    }
  }

  async function setBackendSetting(key, value) {
    const seq = ++codexPlusBackendSettingsSeq;
    codexPlusBackendSettings = { ...codexPlusBackendSettings, [key]: value };
    codexPlusBackendSettingsLoaded = true;
    refreshCodexPlusBackendToggles();
    try {
      const settings = await postJson("/settings/set", { [key]: value });
      if (seq === codexPlusBackendSettingsSeq) {
        codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
      }
    } finally {
      refreshCodexPlusBackendToggles();
    }
  }

  function refreshCodexPlusBackendToggles() {
    document.querySelectorAll(".codex-plus-toggle[data-codex-backend-setting]").forEach((button) => {
      const key = button.getAttribute("data-codex-backend-setting");
      button.dataset.enabled = String(!!codexPlusBackendSettings[key]);
      button.setAttribute("aria-checked", button.dataset.enabled);
    });
    syncStepwisePanel();
    runScanStep(syncCodexPlusTypingEffects);
    if (typeof installCodexPlusCustomLayout === "function") runScanStep(installCodexPlusCustomLayout);
    renderCodexPlusMenu();
    scan();
  }

  let codexPlusUserScripts = { enabled: true, builtin_dir: "", user_dir: "", scripts: [] };
  // 单独跟踪「读过了没有」：scripts 为空既可能是真没有脚本，也可能是还没读到。
  // 不区分就会在无后端时把「正在读取」直接显示成「未发现」。
  let codexPlusUserScriptsLoaded = false;
  // 市场清单。为空 + 未加载 = 还在拉；加载过为空 = 市场里确实没东西。
  let codexPlusScriptMarket = { scripts: [], loaded: false, loading: false, message: "" };
  // 「拓展」页左面板的搜索关键词，纯前端过滤。
  let codexPlusExtensionsQuery = "";
  let codexPlusExtensionsFilter = "market";
  // 当前选中的拓展（左面板点开后右侧显示详情）。空 = 还没选。
  let codexPlusExtensionsSelected = null;
  // 默认扩展图标：VSCode codicon 的 `extensions` 字形（\eae6），
  // 从本机 VSCode 的 codicon.ttf 抽出轮廓后归一到 16x16 视口。
  // 市场清单目前没有图标字段，所有市场条目都用它；本地脚本同理。
  const codexPlusDefaultExtensionIconPath = "M15.0 4.95 Q15.0 4.37 14.63 3.99 L12.01 1.37 Q11.63 1.0 11.05 1.0 Q10.46 1.0 10.08 1.37 L8.0 3.46 L8.0 3.3 Q8.0 2.71 7.6 2.31 Q7.2 1.91 6.61 1.91 L2.39 1.91 Q1.8 1.91 1.4 2.31 Q1.0 2.71 1.0 3.3 L1.0 13.61 Q1.0 14.2 1.4 14.6 Q1.8 15.0 2.39 15.0 L12.7 15.0 Q13.24 15.0 13.66 14.6 Q14.09 14.2 14.09 13.61 L14.09 9.39 Q14.09 8.8 13.66 8.4 Q13.24 8.0 12.7 8.0 L12.54 8.0 L14.63 5.92 Q15.0 5.54 15.0 4.95 Z M2.39 2.87 L6.61 2.87 Q6.77 2.87 6.93 3.0 Q7.09 3.14 7.09 3.3 L7.09 8.0 L1.91 8.0 L1.91 3.3 Q1.91 3.14 2.04 3.0 Q2.18 2.87 2.39 2.87 Z M1.91 13.61 L1.91 8.91 L7.09 8.91 L7.09 14.09 L2.39 14.09 Q2.18 14.09 2.04 13.96 Q1.91 13.82 1.91 13.61 Z M13.13 9.39 L13.13 13.61 Q13.13 13.82 13.0 13.96 Q12.86 14.09 12.7 14.09 L8.0 14.09 L8.0 8.91 L12.7 8.91 Q12.86 8.91 13.0 9.07 Q13.13 9.23 13.13 9.39 Z M8.0 8.0 L8.0 6.45 L9.55 8.0 Z M13.93 5.27 L11.37 7.84 Q11.21 8.0 11.02 8.0 Q10.83 8.0 10.73 7.84 L8.11 5.27 Q8.0 5.11 8.0 4.93 Q8.0 4.74 8.11 4.63 L10.73 2.02 Q10.83 1.91 11.02 1.91 Q11.21 1.91 11.37 2.02 L13.93 4.63 Q14.09 4.74 14.09 4.93 Q14.09 5.11 13.93 5.27 Z";
  let codexPlusBackendStatus = window.__codexPlusBackendStatus || { status: "checking", message: "正在检查后端…" };
  let codexPlusBackendCheckSeq = 0;
  let codexPlusBackendCheckInFlight = false;
  let codexPlusBackendFailureCount = 0;
  const CODEX_PLUS_BACKEND_FAILURE_THRESHOLD = 3;
  // 桥接通道（binding）与后端可用性分开统计：HTTP 回落成功会让后端状态保持绿色，
  // 但桥接持续失败时必须把降级呈现出来，否则启动器侧的重注入修复循环对用户完全不可见（issue #2169）。
  let codexPlusBridgeFailureCount = 0;
  const CODEX_PLUS_BRIDGE_FAILURE_THRESHOLD = 3;
  const codexPlusBackendGeneration = (Number(window.__codexPlusBackendGeneration) || 0) + 1;
  window.__codexPlusBackendGeneration = codexPlusBackendGeneration;

  function recordCodexPlusBridgeHealth(field) {
    if (codexPlusBackendGeneration !== window.__codexPlusBackendGeneration) return;
    const health = window.__codexPlusBridgeHealth || (window.__codexPlusBridgeHealth = {});
    health[field] = Date.now();
  }

  function recordCodexPlusBridgeSuccess() {
    recordCodexPlusBridgeHealth("lastSuccessAt");
    codexPlusBridgeFailureCount = 0;
  }

  function recordCodexPlusBridgeAttempt() {
    recordCodexPlusBridgeHealth("lastAttemptAt");
  }

  function recordCodexPlusBridgeFailure() {
    codexPlusBridgeFailureCount += 1;
  }

  function renderBackendStatus() {
    const bridgeDegraded = codexPlusBridgeFailureCount >= CODEX_PLUS_BRIDGE_FAILURE_THRESHOLD;
    const rawStatus = codexPlusBackendStatus.status || "failed";
    const status = bridgeDegraded && rawStatus === "ok" ? "degraded" : rawStatus;
    if (codexPlusBackendStatus.version) {
      codexPlusVersion = codexPlusBackendStatus.version;
      document.querySelectorAll("[data-codex-plus-version]").forEach((node) => {
        node.textContent = `Codex++ ${codexPlusVersion}`;
      });
    }
    const labelFallback = status === "ok" ? "后端已连接" : status === "degraded" ? "桥接降级，自动修复中" : status === "checking" ? "正在检查后端…" : "未连接";
    const label = document.querySelector("[data-codex-backend-status]");
    if (label) {
      label.dataset.status = status;
      label.textContent = status === "degraded" ? labelFallback : (codexPlusBackendStatus.message || labelFallback);
    }
    document.querySelectorAll("[data-codex-backend-indicator]").forEach((indicator) => {
      indicator.dataset.status = status;
      indicator.title = status === "ok" ? "后端已连接" : status === "degraded" ? "后端可达，桥接降级，正在自动修复" : status === "checking" ? "正在检查后端" : "未连接";
    });
    const sidebarStatus = document.querySelector(`#${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status`);
    if (sidebarStatus) {
      sidebarStatus.dataset.status = status;
      sidebarStatus.title = status === "ok" ? "后端已连接" : status === "degraded" ? "后端可达，桥接降级，正在自动修复" : status === "checking" ? "正在检查后端" : "未连接";
    }
    refreshCodexServiceTierControls();
  }

  function withBackendTimeout(request) {
    return Promise.race([
      request,
      new Promise((resolve) => setTimeout(() => resolve({ status: "failed", message: "后端检查超时", timeout: true }), 2000)),
    ]);
  }

  async function checkBackendStatus() {
    if (codexPlusBackendCheckInFlight) return;
    codexPlusBackendCheckInFlight = true;
    const seq = ++codexPlusBackendCheckSeq;
    try {
      const nextStatus = await postJson("/backend/status", {});
      if (seq !== codexPlusBackendCheckSeq || codexPlusBackendGeneration !== window.__codexPlusBackendGeneration) return;
      if (nextStatus?.status === "ok") {
        codexPlusBackendFailureCount = 0;
        codexPlusBackendStatus = window.__codexPlusBackendStatus = nextStatus;
        if (typeof nextStatus.hideOfficialUsageAlert === "boolean") {
          window.__CODEX_PLUS_HIDE_OFFICIAL_USAGE_ALERT__ = nextStatus.hideOfficialUsageAlert;
          syncOfficialUsagePolicy();
        }
        void syncBackendSettingsFromHeartbeat();
      } else {
        codexPlusBackendFailureCount += 1;
        sendCodexPlusDiagnostic("backend_check_failed", {
          status: nextStatus?.status || "unknown",
          message: nextStatus?.message || "",
          timeout: !!nextStatus?.timeout,
          consecutiveFailures: codexPlusBackendFailureCount,
        });
        if (codexPlusBackendFailureCount >= CODEX_PLUS_BACKEND_FAILURE_THRESHOLD) {
          codexPlusBackendStatus = window.__codexPlusBackendStatus = nextStatus;
        }
      }
      renderBackendStatus();
    } finally {
      codexPlusBackendCheckInFlight = false;
    }
  }

  async function openManagerFromCodex() {
    const result = await postJson("/manager/open", {});
    if (result.status === "ok") {
      showToast("管理工具已打开", null);
    } else {
      showToast(result.message || "打开管理工具失败", null);
    }
  }

  function scheduleBackendHeartbeat() {
    if (codexPlusBackendGeneration !== window.__codexPlusBackendGeneration) return;
    if (window.__codexPlusBackendHeartbeat &&
        window.__codexPlusBackendHeartbeatGeneration === codexPlusBackendGeneration) return;
    if (window.__codexPlusBackendHeartbeat) clearInterval(window.__codexPlusBackendHeartbeat);
    window.__codexPlusBackendHeartbeatGeneration = codexPlusBackendGeneration;
    window.__codexPlusBackendHeartbeat = setInterval(checkBackendStatus, 5000);
    checkBackendStatus();
  }

  function userScriptStatusLabel(status) {
    return { loaded: "已加载", failed: "失败", disabled: "已禁用", not_loaded: "未加载", loading: "加载中" }[status] || status || "未知";
  }

  // 使用已安装 Lucide Search / RotateCw / Plus / ArrowLeft 的 ISC 图标路径。
  const codexPlusDiscoveryIcons = {
    search: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>',
    refresh: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/></svg>',
    plus: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14M12 5v14"/></svg>',
    back: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m12 19-7-7 7-7M19 12H5"/></svg>',
  };

  /** 原生风格侧栏：浏览入口和已安装拓展的单行名称。 */
  function renderCodexPlusExtensionsNav() {
    const { installed } = codexPlusExtensionsEntries();
    const shown = filterCodexPlusExtensionsEntries(installed);
    const rows = shown.map((entry) => {
      const selected = codexPlusExtensionsSelected?.kind === "installed" && codexPlusExtensionsSelected.key === entry.key;
      const marketItem = codexPlusExtensionMarketItem(entry);
      return `
        <button type="button" class="codex-plus-page-nav-item codex-plus-extensions-nav-entry" data-codex-extensions-select="installed:${escapeHtml(entry.key)}" data-active="${String(selected)}" title="${escapeHtml(entry.name)}" aria-label="${escapeHtml(entry.name)}">
          <span class="codex-plus-page-nav-item-icon" aria-hidden="true">${extensionIconMarkup(marketItem?.icon)}</span>
          <span class="codex-plus-page-nav-item-text"><span class="codex-plus-page-nav-item-name">${escapeHtml(entry.name)}</span></span>
          <span class="codex-plus-page-nav-item-state" data-state="${entry.enabled ? "on" : "off"}" title="${entry.enabled ? "已启用" : "已禁用"}"></span>
        </button>
      `;
    }).join("");
    const empty = codexPlusExtensionsQuery.trim() ? "没有匹配的已安装拓展。" : codexPlusUserScriptsLoaded ? "未发现已安装的拓展。" : "正在读取用户拓展…";
    return `
      <button type="button" class="codex-plus-page-nav-item codex-plus-extensions-nav-entry" data-codex-extensions-browse="market" data-active="${String(!codexPlusExtensionsSelected && codexPlusExtensionsFilter === "market")}" title="浏览拓展" aria-label="浏览拓展">
        <span class="codex-plus-page-nav-item-icon" aria-hidden="true">${extensionIconSvg()}</span>
        <span class="codex-plus-page-nav-item-text"><span class="codex-plus-page-nav-item-name">浏览拓展</span></span>
      </button>
      <div class="codex-plus-page-nav-group">
        <div class="codex-plus-page-nav-group-head"><span>已安装</span><span class="codex-plus-page-nav-group-count">${installed.length}</span></div>
        ${rows || `<div class="codex-plus-page-nav-empty">${escapeHtml(empty)}</div>`}
      </div>
    `;
  }

  /** 主区使用原生插件列表的图标、简介和独立操作列，避免嵌套按钮。 */
  function renderCodexPlusExtensionsOverview() {
    const { installed, market } = codexPlusExtensionsEntries();
    const isInstalled = codexPlusExtensionsFilter === "installed";
    const entries = filterCodexPlusExtensionsEntries(isInstalled ? installed : market);
    const cards = entries.map((entry) => {
      const item = codexPlusExtensionMarketItem(entry);
      const description = item?.description || entry.script?.description || entry.meta || "";
      const select = `${entry.kind}:${escapeHtml(entry.key)}`;
      const action = entry.kind === "market"
        ? `<button type="button" class="codex-plus-discovery-icon-button" data-codex-market-install="${escapeHtml(entry.key)}" aria-label="安装 ${escapeHtml(entry.name)}" title="安装 ${escapeHtml(entry.name)}">${codexPlusDiscoveryIcons.plus}</button>`
        : `<button type="button" class="codex-plus-toggle" data-codex-user-script-key="${escapeHtml(entry.key)}" data-enabled="${String(entry.enabled)}" role="switch" aria-checked="${String(entry.enabled)}" aria-label="启用 ${escapeHtml(entry.name)}"><span></span></button>`;
      return `
        <article class="codex-plus-extension-card">
          <button type="button" class="codex-plus-extension-card-select" data-codex-extensions-select="${select}" aria-label="查看 ${escapeHtml(entry.name)}">
            <span class="codex-plus-extension-card-icon" aria-hidden="true">${extensionIconMarkup(item?.icon)}</span>
            <span class="codex-plus-extension-card-copy">
              <span class="codex-plus-extension-card-title">${escapeHtml(entry.name)}</span>
              <span class="codex-plus-extension-card-description">${escapeHtml(description)}</span>
            </span>
          </button>
          <div class="codex-plus-extension-card-actions">${action}</div>
        </article>
      `;
    }).join("");
    const empty = codexPlusExtensionsQuery.trim() ? `没有匹配「${codexPlusExtensionsQuery}」的拓展。`
      : isInstalled ? (codexPlusUserScriptsLoaded ? "未发现已安装的拓展。" : "正在读取用户拓展…")
      : codexPlusScriptMarket.loading && !codexPlusScriptMarket.loaded ? "正在读取拓展…"
      : codexPlusScriptMarket.message || "市场里没有可安装的拓展。";
    return `
      <div class="codex-plus-extensions-filter" role="group" aria-label="拓展来源">
        <button type="button" data-codex-extensions-browse="market" data-active="${String(!isInstalled)}" aria-pressed="${String(!isInstalled)}">市场</button>
        <button type="button" data-codex-extensions-browse="installed" data-active="${String(isInstalled)}" aria-pressed="${String(isInstalled)}">已安装</button>
      </div>
      <h2 class="codex-plus-discovery-section-title">${isInstalled ? "已安装拓展" : "市场拓展"}</h2>
      <div class="codex-plus-extensions-grid" data-codex-plus-extensions-list="true">
        ${cards || `<div class="codex-plus-extensions-detail-empty" role="status">${escapeHtml(empty)}</div>`}
      </div>
    `;
  }

  /** 只刷新侧栏和结果区；搜索输入框保持原 DOM 与光标位置。 */
  function refreshCodexPlusExtensionsView() {
    if (codexPlusActiveEntry() !== "extensions") return;
    if (codexPlusExtensionsSelected && !codexPlusExtensionsSelectionDetail()) codexPlusExtensionsSelected = null;
    const active = document.activeElement;
    const focused = active?.getAttribute?.("data-codex-user-script-key");
    const focusedBrowse = active?.getAttribute?.("data-codex-extensions-browse");
    const focusedNav = !!active?.closest?.("[data-codex-plus-page-nav-body]");
    const body = document.querySelector("[data-codex-plus-page-nav-body]");
    if (body) body.innerHTML = renderCodexPlusExtensionsNav();
    const detail = document.querySelector("[data-codex-plus-extensions-detail]");
    if (detail) detail.innerHTML = renderCodexPlusExtensionsDetail();
    if (focused) {
      Array.from(detail?.querySelectorAll("[data-codex-user-script-key]") || [])
        .find((button) => button.getAttribute("data-codex-user-script-key") === focused)?.focus({ preventScroll: true });
    } else if (focusedBrowse) {
      Array.from((focusedNav ? body : detail)?.querySelectorAll("[data-codex-extensions-browse]") || [])
        .find((button) => button.getAttribute("data-codex-extensions-browse") === focusedBrowse)?.focus({ preventScroll: true });
    }
  }

  /**
   * 解析当前选中项，拿到本地脚本与市场条目两边的信息。
   *
   * 已安装的市场脚本，本地清单里有 `market_id`，据此把市场的描述/作者等补上；
   * 纯本地脚本则只有本地那几个字段。
   */
  function codexPlusExtensionsSelectionDetail() {
    const sel = codexPlusExtensionsSelected;
    if (!sel) return null;
    const local = sel.kind === "installed"
      ? (codexPlusUserScripts.scripts || []).find((script) => script.key === sel.key) || null
      : null;
    const marketId = sel.kind === "market" ? sel.key : (local?.market_id || "");
    const marketItem = marketId
      ? (codexPlusScriptMarket.scripts || []).find((item) => item.id === marketId) || null
      : null;
    if ((sel.kind === "installed" && !local) || (sel.kind === "market" && !marketItem)) return null;
    return { sel, local, marketItem };
  }

  function extensionIconSvg() {
    return `<svg viewBox="0 0 16 16" fill="currentColor"><path d="${codexPlusDefaultExtensionIconPath}"/></svg>`;
  }

  /**
   * 条目图标：市场清单给了 `icon` 就用它，否则回退 VSCode 的默认扩展字形。
   *
   * 回退是必须的——清单里的老条目没有这个字段，而且 icon 指的是外链，
   * 加载失败时若不兜底就会留一块空白。失败替换交给委托监听（见 handleExtensionIconError），
   * 不用内联 onerror，避免在字符串拼 HTML 时引入另一处转义面。
   */
  function extensionIconMarkup(icon) {
    const url = String(icon || "").trim();
    if (!url) return extensionIconSvg();
    return `<img class="codex-plus-extensions-icon-img" src="${escapeHtml(url)}" alt="" loading="lazy" />`;
  }

  /**
   * 图标加载失败时换回默认字形。
   *
   * `error` 事件不冒泡，只能在捕获阶段用委托收到；换掉节点本身即可，
   * 再失败也不会递归——替换出来的 svg 不触发 error。
   */
  function handleExtensionIconError(event) {
    const img = event.target;
    if (!(img instanceof HTMLImageElement) || !img.classList.contains("codex-plus-extensions-icon-img")) return;
    const holder = document.createElement("span");
    holder.innerHTML = extensionIconSvg();
    const svg = holder.firstElementChild;
    if (svg) img.replaceWith(svg);
  }

  /** 原生风格详情：居中展示图标、名称、介绍与管理操作。 */
  function renderCodexPlusExtensionsDetail() {
    const detail = codexPlusExtensionsSelectionDetail();
    if (!detail) {
      codexPlusExtensionsSelected = null;
      return renderCodexPlusExtensionsOverview();
    }
    const { sel, local, marketItem } = detail;
    const name = marketItem?.name || local?.name || sel.key;
    const version = marketItem?.version || local?.version || "";
    const author = marketItem?.author || "";
    const description = marketItem?.description || local?.description || "";
    const tags = marketItem?.tags || [];
    const requirements = marketItem?.requirements || [];
    const limitations = marketItem?.limitations || [];
    const homepage = marketItem?.homepage || local?.homepage || "";
    const isInstalled = sel.kind === "installed";
    const updateAvailable = isInstalled && marketItem && version && local?.version && local.version !== version;

    // 详情沿用列表的内容宽度，管理操作保持独立。
    const publisherLine = [
      author ? escapeHtml(author) : "",
      version ? `v${escapeHtml(version)}` : "",
      local ? `${local.source === "builtin" ? "内置" : "用户"} · ${escapeHtml(userScriptStatusLabel(local.status))}` : "",
    ].filter(Boolean).join('<span class="codex-plus-extensions-detail-sep">·</span>');

    const actions = [];
    if (isInstalled) {
      actions.push(`
        <button type="button" class="codex-plus-toggle" data-codex-user-script-key="${escapeHtml(local?.key || "")}" data-enabled="${String(!!local?.enabled)}" role="switch" aria-checked="${String(!!local?.enabled)}" aria-label="启用 ${escapeHtml(name)}"><span></span></button>
      `);
      // 内置脚本在只读目录里，删不掉；只给用户目录的脚本提供卸载。
      if (local?.source === "user") {
        actions.push(`<button type="button" class="codex-plus-extensions-detail-button" data-codex-extensions-uninstall="${escapeHtml(local.key)}">卸载</button>`);
      }
    } else if (marketItem) {
      actions.push(`<button type="button" class="codex-plus-extensions-detail-button codex-plus-extensions-detail-primary" data-codex-market-install="${escapeHtml(marketItem.id)}">安装</button>`);
    }

    // 使用要求与限制保留为独立正文分区。
    const list = (title, items) => items.length
      ? `<div class="codex-plus-extensions-detail-section"><div class="codex-plus-extensions-detail-section-title">${escapeHtml(title)}</div><ul>${items.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul></div>`
      : "";

    return `
      <button type="button" class="codex-plus-extensions-back" data-codex-extensions-browse="${codexPlusExtensionsFilter}" aria-label="返回拓展列表">${codexPlusDiscoveryIcons.back}<span>返回拓展列表</span></button>
      <div class="codex-plus-extensions-detail-head">
        <div class="codex-plus-extensions-detail-icon" aria-hidden="true">${extensionIconMarkup(marketItem?.icon)}</div>
        <div class="codex-plus-extensions-detail-heading">
          <div class="codex-plus-extensions-detail-title">${escapeHtml(name)}</div>
          ${publisherLine ? `<div class="codex-plus-extensions-detail-meta">${publisherLine}</div>` : ""}
          ${updateAvailable ? `<div class="codex-plus-extensions-detail-update">有新版本 v${escapeHtml(version)} 可更新</div>` : ""}
        </div>
        <div class="codex-plus-extensions-detail-actions">${actions.join("")}</div>
      </div>
      <div class="codex-plus-extensions-detail-body">
        ${description ? `<div class="codex-plus-extensions-detail-description">${escapeHtml(description)}</div>` : ""}
        ${tags.length ? `<div class="codex-plus-extensions-detail-tags">${tags.map((tag) => `<span>${escapeHtml(tag)}</span>`).join("")}</div>` : ""}
        ${list("使用要求", requirements)}
        ${list("已知限制", limitations)}
        ${homepage ? `<div class="codex-plus-extensions-detail-link"><a href="${escapeHtml(homepage)}" target="_blank" rel="noreferrer">${escapeHtml(homepage)}</a></div>` : ""}
        ${local?.error ? `<div class="codex-plus-extensions-detail-error">${escapeHtml(local.error)}</div>` : ""}
      </div>
    `;
  }

  /** 卸载用户脚本：删文件 + 清记录，然后刷新两侧。 */
  async function uninstallUserScript(key) {
    if (!key) return;
    await postJson("/user-scripts/delete", { key });
    if (codexPlusExtensionsSelected?.kind === "installed" && codexPlusExtensionsSelected.key === key) {
      codexPlusExtensionsSelected = null;
    }
    await loadUserScripts();
    refreshCodexPlusExtensionsView();
  }

  /** 左面板的导航项。Codex++ 页面切分组，「拓展」页面列脚本。 */
  function renderCodexPlusPageNavItems(tab) {
    if (tab === codexPlusExtensionsTab) return renderCodexPlusExtensionsNav();
    return [
      { key: "home", label: "主页" },
      { key: "sponsor", label: "推荐内容" },
    ].map((item) => `
      <button type="button" class="codex-plus-page-nav-item" data-codex-plus-page-nav="${item.key}" data-active="${String(tab === item.key)}">${item.label}</button>
    `).join("");
  }

  /**
   * 页面模式下把单栏内容改造成两栏：左面板 + 右内容区。
   *
   * 只搬动已有的 .codex-plus-modal-body，不重建里面那些 data-codex-* 挂载点，
   * 免得 renderUserScripts / 各类 toggle 的 querySelector 找不到目标。
   */
  function installCodexPlusPageLayout(overlay, tab) {
    if (!overlay || overlay.querySelector(".codex-plus-page-layout")) return;
    const content = overlay.querySelector(".codex-plus-modal-content");
    const body = content?.querySelector(".codex-plus-modal-body");
    if (!content || !body) return;
    const layout = document.createElement("div");
    layout.className = "codex-plus-page-layout";
    const main = document.createElement("div");
    main.className = "codex-plus-page-main";
    // 只有「拓展」需要左面板（它是脚本列表）。主页和推荐内容都是单栏内容页，
    // 页面切换交给图标栏那三个入口，再列一遍就是重复。
    if (tab === codexPlusExtensionsTab) {
      const nav = document.createElement("div");
      nav.className = "codex-plus-page-nav";
      nav.innerHTML = `
        <div class="codex-plus-page-nav-header"><div class="codex-plus-page-nav-title">${codexPlusPageTitle(tab)}</div></div>
        <div class="codex-plus-page-nav-body" data-codex-plus-page-nav-body="true">${renderCodexPlusPageNavItems(tab)}</div>
      `;
      layout.appendChild(nav);
    }
    content.appendChild(layout);
    layout.appendChild(main);
    main.appendChild(body);
    if (tab === "home") {
      // 设置标题和卡片共用居中栏，并随内容滚动，保持原生设置页的阅读节奏。
      const header = content.querySelector(".codex-plus-modal-header");
      if (header) body.querySelector(".codex-plus-settings")?.prepend(header);
    }
  }

  /** 页面标题：每个 rail 入口一个名字，和图标栏上的标签保持一致。 */
  function codexPlusPageTitle(tab) {
    if (tab === codexPlusExtensionsTab) return "拓展";
    if (tab === codexPlusSponsorTab) return "推荐内容";
    return "Codex++";
  }

  /** 左面板内容随当前分组刷新（切 tab 后调用）。 */
  function refreshCodexPlusPageNav(tab) {
    const body = document.querySelector("[data-codex-plus-page-nav-body]");
    if (!body) return;
    const title = document.querySelector(".codex-plus-page-nav-title");
    if (title) title.textContent = tab === codexPlusExtensionsTab ? "拓展" : "Codex++";
    body.innerHTML = renderCodexPlusPageNavItems(tab);
  }

  /**
   * 脚本清单变化后同步左面板。
   *
   * 原来这里还要往「用户脚本」区块的开关与目录文本里写值，那个区块已经删掉，
   * 侧栏显示已安装清单，主区列表与详情由 refreshCodexPlusExtensionsView 同步。
   */
  function renderUserScripts() {
    // 左面板也要跟着刷新，否则脚本的启停/状态变化不会反映到列表上。
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusPageNav(codexPlusExtensionsTab);
  }

  async function loadUserScripts(path = "/user-scripts/list", payload = {}) {
    const requestPayload = path === "/user-scripts/list"
      ? { ...payload, runtime_status: window.__codexPlusUserScripts?.scripts || {} }
      : payload;
    const result = await postJson(path, requestPayload);
    if (result?.scripts) {
      codexPlusUserScripts = result;
      codexPlusUserScriptsLoaded = true;
      renderUserScripts();
      // 已安装状态变了，市场的「已安装/有更新」标记也要跟着刷新。
      if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
    }
  }

  /**
   * 拉市场清单。
   *
   * 清单与「已安装」状态都由后端合并好（见 script_market::market_scripts_payload），
   * 前端只需合并本地脚本清单来显示来源与状态。
   */
  async function loadScriptMarket(force = false) {
    if (codexPlusScriptMarket.loading) return;
    if (codexPlusScriptMarket.loaded && !force) return;
    codexPlusScriptMarket = { ...codexPlusScriptMarket, loading: true };
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
    const result = await postJson("/script-market/list", {});
    if (Array.isArray(result?.scripts)) {
      codexPlusScriptMarket = {
        scripts: result.scripts,
        loaded: true,
        loading: false,
        message: result.message || "",
      };
    } else {
      codexPlusScriptMarket = {
        ...codexPlusScriptMarket,
        loaded: true,
        loading: false,
        message: result?.message || "拓展加载失败",
      };
    }
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
  }

  async function installScriptFromMarket(id) {
    if (!id) return;
    const result = await postJson("/script-market/install", { id });
    if (Array.isArray(result?.scripts)) {
      codexPlusScriptMarket = {
        scripts: result.scripts,
        loaded: true,
        loading: false,
        message: result.message || "",
      };
    }
    // 后端装完会把本地清单一起带回来，省一次往返。
    if (result?.user_scripts?.scripts) {
      codexPlusUserScripts = result.user_scripts;
      codexPlusUserScriptsLoaded = true;
      renderUserScripts();
    } else {
      await loadUserScripts();
    }
    if (codexPlusExtensionsSelected?.kind === "market" && codexPlusExtensionsSelected.key === id) {
      const installed = (codexPlusUserScripts.scripts || []).find((script) => script.market_id === id);
      if (installed) codexPlusExtensionsSelected = { kind: "installed", key: installed.key };
    }
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
  }

  /**
   * 市场条目与本地脚本合并后的视图。
   *
   * 本地有市场装来的脚本（带 market_id），据此判断「已安装」并给出卸载入口；
   * 「市场」分组只列还没装的，避免同一脚本出现两次。
   */
  function codexPlusExtensionsEntries() {
    const local = codexPlusUserScripts.scripts || [];
    const marketById = new Map((codexPlusScriptMarket.scripts || []).map((item) => [item.id, item]));
    const localByMarketId = new Map(
      local.filter((script) => script.market_id).map((script) => [script.market_id, script]),
    );
    const installed = local.map((script) => ({
      kind: "installed",
      key: script.key,
      name: marketById.get(script.market_id)?.name || script.name || script.key,
      meta: `${script.source === "builtin" ? "内置" : script.market_id ? "市场" : "用户"} · ${userScriptStatusLabel(script.status)}`,
      enabled: !!script.enabled,
      script,
    }));
    const market = (codexPlusScriptMarket.scripts || [])
      .filter((item) => !localByMarketId.has(item.id))
      .map((item) => ({
        kind: "market",
        key: item.id,
        name: item.name || item.id,
        meta: `${item.author || "未知作者"} · v${item.version}`,
        item,
      }));
    const available = (codexPlusScriptMarket.scripts || []).filter((item) => localByMarketId.has(item.id));
    return { installed, market, available, localByMarketId };
  }

  /**
   * 取条目对应的市场清单项。
   *
   * 市场条目自带 `item`；已安装条目只有本地脚本（`script`），要靠脚本上的
   * `market_id` 回查，否则拿不到图标、简介、标签。纯本地脚本两者都没有。
   */
  function codexPlusExtensionMarketItem(entry) {
    if (entry.kind === "market") return entry.item || null;
    const marketId = entry.script?.market_id;
    if (!marketId) return null;
    return (codexPlusScriptMarket.scripts || []).find((item) => item.id === marketId) || null;
  }

  function filterCodexPlusExtensionsEntries(entries) {
    const query = codexPlusExtensionsQuery.trim().toLowerCase();
    if (!query) return entries;
    return entries.filter((entry) => {
      const marketItem = codexPlusExtensionMarketItem(entry);
      return [entry.name, entry.key, entry.script?.name, entry.meta, marketItem?.description, ...(marketItem?.tags || [])]
        .filter(Boolean)
        .some((text) => String(text).toLowerCase().includes(query));
    });
  }

  const codexPlusAdsUrl = "/ads";
  let codexPlusAds = [];
  let codexPlusAdsLoaded = false;

  function isCodexPlusAdExpired(ad) {
    if (!ad.expires_at) return false;
    const expiresAt = Date.parse(ad.expires_at);
    return Number.isFinite(expiresAt) && expiresAt < Date.now();
  }

  function normalizeCodexPlusAds(payload) {
    if (!payload || !Array.isArray(payload.ads)) return [];
    // 只留赞助商推荐。上游清单里的 normal 条目不属于合作伙伴分组，
    // 放进来只会让「有数据但渲染不出东西」的状态变得难以判断。
    return payload.ads.filter((ad) => {
      return ad && ad.type === "sponsor" && ad.title && ad.description && ad.url && !isCodexPlusAdExpired(ad);
    }).map((ad) => ({
      id: String(ad.id || ad.title),
      type: ad.type,
      title: String(ad.title),
      description: String(ad.description),
      url: String(ad.url),
      image: ad.image ? String(ad.image) : "",
      expires_at: ad.expires_at ? String(ad.expires_at) : "",
      highlights: Array.isArray(ad.highlights) ? ad.highlights.map((item) => String(item)).filter(Boolean) : [],
    }));
  }

  function formatCodexPlusAdTitle(title) {
    const value = String(title || "");
    return value.split(/[｜|]/, 1)[0].trim() || value;
  }

  /** 推荐卡片的右上角外链箭头，跟 VSCode/Codex 的「新窗口打开」同一语义。 */
  const codexPlusAdArrowIcon = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5.5 10.5 10.5 5.5"/><path d="M6.5 5.5h4v4"/></svg>';

  function renderCodexPlusAdGroup() {
    const ads = codexPlusAds;
    if (!ads.length) return `<div class="codex-plus-ad-empty">暂无推荐内容。</div>`;
    return ads.map((ad) => {
      const name = formatCodexPlusAdTitle(ad.title);
      // 底部那条优惠信息：取第一个亮点。没有就不渲染整条，免得留一段空白。
      const promo = (ad.highlights || []).find(Boolean) || "";
      // 没有配图时用名称首字当占位，比空一块更整齐。
      const icon = ad.image
        ? `<img class="codex-plus-ad-icon" src="${escapeHtml(ad.image)}" alt="" loading="lazy" />`
        : `<span class="codex-plus-ad-icon codex-plus-ad-icon-fallback" aria-hidden="true">${escapeHtml(name.slice(0, 1))}</span>`;
      return `
        <a class="codex-plus-ad-card" data-codex-plus-ad-url="${escapeHtml(ad.url)}" href="${escapeHtml(ad.url)}" rel="noreferrer" title="${escapeHtml(name)}">
          <span class="codex-plus-ad-main">
            ${icon}
            <span class="codex-plus-ad-text">
              <span class="codex-plus-ad-title">${escapeHtml(name)}</span>
              <span class="codex-plus-ad-description">${escapeHtml(ad.description)}</span>
              ${promo ? `<span class="codex-plus-ad-promo">${escapeHtml(promo)}</span>` : ""}
            </span>
            <span class="codex-plus-ad-arrow" aria-hidden="true">${codexPlusAdArrowIcon}</span>
          </span>
        </a>
      `;
    }).join("");
  }

  function renderCodexPlusAds() {
    if (!codexPlusAdsLoaded) return `<div class="codex-plus-ad-empty">推荐内容加载中…</div>`;
    if (!codexPlusAds.length) return `<div class="codex-plus-ad-empty">暂无推荐内容。</div>`;
    // 合作伙伴用同一套原生列表节奏展示，文案与外链沿用后端数据。
    return `<div class="codex-plus-ad-list">${renderCodexPlusAdGroup()}</div>`;
  }

  function cacheBustCodexPlusAdUrl(url, version) {
    return `${url}${url.includes("?") ? "&" : "?"}v=${version}`;
  }

  async function directFetchCodexPlusAds() {
    const urls = [
      "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
      "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json",
    ];
    let lastError = null;
    const cacheBust = Date.now();
    for (const url of urls) {
      try {
        const response = await fetch(cacheBustCodexPlusAdUrl(url, cacheBust), {
          headers: { "Accept": "application/json" },
          cache: "no-store",
        });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return await response.json();
      } catch (error) {
        lastError = error;
      }
    }
    throw lastError || new Error("ad list unavailable");
  }

  async function fetchCodexPlusAds() {
    try {
      const localPayload = await postJson(codexPlusAdsUrl, {});
      codexPlusAds = normalizeCodexPlusAds(localPayload?.ads ? localPayload : localPayload?.payload);
      if (!codexPlusAds.length) codexPlusAds = normalizeCodexPlusAds(await directFetchCodexPlusAds());
    } catch (error) {
      sendCodexPlusDiagnostic("ads_fetch_failed", {
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
      codexPlusAds = [];
    } finally {
      codexPlusAdsLoaded = true;
      const panel = document.querySelector('[data-codex-plus-panel="sponsor"] .codex-plus-ad-remote');
      if (panel) panel.innerHTML = renderCodexPlusAds();
    }
  }

  function selectCodexPlusTab(tab) {
    closeCodexPlusTypingEffectDropdown();
    // 归一化后再比对：panel 用的是 extensions，而旧调用点仍传 userScripts，
    // 不统一就会两边都对不上、所有 panel 全被隐藏。
    const normalized = codexPlusModalTab(tab);
    document.querySelectorAll(".codex-plus-modal-content").forEach((modal) => {
      modal.dataset.codexPlusActiveTab = normalized;
    });
    document.querySelectorAll("[data-codex-plus-panel]").forEach((panel) => {
      panel.hidden = codexPlusModalTab(panel.getAttribute("data-codex-plus-panel")) !== normalized;
    });
    if (normalized === codexPlusExtensionsTab) {
      loadUserScripts();
      // 市场清单拉过一次就缓存，切换分组不再重复请求；失败后可从界面手动刷新。
      void loadScriptMarket();
    }
    refreshCodexPlusPageNav(normalized);
  }

  /** 两个 rail 入口各自对应一个页面，激活态要分别判断，不能只看页面开着没有。 */
  function codexPlusActiveEntry() {
    const overlay = document.querySelector(`.${codexPlusPageClass}`);
    if (!overlay) return null;
    const tab = overlay.querySelector(".codex-plus-modal-content")?.dataset?.codexPlusActiveTab;
    if (tab === codexPlusExtensionsTab) return "extensions";
    if (tab === codexPlusSponsorTab) return "sponsor";
    if (tab === codexPlusPluginMarketTab) return "plugin-market";
    return "home";
  }

  function setCodexPlusSidebarNavActive(active, entry = "home") {
    const nav = document.getElementById(codexPlusSidebarNavId);
    const button = nav?.querySelector("button");
    if (button) {
      const on = Boolean(active) && entry === "home";
      button.dataset.active = String(on);
      button.setAttribute("aria-current", on ? "page" : "false");
    }
    const pluginButton = document.getElementById(codexPlusSidebarPluginMarketId)?.querySelector("button");
    if (pluginButton) {
      const on = Boolean(active) && entry === "plugin-market";
      pluginButton.dataset.active = String(on);
      pluginButton.setAttribute("aria-current", on ? "page" : "false");
    }
    [
      [codexPlusRailNavId, "home"],
      [codexPlusRailExtensionsId, "extensions"],
      [codexPlusRailSponsorId, "sponsor"],
      [codexPlusRailPluginMarketId, "plugin-market"],
    ].forEach(([id, name]) => {
      const railButton = document.querySelector(`#${id} > button`);
      if (!railButton) return;
      const on = Boolean(active) && entry === name;
      railButton.dataset.active = String(on);
      railButton.setAttribute("aria-current", on ? "page" : "false");
      // 原生 rail 按钮的选中色由 data-selected 驱动（且需无 data-suppress-active-style）。
      if (on) railButton.setAttribute("data-selected", "");
      else railButton.removeAttribute("data-selected");
    });
    syncCodexPlusRailNativeSelection();
  }

  /**
   * 我们的页面是叠加在 Codex 上的，Codex 不知道，所以它自己那个 destination
   * 的选中态会一直留着，表现为 rail 上同时亮两个。页面打开时给根节点打标记，
   * 由 CSS 把原生选中项压成未选中；关掉即移除标记，原生状态自动恢复。
   *
   * 未选中的颜色取自当前主题下真实的未选中项，避免把深浅色写死。
   */
  function syncCodexPlusRailNativeSelection() {
    const root = document.documentElement;
    if (!root) return;
    if (!document.querySelector(`.${codexPlusPageClass}`)) {
      root.removeAttribute("data-codex-plus-page-open");
      root.style.removeProperty("--codex-plus-rail-dim");
      return;
    }
    const rail = document.querySelector(codexPlusRailSelector);
    const unselected = rail?.querySelector(`${codexPlusRailDestinationSelector}:not([aria-current="page"])`);
    const dim = unselected ? getComputedStyle(unselected).color : "";
    if (dim) root.style.setProperty("--codex-plus-rail-dim", dim);
    root.setAttribute("data-codex-plus-page-open", "");
  }

  function positionCodexPlusPage(overlay) {
    if (!overlay?.classList?.contains(codexPlusPageClass)) return;
    const sidebar = document.querySelector("aside.app-shell-left-panel");
    const rect = sidebar?.getBoundingClientRect?.();
    const rail = document.querySelector(codexPlusRailSelector);
    const railRect = rail?.getBoundingClientRect?.();
    // 新版：页面要顶替原生侧边栏——从图标栏右边界起铺满，把宽面板整个盖住，
    // 而不是并排在其右侧多出一列（那样会变成「图标栏 + 会话列表 + 我们的面板」三段）。
    // 旧版没有图标栏，我们的入口就在 aside 内部，此时退回 aside 右边界。
    const left = railRect && railRect.width > 0
      ? Math.max(0, railRect.right)
      : (rect && rect.width > 0 ? Math.max(0, rect.right) : 0);
    // 量出来的是视觉坐标，而 overlay 在缩放空间里布局，所以统一折算成布局坐标。
    // CSS 的 calc(100vw / zoom - left) 用的也是这个空间的量，两边才配得上。
    const zoom = codexPlusWindowZoom();
    const layoutLeft = zoom === 1 ? left : left / zoom;
    overlay.style.setProperty("--codex-plus-page-left", `${layoutLeft}px`);
    overlay.style.left = `${layoutLeft}px`;
    // 官方顶部有一条 header（返回/前进/隐藏侧边栏），图标栏与侧边栏都从它的下沿开始。
    // 我们的 overlay 若从 y=0 铺满就会把整条 header 盖住——用户反馈「比官方少了一条顶部栏」
    // 就是这个原因。这里同样量图标栏的顶边（而非硬编码高度），让 overlay 从 header 下沿开始。
    const top = railRect && railRect.height > 0
      ? Math.max(0, railRect.top)
      : (rect && rect.height > 0 ? Math.max(0, rect.top) : 0);
    const layoutTop = zoom === 1 ? top : top / zoom;
    overlay.style.setProperty("--codex-plus-page-top", `${layoutTop}px`);
    overlay.style.top = `${layoutTop}px`;
    // 右侧与下方官方各留了一圈槽：整行的 [data-app-shell-workspace-row] 比视口小
    // （真机 1715x984 / 视口 1719x988，即右、下各 4px），官方内容面板正好收在行的右下角。
    // 我们原先 right/bottom 都贴 0，于是比官方多占这 4px。这里量取而不是硬编码 4。
    const row = document.querySelector("[data-app-shell-workspace-row]");
    const rowRect = row?.getBoundingClientRect?.();
    const rightGutter = rowRect && rowRect.width > 0 ? Math.max(0, window.innerWidth - rowRect.right) : 0;
    const bottomGutter = rowRect && rowRect.height > 0 ? Math.max(0, window.innerHeight - rowRect.bottom) : 0;
    const layoutRight = zoom === 1 ? rightGutter : rightGutter / zoom;
    const layoutBottom = zoom === 1 ? bottomGutter : bottomGutter / zoom;
    overlay.style.setProperty("--codex-plus-page-right", `${layoutRight}px`);
    overlay.style.setProperty("--codex-plus-page-bottom", `${layoutBottom}px`);
    overlay.style.right = `${layoutRight}px`;
    overlay.style.bottom = `${layoutBottom}px`;
    // 圆角同样量取官方面板自身的值，不写死 12px。
    // 这里量的是 _PageSurface_：官方那个与我们 overlay 同格子的页面面板（rect 都是
    // [52, 44, 1663, 940]），它四角同为 12px，左侧那一角也真实可见——真机像素扫描确认
    // 官方左边缘从 y=44 的 x=62 收到 y=54 的 x=52，是一条完整的弧。
    // 别改用 main[data-app-shell-main-surface] 的 --app-shell-main-surface-clip-start-radius：
    // 那个元素左边缘在 362（缩在侧边栏后面），左侧还用 inset 负内缩把圆角裁掉，start 恒为 0，
    // 会让人误判左侧不该圆——第一版就是这么写错的，用户反馈「少一个圆角」正是缺了左边两个角。
    // 类名是 CSS Modules 的哈希名，但 _PageSurface_ 这个片段稳定，且不会命中 _PageSurfaceLayout_
    //（其后紧跟 L 而非 _）。量到的是视觉值，同样折算成布局坐标。读不到就保持 0，不做猜测。
    const pageSurface = document.querySelector('[class*="_PageSurface_"]');
    const pageSurfaceStyle = pageSurface ? getComputedStyle(pageSurface) : null;
    const mainSurfaceStyle = (() => {
      const main = document.querySelector("main[data-app-shell-main-surface]");
      return main ? getComputedStyle(main) : null;
    })();
    const radius =
      parseFloat(pageSurfaceStyle?.borderTopLeftRadius || "") ||
      parseFloat(mainSurfaceStyle?.getPropertyValue("--app-shell-main-surface-clip-end-radius") || "") ||
      0;
    const layoutRadius = zoom === 1 ? radius : radius / zoom;
    overlay.style.setProperty("--codex-plus-page-radius", `${layoutRadius}px`);
  }

  function codexPlusHostUsesLightTheme() {
    const root = document.documentElement;
    const body = document.body;
    const explicitTheme = [
      root?.getAttribute("data-theme"),
      body?.getAttribute("data-theme"),
      root?.getAttribute("data-color-scheme"),
      body?.getAttribute("data-color-scheme"),
    ].filter(Boolean).join(" ").toLowerCase();
    if (/\b(light|light-mode|theme-light)\b/.test(explicitTheme)) return true;
    if (/\b(dark|dark-mode|theme-dark)\b/.test(explicitTheme)) return false;

    const themeClasses = [root?.className, body?.className]
      .filter((value) => typeof value === "string")
      .join(" ")
      .toLowerCase();
    if (/(^|\s)(light|light-mode|theme-light)(\s|$)/.test(themeClasses)) return true;
    if (/(^|\s)(dark|dark-mode|theme-dark)(\s|$)/.test(themeClasses)) return false;

    return !window.matchMedia?.("(prefers-color-scheme: dark)")?.matches;
  }

  /**
   * 取 Codex 当前的界面缩放系数。
   *
   * Codex 的界面缩放的实现是给内层布局节点设 CSS `zoom`（例如 1.2），而不是改
   * documentElement，所以固定在 body 下的 overlay 不会自动跟随。这里把它读出来，
   * 由 applyCodexPlusZoom 自己套上。
   */
  function codexPlusWindowZoom() {
    const raw = getComputedStyle(document.documentElement).getPropertyValue(codexPlusWindowZoomVar);
    const value = Number.parseFloat(raw);
    return Number.isFinite(value) && value > 0 ? value : 1;
  }

  /**
   * 让 overlay 跟随 Codex 的界面缩放，并保持满屏。
   *
   * 直接给 fixed 元素设 zoom 的话，它自身的 `inset: 0` / `100vw` 都还是按未缩放的
   * 视口算，再乘 zoom 就溢出（实测 zoom=1.2 时 100vw 得到 2072px，视口只有 1727）。
   * 所以 overlay 自身的尺寸改用 `calc(100vw / var(--codex-plus-zoom))` 抵消，
   * 内部子元素则用百分比——它们在缩放空间里，百分比本来就对。
   *
   * 字号不在这里动：由 CSS 跟随 zoom 自然放大，与原生行为一致。
   */
  function applyCodexPlusZoom(overlay) {
    if (!overlay?.style) return 1;
    const zoom = codexPlusWindowZoom();
    overlay.style.setProperty("--codex-plus-zoom", String(zoom));
    if (zoom === 1) {
      overlay.style.removeProperty("zoom");
      return 1;
    }
    overlay.style.setProperty("zoom", String(zoom));
    return zoom;
  }

  /**
   * 用 Codex 自己的语义令牌刷新 overlay 的变量。
   *
   * 早先这里内联写死了一套 zinc 调色板（深色正文 #f3f4f6、次要 #a1a1aa），
   * 内联优先级最高，把样式表里本来正确的令牌链整个盖掉了，表现为我们的文字
   * 比原生偏白偏冷。现在改成读宿主算好的值——取不到才回落到兜底色，
   * 这样浅色/深色主题都跟原生同源，也不会在 Codex 调色板变动后失配。
   */
  function applyCodexPlusTheme(overlay) {
    if (!overlay?.style) return;
    const light = codexPlusHostUsesLightTheme();
    const host = getComputedStyle(document.documentElement);
    const read = (names, fallback) => {
      for (const name of names) {
        const value = host.getPropertyValue(name).trim();
        if (value) return value;
      }
      return fallback;
    };
    const variables = {
      "--codex-plus-bg-primary": read(
        ["--color-token-bg-primary", "--token-bg-primary", "--app-color-background-surface"],
        light ? "#ffffff" : "#141414",
      ),
      "--codex-plus-bg-secondary": read(
        ["--color-token-bg-secondary", "--token-bg-secondary", "--color-surface-secondary"],
        light ? "#f7f7f7" : "#2f2f2f",
      ),
      "--codex-plus-bg-elevated": read(
        ["--color-token-dropdown-background", "--color-surface-elevated-secondary", "--color-token-bg-elevated-secondary"],
        light ? "#ffffff" : "#2f2f2f",
      ),
      // 设置页使用独立 surface，正文背景令牌在部分版本中比原生设置页更深。
      "--codex-plus-settings-page-bg": read(
        ["--color-background-surface", "--app-color-background-surface"],
        light ? "#ffffff" : "#181818",
      ),
      "--codex-plus-nav-bg": read(
        ["--color-token-sidebar-background", "--color-background-sidebar"],
        light ? "#f7f7f7" : "#1e1e1e",
      ),
      "--codex-plus-nav-selected-bg": read(
        ["--color-token-interactive-bg-secondary-selected", "--color-background-primary-soft-active"],
        light ? "#e9e9e9" : "#2f2f2f",
      ),
      "--codex-plus-settings-card-bg": read(
        ["--color-background-surface-secondary", "--color-background-surface-raised"],
        light ? "#f7f7f7" : "#232323",
      ),
      "--codex-plus-switch-on": read(
        ["--color-background-accent-solid", "--color-token-bg-accent"],
        "#3485ff",
      ),
      "--codex-plus-switch-off": read(
        ["--color-background-switch-off", "--color-background-secondary-solid"],
        light ? "#dedede" : "#3a3a3a",
      ),
      "--codex-plus-switch-thumb": read(
        ["--color-background-switch-thumb"],
        "#ffffff",
      ),
      "--codex-plus-bg-hover": read(
        ["--color-token-interactive-bg-secondary-hover", "--color-background-primary-soft-hover", "--token-list-hover-background"],
        light ? "rgba(0,0,0,.06)" : "rgba(255,255,255,.08)",
      ),
      "--codex-plus-bg-selected": read(
        ["--color-token-interactive-bg-secondary-selected", "--color-background-primary-soft-active"],
        light ? "rgba(0,0,0,.08)" : "rgba(255,255,255,.12)",
      ),
      "--codex-plus-text": read(
        ["--color-token-text-primary", "--color-text-primary", "--token-text-primary"],
        light ? "#171717" : "#dfdfdf",
      ),
      "--codex-plus-text-secondary": read(
        ["--color-token-text-secondary", "--color-text-secondary-solid", "--color-text-secondary"],
        light ? "#5d5d5d" : "rgba(255,255,255,.71)",
      ),
      "--codex-plus-text-tertiary": read(
        ["--color-token-text-tertiary", "--color-text-tertiary"],
        light ? "#8a8a8a" : "rgba(255,255,255,.498)",
      ),
      "--codex-plus-border": read(
        ["--color-token-border-default", "--color-token-border", "--color-border-primary-outline"],
        light ? "rgba(0,0,0,.12)" : "rgba(255,255,255,.084)",
      ),
      "--codex-plus-border-subtle": read(
        ["--color-token-border-subtle", "--color-border-disabled"],
        light ? "rgba(0,0,0,.08)" : "rgba(255,255,255,.06)",
      ),
      "--codex-plus-danger": read(
        ["--color-text-danger", "--color-token-text-error"],
        light ? "#dc2626" : "#ff6764",
      ),
      "--codex-plus-success": read(
        ["--color-text-success", "--app-color-text-success"],
        light ? "#15803d" : "#40c977",
      ),
      "--codex-plus-warning": read(
        ["--color-text-warning", "--color-text-caution-surface"],
        light ? "#a16207" : "#ffc300",
      ),
    };
    Object.entries(variables).forEach(([name, value]) => overlay.style.setProperty(name, value));
    overlay.dataset.codexPlusTheme = light ? "light" : "dark";
  }

  /**
   * 规范化页面/tab 名。
   *
   * 用户脚本从 Codex++ 弹窗里拆出来成了独立的「拓展」页面，
   * 这里把旧名 userScripts 也映射过去，避免存量调用点失效。
   */
  function codexPlusModalTab(tab) {
    if (tab === "extensions" || tab === "userScripts") return codexPlusExtensionsTab;
    if (tab === "sponsor") return "sponsor";
    return "home";
  }

  let codexPlusTypingEffectDropdownCleanup = null;

  /** 保留 select 的值与 change 契约，只替换原生系统菜单的呈现。 */
  function syncCodexPlusTypingEffectDropdown(select) {
    const overlay = select.closest(".codex-plus-modal-overlay, .codex-plus-page-overlay");
    const trigger = overlay?.querySelector("[data-codex-plus-typing-effect-trigger]");
    if (!trigger) return;
    trigger.disabled = select.disabled;
    const label = trigger.querySelector("[data-codex-plus-dropdown-label]");
    if (label) label.textContent = select.selectedOptions[0]?.textContent || "关闭";
    overlay.querySelectorAll("[data-codex-plus-typing-effect-option]").forEach((option) => {
      const selected = option.dataset.codexPlusTypingEffectOption === select.value;
      option.setAttribute("aria-selected", String(selected));
      option.tabIndex = selected ? 0 : -1;
    });
    if (select.disabled) closeCodexPlusTypingEffectDropdown();
  }

  function closeCodexPlusTypingEffectDropdown(restoreFocus = false) {
    const menu = document.querySelector("[data-codex-plus-typing-effect-menu]");
    const trigger = menu?.parentElement?.querySelector("[data-codex-plus-typing-effect-trigger]");
    if (menu) menu.hidden = true;
    trigger?.setAttribute("aria-expanded", "false");
    codexPlusTypingEffectDropdownCleanup?.();
    codexPlusTypingEffectDropdownCleanup = null;
    if (restoreFocus && trigger?.isConnected && !trigger.disabled) trigger.focus();
  }

  function openCodexPlusTypingEffectDropdown(overlay, last = false) {
    const select = overlay.querySelector("[data-codex-plus-typing-effect]");
    const trigger = overlay.querySelector("[data-codex-plus-typing-effect-trigger]");
    const menu = overlay.querySelector("[data-codex-plus-typing-effect-menu]");
    if (!select || select.disabled || !trigger || !menu) return;
    closeCodexPlusTypingEffectDropdown();
    syncCodexPlusTypingEffectDropdown(select);
    menu.hidden = false;
    trigger.setAttribute("aria-expanded", "true");
    // 菜单直接挂 overlay，避开滚动内容的裁剪；坐标换算与页面缩放保持同源。
    const frame = overlay.getBoundingClientRect();
    const anchor = trigger.getBoundingClientRect();
    const zoom = codexPlusWindowZoom();
    const width = frame.width / zoom;
    const height = frame.height / zoom;
    menu.style.maxHeight = `${Math.max(0, height - 16)}px`;
    const left = (anchor.right - frame.left) / zoom - menu.offsetWidth;
    const below = (anchor.bottom - frame.top) / zoom + 4;
    const above = (anchor.top - frame.top) / zoom - menu.offsetHeight - 4;
    const top = below + menu.offsetHeight > height - 8 && above >= 8 ? above : below;
    menu.style.left = `${Math.max(8, Math.min(left, width - menu.offsetWidth - 8))}px`;
    menu.style.top = `${Math.max(8, Math.min(top, height - menu.offsetHeight - 8))}px`;
    const options = Array.from(menu.querySelectorAll("[role=option]"));
    const focused = last ? options[options.length - 1] : options.find((option) => option.getAttribute("aria-selected") === "true") || options[0];
    focused?.focus({ preventScroll: true });
    const controller = new AbortController();
    const close = () => closeCodexPlusTypingEffectDropdown();
    document.addEventListener("pointerdown", (event) => {
      if (!menu.contains(event.target) && !trigger.contains(event.target)) close();
    }, { capture: true, signal: controller.signal });
    overlay.addEventListener("scroll", (event) => {
      if (!menu.contains(event.target)) close();
    }, { capture: true, signal: controller.signal });
    window.addEventListener("resize", close, { signal: controller.signal });
    window.addEventListener("blur", close, { signal: controller.signal });
    const observer = new MutationObserver(() => {
      if (!overlay.isConnected) close();
    });
    observer.observe(document.body, { childList: true });
    codexPlusTypingEffectDropdownCleanup = () => {
      controller.abort();
      observer.disconnect();
    };
  }

  function openCodexPlusModal(options = {}) {
    const pageMode = options.page === true;
    const initialTab = codexPlusModalTab(options.tab);
    closeCodexPlusTypingEffectDropdown();
    document.querySelectorAll(".codex-plus-modal-overlay").forEach((node) => node.remove());
    document.querySelectorAll(`.${codexPlusPageClass}, [data-codex-plus-dialog="true"]`).forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = pageMode ? codexPlusPageClass : "codex-plus-modal-overlay";
    overlay.dataset.codexPlusPage = String(pageMode);
    applyCodexPlusTheme(overlay);
    // 跟随 Codex 的界面缩放。必须在写 innerHTML 之前设好，否则内部那些
    // calc(100% / var(--codex-plus-zoom-inverse)) 会先按 1 算一遍再被 zoom 放大。
    applyCodexPlusZoom(overlay);
    // 下拉箭头与勾选图标使用已安装的 Lucide ChevronDown / Check（ISC）路径。
    overlay.innerHTML = `
      <div class="codex-plus-modal-content" role="dialog" aria-modal="true" aria-label="Codex++">
        <div class="codex-plus-modal-header">
          <div class="codex-plus-modal-title"><span class="codex-plus-backend-indicator" data-codex-backend-indicator="true" data-status="checking"></span><span data-codex-plus-version="true">Codex++ ${codexPlusVersion}</span></div>
          ${pageMode ? "" : `<button type="button" class="codex-plus-modal-close" aria-label="关闭">×</button>`}
        </div>
        <div class="codex-plus-modal-body">
          <div class="codex-plus-panel codex-plus-settings" data-codex-plus-panel="home">
            <section class="codex-plus-settings-section" aria-label="常规">
              <h2 class="codex-plus-settings-section-title">常规</h2>
              <div class="codex-plus-settings-card">
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">后端连接</div><div class="codex-plus-row-description">每 5 秒检查一次 launcher 后端状态。</div></div>
                  <div class="codex-plus-backend-status">
                    <div class="codex-plus-backend-label" data-codex-backend-status="true" data-status="checking">正在检查后端…</div>
                  </div>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">Codex增强</div><div class="codex-plus-row-description">关闭后停用删除、导出、插件相关和菜单位置增强。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-backend-setting="enhancementsEnabled"><span></span></button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">插件市场解锁</div><div class="codex-plus-row-description">扩展插件市场请求，尽量显示完整插件列表。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="pluginMarketplaceUnlock"><span></span></button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">模型白名单解锁</div><div class="codex-plus-row-description">从环境变量和 Codex config.toml 中的中转站 /v1/models 拉取模型，并补进模型选择列表。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="modelWhitelistUnlock"><span></span></button>
                </div>
              </div>
            </section>
            <section class="codex-plus-settings-section" aria-label="模型与服务">
              <h2 class="codex-plus-settings-section-title">模型与服务</h2>
              <div class="codex-plus-settings-card">
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">Fast 按钮</div><div class="codex-plus-row-description">显示服务模式切换按钮；Fast 仅支持 ${codexServiceTierFastModelListLabel()}，其他模型按 Standard 发送。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="serviceTierControls"><span></span></button>
                </div>
                <div class="codex-plus-row" data-codex-service-tier-controls="true">
                  <div><div class="codex-plus-row-title">服务模式</div><div class="codex-plus-row-description">继承优先读取 Codex 应用内设置，其次读取 config.toml 的 service_tier；全局模式覆盖全部 thread；自定义允许按 thread 覆盖。</div></div>
                  <div class="codex-plus-service-tier-control">
                    <div class="codex-plus-service-tier-status" data-codex-service-tier-status="true" data-status="loading">正在读取…</div>
                    <div class="codex-plus-service-tier-actions">
                      <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-inherit="true">继承</button>
                      <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-standard="true">全局 Standard</button>
                      <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-fast="true">全局 Fast</button>
                      <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-custom="true">自定义</button>
                    </div>
                    <div class="codex-plus-service-tier-actions codex-plus-service-tier-thread-actions">
                      <span class="codex-plus-service-tier-thread-label">当前 thread 覆盖</span>
                      <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-thread-inherit="true" title="当前 thread 不单独覆盖，继承 Codex 默认设置">继承</button>
                      <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-thread-standard="true" title="仅当前 thread 使用 Standard，并切到自定义模式">Standard</button>
                      <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-thread-fast="true" title="仅当前 thread 使用 Fast，并切到自定义模式">Fast</button>
                    </div>
                  </div>
                </div>
              </div>
            </section>
            <section class="codex-plus-settings-section" aria-label="对话与输入">
              <h2 class="codex-plus-settings-section-title">对话与输入</h2>
              <div class="codex-plus-settings-card">
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">悬浮球 · 下一步建议</div><div class="codex-plus-row-description">生成下一步建议。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="stepwise"><span></span></button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">悬浮球 · 回答大纲</div><div class="codex-plus-row-description">整理回答结构。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="answerOutline"><span></span></button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">会话删除</div><div class="codex-plus-row-description">在会话列表悬停显示删除按钮，并支持撤销。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="sessionDelete"><span></span></button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">Markdown 导出</div><div class="codex-plus-row-description">在会话列表显示导出按钮，按本地 rollout 导出带时间戳的 Markdown。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="markdownExport"><span></span></button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">粘贴修复</div><div class="codex-plus-row-description">从 Word 等富文本来源粘贴到 Codex composer 时只保留纯文本，避免被识别为图片/文件附件。需重启 Codex 才生效。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="pasteFix"><span></span></button>
                </div>
              </div>
            </section>
            <section class="codex-plus-settings-section" aria-label="外观与布局">
              <h2 class="codex-plus-settings-section-title">外观与布局</h2>
              <div class="codex-plus-settings-card">
                ${codexPlusIsWindowsPlatform ? `<div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">桌宠跟随真实鼠标</div><div class="codex-plus-row-description">仅支持 V2 桌宠；不会修改宠物文件。将 V2 的 Computer Use 光标朝向动作映射到真实鼠标，V1 开启后安全不生效；拖拽、原生悬停或 Computer Use 活跃时自动让步。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="petRealMouseLook"><span></span></button>
                </div>` : ""}
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">打字特效（彩虹 P）</div><div class="codex-plus-row-description">在输入框的光标附近显示彩虹粒子、烟花或星光。系统减少动态效果时暂停。</div></div>
                  <select class="codex-plus-typing-effect-select" data-codex-plus-typing-effect="true" aria-label="打字特效（彩虹 P）" hidden aria-hidden="true" tabindex="-1">
                    <option value="off">关闭</option>
                    <option value="rainbow">彩虹粒子</option>
                    <option value="fireworks">烟花</option>
                    <option value="stars">星光</option>
                  </select>
                  <button type="button" class="codex-plus-dropdown-trigger" data-codex-plus-typing-effect-trigger="true" aria-label="打字特效（彩虹 P）" aria-haspopup="listbox" aria-expanded="false" aria-controls="codex-plus-typing-effect-menu">
                    <span data-codex-plus-dropdown-label="true">关闭</span>
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
                  </button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">会话 ID 标识</div><div class="codex-plus-row-description">在侧边栏会话标题前显示短 ID 和 UUIDv7 创建时间，方便定位历史会话。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="threadIdBadge"><span></span></button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">对话居中宽度</div><div class="codex-plus-row-description">开启后把主对话和输入框限制到固定最大宽度，适合大屏阅读。</div></div>
                  <div class="codex-plus-width-control">
                    <input class="codex-plus-width-input" data-codex-plus-conversation-view-width="true" min="${conversationViewMinWidth}" max="${conversationViewMaxAllowedWidth}" step="10" type="number" value="${conversationViewWidth()}">
                    <button type="button" class="codex-plus-toggle" data-codex-plus-setting="conversationView"><span></span></button>
                  </div>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">切换对话保留位置</div><div class="codex-plus-row-description">开启后在不同 thread 之间切换时恢复到上一次浏览位置，不再自动跳到底部。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-plus-setting="threadScrollRestore"><span></span></button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">自定义布局</div><div class="codex-plus-row-description">侧边图标与会话栏作为整体移动；拖动侧边栏、任务信息卡或输入框时，其他面板会弹性让位；支持磁吸和下次打开恢复。</div></div>
                  <div class="codex-plus-width-control">
                    <button type="button" class="codex-plus-action-button" data-codex-plus-layout-edit="true">编辑布局</button>
                    <button type="button" class="codex-plus-toggle" data-codex-plus-setting="customLayout"><span></span></button>
                  </div>
                </div>
              </div>
            </section>
            <section class="codex-plus-settings-section" aria-label="工具与关于">
              <h2 class="codex-plus-settings-section-title">工具与关于</h2>
              <div class="codex-plus-settings-card">
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">历史会话修复</div><div class="codex-plus-row-description">切换官方登录、混合 API 或纯 API 后，让旧对话重新显示在当前模式下。</div></div>
                  <button type="button" class="codex-plus-toggle" data-codex-backend-setting="providerSyncEnabled"><span></span></button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">管理工具</div><div class="codex-plus-row-description">配置增强功能、模型和语音服务。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-open-manager="true">打开管理工具</button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">打开 DevTools</div><div class="codex-plus-row-description">打开当前 Codex 页面开发者工具，方便查看用户拓展报错。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-open-devtools="true">打开 DevTools</button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">关于 Codex++</div><div class="codex-plus-about">Codex++ 是通过外部 launcher 注入的增强菜单，不修改 Codex App 原始安装文件。<br>Build: <span data-codex-plus-build="true">${codexPlusBuild}</span><br>GitHub: <a href="https://github.com/BigPizzaV3/CodexPlusPlus" target="_blank" rel="noreferrer">https://github.com/BigPizzaV3/CodexPlusPlus</a><br>Discord: <a href="https://discord.gg/y96kX7A76v" target="_blank" rel="noreferrer">https://discord.gg/y96kX7A76v</a><br>Telegram: <a href="https://t.me/CodexPlusPlus" target="_blank" rel="noreferrer">https://t.me/CodexPlusPlus</a></div></div>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">Discord 社区</div><div class="codex-plus-row-description">加入 Discord 获取更新消息、反馈问题或交流使用体验。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-plus-discord="true">打开 Discord</button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">Telegram 频道</div><div class="codex-plus-row-description">加入 Telegram 获取更新消息和交流使用体验。</div></div>
                  <button type="button" class="codex-plus-action-button" data-codex-plus-telegram="true">打开 Telegram</button>
                </div>
                <div class="codex-plus-row">
                  <div><div class="codex-plus-row-title">提出问题</div><div class="codex-plus-row-description">打开 GitHub Issues 反馈问题或建议。</div></div>
                  <button type="button" class="codex-plus-issue-button" data-codex-plus-issue="true">提出问题</button>
                </div>
              </div>
            </section>
            ${renderCodexPlusExtensionMenuRows()}
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="${codexPlusExtensionsTab}" hidden>
            <div class="codex-plus-discovery codex-plus-extensions-page">
              <div class="codex-plus-discovery-header">
                <div><h1 class="codex-plus-discovery-title">拓展</h1><p class="codex-plus-discovery-description">为 Codex 添加更多增强功能。</p></div>
                <div class="codex-plus-discovery-toolbar">
                  <label class="codex-plus-discovery-search">${codexPlusDiscoveryIcons.search}<input type="search" data-codex-extensions-search="true" aria-label="搜索拓展" placeholder="搜索拓展" value="${escapeHtml(codexPlusExtensionsQuery)}" spellcheck="false" /></label>
                  <button type="button" class="codex-plus-discovery-icon-button" data-codex-market-refresh="true" title="刷新拓展" aria-label="刷新拓展">${codexPlusDiscoveryIcons.refresh}</button>
                </div>
              </div>
              <div class="codex-plus-extensions-detail" data-codex-plus-extensions-detail="true">${renderCodexPlusExtensionsDetail()}</div>
            </div>
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="sponsor" hidden>
            <div class="codex-plus-discovery codex-plus-sponsor-page">
              <div class="codex-plus-discovery-header">
                <div><h1 class="codex-plus-discovery-title">推荐内容</h1><p class="codex-plus-discovery-description codex-plus-sponsor-text">以下推荐来自支持 Codex++ 继续维护的合作方。</p></div>
              </div>
              <h2 class="codex-plus-discovery-section-title">合作伙伴</h2>
              <div class="codex-plus-ad-remote">${renderCodexPlusAds()}</div>
            </div>
          </div>
        </div>
      </div>
      <div id="codex-plus-typing-effect-menu" class="codex-plus-dropdown-menu" data-codex-plus-typing-effect-menu="true" role="listbox" aria-label="打字特效（彩虹 P）" hidden>
        ${[["off", "关闭"], ["rainbow", "彩虹粒子"], ["fireworks", "烟花"], ["stars", "星光"]].map(([value, label]) => `
          <button type="button" class="codex-plus-dropdown-option" data-codex-plus-typing-effect-option="${value}" role="option" aria-selected="false" tabindex="-1">
            <span>${label}</span><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>
          </button>
        `).join("")}
      </div>
    `;
    overlay.querySelectorAll(".codex-plus-toggle[data-codex-plus-setting], .codex-plus-toggle[data-codex-backend-setting]").forEach((button) => {
      button.setAttribute("role", "switch");
      button.setAttribute("aria-checked", "false");
      button.setAttribute("aria-label", button.closest(".codex-plus-row")?.querySelector(".codex-plus-row-title")?.textContent || "设置");
    });
    const closeButton = overlay.querySelector(".codex-plus-modal-close");
    closeButton?.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      overlay.remove();
      if (pageMode) setCodexPlusSidebarNavActive(false);
    }, true);
    overlay.addEventListener("input", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      const searchInput = target?.closest("[data-codex-extensions-search]");
      if (searchInput) {
        codexPlusExtensionsQuery = searchInput.value;
        codexPlusExtensionsSelected = null;
        refreshCodexPlusExtensionsView();
        return;
      }
      const widthInput = target?.closest("[data-codex-plus-conversation-view-width]");
      if (widthInput) setConversationViewWidth(widthInput.value);
    }, true);
    overlay.addEventListener("change", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      const typingEffectSelect = target?.closest("[data-codex-plus-typing-effect]");
      if (typingEffectSelect) {
        if (!typingEffectSelect.disabled && ["off", "rainbow", "fireworks", "stars"].includes(typingEffectSelect.value)) {
          setCodexPlusSetting("typingEffect", typingEffectSelect.value);
        }
        return;
      }
      const widthInput = target?.closest("[data-codex-plus-conversation-view-width]");
      if (widthInput) {
        const width = normalizeConversationViewWidth(widthInput.value);
        widthInput.value = String(width || conversationViewWidth());
        setConversationViewWidth(widthInput.value);
      }
    }, true);
    overlay.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      // 拓展注册的菜单项。放在最前面是因为它的判定完全基于自己的 data 属性，
      // 与下面那些内置分支不会重叠；万一将来重叠，也应当由拓展优先拿到。
      if (handleCodexPlusExtensionMenuClick(target)) return;
      const typingEffectTrigger = target?.closest("[data-codex-plus-typing-effect-trigger]");
      if (typingEffectTrigger) {
        const menu = overlay.querySelector("[data-codex-plus-typing-effect-menu]");
        if (menu?.hidden) openCodexPlusTypingEffectDropdown(overlay);
        else closeCodexPlusTypingEffectDropdown();
        return;
      }
      const typingEffectOption = target?.closest("[data-codex-plus-typing-effect-option]");
      if (typingEffectOption) {
        const select = overlay.querySelector("[data-codex-plus-typing-effect]");
        if (select && !select.disabled) {
          const value = typingEffectOption.dataset.codexPlusTypingEffectOption;
          if (select.value !== value) {
            select.value = value;
            select.dispatchEvent(new Event("change", { bubbles: true }));
          }
        }
        closeCodexPlusTypingEffectDropdown(true);
        return;
      }
      const extensionsBrowse = target?.closest("[data-codex-extensions-browse]");
      if (extensionsBrowse) {
        codexPlusExtensionsFilter = extensionsBrowse.dataset.codexExtensionsBrowse === "installed" ? "installed" : "market";
        codexPlusExtensionsSelected = null;
        refreshCodexPlusExtensionsView();
        return;
      }
      // 左面板的分组导航（仅拓展页有左面板）。
      const pageNav = target?.closest("[data-codex-plus-page-nav]");
      if (pageNav) {
        selectCodexPlusTab(pageNav.getAttribute("data-codex-plus-page-nav"));
        return;
      }
      if (target?.closest("[data-codex-open-devtools]")) {
        postJson("/devtools/open", {});
        return;
      }
      if (target?.closest("[data-codex-open-manager]")) {
        openManagerFromCodex();
        return;
      }
      const layoutEdit = target?.closest("[data-codex-plus-layout-edit]");
      if (layoutEdit) {
        if (layoutEdit.disabled || codexPlusSettings().customLayout !== true) return;
        installCodexPlusCustomLayout();
        closeCodexPlusPage();
        document.querySelectorAll(".codex-plus-modal-overlay").forEach((node) => node.remove());
        window.__codexPlusCustomLayoutRuntime?.setEditing(true);
        return;
      }
      // 推荐卡片用 window.open 而非原生 <a target="_blank">：Codex 是 Electron
      // 应用，原生新窗口跳转在它的 webview 里不会交给系统浏览器（同页的
      // Discord / Telegram / Issues 按钮也一律走 window.open）。
      const adCard = target?.closest("[data-codex-plus-ad-url]");
      if (adCard) {
        const adUrl = adCard.getAttribute("data-codex-plus-ad-url") || "";
        if (/^https?:\/\//i.test(adUrl)) {
          event.preventDefault();
          window.open(adUrl, "_blank", "noopener,noreferrer");
        }
        return;
      }
      if (target?.closest("[data-codex-plus-discord]")) {
        window.open("https://discord.gg/y96kX7A76v", "_blank");
        return;
      }
      if (target?.closest("[data-codex-plus-telegram]")) {
        window.open("https://t.me/CodexPlusPlus", "_blank");
        return;
      }
      const issueButton = target?.closest("[data-codex-plus-issue]");
      if (issueButton) {
        const issueUrl = "https://github.com/BigPizzaV3/CodexPlusPlus/issues";
        window.open(issueUrl, "_blank");
        return;
      }
      if (target?.closest("[data-codex-service-tier-inherit]")) {
        setCodexServiceTierControlMode("inherit");
        return;
      }
      if (target?.closest("[data-codex-service-tier-standard]")) {
        setCodexServiceTierControlMode("global-standard");
        return;
      }
      if (target?.closest("[data-codex-service-tier-fast]")) {
        setCodexServiceTierControlMode("global-fast");
        return;
      }
      if (target?.closest("[data-codex-service-tier-custom]")) {
        setCodexServiceTierControlMode("custom");
        return;
      }
      if (target?.closest("[data-codex-service-tier-thread-inherit]")) {
        setCodexThreadServiceTierMode("inherit");
        return;
      }
      if (target?.closest("[data-codex-service-tier-thread-standard]")) {
        setCodexThreadServiceTierMode("standard");
        return;
      }
      if (target?.closest("[data-codex-service-tier-thread-fast]")) {
        setCodexThreadServiceTierMode("fast");
        return;
      }
      const userScriptToggle = target?.closest("[data-codex-user-script-key]");
      if (userScriptToggle) {
        loadUserScripts("/user-scripts/set-script-enabled", { key: userScriptToggle.getAttribute("data-codex-user-script-key"), enabled: userScriptToggle.dataset.enabled !== "true" });
        return;
      }
      // 市场条目的「安装」。id 挂在行/按钮上，点按钮才算。
      const marketInstall = target?.closest("[data-codex-market-install]");
      if (marketInstall) {
        void installScriptFromMarket(marketInstall.getAttribute("data-codex-market-install"));
        return;
      }
      const extensionsRefresh = target?.closest("[data-codex-market-refresh]");
      if (extensionsRefresh) {
        void loadScriptMarket(true);
        return;
      }
      const extensionsUninstall = target?.closest("[data-codex-extensions-uninstall]");
      if (extensionsUninstall) {
        void uninstallUserScript(extensionsUninstall.getAttribute("data-codex-extensions-uninstall"));
        return;
      }
      // 左面板点行 = 选中并在右侧显示详情。放在安装/卸载之后，
      // 免得点了行内的按钮又被当成一次选中。
      const extensionsSelect = target?.closest("[data-codex-extensions-select]");
      if (extensionsSelect) {
        const [kind, ...rest] = extensionsSelect.getAttribute("data-codex-extensions-select").split(":");
        codexPlusExtensionsSelected = { kind, key: rest.join(":") };
        refreshCodexPlusExtensionsView();
        overlay.querySelector(".codex-plus-extensions-back")?.focus({ preventScroll: true });
        return;
      }
      const toggle = target?.closest("[data-codex-plus-setting]");
      if (toggle) {
        if (toggle.disabled || toggle.dataset.pending === "true") return;
        const key = toggle.getAttribute("data-codex-plus-setting");
        setCodexPlusSetting(key, !codexPlusSettings()[key]);
        return;
      }
      const backendToggle = target?.closest("[data-codex-backend-setting]");
      if (backendToggle) {
        const key = backendToggle.getAttribute("data-codex-backend-setting");
        setBackendSetting(key, !codexPlusBackendSettings[key]);
        return;
      }
    }, true);
    overlay.addEventListener("keydown", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      if (target?.closest("[data-codex-plus-typing-effect-trigger]") && ["ArrowDown", "ArrowUp"].includes(event.key)) {
        event.preventDefault();
        openCodexPlusTypingEffectDropdown(overlay, event.key === "ArrowUp");
        return;
      }
      const menu = target?.closest("[data-codex-plus-typing-effect-menu]");
      if (!menu || menu.hidden) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        closeCodexPlusTypingEffectDropdown(true);
      } else if (event.key === "Tab") {
        // 回到触发器后让浏览器处理 Tab，焦点会自然进入下一项而不会停在隐藏菜单里。
        closeCodexPlusTypingEffectDropdown(true);
      } else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        event.preventDefault();
        const options = Array.from(menu.querySelectorAll("[role=option]"));
        const current = options.indexOf(document.activeElement);
        const next = event.key === "Home" ? 0 : event.key === "End" ? options.length - 1
          : (current + (event.key === "ArrowDown" ? 1 : -1) + options.length) % options.length;
        options.forEach((option, index) => { option.tabIndex = index === next ? 0 : -1; });
        options[next]?.focus({ preventScroll: true });
      }
    }, true);
    // 图标加载失败的回退：error 不冒泡，只能捕获阶段委托。
    overlay.addEventListener("error", handleExtensionIconError, true);
    document.body.appendChild(overlay);
    if (pageMode) {
      positionCodexPlusPage(overlay);
      // 必须在 selectCodexPlusTab 之前建好两栏，否则刷新左面板时找不到容器。
      installCodexPlusPageLayout(overlay, initialTab);
      if (!window.__codexPlusPageResizeHandler) {
        window.__codexPlusPageResizeHandler = () => positionCodexPlusPage(document.querySelector(`.${codexPlusPageClass}`));
        window.addEventListener("resize", window.__codexPlusPageResizeHandler);
      }
    }
    if (!codexPlusAdsLoaded) fetchCodexPlusAds();
    selectCodexPlusTab(initialTab);
    // 必须在 selectCodexPlusTab 之后：激活态要靠 data-codex-plus-active-tab
    // 判断当前是 Codex++ 还是「拓展」，提前调用会永远落到 home 上。
    if (pageMode) setCodexPlusSidebarNavActive(true, codexPlusActiveEntry() || "home");
    renderCodexPlusMenu();
    refreshCodexPlusBackendToggles();
    renderBackendStatus();
    void loadCodexServiceTierState();
    loadUserScripts();
  }

  function openCodexPlusPage() {
    openCodexPlusModal({ page: true });
  }

  /** 「拓展」页面：从弹窗里拆出来的用户脚本，形态对齐 VSCode 的扩展面板。 */
  function openCodexPlusExtensions() {
    codexPlusExtensionsSelected = null;
    openCodexPlusModal({ page: true, tab: codexPlusExtensionsTab });
  }

  /** 「推荐内容」页面：从弹窗的二级 tab 提出来，成为图标栏上的一级入口。 */
  function openCodexPlusSponsor() {
    openCodexPlusModal({ page: true, tab: codexPlusSponsorTab });
  }

  function closeCodexPlusPage() {
    closeCodexPlusTypingEffectDropdown();
    document.querySelectorAll(`.${codexPlusPageClass}`).forEach((node) => node.remove());
    setCodexPlusSidebarNavActive(false);
  }

  function closeCodexPlusPageAfterNativeNavigation() {
    clearTimeout(window.__codexPlusPageNavigationCloseTimer);
    window.__codexPlusPageNavigationCloseTimer = setTimeout(() => {
      window.__codexPlusPageNavigationCloseTimer = null;
      closeCodexPlusPage();
    }, 0);
  }

  function installCodexPlusPageNavigationCloseHandler() {
    document.removeEventListener("click", window.__codexPlusPageNavigationCloseHandler, true);
    window.__codexPlusPageNavigationCloseHandler = (event) => {
      if (!document.querySelector(`.${codexPlusPageClass}`)) return;
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      if (!target?.closest(selectors.sidebarThread)) return;
      // Let Codex's own click handler update its route before removing our page.
      closeCodexPlusPageAfterNativeNavigation();
    };
    document.addEventListener("click", window.__codexPlusPageNavigationCloseHandler, true);
  }

  function installCodexPlusSidebarNavigation() {
    document.querySelectorAll(`#${codexPlusMenuId}, [data-codex-plus-menu="true"]`).forEach((node) => node.remove());
    // 旧版的侧边栏会话列表在 aside 里带 role="navigation"。新版把这个 role 挪去了
    // 缩略图面板/演示目录，所以留一条限定在 aside 内的兜底。
    // 注意：新版图标栏也是 aside 里的 <nav>，且文档顺序在前，而 querySelector 的选择器
    // 列表是按文档顺序取首个命中项的——必须显式排除图标栏，否则会挂到它上面。
    const navigation = document.querySelector('aside.app-shell-left-panel nav[role="navigation"]')
      || Array.from(document.querySelectorAll("aside.app-shell-left-panel nav"))
        .find((nav) => !nav.hasAttribute("data-app-navigation-rail"))
      || null;
    if (!navigation) return;
    const navButtons = Array.from(navigation.querySelectorAll("button"));
    const pluginButton = navButtons.find((button) => {
      if (button.querySelector(selectors.pluginSvgPath)) return true;
      const label = (button.getAttribute("aria-label") || button.textContent || "").trim();
      return /^(插件|Plugins)$/i.test(label);
    });
    const insertionButton = pluginButton || navButtons.find((button) => {
      const label = (button.getAttribute("aria-label") || button.textContent || "").replace(/\s+/g, " ").trim();
      return /^(已安排|Scheduled|拉取请求|Pull requests|新对话|New chat)$/i.test(label);
    });
    if (navigation.dataset.codexPlusSidebarNavigationListener !== "true") {
      navigation.dataset.codexPlusSidebarNavigationListener = "true";
      navigation.addEventListener("click", (event) => {
        const target = event.target instanceof Element ? event.target : event.target?.parentElement;
        if (target?.closest(`#${codexPlusSidebarNavId}, #${codexPlusSidebarPluginMarketId}`)) return;
        if (target?.closest("button, a")) closeCodexPlusPageAfterNativeNavigation();
      }, true);
    }
    let wrapper = document.getElementById(codexPlusSidebarNavId);
    const parent = insertionButton?.parentElement || navigation;
    if (!wrapper || wrapper.parentElement !== parent) {
      wrapper?.remove();
      wrapper = document.createElement("div");
      wrapper.id = codexPlusSidebarNavId;
      wrapper.dataset.codexPlusSidebarNav = "true";
      const button = (insertionButton || document.createElement("button")).cloneNode(true);
      if (!(button instanceof HTMLElement)) return;
      if (!button.className) button.className = "h-token-nav-row w-full flex items-center gap-2 px-3 py-2 text-sm";
      button.type = "button";
      button.removeAttribute("data-state");
      button.removeAttribute("aria-current");
      button.removeAttribute("disabled");
      button.removeAttribute("aria-disabled");
      button.setAttribute("aria-label", "Codex++");
      button.textContent = "";
      button.innerHTML = `<span class="codex-plus-sidebar-nav-icon" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v18M3 12h18M5.5 5.5l13 13M18.5 5.5l-13 13"/></svg></span><span class="truncate">Codex++</span><span class="codex-plus-sidebar-nav-status" data-status="${codexPlusBackendStatus.status || "checking"}" aria-hidden="true"></span>`;
      button.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        openCodexPlusPage();
      }, true);
      wrapper.appendChild(button);
      if (insertionButton?.nextSibling) {
        parent.insertBefore(wrapper, insertionButton.nextSibling);
      } else {
        parent.appendChild(wrapper);
      }
    }
    const status = wrapper.querySelector(".codex-plus-sidebar-nav-status");
    if (status) status.dataset.status = codexPlusBackendStatus.status || "checking";
    installCodexPlusPluginMarketSidebarNavigation(parent, wrapper, insertionButton);
    const active = !!document.querySelector(`.${codexPlusPageClass}`);
    setCodexPlusSidebarNavActive(active, codexPlusActiveEntry() || "home");
  }

  function removeCodexPlusRailNavigation() {
    [codexPlusRailNavId, codexPlusRailExtensionsId, codexPlusRailSponsorId, codexPlusRailPluginMarketId].forEach((id) => document.getElementById(id)?.remove());
  }

  function detachCodexPlusSidebarNavigation() {
    document.getElementById(codexPlusSidebarNavId)?.remove();
    document.getElementById(codexPlusSidebarPluginMarketId)?.remove();
  }

  /**
   * 挑一个原生 rail 按钮当模板。
   *
   * 优先 builtin:projects——它在 primary 区，且不像 builtin:library 那样会走
   * tooltip/triggerRef 的特殊分支。找不到就退回第一个可见 destination。
   */
  function codexPlusRailTemplateButton(rail) {
    const preferred = rail.querySelector(`${codexPlusRailDestinationSelector}[data-sidebar-destination="builtin:projects"]`);
    if (preferred) return preferred;
    const candidates = Array.from(rail.querySelectorAll(codexPlusRailDestinationSelector))
      .filter((node) => node.closest("nav") === rail);
    // 优先挑未选中的：clone 会把选中态的属性和配色一起带过来，
    // 表现为入口在没有任何页面打开时也显示成选中。
    const isSelected = (node) => node.getAttribute("aria-current") === "page" || node.hasAttribute("data-selected");
    return candidates.find((node) => !isSelected(node)) || candidates[0] || null;
  }

  function codexPlusRailPrimaryAnchor(rail) {
    const fixedIds = [
      'builtin:home',
      'builtin:customize',
    ];
    const buttons = Array.from(rail.querySelectorAll(codexPlusRailDestinationSelector));
    return buttons.find((node) => {
      const id = node.getAttribute("data-sidebar-destination") || "";
      return id && !fixedIds.includes(id);
    }) || null;
  }

  function createCodexPlusRailButton({ id, template, label, iconMarkup, withStatus, onActivate }) {
    const wrapper = document.createElement("div");
    wrapper.id = id;
    wrapper.dataset.codexPlusRail = id === codexPlusRailExtensionsId ? "extensions" : "home";
    // 模板拿不到时不回退到旧模式，而是自建一个按钮：rail 上 destination 可能在
    // 登录态/接口就绪前还是空的，那只是暂时状态，不该让入口整个消失。
    const button = template
      ? template.cloneNode(true)
      : document.createElement("button");
    if (!(button instanceof HTMLElement)) return null;
    button.type = "button";
    // 留着 data-sidebar-destination 会被 Codex 的自定义/排序逻辑当成真的 destination。
    button.removeAttribute("data-sidebar-destination");
    button.removeAttribute("data-state");
    button.removeAttribute("disabled");
    button.removeAttribute("aria-disabled");
    // 选中态由 data-selected 驱动，但它只在没有 data-suppress-active-style 时生效。
    // 模板若是未选中的按钮，会带着 suppress 过来，压制掉我们的选中样式——
    // 必须移除，否则按钮永远停在未选中的暗色。
    button.removeAttribute("data-suppress-active-style");
    // 起始为未选中；激活态由 setCodexPlusSidebarNavActive 切换 data-selected。
    button.removeAttribute("data-selected");
    button.removeAttribute("aria-current");
    button.setAttribute("aria-label", label);
    button.setAttribute("title", label);
    button.textContent = "";
    // 原生 rail 按钮是纯图标，没有文字标签，所以只放图标 + 状态点。
    button.innerHTML = `<span class="codex-plus-rail-icon" aria-hidden="true">${iconMarkup}</span>`
      + (withStatus
        ? `<span class="codex-plus-sidebar-nav-status" data-status="${codexPlusBackendStatus.status || "checking"}" aria-hidden="true"></span>`
        : "");
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      onActivate();
    }, true);
    wrapper.appendChild(button);
    return wrapper;
  }

  /**
   * 把 Codex++ / 拓展两个入口挂到新版图标栏。
   *
   * Codex 的 rail 渲染晚于注入，所以这里每次 scan 都会被调用；靠 id 判存避免重复插入。
   */
  function installCodexPlusRailNavigation() {
    document.querySelectorAll(`#${codexPlusMenuId}, [data-codex-plus-menu="true"]`).forEach((node) => node.remove());
    const rail = document.querySelector(codexPlusRailSelector);
    if (!rail) return false;
    // 注意：模板按钮可能在 rail 还没渲染出 destination 时拿不到（登录态/接口未就绪）。
    // 那只是暂时状态，不能因此判定"没有 rail"而回退旧模式，否则入口会整个消失。
    const template = codexPlusRailTemplateButton(rail);

    // 旧逻辑把"点原生导航就关掉 Codex++ 页面"的监听挂在侧边栏的 navigation 上，
    // 但 rail 模式下那个函数会提前 return，监听压根装不上，所以这里补一份。
    if (rail.dataset.codexPlusRailNavigationListener !== "true") {
      rail.dataset.codexPlusRailNavigationListener = "true";
      rail.addEventListener("click", (event) => {
        const target = event.target instanceof Element ? event.target : event.target?.parentElement;
        if (target?.closest(`#${codexPlusRailNavId}, #${codexPlusRailExtensionsId}, #${codexPlusRailSponsorId}, #${codexPlusRailPluginMarketId}`)) return;
        // 拓展入口的 id 是动态生成的，不在上面三个之内。不排除它，点拓展入口会被
        // 当成「点了原生导航按钮」，刚打开的拓展页面立刻被关掉。
        if (target?.closest(`[${codexPlusExtensionConstants.extensionAttribute}]`)) return;
        if (target?.closest("button, a")) closeCodexPlusPageAfterNativeNavigation();
      }, true);
    }

    const icons = {
      home: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v18M3 12h18M5.5 5.5l13 13M18.5 5.5l-13 13"/></svg>',
      // 「拓展」直接用 VSCode 的扩展字形（就是列表里默认图标那一份），
      // 和页面内部保持同一个符号，不再另画一个近似图形。
      extensions: `<svg viewBox="0 0 16 16" fill="currentColor"><path d="${codexPlusDefaultExtensionIconPath}"/></svg>`,
      // Lucide 的 megaphone：与 home 同一套 24 格线性风格，笔画宽度和端点也一致。
      sponsor: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m3 11 18-5v12L3 14v-3z"/><path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"/></svg>',
    };

    const specs = [
      { id: codexPlusRailNavId, label: "Codex++", iconMarkup: icons.home, withStatus: true, onActivate: openCodexPlusPage },
      { id: codexPlusRailExtensionsId, label: "拓展", iconMarkup: icons.extensions, withStatus: false, onActivate: openCodexPlusExtensions },
      { id: codexPlusRailPluginMarketId, label: "CodeX 插件市场", iconMarkup: codexPlusPluginMarketIconMarkup(), withStatus: false, onActivate: openCodexPlusNativePluginMarket },
      { id: codexPlusRailSponsorId, label: "推荐内容", iconMarkup: icons.sponsor, withStatus: false, onActivate: openCodexPlusSponsor },
    ];

    const anchor = codexPlusRailPrimaryAnchor(rail);
    // 插到锚点所在的父容器里，而不是 nav 顶层：原生按钮可能嵌在 nav 内部的分组 div 中，
    // 直接插顶层会破坏它的 flex 布局。
    const host = anchor?.parentElement || rail;
    let cursor = anchor;
    specs.forEach((spec) => {
      let wrapper = document.getElementById(spec.id);
      if (!wrapper || wrapper.parentElement !== host) {
        wrapper?.remove();
        wrapper = createCodexPlusRailButton({ ...spec, template });
        if (!wrapper) return;
        if (spec.id === codexPlusRailPluginMarketId) markCodexPlusExtensionNode(wrapper, "builtin-plugin-market");
      }
      // 顺序：Codex++ 在前，「拓展」在后；紧跟在 primary 区锚点后面。
      if (cursor?.nextSibling) {
        host.insertBefore(wrapper, cursor.nextSibling);
      } else if (cursor) {
        host.appendChild(wrapper);
      } else {
        host.insertBefore(wrapper, host.firstElementChild);
      }
      cursor = wrapper;
    });

    const status = document.getElementById(codexPlusRailNavId)?.querySelector(".codex-plus-sidebar-nav-status");
    if (status) status.dataset.status = codexPlusBackendStatus.status || "checking";
    if (codexPlusActiveEntry() === "plugin-market") setCodexPlusSidebarNavActive(true, "plugin-market");
    return true;
  }

  /** 图标栏存在时走它，否则回退到旧版宽面板侧边栏入口。两条路径互斥，不会重复出现。 */
