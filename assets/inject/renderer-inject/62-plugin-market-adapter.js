  /**
   * 复用原生插件页面：仅接管带独立索引标记的详情与安装，其他插件仍走原接口。
   * 当前验证的传输有 client.sendRequest、fetch envelope、mcp-request envelope，
   * 以及页面到宿主的 MessagePort RPC（codexPlusPluginNativeRpcHandler，Codex 26.1117 上的插件请求只走这条）。
   * 安装请求只发一次；桥接超时后查询后台任务，不能回落到原生 plugin/install。
   * 装完后按包里声明的连接器提示下一步：MCP 服务可以直接登录，ChatGPT 托管的要有 ChatGPT 登录。
   */
  const codexPlusPluginNativeVersion = "1";
  const codexPlusPluginNativeState = window.__codexPlusPluginNativeState || {
    bypass: Symbol("codex-plus-plugin-native-bypass"), requests: new Map(), synthetic: new WeakSet(),
    ids: new Map(), catalogs: new Map(), installJobs: new Map(),
  };
  window.__codexPlusPluginNativeState = codexPlusPluginNativeState;
  const codexPlusPluginNativeBypass = codexPlusPluginNativeState.bypass;
  const codexPlusPluginNativeRequests = codexPlusPluginNativeState.requests;
  const codexPlusPluginNativeSyntheticMessages = codexPlusPluginNativeState.synthetic;
  const codexPlusPluginNativeIds = codexPlusPluginNativeState.ids;
  const codexPlusPluginNativeCatalogs = codexPlusPluginNativeState.catalogs;
  const codexPlusPluginNativeInstallJobs = codexPlusPluginNativeState.installJobs;

  function codexPlusPluginNativeOperation(method, params, hostId) {
    const wrapped = String(method || "") === "send-cli-request-for-host";
    const requestParams = wrapped ? params?.params || {} : params || {};
    const hosts = [hostId, params?.hostId, wrapped ? requestParams?.hostId : null].filter((value) => value != null);
    return {
      method: appServerModelRequestMethod(String(method || ""), params),
      params: requestParams,
      hostId: hosts.find((value) => value !== "local") ?? hosts[0],
    };
  }

  function codexPlusPluginNativeSource(params) {
    const path = String(params?.marketplacePath || "").replace(/\\/g, "/");
    const pathMatch = /(?:^|\/)codex-plus-plugin-market\/virtual-(public|full)\/\.agents\/plugins\/marketplace\.json$/.exec(path);
    if (pathMatch) return pathMatch[1];
    const name = String(params?.remoteMarketplaceName || params?.localMarketplaceName || "");
    const nameMatch = /^codex-plus-index-(public|full)$/.exec(name);
    return nameMatch ? nameMatch[1] : "";
  }

  function codexPlusPluginNativeLocal(operation) {
    return operation.hostId == null || operation.hostId === "local";
  }

  function codexPlusPluginNativeListAllowed(operation) {
    if (!codexPlusPluginNativeLocal(operation)) return false;
    if (operation.method === "installed-plugins") return true;
    if (operation.method === "search-plugins") {
      // 只在「全部 / OpenAI」目录的首页结果里补索引条目；个人、工作区目录与翻页不混入。
      const scope = operation.params?.scope;
      return (scope == null || scope === "global") && operation.params?.cursor == null;
    }
    if (operation.method !== "list-plugins") return false;
    const kinds = operation.params?.marketplaceKinds;
    return !Array.isArray(kinds) || kinds.length === 0 || kinds.includes("local");
  }

  function codexPlusPluginNativeRemember(result) {
    const marketplaces = result?.marketplaces;
    if (!Array.isArray(marketplaces)) return;
    marketplaces.forEach((market) => {
      const source = codexPlusPluginNativeSource({ marketplacePath: market.path, localMarketplaceName: market.name });
      if (!source || !Array.isArray(market.plugins)) return;
      market.plugins.forEach((plugin) => {
        if (typeof plugin?.name === "string" && typeof plugin?.codexPlusIndexId === "string") {
          codexPlusPluginNativeIds.set(`${source}:${plugin.name}`, plugin.codexPlusIndexId);
        }
      });
    });
  }

  async function codexPlusPluginNativeBackend(method, params, source) {
    let result;
    try {
      result = await postJson("/plugin-market/native", { method, params: params || {}, ...(source ? { source } : {}) });
    } catch (error) {
      // 传输中断不能证明后台未接到请求；安装提交后的异常要查询原任务。
      const failure = error instanceof Error ? error : new Error(String(error));
      failure.outcomeUnknown = true;
      failure.timeout = /timeout|timed out|超时|deadline/i.test(failure.message);
      throw failure;
    }
    if (!result || typeof result !== "object" || result.status === "failed") {
      const error = new Error(result?.message || "Codex 插件市场请求失败");
      error.timeout = result?.timeout === true || /timeout|timed out|超时/i.test(error.message);
      throw error;
    }
    return result;
  }

  async function codexPlusPluginNativeCatalog(operation) {
    const key = operation.method;
    const now = Date.now();
    const cached = codexPlusPluginNativeCatalogs.get(key);
    const ttl = key === "installed-plugins" ? 1000 : 60_000;
    if (cached?.promise) return JSON.parse(JSON.stringify(await cached.promise));
    if (cached?.value && !operation.params?.forceRefetch && now - cached.at < ttl) {
      return JSON.parse(JSON.stringify(cached.value));
    }
    const entry = { at: now, value: null, promise: null };
    const promise = codexPlusPluginNativeBackend(operation.method, operation.params).then((result) => {
      if (!Array.isArray(result.marketplaces)) throw new Error("Codex 插件市场列表格式错误");
      codexPlusPluginNativeRemember(result);
      entry.value = result;
      entry.at = Date.now();
      return result;
    }).finally(() => { entry.promise = null; });
    entry.promise = promise;
    codexPlusPluginNativeCatalogs.set(key, entry);
    return JSON.parse(JSON.stringify(await promise));
  }

  function codexPlusPluginNativeIndexMarket(market) {
    return Boolean(codexPlusPluginNativeSource({ marketplacePath: market?.path, localMarketplaceName: market?.name }));
  }

  /**
   * Codex 26.1117 的插件页按市场名分页签：名字或显示名规整后落在这几个里的进「公开」，其余进「个人」。
   * 索引目录的显示名（后端 native_market）也按这条规则取，所以和官方插件同在「公开」页签、一起按分类分组。
   */
  const codexPlusPluginNativePublicMarketNames = new Set([
    "codex-official", "openai-bundled", "openai-curated", "openai-curated-remote", "openai-primary-runtime",
  ]);

  function codexPlusPluginNativePublicMarket(market) {
    return [market?.name, market?.interface?.displayName].some((value) => typeof value === "string"
      && codexPlusPluginNativePublicMarketNames.has(value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "")));
  }

  /** 同在「公开」页签的官方市场里已有同名插件时，不再列索引里的那一个；已经从索引装上的保留，方便管理。 */
  function codexPlusPluginNativeWithoutOfficial(extra, base) {
    const official = new Set((Array.isArray(base.marketplaces) ? base.marketplaces : [])
      .filter((market) => codexPlusPluginNativePublicMarket(market) && !codexPlusPluginNativeIndexMarket(market))
      .flatMap((market) => (Array.isArray(market.plugins) ? market.plugins : []).map((plugin) => plugin?.name)));
    if (official.size === 0 || !Array.isArray(extra?.marketplaces)) return extra;
    // 被官方同名插件顶掉的索引条目：记下它的索引 id，排版时官方那一个排在它的位置上。
    const twins = {};
    return {
      ...extra,
      marketplaces: extra.marketplaces.map((market) => codexPlusPluginNativeIndexMarket(market) && Array.isArray(market.plugins)
        ? {
          ...market,
          plugins: market.plugins.filter((plugin) => {
            if (plugin?.installed === true || !official.has(plugin?.name)) return true;
            if (typeof plugin.codexPlusIndexId === "string") twins[plugin.name] = plugin.codexPlusIndexId;
            return false;
          }),
        }
        : market),
      codexPlusOfficialTwins: twins,
    };
  }

  /**
   * 有官方目录的分组（索引里的 `sections`）时，把「公开」页签里的本机插件和官方插件排成和官方目录一样的分组：
   * 组名用官方目录的（按当前界面语言）、组内按官方顺序，人工编排的合集（热门等）另外补一份条目。
   * 只改顺序和组名，条目本身仍是 plugin/list 返回的那些。
   */
  function codexPlusPluginNativeSectionLabel(section) {
    return codexPlusResolveText(section?.title).trim();
  }

  /** 组的标识：官方目录的 slug（`developer-tools`）按文字也能对上包清单里的分类（`Developer Tools`）。 */
  function codexPlusPluginNativeSectionKey(label) {
    return String(label || "").trim().toLowerCase().replaceAll(" ", "-");
  }

  /**
   * 按官方目录排版：页面是按每个插件的分类文字分组的（分组标题就是这段文字），所以
   * - 官方分组里的插件，分类换成那一组在当前界面语言下的组名；人工编排的合集另列一份、分类写合集名；
   * - 不在官方分组里的按自己的分类归组（英文分类名换成同一组的组名），排在官方列过的后面。
   * 组内先后靠返回顺序：页面按市场数组依次展开，所以同一个市场拆成几段、和别的市场穿插着排
   * （官方插件才能排到它在官方目录里的位置）。组的先后页面按组名排，另用样式纠正（codexPlusPluginNativeSectionOrder）。
   */
  function codexPlusPluginNativeArrange(result, method) {
    const { codexPlusSections, codexPlusOfficialTwins, ...rest } = result || {};
    const sections = method === "list-plugins" && Array.isArray(codexPlusSections) ? codexPlusSections : [];
    const markets = Array.isArray(rest.marketplaces) ? rest.marketplaces : [];
    if (sections.length === 0 || !markets.some(codexPlusPluginNativeIndexMarket)) return rest;
    const labels = sections.map(codexPlusPluginNativeSectionLabel);
    const rankByKey = new Map();
    sections.forEach((section, rank) => {
      for (const key of [section.slug, section.title?.en, labels[rank]]) if (key) rankByKey.set(codexPlusPluginNativeSectionKey(key), rank);
    });
    // 本机目录永远算在内；有官方目录时，原生会排进「公开」页签的市场（官方那几个）一起排。
    const official = markets.some((market) => codexPlusPluginNativePublicMarket(market) && !codexPlusPluginNativeIndexMarket(market));
    const arranged = (market) => codexPlusPluginNativeIndexMarket(market) || (official && codexPlusPluginNativePublicMarket(market));
    const entries = [];
    markets.forEach((market) => {
      if (!arranged(market)) return;
      for (const plugin of market.plugins || []) entries.push({ market, plugin });
    });
    // 官方目录按索引 id 列成员：索引条目用自己的 id，顶掉同名索引条目的官方插件用被顶掉那一个的 id。
    const keyed = new Map();
    for (const entry of entries) {
      const key = entry.plugin?.codexPlusIndexId ?? codexPlusOfficialTwins?.[entry.plugin?.name];
      if (typeof key === "string") keyed.set(key, entry);
    }
    const categorized = (plugin, label) => plugin.interface && typeof plugin.interface === "object"
      ? { ...plugin, interface: { ...plugin.interface, category: label } } : plugin;
    const listed = [];
    const placed = new Set();
    sections.forEach((section, rank) => {
      (section.plugins || []).forEach((id, member) => {
        const entry = keyed.get(id);
        if (!entry) return;
        if (section.curated) {
          // 合集里另列一份（官方目录里同一个插件既在合集、又在自己的分类里）：名字不变，详情和安装仍按名字找。
          // 装好的只列在自己的分类里，切换和卸载按原来的 id 认。
          const { name, installed } = entry.plugin;
          if (installed === true || typeof name !== "string" || typeof entry.market.name !== "string") return;
          listed.push({ market: entry.market, rank: [rank, member], plugin: { ...categorized(entry.plugin, labels[rank]), id: `${name}@${entry.market.name}-${section.slug}` } });
        } else if (!placed.has(entry)) {
          placed.add(entry);
          listed.push({ market: entry.market, rank: [rank, member], plugin: categorized(entry.plugin, labels[rank]) });
        }
      });
    });
    const otherAt = rankByKey.get("other") ?? sections.length;
    entries.forEach((entry, position) => {
      if (placed.has(entry)) return;
      const rank = rankByKey.get(codexPlusPluginNativeSectionKey(entry.plugin?.interface?.category || "Other")) ?? otherAt;
      listed.push({ market: entry.market, rank: [rank + 0.5, position], plugin: rank < sections.length ? categorized(entry.plugin, labels[rank]) : entry.plugin });
    });
    listed.sort((left, right) => left.rank[0] - right.rank[0] || left.rank[1] - right.rank[1]);
    const output = [];
    for (const item of listed) {
      const last = output.at(-1);
      if (last && last.name === item.market.name && last.path === item.market.path) last.plugins.push(item.plugin);
      else output.push({ ...item.market, plugins: [item.plugin] });
    }
    for (const market of markets) if (!arranged(market)) output.push(market);
    codexPlusPluginNativeSectionOrder(labels);
    return { ...rest, marketplaces: output, featuredPluginIds: [] };
  }

  const codexPlusPluginNativeSectionStyleId = "codex-plus-plugin-section-order";

  /** 页面按组名的字母序排分组；这里按官方顺序给每一组定位置（分组的容器是纵向 flex）。传 null 撤掉。 */
  function codexPlusPluginNativeSectionOrder(labels) {
    let style = document.getElementById?.(codexPlusPluginNativeSectionStyleId);
    if (!labels) {
      style?.remove();
      return;
    }
    if (!style) {
      style = document.createElement("style");
      style.id = codexPlusPluginNativeSectionStyleId;
      (document.head || document.documentElement).appendChild(style);
    }
    // 分组的 id 是「plugins-」加组名（去首尾空白、转小写、空格换连字符）。
    style.textContent = labels.map((label, order) => {
      const id = `plugins-${codexPlusPluginNativeSectionKey(label)}`.replace(/["\\]/g, "\\$&");
      return `section[id="${id}"] { order: ${order}; }`;
    }).join("\n");
  }

  function codexPlusPluginNativeMerge(original, extra, method) {
    const base = original && typeof original === "object" ? original : {};
    extra = codexPlusPluginNativeWithoutOfficial(extra, base);
    const managedIds = new Set((extra?.marketplaces || []).flatMap((market) =>
      codexPlusPluginNativeIndexMarket(market) ? (market.plugins || []).map((plugin) => plugin.id) : []));
    const markets = Array.isArray(base.marketplaces) ? base.marketplaces.map((market) => {
      if (!/^codex-plus-plugin-market-[a-f0-9]{12}$/.test(market.name || "") || !Array.isArray(market.plugins)) return market;
      // 已安装目录也会从原生本地市场返回；只消除我们自己 namespace 内同 id 的重复。
      return { ...market, plugins: market.plugins.filter((plugin) => !managedIds.has(plugin.id)) };
    }) : [];
    for (const market of extra?.marketplaces || []) {
      const existing = markets.find((item) => item.name === market.name && item.path === market.path);
      if (!existing) {
        markets.push(market);
      } else {
        const ids = new Set((existing.plugins || []).map((plugin) => plugin.id));
        existing.plugins = [...(existing.plugins || []), ...(market.plugins || []).filter((plugin) => !ids.has(plugin.id))];
      }
    }
    const result = {
      ...base, marketplaces: markets,
      marketplaceLoadErrors: [...(Array.isArray(base.marketplaceLoadErrors) ? base.marketplaceLoadErrors : []), ...(extra?.marketplaceLoadErrors || [])],
      __codexPlusIndexMerged: true,
    };
    if (method === "list-plugins") {
      result.featuredPluginIds = [...new Set([...(base.featuredPluginIds || []), ...(extra?.featuredPluginIds || [])])];
      // 有官方分组时按官方目录排版（组名、顺序、合集），并清掉原生 Featured：合集已经由分组给出。
      if (Array.isArray(extra?.codexPlusSections) && extra.codexPlusSections.length > 0) {
        result.codexPlusSections = extra.codexPlusSections;
        result.codexPlusOfficialTwins = extra.codexPlusOfficialTwins;
      }
      // 索引目录并进了插件页，分类标题才做成链接（见 codexPlusPluginCategoryOpen）。
      const listed = (extra?.marketplaces || []).some((market) => codexPlusPluginNativeIndexMarket(market) && market.plugins?.length > 0);
      document.documentElement?.toggleAttribute(codexPlusPluginCategoryAttribute, listed);
      if (!listed) codexPlusPluginNativeSectionOrder(null);
    }
    codexPlusPluginNativeRemember(result);
    return codexPlusPluginNativeArrange(result, method);
  }

  /**
   * 「公开」页签的分类每组只显示 6 个。Codex 只给云端目录（要 ChatGPT 登录）的分类标题做了「看这一类全部」的链接，
   * 本机目录的分类标题是纯文字，但同一个分类页对本机目录一样有效：这里把纯文字标题补成链接（样式见 10-style）。
   * 「个人」页签按市场分组的标题不归这里管。
   */
  const codexPlusPluginCategoryAttribute = "data-codex-plus-plugin-categories";
  const codexPlusPluginCategoryTitle = 'section[id^="plugins-"]:not([id^="plugins-marketplace-"], #plugins-created-by-you, #plugins-shared-with-you)'
    + ' > [data-slot="section-header"] > h2:not(:has(*))';

  function codexPlusPluginCategoryOpen(event) {
    if (event.button !== 0 || event.defaultPrevented || !document.documentElement.hasAttribute(codexPlusPluginCategoryAttribute)) return;
    const title = event.target instanceof Element ? event.target.closest(codexPlusPluginCategoryTitle) : null;
    if (!title) return;
    // 分类页按「plugins-<分类>」找本机目录里的这一组，分类就是这一节 id 去掉前缀。
    const category = title.closest("section").id.slice("plugins-".length);
    if (!category) return;
    window.dispatchEvent(new MessageEvent("message", {
      data: { type: "navigate-to-route", path: `/plugins?category=${encodeURIComponent(category)}` },
      source: window, origin: window.location.origin,
    }));
  }

  function installCodexPlusPluginCategoryLinks() {
    document.removeEventListener("click", window.__codexPlusPluginCategoryOpen);
    window.__codexPlusPluginCategoryOpen = codexPlusPluginCategoryOpen;
    document.addEventListener("click", codexPlusPluginCategoryOpen);
  }

  function codexPlusPluginNativeSearchTerms(value) {
    return String(value || "").trim().toLowerCase().split(/\s+/).filter(Boolean);
  }

  /** 返回 -1 表示不匹配；0 表示名称命中，排在描述 / 关键词命中（1）之前。 */
  function codexPlusPluginNativeSearchRank(plugin, terms) {
    const ui = plugin?.interface || {};
    const title = [plugin?.name, ui.displayName].filter((value) => typeof value === "string").join(" ").toLowerCase();
    const text = [title, ui.shortDescription, ui.developerName, ...(Array.isArray(plugin?.keywords) ? plugin.keywords : [])]
      .filter((value) => typeof value === "string").join(" ").toLowerCase();
    if (!terms.every((term) => text.includes(term))) return -1;
    return terms.every((term) => title.includes(term)) ? 0 : 1;
  }

  /**
   * 新版 Codex（app-server ≥ 0.147.0-alpha.6，API Key / ChatGPT 登录都走）的插件页搜索改成
   * 服务端 plugin/search，不再在 plugin/list 结果里本地过滤，所以只合并列表时搜不到索引插件（#2790）。
   * 这里按同一份索引目录本地匹配，追加到原生结果后面；原生已返回的同 id 条目不重复。
   */
  async function codexPlusPluginNativeSearchExtend(operation, original) {
    const base = original && typeof original === "object" ? original : {};
    const data = Array.isArray(base.data) ? base.data : [];
    const terms = codexPlusPluginNativeSearchTerms(operation.params?.searchTerm);
    const result = { ...base, data, nextCursor: base.nextCursor ?? null, __codexPlusIndexMerged: true };
    if (terms.length === 0) return result;
    let catalog;
    try {
      // 与插件页列表共用同一个目录缓存键，连续输入不会反复拉索引。
      catalog = await codexPlusPluginNativeCatalog({ ...operation, method: "list-plugins", params: {} });
    } catch {
      // 索引暂不可用时保留原生搜索结果；列表页会显示具体加载错误。
      return result;
    }
    const seen = new Set(data.map((entry) => entry?.plugin?.id).filter(Boolean));
    const official = new Set(data.filter((entry) => !/^codex-plus-/.test(entry?.marketplaceName || ""))
      .map((entry) => entry?.plugin?.name).filter(Boolean));
    const limit = Number.isInteger(operation.params?.limit) && operation.params.limit > 0 ? operation.params.limit : 50;
    const matches = [];
    for (const market of catalog.marketplaces || []) {
      if (!codexPlusPluginNativeIndexMarket(market)) continue;
      for (const plugin of market.plugins || []) {
        if (typeof plugin?.id !== "string" || seen.has(plugin.id)) continue;
        // 原生结果里已有同名插件：和列表一样不重复列，已装的保留。
        if (plugin.installed !== true && official.has(plugin.name)) continue;
        const rank = codexPlusPluginNativeSearchRank(plugin, terms);
        if (rank < 0) continue;
        seen.add(plugin.id);
        matches.push({ rank, entry: { marketplaceName: market.name, marketplacePath: market.path, plugin } });
      }
    }
    matches.sort((left, right) => left.rank - right.rank);
    result.data = [...data, ...matches.slice(0, limit).map((match) => match.entry)];
    return result;
  }

  async function codexPlusPluginNativeExtend(operation, original) {
    if (original?.__codexPlusIndexMerged) return original;
    if (operation.method === "search-plugins") return await codexPlusPluginNativeSearchExtend(operation, original);
    try {
      return codexPlusPluginNativeMerge(original, await codexPlusPluginNativeCatalog(operation), operation.method);
    } catch (error) {
      // 自定义市场暂时离线时保留原生列表，不能把系统插件清空。
      const fallbackPath = codexPlusIsWindowsPlatform ? "C:\\codex-plus-plugin-market" : "/codex-plus-plugin-market";
      return codexPlusPluginNativeMerge(original, { marketplaceLoadErrors: [{ marketplacePath: fallbackPath, message: error?.message || "Codex 插件市场暂时不可用" }] }, operation.method);
    }
  }

  function codexPlusPluginNativeInvalidate(result) {
    codexPlusPluginNativeCatalogs.clear();
    clearPluginMarketplaceQueryCache();
    void codexPlusPluginNativeConnectorHint(result);
  }

  // 带按钮的提示要留够时间看完再点。
  const codexPlusPluginNativeHintMs = 60_000;

  /** 本机 app-server 里这些 MCP 服务的登录状态（name → authStatus）；还没加载出来的不在表里。 */
  async function codexPlusPluginNativeMcpAuth(client, names) {
    const wanted = new Set(names);
    const found = new Map();
    let cursor = null;
    for (let page = 0; page < 10; page += 1) {
      const result = await client.sendRequest("mcpServerStatus/list", { ...(cursor ? { cursor } : {}), limit: 100, detail: "toolsAndAuthOnly" });
      for (const entry of Array.isArray(result?.data) ? result.data : []) {
        if (wanted.has(entry?.name)) found.set(entry.name, entry.authStatus);
      }
      cursor = result?.nextCursor ?? null;
      if (!cursor) break;
    }
    return found;
  }

  /**
   * 装完后的提示。后端在安装结果里带上包内声明的连接器：
   * codexPlusMcpServers 是 `.mcp.json` 里的服务名，codexPlusHostedOnly 表示只有 ChatGPT 托管的连接器、
   * 而 Codex 现在没有 ChatGPT 登录。Codex 读配置时就会加载新装插件的 MCP 服务，不用等重启。
   */
  async function codexPlusPluginNativeConnectorHint(result) {
    const servers = (Array.isArray(result?.codexPlusMcpServers) ? result.codexPlusMcpServers : [])
      .filter((name) => typeof name === "string" && name);
    if (servers.length === 0) {
      if (result?.codexPlusHostedOnly !== true) {
        showToast("插件已安装，新开聊天或重启 Codex 后加载。");
        return;
      }
      showToast("插件已安装。它的连接器由 ChatGPT 托管，要有 ChatGPT 登录才能连接；Skills 新开聊天后可用。", {
        durationMs: codexPlusPluginNativeHintMs, actionLabel: "添加 ChatGPT 账号",
        onAction: () => postJson("/manager/open", { page: "relay" }),
      });
      return;
    }
    let auth = new Map();
    try {
      auth = await codexPlusPluginNativeMcpAuth(await codexPlusAppServerClient("local"), servers);
    } catch {
      // 查不到状态时按「还没加载」处理，下面给出手动登录的入口。
    }
    const pending = servers.filter((name) => auth.get(name) === "notLoggedIn");
    if (pending.length > 0) {
      codexPlusPluginNativeOfferMcpLogin(`插件已安装。连接器 ${pending[0]} 要登录后才能使用。`, pending);
    } else if (servers.every((name) => auth.has(name))) {
      // 已经登录过，或这个服务不需要登录。
      showToast("插件已安装，新开聊天或重启 Codex 后加载。");
    } else {
      showToast(`插件已安装。连接器 ${servers.join("、")} 还没加载：重启 Codex 后，需要登录的话到「设置 → MCP」。`, {
        durationMs: codexPlusPluginNativeHintMs, actionLabel: "打开 MCP 设置",
        onAction: () => {
          // Codex 只认 source、origin 都是本页的宿主消息，缺一个就不跳转。
          window.dispatchEvent(new MessageEvent("message", {
            data: { type: "navigate-to-route", path: "/settings/mcp-settings" }, source: window, origin: window.location.origin,
          }));
        },
      });
    }
  }

  function codexPlusPluginNativeOfferMcpLogin(message, names) {
    const [name, ...rest] = names;
    showToast(message, {
      durationMs: codexPlusPluginNativeHintMs, actionLabel: "登录",
      onAction: () => codexPlusPluginNativeMcpLogin(name, rest),
    });
  }

  /**
   * 用 Codex 自带的 MCP OAuth 登录：授权页交给系统浏览器，回调由本机 app-server 接收并保存凭据，
   * 这里不经手 token。以后要重新登录走原生的「设置 → MCP」。
   */
  async function codexPlusPluginNativeMcpLogin(name, rest) {
    const client = await codexPlusAppServerClient("local");
    const watch = { subscription: null, handle: null, timer: 0, loginId: "" };
    const release = () => {
      clearTimeout(watch.timer);
      try { watch.handle?.[Symbol.dispose]?.(); } catch {}
      try { watch.subscription?.[Symbol.dispose]?.(); } catch {}
    };
    watch.subscription = client.subscribe({
      type: "notification",
      key: { hostId: "local" },
      methods: ["mcpServer/oauthLogin/completed"],
      listener: ({ params }) => {
        if (params?.name !== name) return;
        if (watch.loginId && params.loginId && params.loginId !== watch.loginId) return;
        release();
        if (params.success !== true) {
          showToast(`连接器 ${name} 的登录没有完成${params.error ? `：${params.error}` : "。"}`, { type: "warn" });
          return;
        }
        // 让已经打开的会话也重新连接这个服务。
        Promise.resolve(client.sendRequest("config/mcpServer/reload", {})).catch(() => {});
        if (rest.length > 0) codexPlusPluginNativeOfferMcpLogin(`连接器 ${name} 已登录。${rest[0]} 也要登录。`, rest);
        else showToast(`连接器 ${name} 已登录，新开聊天后可用。`, { type: "success" });
      },
    });
    watch.handle = await watch.subscription;
    // 授权页一直没点完就不再等；之后可以从「设置 → MCP」重新登录。
    watch.timer = setTimeout(release, 15 * 60_000);
    try {
      const result = await client.sendRequest("mcpServer/oauth/login", { name });
      const url = String(result?.authorizationUrl || "");
      if (!/^https?:\/\/[^\s/]/i.test(url)) throw new Error("Codex 没有返回授权地址");
      watch.loginId = typeof result.loginId === "string" ? result.loginId : "";
      // Electron 里只有 window.open 会把链接交给系统浏览器。
      window.open(url, "_blank", "noopener,noreferrer");
    } catch (error) {
      release();
      throw new Error(`连接器 ${name} 的登录没能开始：${error?.message || error}`);
    }
  }

  async function codexPlusPluginNativeManaged(operation, source) {
    if (!codexPlusPluginNativeLocal(operation)) throw new Error("Codex 插件市场目前仅支持本机安装，请切换到本机。");
    if (operation.method === "read-plugin") {
      const result = await codexPlusPluginNativeBackend(operation.method, operation.params, source);
      const summary = result?.plugin?.summary;
      if (typeof summary?.codexPlusIndexId === "string") codexPlusPluginNativeIds.set(`${source}:${operation.params.pluginName}`, summary.codexPlusIndexId);
      return result;
    }
    const key = `${source}:${operation.params.pluginName || ""}`;
    if (codexPlusPluginNativeInstallJobs.has(key)) return await codexPlusPluginNativeInstallJobs.get(key);
    const promise = (async () => {
      let id = codexPlusPluginNativeIds.get(key);
      if (!id) {
        // 未浏览列表也可能直接进入详情，先获取真实索引 id；这一步不会下载插件文件。
        const detail = await codexPlusPluginNativeBackend("read-plugin", operation.params, source);
        id = detail?.plugin?.summary?.codexPlusIndexId;
        if (typeof id !== "string" || !id) throw new Error("插件索引缺少安装标识，请刷新目录后重试。");
        codexPlusPluginNativeIds.set(key, id);
      }
      try {
        const result = await codexPlusPluginNativeBackend("install-plugin", operation.params, source);
        if (result?.authPolicy && Array.isArray(result.appsNeedingAuth)) {
          codexPlusPluginNativeInvalidate(result);
          return result;
        }
        throw new Error("正在等待插件安装结果");
      } catch (error) {
        if (!error?.timeout && !error?.outcomeUnknown && !/正在等待/.test(error?.message || "")) throw error;
      }
      // 安装已经提交，后续只能读取同一后台任务，绝不能再次提交或发给原生接口。
      const deadline = Date.now() + 10 * 60_000;
      let communicationFailures = 0;
      const outcomeUnknown = () => new Error("暂时无法确认安装结果，请刷新已安装目录查看；后台任务可能仍在运行。");
      while (Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 1500));
        let status;
        try { status = await postJson("/plugin-market/install-status", { source, id }); } catch {
          if (++communicationFailures >= 5) throw outcomeUnknown();
          continue;
        }
        if (status?.busy === true) { communicationFailures = 0; continue; }
        if (status?.busy === false && status.stage === "complete" && status.nativeResult?.authPolicy && Array.isArray(status.nativeResult.appsNeedingAuth)) {
          codexPlusPluginNativeInvalidate(status.nativeResult);
          return status.nativeResult;
        }
        if (status?.busy === false) throw new Error(status.message || "插件安装未完成，请重试。");
        if (++communicationFailures >= 5) throw outcomeUnknown();
      }
      throw outcomeUnknown();
    })().finally(() => { codexPlusPluginNativeInstallJobs.delete(key); });
    codexPlusPluginNativeInstallJobs.set(key, promise);
    return await promise;
  }

  /** 返回 null 表示原接口继续执行；返回 Promise 表示已由本层接管。 */
  function codexPlusPluginNativeInterceptClient(method, params, options, original, hostId) {
    if (!codexPluginMarketplacePatchEnabled() || options?.[codexPlusPluginNativeBypass]) return null;
    const operation = codexPlusPluginNativeOperation(method, params, hostId);
    const source = codexPlusPluginNativeSource(operation.params);
    if (source && ["read-plugin", "install-plugin"].includes(operation.method)) return codexPlusPluginNativeManaged(operation, source);
    if (!codexPlusPluginNativeListAllowed(operation)) return null;
    return (async () => {
      let result;
      try {
        result = await original({ ...(options || {}), [codexPlusPluginNativeBypass]: true });
      } catch (error) {
        if (!pluginMarketplaceRemoteAuthError(error)) throw error;
        result = operation.method === "search-plugins"
          ? { data: [], nextCursor: null }
          : { marketplaces: [], marketplaceLoadErrors: [], featuredPluginIds: [] };
      }
      return await codexPlusPluginNativeExtend(operation, result);
    })();
  }

  /**
   * RPC 传输（见 91-extension-rpc）下的一次插件请求。返回 null 表示不管；`{ answer }` 表示由本层作答、
   * 原生不执行；`{ extend }` 表示等原生回包后补索引条目，extend 返回 null 时回包原样放行。
   */
  function codexPlusPluginNativeRpcHandler(method, params, hostId) {
    if (!codexPluginMarketplacePatchEnabled()) return null;
    const operation = codexPlusPluginNativeOperation(method, params, hostId);
    const source = codexPlusPluginNativeSource(operation.params);
    if (source && ["read-plugin", "install-plugin"].includes(operation.method)) {
      return { answer: () => codexPlusPluginNativeManaged(operation, source) };
    }
    if (!codexPlusPluginNativeListAllowed(operation)) return null;
    return {
      extend(result, error) {
        if (error != null) return pluginMarketplaceRemoteAuthError(error) ? codexPlusPluginNativeExtend(operation, {}) : null;
        // 回包按序号对回请求；形状不符说明没对上，不能把目录补进别的结果里。
        const shaped = operation.method === "search-plugins" ? Array.isArray(result?.data) : Array.isArray(result?.marketplaces);
        return shaped ? codexPlusPluginNativeExtend(operation, result) : null;
      },
    };
  }

  function codexPlusPluginNativeEnvelope(message) {
    if (!message || typeof message !== "object") return null;
    if (message.type === "mcp-request" && message.request?.id != null) {
      return {
        transport: "mcp", id: message.request.id,
        key: `mcp:${message.hostId || "local"}:${message.request.id}`,
        operation: codexPlusPluginNativeOperation(message.request.method, message.request.params, message.hostId),
        request: message,
      };
    }
    if (message.type === "fetch" && message.requestId != null && typeof message.url === "string") {
      let params = message.body || {};
      if (typeof params === "string") {
        try { params = JSON.parse(params); } catch { return null; }
      }
      return {
        transport: "fetch", id: message.requestId, key: `fetch:${message.requestId}`,
        operation: codexPlusPluginNativeOperation(message.url, params, message.hostId), request: message,
      };
    }
    return null;
  }

  function codexPlusPluginNativeDeliver(data) {
    codexPlusPluginNativeSyntheticMessages.add(data);
    window.dispatchEvent(new MessageEvent("message", { data, source: window, origin: window.location.origin }));
  }

  function codexPlusPluginNativeReply(envelope, result, error) {
    if (envelope.transport === "fetch") {
      codexPlusPluginNativeDeliver({ type: "fetch-response", requestId: envelope.id, responseType: error ? "error" : "success", status: error ? 500 : 200,
        headers: { "content-type": "application/json" }, ...(error ? { error: error.message || String(error) } : { bodyJsonString: JSON.stringify(result) }) });
    } else {
      codexPlusPluginNativeDeliver({ type: "mcp-response", hostId: envelope.request.hostId || "local", message: { id: envelope.id,
        ...(error ? { error: { code: -32000, message: error.message || String(error) } } : { result }) } });
    }
  }

  function codexPlusPluginNativeInterceptOutgoing(message) {
    if (!codexPluginMarketplacePatchEnabled()) return false;
    const envelope = codexPlusPluginNativeEnvelope(message);
    if (!envelope) return false;
    const source = codexPlusPluginNativeSource(envelope.operation.params);
    if (source && ["read-plugin", "install-plugin"].includes(envelope.operation.method)) {
      void codexPlusPluginNativeManaged(envelope.operation, source).then(
        (result) => codexPlusPluginNativeReply(envelope, result),
        (error) => codexPlusPluginNativeReply(envelope, null, error));
      return true;
    }
    if (codexPlusPluginNativeListAllowed(envelope.operation)) {
      codexPlusPluginNativeRequests.set(envelope.key, envelope);
      // 只跟踪当前原生请求；超时不自发重发任何请求。
      setTimeout(() => codexPlusPluginNativeRequests.delete(envelope.key), 90_000);
    }
    return false;
  }

  function codexPlusPluginNativeSuppressForwarded(message) {
    if (!codexPluginMarketplacePatchEnabled()) return false;
    const envelope = codexPlusPluginNativeEnvelope(message);
    return Boolean(envelope && codexPlusPluginNativeSource(envelope.operation.params)
      && ["read-plugin", "install-plugin"].includes(envelope.operation.method));
  }

  function codexPlusPluginNativeInterceptIncoming(data) {
    if (!data || codexPlusPluginNativeSyntheticMessages.has(data)) return false;
    const fetchResponse = data.type === "fetch-response";
    const mcpResponse = data.type === "mcp-response";
    if (!fetchResponse && !mcpResponse) return false;
    const key = fetchResponse ? `fetch:${data.requestId}` : `mcp:${data.hostId || "local"}:${data.message?.id}`;
    const envelope = codexPlusPluginNativeRequests.get(key);
    if (!envelope) return false;
    codexPlusPluginNativeRequests.delete(key);
    // 返回 true 意味着这个回包已被我们从原生路径摘下：后续处理必须限时，
    // 超时或出错都要把原始回包原样投回渲染层，绝不能让宿主请求永久 pending
    // （issue #3098：首批 vscode 查询全部停在 fetching 就是这条链挂死的表现）。
    let delivered = false;
    const deliverOriginal = () => {
      if (delivered) return;
      delivered = true;
      codexPlusPluginNativeDeliver(data);
    };
    void (async () => {
      try {
        await Promise.race([
          (async () => {
            let payload;
            let nativeError;
            let wrapped = false;
            if (fetchResponse) {
              if (data.responseType === "error" || Number(data.status) >= 400) nativeError = data.error || data.bodyJsonString || "原生插件接口请求失败";
              try { payload = "body" in data ? data.body : JSON.parse(data.bodyJsonString || "{}"); } catch {
                deliverOriginal();
                return;
              }
              if (payload?.error) nativeError = payload.error;
              wrapped = payload && typeof payload === "object" && Object.prototype.hasOwnProperty.call(payload, "id");
            } else {
              payload = data.message?.result;
              nativeError = data.message?.error;
            }
            if (nativeError && !pluginMarketplaceRemoteAuthError(nativeError)) {
              deliverOriginal();
              return;
            }
            const original = nativeError ? {} : wrapped ? payload.result : payload;
            const merged = await codexPlusPluginNativeExtend(envelope.operation, original);
            let next;
            if (fetchResponse) {
              const response = wrapped ? { ...payload, result: merged } : merged;
              if (wrapped) delete response.error;
              next = { ...data, responseType: "success", status: 200, bodyJsonString: JSON.stringify(response) };
              delete next.body;
              delete next.error;
            } else {
              const message = { ...data.message, result: merged };
              delete message.error;
              next = { ...data, message };
            }
            if (!delivered) {
              delivered = true;
              codexPlusPluginNativeDeliver(next);
            }
          })(),
          new Promise((_resolve, reject) => {
            setTimeout(() => reject(new Error("插件市场回包处理超时")), 15_000);
          }),
        ]);
      } catch (error) {
        try { codexPlusMarkExtensionFailure("", `插件市场回包处理失败，已原样转发：${String(error?.message || error)}`); } catch {}
      } finally {
        deliverOriginal();
      }
    })();
    return true;
  }

  function installCodexPlusPluginNativeTransportListener() {
    if (window.__codexPlusPluginNativeResponseListener) window.removeEventListener("message", window.__codexPlusPluginNativeResponseListener, true);
    window.__codexPlusPluginNativeResponseListener = (event) => {
      if (event.source != null && event.source !== window) return;
      if (codexPlusPluginNativeInterceptIncoming(event.data)) event.stopImmediatePropagation();
    };
    window.addEventListener("message", window.__codexPlusPluginNativeResponseListener, true);
  }

  function ensureCodexPlusPluginNativeTransportReinjection() {
    if (!codexPluginMarketplacePatchEnabled()) return;
    const bridge = window.electronBridge;
    if (bridge?.__codexPluginMarketplaceOriginalSendMessageFromView
        && bridge.__codexPluginMarketplaceBridgePatch !== codexPluginMarketplaceUnlockVersion
        && bridge.sendMessageFromView?.__codexPlusPluginNativeTransport !== codexPlusPluginNativeVersion) {
      const original = bridge.sendMessageFromView.bind(bridge);
      const patched = (message) => codexPlusPluginNativeInterceptOutgoing(message) ? Promise.resolve() : original(message);
      patched.__codexPlusPluginNativeTransport = codexPlusPluginNativeVersion;
      bridge.sendMessageFromView = patched;
    }
    if (window.__codexPluginMarketplaceOriginalDispatchEvent
        && window.__codexPluginMarketplaceWindowEventPatch !== codexPluginMarketplaceUnlockVersion
        && window.dispatchEvent?.__codexPlusPluginNativeTransport !== codexPlusPluginNativeVersion) {
      const original = window.dispatchEvent.bind(window);
      const patched = (event) => {
        if (event?.type === "codex-message-from-view") {
          if (event.__codexForwardedViaBridge && codexPlusPluginNativeSuppressForwarded(event.detail)) return true;
          if (!event.__codexForwardedViaBridge && codexPlusPluginNativeInterceptOutgoing(event.detail)) return true;
        }
        if (event?.type === "message" && codexPlusPluginNativeInterceptIncoming(event.data)) return true;
        return original(event);
      };
      patched.__codexPlusPluginNativeTransport = codexPlusPluginNativeVersion;
      window.dispatchEvent = patched;
    }
  }

  function codexPlusNativePluginNavigationEntry() {
    // 每次重查原生导航；注入入口和第三方 Plugins 按钮不能充当原生目标。
    const destinations = Array.from(document.querySelectorAll('nav [data-sidebar-destination], aside.app-shell-left-panel nav button, nav[data-app-navigation-rail] button'));
    return destinations.find((button) => {
      if (button.closest('[data-codex-plus-ext], [data-codex-plus-rail]')) return false;
      if (button.closest(`#${codexPlusSidebarPluginMarketId}, #${codexPlusRailPluginMarketId}`)) return false;
      if (typeof isExtensionUiNode === "function" && isExtensionUiNode(button)) return false;
      if (typeof visibleElement === "function" && !visibleElement(button)) return false;
      const destination = (button.getAttribute("data-sidebar-destination") || "").trim();
      const label = (button.getAttribute("aria-label") || button.textContent || "").replace(/\s+/g, " ").trim();
      return destination === "plugins" || destination === "builtin:plugins" || /^(插件|Plugins)$/i.test(label);
    });
  }

  function openCodexPlusNativePluginMarket() {
    closeCodexPlusPage();
    clearPluginMarketplaceQueryCache();
    const native = codexPlusNativePluginNavigationEntry();
    if (native && !native.disabled && native.getAttribute("aria-disabled") !== "true") {
      native.click();
      return;
    }
    // 没有可验证的原生入口时打开管理工具，不猜测内部路由或跳转外部浏览器。
    void postJson("/manager/open", { page: "pluginMarket" });
  }
