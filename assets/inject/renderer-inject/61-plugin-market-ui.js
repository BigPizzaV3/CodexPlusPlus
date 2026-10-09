  /**
   * 独立插件市场只加载索引；安装由 launcher 下载所选插件并写入本地安装目录。
   * 不调用 Codex 的 plugin/install，避免在浏览目录时触发服务连接或 OAuth。
   * 状态存数据，页面每次打开重建；请求结果通过来源/序号校验后才刷新当前页面。
   */
  const codexPlusPluginMarketState = window.__codexPlusPluginMarketState || {
    source: "public", query: "", installedOnly: false, page: 1,
    catalogs: {}, jobs: {},
  };
  window.__codexPlusPluginMarketState = codexPlusPluginMarketState;
  Object.values(codexPlusPluginMarketState.jobs).forEach((job) => {
    clearTimeout(job.timer);
    job.timer = null;
  });
  const codexPlusPluginMarketPageSize = 50;

  function codexPlusPluginMarketCatalog(source = codexPlusPluginMarketState.source) {
    if (!codexPlusPluginMarketState.catalogs[source]) {
      codexPlusPluginMarketState.catalogs[source] = {
        plugins: [], loaded: false, loading: false, message: "", revision: 0, cached: false,
      };
    }
    return codexPlusPluginMarketState.catalogs[source];
  }

  function codexPlusPluginMarketPanel() {
    return document.querySelector('[data-codex-plugin-market-panel="true"]');
  }

  function codexPlusPluginMarketFilteredPlugins() {
    const state = codexPlusPluginMarketState;
    const query = state.query.trim().toLocaleLowerCase();
    return codexPlusPluginMarketCatalog().plugins.filter((plugin) => {
      if (state.installedOnly && !plugin.installed) return false;
      if (!query) return true;
      const text = [plugin.displayName, plugin.name, plugin.description, ...(Array.isArray(plugin.tags) ? plugin.tags : [])].join(" ").toLocaleLowerCase();
      return text.includes(query);
    });
  }

  function codexPlusPluginMarketBytes(bytes) {
    const value = Number(bytes);
    if (!Number.isFinite(value) || value <= 0) return "";
    if (value < 1024 * 1024) return `${Math.ceil(value / 1024)} KB`;
    return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  }

  function codexPlusPluginMarketStage(stage) {
    return ({ queued: "等待下载", downloading: "下载中", extracting: "解压中", installing: "安装中", validating: "校验中", complete: "安装完成" })[stage] || "正在安装";
  }

  function codexPlusPluginMarketCard(plugin, source) {
    const job = codexPlusPluginMarketState.jobs[`${source}:${plugin.id}`];
    const busy = job?.busy === true;
    const installed = plugin.installed === true;
    const label = busy ? `${codexPlusPluginMarketStage(job.stage)}…` : plugin.updateAvailable ? "更新" : installed ? "已安装" : "下载安装";
    const meta = [plugin.version ? `v${plugin.version}` : "", plugin.author || "", codexPlusPluginMarketBytes(plugin.bytes), Number(plugin.skills) > 0 ? `${plugin.skills} 个技能` : ""].filter(Boolean).join(" · ");
    const tags = (Array.isArray(plugin.tags) ? plugin.tags : []).slice(0, 4);
    return `<article class="codex-plus-plugin-card" role="listitem">
      <div class="codex-plus-plugin-card-info">
        <div class="codex-plus-plugin-name">${escapeHtml(plugin.displayName || plugin.name || plugin.id)}</div>
        <div class="codex-plus-plugin-id">${escapeHtml(plugin.name || plugin.id)}</div>
        <div class="codex-plus-plugin-description">${escapeHtml(plugin.description || "暂无介绍")}</div>
        <div class="codex-plus-plugin-meta">${escapeHtml(meta)}</div>
        <div class="codex-plus-plugin-tags">${tags.map((tag) => `<span>${escapeHtml(String(tag))}</span>`).join("")}${plugin.requiresAuth ? "<span>需要服务授权</span>" : ""}${plugin.license ? `<span>${escapeHtml(plugin.license)}</span>` : ""}</div>
        ${job?.message ? `<div class="codex-plus-plugin-job-message" data-status="${job.failed ? "failed" : "ok"}">${escapeHtml(job.message)}</div>` : ""}
      </div>
      <button type="button" class="codex-plus-button codex-plus-plugin-install" data-codex-plugin-install="${escapeHtml(plugin.id)}" aria-label="${escapeHtml(`${label} ${plugin.displayName || plugin.name || plugin.id}`)}" ${busy || (installed && !plugin.updateAvailable) ? "disabled" : ""}>${label}</button>
    </article>`;
  }

  function renderCodexPlusPluginMarket() {
    const panel = codexPlusPluginMarketPanel();
    if (!panel) return;
    const state = codexPlusPluginMarketState;
    const catalog = codexPlusPluginMarketCatalog();
    const filtered = codexPlusPluginMarketFilteredPlugins();
    const pages = Math.max(1, Math.ceil(filtered.length / codexPlusPluginMarketPageSize));
    state.page = Math.max(1, Math.min(pages, state.page));
    panel.querySelectorAll("[data-codex-plugin-source]").forEach((button) => {
      const active = button.dataset.codexPluginSource === state.source;
      button.dataset.active = String(active);
      button.setAttribute("aria-pressed", String(active));
    });
    const refresh = panel.querySelector("[data-codex-plugin-refresh]");
    if (refresh) refresh.disabled = catalog.loading;
    const sourceNote = panel.querySelector("[data-codex-plugin-source-note]");
    if (sourceNote) {
      sourceNote.hidden = state.source !== "full";
      sourceNote.textContent = "完整市场需要 GitHub 私有仓库访问权限。请使用有权限的账号执行 gh auth login 后刷新目录。";
    }
    const status = panel.querySelector("[data-codex-plugin-market-status]");
    if (status) status.textContent = catalog.loading ? "正在加载插件目录…" : catalog.message || `${catalog.plugins.length.toLocaleString()} 个插件${catalog.cached ? " · 使用缓存目录" : ""}`;
    const results = panel.querySelector("[data-codex-plugin-results]");
    if (results) results.innerHTML = filtered.length
      ? filtered.slice((state.page - 1) * codexPlusPluginMarketPageSize, state.page * codexPlusPluginMarketPageSize).map((plugin) => codexPlusPluginMarketCard(plugin, state.source)).join("")
      : `<div class="codex-plus-plugin-empty">${catalog.loading ? "正在加载…" : !catalog.loaded ? "目录尚未加载，请刷新重试。" : catalog.plugins.length ? "没有找到符合条件的插件。" : "此来源暂无可显示的插件。"}</div>`;
    const count = panel.querySelector("[data-codex-plugin-result-count]");
    if (count) count.textContent = `${filtered.length.toLocaleString()} 个结果 · 第 ${state.page} / ${pages} 页`;
    const previous = panel.querySelector('[data-codex-plugin-page="previous"]');
    const next = panel.querySelector('[data-codex-plugin-page="next"]');
    if (previous) previous.disabled = state.page <= 1;
    if (next) next.disabled = state.page >= pages;
  }

  async function loadCodexPlusPluginMarket(source, refresh = false) {
    const catalog = codexPlusPluginMarketCatalog(source);
    if (catalog.loading || (catalog.loaded && !refresh)) {
      renderCodexPlusPluginMarket();
      return;
    }
    const revision = ++catalog.revision;
    catalog.loading = true;
    catalog.message = "";
    renderCodexPlusPluginMarket();
    let result;
    try {
      result = await postJson("/plugin-market/list", { source, refresh });
    } catch (error) {
      result = { status: "failed", message: error?.message || "插件目录加载失败" };
    }
    if (catalog.revision !== revision) return;
    catalog.loading = false;
    if (Array.isArray(result?.plugins)) {
      catalog.plugins = result.plugins.filter((plugin) => plugin && typeof plugin.id === "string");
      catalog.loaded = true;
      catalog.cached = result.cached === true;
      catalog.message = result.message || "";
    } else {
      catalog.message = result?.message || "插件目录加载失败，请刷新重试。";
    }
    // 切换来源后，旧请求只更新自己的数据；当前页面始终按当前来源重绘。
    if (codexPlusPluginMarketState.source === source) renderCodexPlusPluginMarket();
  }

  function updateCodexPlusPluginMarketInstalled(source, plugin) {
    if (!plugin || typeof plugin.id !== "string") return;
    const catalog = codexPlusPluginMarketCatalog(source);
    // 安装以 source + id 隔离，另一来源即使同 id 也不能被标为已安装。
    // 丢弃此前发出的本来源清单请求，避免旧 installed 状态覆盖新结果。
    catalog.revision += 1;
    catalog.loading = false;
    catalog.plugins = catalog.plugins.map((item) => {
      if (item.id !== plugin.id) return item;
      const installedVersion = plugin.installedVersion || plugin.version || item.version;
      return { ...item, installed: true, installedVersion, updateAvailable: Boolean(item.version && installedVersion && item.version !== installedVersion) };
    });
  }

  function scheduleCodexPlusPluginMarketInstallPoll(source, id, delay = 1200) {
    const job = codexPlusPluginMarketState.jobs[`${source}:${id}`];
    if (!job?.busy || job.polling || job.timer || !codexPlusPluginMarketPanel()) return;
    job.timer = setTimeout(() => {
      job.timer = null;
      void pollCodexPlusPluginMarketInstall(source, id);
    }, delay);
  }

  async function pollCodexPlusPluginMarketInstall(source, id) {
    const job = codexPlusPluginMarketState.jobs[`${source}:${id}`];
    if (!job?.busy || job.polling) return;
    job.polling = true;
    let result;
    try {
      result = await postJson("/plugin-market/install-status", { source, id });
    } catch (error) {
      result = { timeout: true, message: error?.message || "正在等待安装状态" };
    }
    job.polling = false;
    if (result?.busy === true) {
      job.stage = result.stage || job.stage;
      job.message = result.message || "";
    } else if (result?.busy === false) {
      const completed = result.status === "ok" && result.stage === "complete"
        && (result.plugin || (typeof result.installedVersion === "string" && result.installedVersion.length > 0));
      job.busy = false;
      job.failed = !completed;
      job.message = result.message || (job.failed ? "安装未完成，请重试。" : "安装完成，重启 Codex 后加载插件。");
      if (completed) updateCodexPlusPluginMarketInstalled(source, result.plugin || { id, installedVersion: result.installedVersion });
    } else {
      // 桥接超时不能重新发起安装；持续查询原任务，防止并发写同一插件。
      job.message = result?.message || "正在等待安装状态，可稍后重新打开此页查看。";
    }
    renderCodexPlusPluginMarket();
    scheduleCodexPlusPluginMarketInstallPoll(source, id, result?.timeout ? 4000 : 1200);
  }

  async function installCodexPlusPluginMarketPlugin(source, id) {
    const key = `${source}:${id}`;
    if (codexPlusPluginMarketState.jobs[key]?.busy) return;
    const plugin = codexPlusPluginMarketCatalog(source).plugins.find((item) => item.id === id);
    if (!plugin || (plugin.installed && !plugin.updateAvailable)) return;
    const job = { busy: true, stage: "downloading", message: "正在下载所选插件…", failed: false, timer: null, polling: false };
    codexPlusPluginMarketState.jobs[key] = job;
    renderCodexPlusPluginMarket();
    let result;
    try {
      result = await postJson("/plugin-market/install", { source, id });
    } catch (error) {
      result = { timeout: true, message: error?.message || "正在等待安装结果" };
    }
    if (result?.status === "ok" && result?.plugin && result?.busy !== true) {
      job.busy = false;
      job.message = result.message || "安装完成，重启 Codex 后加载插件。";
      updateCodexPlusPluginMarketInstalled(source, result.plugin);
    } else if (result?.status === "failed" && !result?.timeout && result?.busy !== true && !/超时|timeout/i.test(result.message || "")) {
      job.busy = false;
      job.failed = true;
      job.message = result.message || "安装失败";
    } else {
      job.stage = result?.stage || job.stage;
      job.message = result?.message || "正在等待下载完成…";
      scheduleCodexPlusPluginMarketInstallPoll(source, id);
    }
    renderCodexPlusPluginMarket();
  }

  function openCodexPlusPluginMarket() {
    closeCodexPlusPage();
    document.querySelectorAll('.codex-plus-modal-overlay, [data-codex-plus-dialog="true"]').forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = `${codexPlusPageClass} codex-plus-plugin-market-page`;
    overlay.dataset.codexPlusPage = "true";
    markCodexPlusExtensionNode(overlay, "builtin-plugin-market");
    applyCodexPlusTheme(overlay);
    applyCodexPlusZoom(overlay);
    const state = codexPlusPluginMarketState;
    overlay.innerHTML = `<div class="codex-plus-modal-content" role="region" aria-label="CodeX 插件市场" data-codex-plus-active-tab="${codexPlusPluginMarketTab}">
      <div class="codex-plus-modal-header"><div class="codex-plus-modal-title">CodeX 插件市场</div></div>
      <div class="codex-plus-modal-body"><div class="codex-plus-panel codex-plus-plugin-market-panel" data-codex-plugin-market-panel="true">
        <p class="codex-plus-plugin-intro">检索插件目录，选中后下载并安装。目录浏览仅获取索引，插件文件按需下载。</p>
        <div class="codex-plus-plugin-toolbar">
          <div class="codex-plus-plugin-sources" role="group" aria-label="插件来源"><button type="button" data-codex-plugin-source="public" aria-pressed="false">公开市场</button><button type="button" data-codex-plugin-source="full" aria-pressed="false">完整市场</button></div>
          <button type="button" class="codex-plus-button" data-codex-plugin-refresh="true">刷新目录</button>
        </div>
        <div class="codex-plus-plugin-status" data-codex-plugin-source-note="true" hidden></div>
        <div class="codex-plus-plugin-toolbar"><input type="search" class="codex-plus-plugin-search" data-codex-plugin-query="true" aria-label="搜索插件" placeholder="搜索插件名称、简介或标签" value="${escapeHtml(state.query)}"><label class="codex-plus-plugin-filter"><input type="checkbox" data-codex-plugin-installed="true" ${state.installedOnly ? "checked" : ""}>仅已安装</label></div>
        <div class="codex-plus-plugin-status" data-codex-plugin-market-status="true" role="status" aria-live="polite"></div>
        <div class="codex-plus-plugin-results" data-codex-plugin-results="true" role="list" aria-label="插件结果"></div>
        <div class="codex-plus-plugin-pagination"><span data-codex-plugin-result-count="true"></span><button type="button" class="codex-plus-button" data-codex-plugin-page="previous">上一页</button><button type="button" class="codex-plus-button" data-codex-plugin-page="next">下一页</button></div>
      </div></div></div>`;
    overlay.addEventListener("input", (event) => {
      if (!event.target?.matches?.("[data-codex-plugin-query]")) return;
      state.query = event.target.value;
      state.page = 1;
      renderCodexPlusPluginMarket();
    });
    overlay.addEventListener("change", (event) => {
      if (!event.target?.matches?.("[data-codex-plugin-installed]")) return;
      state.installedOnly = event.target.checked;
      state.page = 1;
      renderCodexPlusPluginMarket();
    });
    overlay.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      const source = target?.closest("[data-codex-plugin-source]");
      if (source) {
        state.source = source.dataset.codexPluginSource === "full" ? "full" : "public";
        state.page = 1;
        void loadCodexPlusPluginMarket(state.source);
        return;
      }
      if (target?.closest("[data-codex-plugin-refresh]")) {
        void loadCodexPlusPluginMarket(state.source, true);
        return;
      }
      const install = target?.closest("[data-codex-plugin-install]");
      if (install && !install.disabled) {
        void installCodexPlusPluginMarketPlugin(state.source, install.dataset.codexPluginInstall);
        return;
      }
      const page = target?.closest("[data-codex-plugin-page]");
      if (page && !page.disabled) {
        state.page += page.dataset.codexPluginPage === "next" ? 1 : -1;
        renderCodexPlusPluginMarket();
        overlay.querySelector(".codex-plus-modal-body")?.scrollTo?.({ top: 0 });
      }
    });
    document.body.appendChild(overlay);
    positionCodexPlusPage(overlay);
    document.querySelectorAll('[data-codex-plus-ext-rail-active="true"]').forEach((node) => {
      node.removeAttribute("data-codex-plus-ext-rail-active");
      const button = node.querySelector("button") || node;
      button.dataset.active = "false";
      button.removeAttribute("data-selected");
      button.removeAttribute("aria-current");
    });
    setCodexPlusSidebarNavActive(true, "plugin-market");
    // 拓展页面可能留下捕获旧 overlay 的 resize 回调，重开后必须改为查询当前 DOM。
    window.removeEventListener("resize", window.__codexPlusPageResizeHandler);
    window.__codexPlusPageResizeHandler = () => positionCodexPlusPage(document.querySelector(`.${codexPlusPageClass}`));
    window.addEventListener("resize", window.__codexPlusPageResizeHandler);
    renderCodexPlusPluginMarket();
    void loadCodexPlusPluginMarket(state.source);
    Object.entries(state.jobs).forEach(([key, job]) => {
      if (!job.busy) return;
      const colon = key.indexOf(":");
      scheduleCodexPlusPluginMarketInstallPoll(key.slice(0, colon), key.slice(colon + 1));
    });
  }
