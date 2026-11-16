  // 拓展页的「社区 / 我发布的 / dsh 插件」分栏，以及精选 / 社区的排序（对齐管理器「拓展」页）。
  // 数据与操作都经桥接交给管理器进程里同一套实现（core 的宿主路由，见 routes::register_host_route）：
  //   /extension-market/community  社区拓展：列表、显示开关、prepare → 确认 → install / discard
  //   /extension-market/published  我发布的：列表、下架、重新上架、删除仓库（gh 鉴权）
  //   /extension-market/dsh        dsh 插件：导入、启停、按行停用 / 配置、重新分析、skill、移除、接入 Codex
  // payload 统一是 `{ action, payload }`。状态放在这里，主区重画时从这里读，不缓存 DOM。

  const codexPlusExtensionSources = {
    /** 精选排序：null = 跟随数据（每个脚本都有更新时间时「最近更新」，否则清单原顺序）。 */
    marketSort: null,
    communitySort: "stars",
    /** 展开着的排序菜单所属分栏（market / community），空 = 收起。 */
    sortMenu: "",
    community: { listing: null, loading: false, loaded: false, error: "", notice: "", preparing: "" },
    mine: { listing: null, loading: false, loaded: false, error: "", notice: "", busy: "" },
    dsh: {
      snapshot: null, loading: false, loaded: false, busy: "", notice: null,
      open: new Set(), editing: null, draft: "", draftError: "",
      source: "", indexUrl: "", market: null,
    },
  };

  const codexPlusExtensionSourceRoutes = {
    community: "/extension-market/community",
    mine: "/extension-market/published",
    dsh: "/extension-market/dsh",
  };

  /** 调宿主路由；后台返回 failed 时抛出它的说明。 */
  async function codexPlusExtensionSourceCall(source, action, payload = {}) {
    const result = await postJson(codexPlusExtensionSourceRoutes[source], { action, payload });
    if (!result || result.status === "failed") {
      const message = result?.message || "未知错误";
      throw new Error(message === "Unknown bridge path" ? "这个功能需要新版 Codex++ 管理工具在运行" : message);
    }
    return result;
  }

  function codexPlusExtensionSourcesRefresh() {
    if (codexPlusActiveEntry() === "extensions") refreshCodexPlusExtensionsView();
  }

  /** 切到某个分栏时按需读取；`force` 时重新读（社区会跳过 10 分钟缓存）。 */
  function codexPlusExtensionSourceEnsureLoaded(filter, force = false) {
    if (filter === "community") {
      if (force || !codexPlusExtensionSources.community.loaded) void loadCodexPlusCommunity(force);
    } else if (filter === "mine") {
      if (force || !codexPlusExtensionSources.mine.loaded) void loadCodexPlusPublished();
    } else if (filter === "dsh") {
      if (force || !codexPlusExtensionSources.dsh.loaded) void runCodexPlusDsh("snapshot", {}, "");
    }
  }

  // ── 排序 ──────────────────────────────────────────────────────────────

  /** 清单里的更新时间（ISO、`YYYY-MM-DD[ HH:mm]`、Unix 秒 / 毫秒）；读不出返回 null。与管理器 parseMarketTime 一致。 */
  function codexPlusParseMarketTime(value) {
    const unix = (number) => (!Number.isFinite(number) || number < 1e8 ? null : number < 1e11 ? Math.round(number * 1000) : Math.round(number));
    if (typeof value === "number") return unix(value);
    const text = String(value ?? "").trim();
    if (!text) return null;
    if (/^\d+(\.\d+)?$/.test(text) && !/^\d{4}$/.test(text)) return unix(Number(text));
    const spaced = /^(\d{4}-\d{2}-\d{2})\s+(\d{1,2})(:\d{2}(?::\d{2}(?:\.\d+)?)?)\s*(.*)$/.exec(text);
    const normalized = spaced
      ? `${spaced[1]}T${spaced[2].padStart(2, "0")}${spaced[3]}${/^(utc|gmt)$/i.test(spaced[4]) ? "Z" : spaced[4]}`
      : text;
    const time = Date.parse(normalized);
    return Number.isNaN(time) ? null : time;
  }

  function codexPlusMarketSortValue() {
    if (codexPlusExtensionSources.marketSort) return codexPlusExtensionSources.marketSort;
    const scripts = codexPlusScriptMarket.scripts || [];
    return scripts.length && scripts.every((script) => codexPlusParseMarketTime(script.updatedAt) !== null) ? "updated" : "default";
  }

  /** 精选里单文件脚本的排序（拓展包固定在前，官方优先，由后端排好）。同值保持清单原顺序。 */
  function codexPlusSortMarketScripts(items) {
    const sort = codexPlusMarketSortValue();
    if (sort === "default") return items.slice();
    return items
      .map((item, index) => ({ item, index, time: codexPlusParseMarketTime(item.updatedAt) }))
      .sort((left, right) => {
        if (sort === "updated") {
          if (left.time !== right.time) {
            if (left.time === null) return 1;
            if (right.time === null) return -1;
            return right.time - left.time;
          }
        } else {
          const byName = String(left.item.name || "").localeCompare(String(right.item.name || ""), undefined, { sensitivity: "base", numeric: true });
          if (byName) return byName;
        }
        return left.index - right.index;
      })
      .map((entry) => entry.item);
  }

  const codexPlusExtensionSortOptions = {
    market: { label: "拓展市场排序", options: [["default", "默认顺序"], ["updated", "最近更新"], ["name", "名称"]] },
    community: { label: "社区拓展排序", options: [["stars", "星数"], ["updated", "最近更新"], ["name", "名称"]] },
  };

  // Codex 原生菜单的两个图标：按钮上的下箭头、选中项的对勾（取自 Codex 菜单）。
  const codexPlusMenuIcons = {
    chevron: '<svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true"><path d="M2.72 4.22a.4.4 0 0 1 .56 0L6 6.93l2.72-2.71a.4.4 0 1 1 .56.56L6.46 7.61a.65.65 0 0 1-.92 0L2.72 4.78a.4.4 0 0 1 0-.56Z" fill="currentColor"/></svg>',
    check: '<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M12.7216 2.99666C12.8863 2.75824 13.2125 2.69837 13.4511 2.86287C13.6895 3.0274 13.75 3.35377 13.5858 3.59236L7.20011 12.8472C6.97922 13.1685 6.52057 13.2111 6.24405 12.9361L2.45987 9.18123C2.25451 8.97704 2.25305 8.64475 2.45694 8.43904C2.66115 8.23343 2.99433 8.23205 3.20011 8.43611L6.62296 11.8326L12.7216 2.99666Z" fill="currentColor"/></svg>',
  };

  /**
   * 分栏条右侧的排序：和 Codex 原生下拉一样是胶囊按钮 + 自绘菜单（不用原生 select，
   * 它的弹出层不跟主题、样子也和 Codex 不一样）。精选：默认 / 最近更新 / 名称；社区：星数 / 最近更新 / 名称。
   */
  function renderCodexPlusExtensionsSortControl(filter) {
    const spec = codexPlusExtensionSortOptions[filter];
    if (!spec) return "";
    const value = filter === "market" ? codexPlusMarketSortValue() : codexPlusExtensionSources.communitySort;
    const current = spec.options.find(([id]) => id === value) || spec.options[0];
    const open = codexPlusExtensionSources.sortMenu === filter;
    const menu = open
      ? `<div class="codex-plus-menu-popover codex-plus-extensions-sort-menu" role="menu" aria-label="${spec.label}">${spec.options
        .map(([id, text]) => `<button type="button" class="codex-plus-menu-item" role="menuitemradio" aria-checked="${String(id === current[0])}" data-codex-extensions-sort-value="${id}"><span class="codex-plus-menu-item-label">${escapeHtml(text)}</span>${id === current[0] ? codexPlusMenuIcons.check : ""}</button>`)
        .join("")}</div>`
      : "";
    return `<div class="codex-plus-extensions-sort" data-codex-extensions-sort="${filter}"><button type="button" class="codex-plus-menu-trigger" data-codex-extensions-sort-toggle="${filter}" aria-haspopup="menu" aria-expanded="${String(open)}" aria-label="${spec.label}：${escapeHtml(current[1])}" title="${spec.label}"><span>${escapeHtml(current[1])}</span>${codexPlusMenuIcons.chevron}</button>${menu}</div>`;
  }

  /** 收起排序菜单；`refocus` 时焦点回到按钮（键盘操作关掉菜单后不丢焦点）。 */
  function closeCodexPlusExtensionsSortMenu(refocus = false) {
    const filter = codexPlusExtensionSources.sortMenu;
    if (!filter) return;
    codexPlusExtensionSources.sortMenu = "";
    window.removeEventListener("keydown", codexPlusExtensionsSortMenuKeydown, true);
    codexPlusExtensionSourcesRefresh();
    if (refocus) document.querySelector(`[data-codex-extensions-sort-toggle="${filter}"]`)?.focus({ preventScroll: true });
  }

  function openCodexPlusExtensionsSortMenu(filter) {
    codexPlusExtensionSources.sortMenu = filter;
    // Codex 在 document 捕获阶段把按键送进输入框，菜单的方向键 / Esc 要在 window 捕获阶段先拿到。
    window.addEventListener("keydown", codexPlusExtensionsSortMenuKeydown, true);
    codexPlusExtensionSourcesRefresh();
    const menu = document.querySelector(".codex-plus-extensions-sort-menu");
    (menu?.querySelector('[aria-checked="true"]') || menu?.querySelector(".codex-plus-menu-item"))?.focus({ preventScroll: true });
  }

  /** 菜单键盘操作：上下移动、Home / End、回车 / 空格选中、Esc / Tab 收起。 */
  function codexPlusExtensionsSortMenuKeydown(event) {
    const menu = document.querySelector(".codex-plus-extensions-sort-menu");
    if (!menu) {
      window.removeEventListener("keydown", codexPlusExtensionsSortMenuKeydown, true);
      return;
    }
    const items = Array.from(menu.querySelectorAll(".codex-plus-menu-item"));
    const index = items.indexOf(document.activeElement);
    let next = null;
    if (event.key === "ArrowDown") next = items[(index + 1) % items.length];
    else if (event.key === "ArrowUp") next = items[(index - 1 + items.length) % items.length];
    else if (event.key === "Home") next = items[0];
    else if (event.key === "End") next = items[items.length - 1];
    else if (event.key === "Enter" || event.key === " ") {
      // 回车 / 空格也在这里接：Codex 在 document 捕获阶段会把按键截走，按钮自己收不到激活。
      if (index < 0) return;
      event.preventDefault();
      event.stopPropagation();
      items[index].click();
      return;
    } else if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      event.stopPropagation();
      closeCodexPlusExtensionsSortMenu(event.key === "Escape");
      return;
    } else {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    next?.focus({ preventScroll: true });
  }

  // ── 公用小件 ──────────────────────────────────────────────────────────

  const codexPlusExtensionSourceIcons = {
    star: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m12 2 3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01z"/></svg>',
    trash: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6"/></svg>',
    plug: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z"/></svg>',
    chevron: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>',
  };

  function codexPlusExtensionSourceMatches(texts) {
    const query = codexPlusExtensionsQuery.trim().toLowerCase();
    return !query || texts.filter(Boolean).join(" ").toLowerCase().includes(query);
  }

  function renderCodexPlusExtensionSourceNotices(state) {
    const notice = state.notice
      ? typeof state.notice === "object"
        ? `<div class="codex-plus-extensions-notice" data-tone="${state.notice.tone === "error" ? "error" : "ok"}" role="status">${escapeHtml(state.notice.text)}</div>`
        : `<div class="codex-plus-extensions-notice" data-tone="ok" role="status">${escapeHtml(state.notice)}</div>`
      : "";
    const error = state.error ? `<div class="codex-plus-extensions-notice" data-tone="error" role="alert">${escapeHtml(state.error)}</div>` : "";
    return `${error}${notice}`;
  }

  function renderCodexPlusExtensionSourceEmpty(text) {
    return `<div class="codex-plus-extensions-grid" data-codex-plus-extensions-list="true"><div class="codex-plus-extensions-detail-empty" role="status">${escapeHtml(text)}</div></div>`;
  }

  /** 卡片：主区是指向仓库的链接（社区 / 我发布的没有本地详情页），右边操作列。 */
  function codexPlusExtensionLinkCard({ url, icon, name, badges = "", description = "", actions = "", dimmed = false, title = "" }) {
    const href = /^https:\/\//i.test(String(url || "")) ? url : "";
    const inner = `
            <span class="codex-plus-extension-card-icon" aria-hidden="true">${extensionIconMarkup(icon)}</span>
            <span class="codex-plus-extension-card-copy">
              <span class="codex-plus-extension-card-heading"><span class="codex-plus-extension-card-title">${escapeHtml(name)}</span>${badges}</span>
              <span class="codex-plus-extension-card-description">${escapeHtml(description)}</span>
            </span>`;
    const hover = title ? ` title="${escapeHtml(title)}"` : "";
    const select = href
      ? `<a class="codex-plus-extension-card-select" href="${escapeHtml(href)}" target="_blank" rel="noreferrer"${hover}>${inner}</a>`
      : `<span class="codex-plus-extension-card-select"${hover}>${inner}</span>`;
    return `
        <article class="codex-plus-extension-card"${dimmed ? ' data-dimmed="true"' : ""}>
          ${select}
          <div class="codex-plus-extension-card-actions">${actions}</div>
        </article>
      `;
  }

  function renderCodexPlusExtensionSourcePane(filter) {
    if (filter === "community") return renderCodexPlusCommunityPane();
    if (filter === "mine") return renderCodexPlusPublishedPane();
    if (filter === "dsh") return renderCodexPlusDshPane();
    return "";
  }

  // ── 社区 ──────────────────────────────────────────────────────────────

  async function loadCodexPlusCommunity(force = false) {
    const state = codexPlusExtensionSources.community;
    if (state.loading) return;
    state.loading = true;
    state.error = "";
    codexPlusExtensionSourcesRefresh();
    try {
      state.listing = await codexPlusExtensionSourceCall("community", "list", { force });
    } catch (error) {
      state.error = String(error?.message || error);
    } finally {
      state.loading = false;
      state.loaded = true;
      codexPlusExtensionSourcesRefresh();
    }
  }

  async function setCodexPlusCommunityEnabled(enabled) {
    const state = codexPlusExtensionSources.community;
    state.error = "";
    try {
      state.listing = await codexPlusExtensionSourceCall("community", "setEnabled", { enabled });
      codexPlusExtensionSourcesRefresh();
      if (enabled) await loadCodexPlusCommunity(false);
    } catch (error) {
      state.error = String(error?.message || error);
      codexPlusExtensionSourcesRefresh();
    }
  }

  /** 社区拓展安装：prepare（下载校验、列出权限）→ 确认框 → install；取消时清掉暂存。 */
  async function installCodexPlusCommunityExtension(repo) {
    const state = codexPlusExtensionSources.community;
    if (!repo || state.preparing) return;
    state.preparing = repo;
    state.error = "";
    state.notice = "";
    codexPlusExtensionSourcesRefresh();
    let prepared = null;
    try {
      prepared = await codexPlusExtensionSourceCall("community", "prepare", { repo });
    } catch (error) {
      state.error = `准备安装 ${repo} 失败：${error?.message || error}`;
      state.preparing = "";
      codexPlusExtensionSourcesRefresh();
      return;
    }
    try {
      const answer = await codexPlusConfirmPreparedInstall(prepared, repo);
      if (!answer) {
        void codexPlusExtensionSourceCall("community", "discard", { token: prepared.token }).catch(() => undefined);
        return;
      }
      const enable = answer.value === "enable";
      const result = await codexPlusExtensionSourceCall("community", "install", { token: prepared.token, enable, replace: !!prepared.conflict });
      const parts = [enable ? "拓展已安装并启用。" : "拓展已安装，尚未启用，可在「本地」里启用。"];
      if (result.hooksError) parts.push(`Codex 钩子配置未能同步：${result.hooksError}`);
      else if (result.hooksAdded) parts.push("Codex 配置里新增了「Codex++ 拓展钩子」，请在 Codex 的 Hooks 设置里信任一次，新开对话生效。");
      state.notice = `${prepared.name || repo} ${prepared.tag || String(prepared.sha || "").slice(0, 7)}：${parts.join("")}`;
      await reloadUserScriptsAfterChange();
      await loadCodexPlusCommunity(false);
    } catch (error) {
      state.error = `安装 ${repo} 失败：${error?.message || error}`;
    } finally {
      state.preparing = "";
      codexPlusExtensionSourcesRefresh();
    }
  }

  /** 按风险列出的徽标（与管理器 riskBadges 一致）。 */
  function codexPlusRiskBadges(permissions = {}) {
    const badges = [];
    if (permissions.backend) badges.push("后台进程");
    if (permissions.hooks?.length) badges.push("钩子");
    if (permissions.gatewayRequest || permissions.gatewayResponse) badges.push("网关");
    if (permissions.tools?.length) badges.push("模型工具");
    return badges;
  }

  function codexPlusEngineWarning(engine) {
    return typeof engine === "number" ? `需要拓展 API v${engine}，请先更新 Codex++` : `需要 Codex++ ${engine}，当前版本不兼容`;
  }

  function codexPlusCommunityEntries() {
    const entries = (codexPlusExtensionSources.community.listing?.entries || [])
      .filter((entry) => codexPlusExtensionSourceMatches([entry.name, entry.id, entry.repo, entry.description, entry.author, ...(entry.keywords || [])]));
    const sort = codexPlusExtensionSources.communitySort;
    const sorted = entries.slice();
    if (sort === "updated") sorted.sort((a, b) => String(b.pushedAt || "").localeCompare(String(a.pushedAt || "")));
    else if (sort === "name") sorted.sort((a, b) => String(a.name || "").localeCompare(String(b.name || "")));
    else sorted.sort((a, b) => (b.stars || 0) - (a.stars || 0));
    // 需要更新 Codex++ 的排到最后。
    return sorted.sort((a, b) => Number(!!b.compatible) - Number(!!a.compatible));
  }

  function renderCodexPlusCommunityCard(entry) {
    const state = codexPlusExtensionSources.community;
    const local = entry.installed?.channel === "local";
    const facts = [
      entry.author || entry.owner,
      entry.version ? `v${entry.version}` : "",
      entry.license,
      entry.pushedAt ? `更新于 ${String(entry.pushedAt).slice(0, 10)}` : "",
      !entry.compatible ? codexPlusEngineWarning(entry.engine) : "",
    ].filter(Boolean).join(" · ");
    const installedText = entry.installed
      ? entry.installed.fromDefaultBranch ? `${entry.installed.version}（${String(entry.installed.sha || "").slice(0, 7)}）` : entry.installed.tag || entry.installed.version
      : "";
    const badges = [
      `<span class="codex-plus-extension-badge" data-tone="neutral" title="GitHub 星数">${codexPlusExtensionSourceIcons.star}${escapeHtml(String(entry.stars || 0))}</span>`,
      ...codexPlusRiskBadges(entry.permissions).map((badge) => codexPlusExtensionBadge(badge, "neutral", "安装前会列出它要的全部权限")),
      entry.warnings?.length ? codexPlusExtensionBadge("可疑", "danger", entry.warnings.join("\n")) : "",
      !entry.compatible ? codexPlusExtensionBadge("需要更新 Codex++", "warn", facts) : "",
      local ? codexPlusExtensionBadge(`本机源码 ${entry.installed.version || ""}`.trim(), "neutral", "你在本机编写并发布的拓展，在「本地」里管理")
        : entry.installed ? codexPlusExtensionBadge(`已安装 ${installedText}`.trim()) : "",
    ].join("");
    const busy = state.preparing === entry.repo;
    const disabled = !entry.compatible || !!state.preparing;
    // 本机源码不给安装：从 GitHub 重装会覆盖本机源码。已安装的给「重新安装」。
    const action = local ? "" : `<button type="button" class="codex-plus-discovery-icon-button" data-codex-community-install="${escapeHtml(entry.repo)}" aria-label="${entry.installed ? "重新安装" : "安装"} ${escapeHtml(entry.name)}" title="${busy ? "正在准备安装…" : entry.installed ? "重新安装" : "安装"}"${disabled ? " disabled" : ""}${busy ? ' data-busy="true"' : ""}>${entry.installed ? codexPlusDiscoveryIcons.refresh : codexPlusDiscoveryIcons.plus}</button>`;
    return codexPlusExtensionLinkCard({
      url: entry.url,
      icon: entry.icon || entry.ownerAvatar,
      name: entry.name || entry.repo,
      badges,
      description: entry.description || facts,
      title: [entry.repo, entry.description, facts, (entry.keywords || []).map((tag) => `#${tag}`).join(" ")].filter(Boolean).join("\n"),
      actions: action,
      dimmed: !entry.compatible,
    });
  }

  function renderCodexPlusCommunityPane() {
    const state = codexPlusExtensionSources.community;
    const listing = state.listing;
    const enabled = listing?.enabled ?? true;
    const banner = `
      <div class="codex-plus-extensions-banner" data-tone="warn">
        <div class="codex-plus-extensions-banner-text">
          <strong>社区拓展未经 Codex++ 审核</strong>
          <span title="这里列出 GitHub 上打了 codexpp-extension 标签的拓展包仓库。作者可以在新版本里加入任意代码，只安装你信任的作者的拓展。安装时锁定到作者发布的版本，并在安装前列出它要的权限。">只安装你信任的作者的拓展；安装前会列出它要的权限。</span>
        </div>
        <label class="codex-plus-extensions-banner-switch"><span>显示社区拓展</span><button type="button" class="codex-plus-toggle" data-codex-community-enabled="${String(enabled)}" data-enabled="${String(enabled)}" role="switch" aria-checked="${String(enabled)}" aria-label="显示社区拓展"><span></span></button></label>
      </div>`;
    const message = listing?.message ? `<div class="codex-plus-extensions-notice" data-tone="${listing.stale ? "error" : "ok"}">${escapeHtml(listing.message)}</div>` : "";
    if (!enabled) {
      return `${banner}${message}${renderCodexPlusExtensionSourceNotices(state)}${renderCodexPlusExtensionSourceEmpty("社区拓展已关闭，不会访问 GitHub。")}`;
    }
    const entries = codexPlusCommunityEntries();
    const installed = entries.filter((entry) => entry.installed).map(renderCodexPlusCommunityCard);
    const available = entries.filter((entry) => !entry.installed).map(renderCodexPlusCommunityCard);
    const minutes = listing?.fetchedAt ? Math.max(0, Math.floor((Date.now() / 1000 - listing.fetchedAt) / 60)) : null;
    const meta = minutes === null ? "" : `<div class="codex-plus-extensions-pane-meta"${listing.problems?.length ? ` title="${listing.problems.length} 个仓库暂时读不到，下次刷新再试"` : ""}>列表更新于 ${minutes} 分钟前</div>`;
    const empty = state.loading || !state.loaded ? "正在读取社区拓展…"
      : listing?.entries?.length ? "没有匹配的拓展。"
      : state.error ? "社区拓展读取失败。"
      : "还没有社区拓展。给仓库加上 codexpp-extension 标签即可出现在这里。";
    const sections = `${installed.length ? renderCodexPlusExtensionsSection("已安装", installed) : ""}${available.length ? renderCodexPlusExtensionsSection("可安装", available) : ""}`;
    return `${banner}${message}${renderCodexPlusExtensionSourceNotices(state)}${meta}${sections || renderCodexPlusExtensionSourceEmpty(empty)}`;
  }

  // ── 我发布的 ──────────────────────────────────────────────────────────

  async function loadCodexPlusPublished() {
    const state = codexPlusExtensionSources.mine;
    if (state.loading) return;
    state.loading = true;
    state.error = "";
    codexPlusExtensionSourcesRefresh();
    try {
      state.listing = await codexPlusExtensionSourceCall("mine", "mine");
    } catch (error) {
      state.error = String(error?.message || error);
    } finally {
      state.loading = false;
      state.loaded = true;
      codexPlusExtensionSourcesRefresh();
    }
  }

  async function runCodexPlusPublished(repo, action, done) {
    const state = codexPlusExtensionSources.mine;
    if (!repo || state.busy) return;
    state.busy = repo;
    state.error = "";
    state.notice = "";
    codexPlusExtensionSourcesRefresh();
    try {
      await codexPlusExtensionSourceCall("mine", action, { repo });
      state.notice = done;
      state.busy = "";
      await loadCodexPlusPublished();
      // 下架 / 删除后社区栏的缓存已在后台清掉，下次切过去重新读。
      codexPlusExtensionSources.community.loaded = false;
    } catch (error) {
      state.error = String(error?.message || error);
    } finally {
      state.busy = "";
      codexPlusExtensionSourcesRefresh();
    }
  }

  /** 删除仓库不可恢复：和管理器一样要把仓库名原样填一遍才删。 */
  async function deleteCodexPlusPublishedRepo(repo) {
    const answer = await codexPlusAsk({
      title: "删除 GitHub 仓库",
      message: `将永久删除这个 GitHub 仓库（含所有提交、release 和 star），无法恢复；本机的拓展包不会被删除。\n\n确认请把仓库名填到下面：\n${repo}`,
      input: { placeholder: repo, default: "" },
      options: [{ value: "delete", label: "永久删除" }],
    });
    if (!answer) return;
    if (String(answer.text || "").trim() !== repo) {
      codexPlusExtensionSources.mine.error = "填的仓库名不一致，没有删除。";
      codexPlusExtensionSourcesRefresh();
      return;
    }
    await runCodexPlusPublished(repo, "deleteRepo", `已删除仓库 ${repo}`);
  }

  function renderCodexPlusPublishedPane() {
    const state = codexPlusExtensionSources.mine;
    const listing = state.listing;
    const entries = (listing?.entries || []).filter((entry) => codexPlusExtensionSourceMatches([entry.repo, entry.id, entry.description]));
    const cards = entries.map((entry) => {
      const pending = state.busy === entry.repo;
      const facts = [
        entry.repo,
        `★ ${entry.stars || 0}`,
        entry.version ? `本机发布 v${entry.version}` : "",
        entry.pushedAt ? `更新于 ${String(entry.pushedAt).slice(0, 10)}` : "",
      ].filter(Boolean).join(" · ");
      const badges = [
        codexPlusExtensionBadge(entry.listed ? "已上架" : "已下架", entry.listed ? "ok" : "neutral"),
        entry.visibility && entry.visibility !== "PUBLIC" ? codexPlusExtensionBadge("私有") : "",
        entry.archived ? codexPlusExtensionBadge("已归档") : "",
      ].join("");
      const disabled = state.busy ? " disabled" : "";
      const toggle = entry.listed
        ? `<button type="button" class="codex-plus-extensions-text-button" data-codex-published-action="unlist" data-codex-published-repo="${escapeHtml(entry.repo)}" title="下架后 ${escapeHtml(entry.repo)} 不再出现在社区拓展里，已安装的用户不受影响；仓库和 release 都保留，之后可以重新上架。"${disabled}>${pending ? "正在处理…" : "下架"}</button>`
        : `<button type="button" class="codex-plus-extensions-text-button" data-codex-published-action="relist" data-codex-published-repo="${escapeHtml(entry.repo)}"${disabled}>${pending ? "正在处理…" : "重新上架"}</button>`;
      const remove = `<button type="button" class="codex-plus-discovery-icon-button" data-codex-published-action="deleteRepo" data-codex-published-repo="${escapeHtml(entry.repo)}" aria-label="删除仓库 ${escapeHtml(entry.repo)}" title="删除仓库"${disabled}>${codexPlusExtensionSourceIcons.trash}</button>`;
      return codexPlusExtensionLinkCard({
        url: entry.url,
        icon: codexPlusGithubAvatar(entry.repo),
        name: entry.id || String(entry.repo || "").split("/")[1] || entry.repo,
        badges,
        description: entry.description || facts,
        title: facts,
        actions: `${toggle}${remove}`,
        dimmed: !entry.listed,
      });
    });
    const title = listing?.account ? `GitHub 账号 ${listing.account} 发布的拓展` : "我发布的拓展";
    const hint = '<div class="codex-plus-extensions-pane-meta" title="列出当前 gh 账号名下带 codexpp-extension 标签的仓库，以及从本机发布、已下架的仓库。下架只是去掉标签，仓库保留；删除仓库不可恢复。">下架只是去掉标签，仓库保留；删除仓库不可恢复。</div>';
    const empty = state.loading || !state.loaded ? "正在读取…"
      : listing?.entries?.length ? "没有匹配的拓展。"
      : state.error ? "读取失败：需要本机装好 gh 并登录。"
      : "还没有发布过拓展。在管理工具「本地」里点拓展包的「…」→「发布到 GitHub」。";
    const body = cards.length ? renderCodexPlusExtensionsSection(title, cards) : `<h2 class="codex-plus-discovery-section-title">${escapeHtml(title)}</h2>${renderCodexPlusExtensionSourceEmpty(empty)}`;
    return `${renderCodexPlusExtensionSourceNotices(state)}${hint}${body}`;
  }

  // ── dsh 插件 ──────────────────────────────────────────────────────────

  /** 跑一个 dsh 操作：成功后换上新的快照，后台的说明显示在提示条里。返回是否成功。 */
  async function runCodexPlusDsh(action, payload = {}, busy = action) {
    const state = codexPlusExtensionSources.dsh;
    if (state.busy) return false;
    state.busy = busy || "snapshot";
    if (action === "snapshot") state.loading = true;
    codexPlusExtensionSourcesRefresh();
    try {
      const snapshot = await codexPlusExtensionSourceCall("dsh", action, payload);
      state.snapshot = snapshot;
      if (snapshot.message) state.notice = { tone: "ok", text: snapshot.message };
      return true;
    } catch (error) {
      state.notice = { tone: "error", text: String(error?.message || error) };
      return false;
    } finally {
      state.busy = "";
      state.loading = false;
      state.loaded = true;
      codexPlusExtensionSourcesRefresh();
    }
  }

  async function importCodexPlusDshSource(value) {
    const source = String(value || "").trim();
    if (!source) return;
    const answer = await codexPlusAsk({
      title: "导入 dsh 插件",
      message: "导入时会在本机运行插件代码来列出它的工具，插件以你的用户权限运行、没有沙箱。只导入你信任的插件。继续吗？",
      options: [{ value: "import", label: "导入" }],
    });
    if (!answer) return;
    if (await runCodexPlusDsh("import", { source }, "import")) codexPlusExtensionSources.dsh.source = "";
    codexPlusExtensionSourcesRefresh();
  }

  async function browseCodexPlusDshIndex() {
    const state = codexPlusExtensionSources.dsh;
    const url = state.indexUrl.trim();
    if (!url || state.busy) return;
    state.busy = "browse";
    codexPlusExtensionSourcesRefresh();
    try {
      const result = await codexPlusExtensionSourceCall("dsh", "browse_index", { url });
      state.market = Array.isArray(result.entries) ? result.entries : [];
      if (!state.market.length) state.notice = { tone: "ok", text: "索引里没有能导入的插件。" };
    } catch (error) {
      state.notice = { tone: "error", text: String(error?.message || error) };
    } finally {
      state.busy = "";
      codexPlusExtensionSourcesRefresh();
    }
  }

  async function removeCodexPlusDshPlugin(plugin) {
    const answer = await codexPlusAsk({
      title: "移除 dsh 插件",
      message: plugin.managed
        ? `移除 ${plugin.name}？下载的包会移入 Codex++ 的备份目录。`
        : `移除 ${plugin.name}？只解除登记，不会删除你的本地目录。`,
      options: [{ value: "remove", label: "移除" }],
    });
    if (answer) await runCodexPlusDsh("remove", { id: plugin.id }, `${plugin.id}:remove`);
  }

  async function saveCodexPlusDshConfig(plugin, rowId) {
    const state = codexPlusExtensionSources.dsh;
    let config = null;
    if (state.draft.trim()) {
      try {
        config = JSON.parse(state.draft);
      } catch (error) {
        state.draftError = `不是有效 JSON：${error?.message || error}`;
        codexPlusExtensionSourcesRefresh();
        return;
      }
    }
    if (config !== null && (typeof config !== "object" || Array.isArray(config))) {
      state.draftError = "配置必须是 JSON 对象";
      codexPlusExtensionSourcesRefresh();
      return;
    }
    if (await runCodexPlusDsh("set_config", { id: plugin.id, rowId, config }, `${plugin.id}:config`)) {
      state.editing = null;
      codexPlusExtensionSourcesRefresh();
    }
  }

  function codexPlusDshSourceText(source = {}) {
    if (source.type === "local") return source.path || "";
    if (source.type === "npm") return `npm: ${source.spec}${source.registry ? `（${source.registry}）` : ""}`;
    if (source.type === "git") return `git: ${source.url}${source.reference ? `#${source.reference}` : ""}${source.path ? ` (${source.path})` : ""}`;
    return "";
  }

  const codexPlusDshStatusLabel = {
    compatible: "完全兼容", partial: "部分兼容", incompatible: "不兼容", error: "加载失败", disabled: "已停用", skills: "按 skill 导入",
  };

  function codexPlusDshMappingLabel(target) {
    return {
      instructions: "提示词 → instructions",
      mirror: "会话镜像（钩子 → dsh 会话日志）",
      commands: "斜杠命令",
      connection: "前端请求路由",
      "gateway:request": "每一步的请求 → 网关",
      "gateway:step": "每一步的请求 → 网关",
      "gateway:response": "回复流 → 网关",
      "gateway:instructions": "提示词组装 → 网关",
      "gateway:error": "失败重试 → 网关",
    }[target] || `钩子 ${target}`;
  }

  function renderCodexPlusDshCompanion(companion) {
    const parts = [];
    if (companion.instructions) parts.push(companion.staticPrompt ? "系统提示词追加到每次请求的 instructions" : "系统提示词（含动态片段）每次请求求值后追加到 instructions");
    if (companion.hooks?.length) parts.push(`事件转成 Codex 钩子 ${companion.hooks.join("、")}`);
    if (companion.gatewayStep) parts.push("每一步的模型请求经网关交给插件（换模型、按步准入、改写工具结果）");
    if (companion.gatewayResponse) parts.push("回复流经网关交给插件（llm/stream、agent/assistant-stream）");
    if (companion.gatewayError) parts.push("agent/request-error 经网关决定失败重试");
    if (companion.commands?.length) parts.push(`斜杠命令 ${companion.commands.map((command) => `/${command.name}`).join(" ")}`);
    const state = !companion.installed ? "配套拓展包未生成，点「重新分析」重试。"
      : !companion.scriptsEnabled ? "拓展总开关已关闭，配套拓展包不会运行。"
      : !companion.enabled ? "配套拓展包已停用。" : "";
    const notes = [
      companion.instructions ? "提示词只对经 Codex++ 本地代理 / 网关发出的请求生效。" : "",
      companion.hooks?.length ? "钩子需在 Codex 的 Hooks 设置里信任「Codex++ 拓展钩子」；插件在常驻进程里运行，内存状态跨事件保留。" : "",
    ].join("");
    return `<div class="codex-plus-dsh-companion">配套拓展「${escapeHtml(companion.id)}」：${escapeHtml(parts.join("；"))}。${escapeHtml(notes)}${state ? `<div class="codex-plus-dsh-warning">${escapeHtml(state)}</div>` : ""}</div>`;
  }

  function renderCodexPlusDshPlugin(plugin) {
    const state = codexPlusExtensionSources.dsh;
    const report = plugin.report || {};
    const rows = report.rows || [];
    const open = state.open.has(plugin.id);
    const skillCount = report.skills?.length || 0;
    const problems = rows.filter((row) => row.status === "incompatible" || row.status === "error").length + (report.diagnostics?.length || 0);
    const busy = !!state.busy;
    const id = escapeHtml(plugin.id);
    const badges = [
      plugin.version ? codexPlusExtensionBadge(`v${plugin.version}`) : "",
      report.kind === "bundle" ? codexPlusExtensionBadge("组合包") : "",
      codexPlusExtensionBadge(`${plugin.toolCount || 0} 个工具`),
      skillCount ? codexPlusExtensionBadge(`${skillCount} 个 skill`) : "",
      problems ? codexPlusExtensionBadge(`${problems} 项不兼容`, "warn") : "",
    ].join("");
    const source = codexPlusDshSourceText(plugin.source);
    const head = `
        <article class="codex-plus-extension-card codex-plus-dsh-plugin"${plugin.enabled ? "" : ' data-dimmed="true"'}>
          <button type="button" class="codex-plus-extension-card-select" data-codex-dsh-expand="${id}" aria-expanded="${String(open)}" title="${escapeHtml(source)}">
            <span class="codex-plus-extension-card-icon" data-tone="${plugin.enabled ? "ok" : "off"}" aria-hidden="true">${codexPlusExtensionSourceIcons.plug}</span>
            <span class="codex-plus-extension-card-copy">
              <span class="codex-plus-extension-card-heading"><span class="codex-plus-extension-card-title">${escapeHtml(report.title || plugin.name)}</span>${badges}</span>
              <span class="codex-plus-extension-card-description">${escapeHtml(report.description || source)}</span>
            </span>
            <span class="codex-plus-dsh-chevron" data-open="${String(open)}" aria-hidden="true">${codexPlusExtensionSourceIcons.chevron}</span>
          </button>
          <div class="codex-plus-extension-card-actions">
            <button type="button" class="codex-plus-toggle" data-codex-dsh-toggle="${id}" data-enabled="${String(!!plugin.enabled)}" role="switch" aria-checked="${String(!!plugin.enabled)}" aria-label="启用插件 ${escapeHtml(plugin.name)}" title="停用后 Codex 里不再提供这个插件的工具"${busy ? " disabled" : ""}><span></span></button>
          </div>
        </article>`;
    if (!open) return head;
    const disabled = busy ? " disabled" : "";
    const actions = [
      `<button type="button" class="codex-plus-extensions-text-button" data-codex-dsh-action="reinspect" data-codex-dsh-id="${id}"${disabled}>重新分析</button>`,
      skillCount ? `<button type="button" class="codex-plus-extensions-text-button" data-codex-dsh-action="import_skills" data-codex-dsh-id="${id}"${disabled}>${plugin.importedSkills?.length ? "重新导入 skill" : "导入 skill"}</button>` : "",
      plugin.mcpJson ? `<button type="button" class="codex-plus-extensions-text-button" data-codex-dsh-copy-mcp="${id}">复制 MCP JSON</button>` : "",
      `<button type="button" class="codex-plus-extensions-text-button" data-tone="danger" data-codex-dsh-action="remove" data-codex-dsh-id="${id}"${disabled}>移除</button>`,
    ].join("");
    const rowHtml = rows.map((row) => {
      const rowId = escapeHtml(row.id);
      const configurable = (row.kind === "plugin" && row.status !== "incompatible") || row.status === "disabled";
      const rowDisabled = (plugin.disabledRows || []).includes(row.id);
      const editing = state.editing?.plugin === plugin.id && state.editing?.row === row.id;
      const editor = editing ? `
              <div class="codex-plus-dsh-config">
                <textarea data-codex-dsh-draft="true" rows="6" spellcheck="false" aria-label="插件配置（JSON）">${escapeHtml(state.draft)}</textarea>
                <small>浅合并到组合包里的配置之上；留空表示清除。</small>
                ${state.draftError ? `<div class="codex-plus-dsh-warning">${escapeHtml(state.draftError)}</div>` : ""}
                <div class="codex-plus-dsh-config-actions">
                  <button type="button" class="codex-plus-extensions-detail-button codex-plus-extensions-detail-primary" data-codex-dsh-save-config="${id}" data-codex-dsh-row="${rowId}"${disabled}>保存并重新分析</button>
                  <button type="button" class="codex-plus-extensions-text-button" data-codex-dsh-cancel-config="true">取消</button>
                </div>
              </div>` : "";
      return `
            <div class="codex-plus-dsh-row">
              <div class="codex-plus-dsh-row-head">
                <span class="codex-plus-dsh-status" data-status="${escapeHtml(row.status)}">${escapeHtml(codexPlusDshStatusLabel[row.status] || row.status)}</span>
                <strong>${rowId}</strong>
                ${row.module && row.module !== row.id ? `<code>${escapeHtml(row.module)}</code>` : ""}
                <span class="codex-plus-dsh-row-actions">${configurable ? `
                  <button type="button" class="codex-plus-extensions-text-button" data-codex-dsh-edit-config="${id}" data-codex-dsh-row="${rowId}"${disabled}>配置</button>
                  <button type="button" class="codex-plus-extensions-text-button" data-codex-dsh-row-toggle="${id}" data-codex-dsh-row="${rowId}" data-enabled="${String(rowDisabled)}"${disabled}>${rowDisabled ? "启用此行" : "停用此行"}</button>` : ""}
                </span>
              </div>
              ${row.mappings?.length ? `<div class="codex-plus-dsh-mappings">${row.mappings.map((target) => `<span>${escapeHtml(codexPlusDshMappingLabel(target))}</span>`).join("")}</div>` : ""}
              ${row.tools?.length ? `<ul class="codex-plus-dsh-tools">${row.tools.map((tool) => `<li><code>${escapeHtml(tool.name)}</code><span>${escapeHtml(tool.description || "")}</span></li>`).join("")}</ul>` : ""}
              ${[...(row.reasons || []), ...(row.notes || [])].map((text) => `<div class="codex-plus-dsh-reason">${escapeHtml(text)}</div>`).join("")}
              ${editor}
            </div>`;
    }).join("");
    const skills = skillCount
      ? `<div class="codex-plus-dsh-line">${escapeHtml(plugin.importedSkills?.length ? `已导入 Skills：${plugin.importedSkills.join("、")}` : `包含 skill：${(report.skills || []).map((skill) => skill.name).join("、")}`)}</div>`
      : "";
    const mcp = plugin.mcpJson ? '<div class="codex-plus-dsh-line">包里有 MCP 配置，复制后可在 MCP 设置里用 JSON 导入。</div>' : "";
    return `${head}
        <div class="codex-plus-dsh-detail">
          <div class="codex-plus-dsh-source">${escapeHtml(source)}</div>
          <div class="codex-plus-dsh-actions">${actions}</div>
          ${(report.diagnostics || []).map((text) => `<div class="codex-plus-dsh-warning">${escapeHtml(text)}</div>`).join("")}
          ${rowHtml}
          ${plugin.companion ? renderCodexPlusDshCompanion(plugin.companion) : ""}
          ${skills}${mcp}
        </div>`;
  }

  function renderCodexPlusDshPane() {
    const state = codexPlusExtensionSources.dsh;
    const snapshot = state.snapshot;
    const plugins = snapshot?.plugins || [];
    const integration = snapshot?.integration;
    const busy = state.busy ? " disabled" : "";
    let banner = "";
    if (snapshot && !snapshot.toolchain?.node) {
      banner = '<div class="codex-plus-extensions-banner" data-tone="warn"><div class="codex-plus-extensions-banner-text"><strong>没有找到 Node 运行时</strong><span>请安装 Codex App（自带 Node）或 Node.js 后再导入。</span></div></div>';
    } else if (integration && !integration.registered) {
      banner = `<div class="codex-plus-extensions-banner" data-tone="warn"><div class="codex-plus-extensions-banner-text"><strong>dsh 插件 MCP 未接入 Codex</strong><span>${escapeHtml(integration.status === "conflict" ? integration.message : "工具插件需要先把 dsh 插件 MCP 接入 Codex（只写入 Codex++ 托管的一条 MCP 配置）。")}</span></div><button type="button" class="codex-plus-extensions-detail-button" data-codex-dsh-action="connect_codex"${busy || integration.status === "conflict" ? " disabled" : ""}>${state.busy === "connect_codex" ? "正在接入…" : "接入 Codex"}</button></div>`;
    }
    const shown = plugins.filter((plugin) => codexPlusExtensionSourceMatches([plugin.name, plugin.id, plugin.report?.title, plugin.report?.description]));
    const toolTotal = plugins.filter((plugin) => plugin.enabled).reduce((sum, plugin) => sum + (plugin.toolCount || 0), 0);
    const aside = integration?.registered ? ` <span class="codex-plus-extensions-pane-meta" title="改动在新开 Codex 对话后生效">已接入 Codex · ${toolTotal} 个工具</span>` : "";
    const list = shown.length
      ? `<div class="codex-plus-dsh-list">${shown.map(renderCodexPlusDshPlugin).join("")}</div>`
      : renderCodexPlusExtensionSourceEmpty(state.loading || !state.loaded ? "正在读取 dsh 插件…" : plugins.length ? "没有匹配的插件。" : "还没有导入 dsh 插件。在下面填 npm 包名、git 地址或本地目录。");
    const intro = "导入 dsh（deepseek-harness）插件：工具插件经 Codex++ 托管的 MCP 在 Codex 里调用，skill 导入 Codex Skills，MCP 配置可一键复制导入；系统提示词与工具前后、回合前后的事件经自动生成的配套拓展转成网关 instructions 与 Codex 钩子。依赖 dsh agent 循环、会话或模型服务的插件无法在 Codex 里运行，会逐项说明原因。";
    const market = state.market?.length
      ? `<ul class="codex-plus-dsh-tools codex-plus-dsh-market">${state.market.map((entry, index) => `<li><code>${escapeHtml(entry.title || entry.name)}${entry.version ? ` ${escapeHtml(entry.version)}` : ""}</code><span>${escapeHtml(entry.description || "")}${entry.note ? `（${escapeHtml(entry.note)}）` : ""}</span><button type="button" class="codex-plus-extensions-text-button" data-codex-dsh-import-index="${index}"${busy}>导入</button></li>`).join("")}</ul>`
      : "";
    const npmHint = snapshot && !snapshot.toolchain?.npm ? '<div class="codex-plus-extensions-pane-meta">未找到 npm：只能导入本地目录，或不需要额外依赖的 git 仓库。</div>' : "";
    return `${banner}${renderCodexPlusExtensionSourceNotices(state)}
      <h2 class="codex-plus-discovery-section-title">已导入 <span class="codex-plus-discovery-section-count">${plugins.length}</span>${aside}</h2>
      ${list}
      <h2 class="codex-plus-discovery-section-title" title="${escapeHtml(intro)}">导入插件</h2>
      <div class="codex-plus-dsh-import">
        <input type="text" data-codex-dsh-source="true" value="${escapeHtml(state.source)}" spellcheck="false" aria-label="插件来源" placeholder="npm 包名（如 @scope/dsh-tool-x，可加 #registry:地址）、git 地址（可加 #分支&path:子目录）或本地目录绝对路径"${busy} />
        <button type="button" class="codex-plus-extensions-detail-button codex-plus-extensions-detail-primary" data-codex-dsh-import="true"${busy}>${state.busy === "import" ? "正在导入…" : "导入"}</button>
      </div>
      ${npmHint}
      <details class="codex-plus-dsh-index"${state.market?.length ? " open" : ""}>
        <summary>从插件市场索引导入</summary>
        <div class="codex-plus-dsh-import">
          <input type="text" data-codex-dsh-index-url="true" value="${escapeHtml(state.indexUrl)}" spellcheck="false" aria-label="插件市场索引地址" placeholder="dsh 插件市场索引地址（格式 1 的 JSON），列出后可逐个导入"${busy} />
          <button type="button" class="codex-plus-extensions-detail-button" data-codex-dsh-browse="true"${busy}>${state.busy === "browse" ? "正在读取…" : "浏览索引"}</button>
        </div>
        ${market}
      </details>`;
  }

  // ── 事件 ──────────────────────────────────────────────────────────────

  function codexPlusDshPluginById(id) {
    return (codexPlusExtensionSources.dsh.snapshot?.plugins || []).find((plugin) => plugin.id === id) || null;
  }

  /** 拓展页的点击：命中这些分栏的按钮就处理并返回 true。 */
  function handleCodexPlusExtensionSourcesClick(target, event) {
    if (!target?.closest) return false;
    const dsh = codexPlusExtensionSources.dsh;
    const sortToggle = target.closest("[data-codex-extensions-sort-toggle]");
    if (sortToggle) {
      const filter = sortToggle.getAttribute("data-codex-extensions-sort-toggle");
      if (codexPlusExtensionSources.sortMenu === filter) closeCodexPlusExtensionsSortMenu(true);
      else openCodexPlusExtensionsSortMenu(filter);
      return true;
    }
    const sortValue = target.closest("[data-codex-extensions-sort-value]");
    if (sortValue) {
      const filter = codexPlusExtensionSources.sortMenu;
      const value = sortValue.getAttribute("data-codex-extensions-sort-value");
      if (filter === "market") codexPlusExtensionSources.marketSort = value;
      else if (filter === "community") codexPlusExtensionSources.communitySort = value;
      closeCodexPlusExtensionsSortMenu(true);
      return true;
    }
    // 点菜单外面：收起菜单，这次点击照常交给后面的处理（和 Codex 原生菜单一样不吞点击）。
    if (codexPlusExtensionSources.sortMenu && !target.closest(".codex-plus-extensions-sort")) closeCodexPlusExtensionsSortMenu();
    const communityInstall = target.closest("[data-codex-community-install]");
    if (communityInstall) {
      if (!communityInstall.disabled) void installCodexPlusCommunityExtension(communityInstall.getAttribute("data-codex-community-install"));
      return true;
    }
    const communitySwitch = target.closest("[data-codex-community-enabled]");
    if (communitySwitch) {
      void setCodexPlusCommunityEnabled(communitySwitch.getAttribute("data-codex-community-enabled") !== "true");
      return true;
    }
    const published = target.closest("[data-codex-published-action]");
    if (published) {
      if (published.disabled) return true;
      const repo = published.getAttribute("data-codex-published-repo") || "";
      const action = published.getAttribute("data-codex-published-action");
      if (action === "deleteRepo") void deleteCodexPlusPublishedRepo(repo);
      else if (action === "unlist") void runCodexPlusPublished(repo, "unlist", `已下架 ${repo}`);
      else if (action === "relist") void runCodexPlusPublished(repo, "relist", `已重新上架 ${repo}，社区列表刷新后可见`);
      return true;
    }
    const expand = target.closest("[data-codex-dsh-expand]");
    if (expand) {
      const id = expand.getAttribute("data-codex-dsh-expand");
      if (dsh.open.has(id)) dsh.open.delete(id);
      else dsh.open.add(id);
      codexPlusExtensionSourcesRefresh();
      return true;
    }
    const toggle = target.closest("[data-codex-dsh-toggle]");
    if (toggle) {
      const id = toggle.getAttribute("data-codex-dsh-toggle");
      if (!toggle.disabled) void runCodexPlusDsh("set_enabled", { id, enabled: toggle.getAttribute("data-enabled") !== "true" }, `${id}:toggle`);
      return true;
    }
    const action = target.closest("[data-codex-dsh-action]");
    if (action) {
      if (action.disabled) return true;
      const name = action.getAttribute("data-codex-dsh-action");
      const id = action.getAttribute("data-codex-dsh-id") || "";
      if (name === "remove") {
        const plugin = codexPlusDshPluginById(id);
        if (plugin) void removeCodexPlusDshPlugin(plugin);
      } else if (name === "connect_codex") {
        void runCodexPlusDsh("connect_codex", {}, "connect_codex");
      } else if (name === "reinspect" || name === "import_skills") {
        void runCodexPlusDsh(name, { id }, `${id}:${name}`);
      }
      return true;
    }
    const copy = target.closest("[data-codex-dsh-copy-mcp]");
    if (copy) {
      const plugin = codexPlusDshPluginById(copy.getAttribute("data-codex-dsh-copy-mcp"));
      if (plugin?.mcpJson) {
        Promise.resolve(navigator.clipboard?.writeText(plugin.mcpJson))
          .then(() => showToast("MCP JSON 已复制", null), () => showToast("复制失败", { type: "error" }));
      }
      return true;
    }
    const rowToggle = target.closest("[data-codex-dsh-row-toggle]");
    if (rowToggle) {
      const id = rowToggle.getAttribute("data-codex-dsh-row-toggle");
      const rowId = rowToggle.getAttribute("data-codex-dsh-row");
      if (!rowToggle.disabled) void runCodexPlusDsh("set_row_enabled", { id, rowId, enabled: rowToggle.getAttribute("data-enabled") === "true" }, `${id}:row:${rowId}`);
      return true;
    }
    const edit = target.closest("[data-codex-dsh-edit-config]");
    if (edit) {
      const plugin = codexPlusDshPluginById(edit.getAttribute("data-codex-dsh-edit-config"));
      const rowId = edit.getAttribute("data-codex-dsh-row");
      if (plugin) {
        const current = plugin.configs?.[rowId];
        dsh.editing = { plugin: plugin.id, row: rowId };
        dsh.draft = current === undefined ? "{\n  \n}" : JSON.stringify(current, null, 2);
        dsh.draftError = "";
        codexPlusExtensionSourcesRefresh();
      }
      return true;
    }
    if (target.closest("[data-codex-dsh-cancel-config]")) {
      dsh.editing = null;
      codexPlusExtensionSourcesRefresh();
      return true;
    }
    const save = target.closest("[data-codex-dsh-save-config]");
    if (save) {
      const plugin = codexPlusDshPluginById(save.getAttribute("data-codex-dsh-save-config"));
      if (plugin && !save.disabled) void saveCodexPlusDshConfig(plugin, save.getAttribute("data-codex-dsh-row"));
      return true;
    }
    if (target.closest("[data-codex-dsh-import]")) {
      void importCodexPlusDshSource(dsh.source);
      return true;
    }
    if (target.closest("[data-codex-dsh-browse]")) {
      void browseCodexPlusDshIndex();
      return true;
    }
    const indexImport = target.closest("[data-codex-dsh-import-index]");
    if (indexImport) {
      const entry = dsh.market?.[Number(indexImport.getAttribute("data-codex-dsh-import-index"))];
      if (entry?.source && !indexImport.disabled) void importCodexPlusDshSource(entry.source);
      return true;
    }
    return false;
  }

  /** 输入框：只记进状态，不重画（重画会丢光标）。 */
  function handleCodexPlusExtensionSourcesInput(target) {
    if (!target?.closest) return false;
    const dsh = codexPlusExtensionSources.dsh;
    if (target.closest("[data-codex-dsh-source]")) {
      dsh.source = target.value;
      return true;
    }
    if (target.closest("[data-codex-dsh-index-url]")) {
      dsh.indexUrl = target.value;
      return true;
    }
    if (target.closest("[data-codex-dsh-draft]")) {
      dsh.draft = target.value;
      return true;
    }
    return false;
  }

  // ── 拓展页分栏到此为止 ──
