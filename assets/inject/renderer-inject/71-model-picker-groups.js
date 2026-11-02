  /**
   * 模型选择器按供应商图标导航。
   *
   * 背景：按模型聚合（`RelayMode::Aggregate`）后，一份 catalog 里会混进多个成员
   * 供应商的模型。原生选择器是一维列表，聚合目录只能用 `display_name` 尾部的
   * `（供应商）` 标注归属，四十多个模型平铺下来的观感就是"供应商名堆在一起"。
   *
   * 做法（不改模型数据、不接管点击）：
   *   1. 找到原生面板（挂在 popper 里、可见的 `[data-model-picker-view]`，宫格视图跳过），
   *      从行文本尾部的 `（供应商）` 解析归属；解析不到时回退去 `/codex-model-catalog`
   *      的 `modelMetadata[slug]`（聚合目录发布的 `codex_plus_group`，或 `description`
   *      里的 `上游 · 供应商`）取权威归属。已知归属会在会话内累积，目录快照被整体替换
   *      也不会把默认成员那一组变成空标题。
   *   2. 先显示供应商图标网格，点进去只显示这一家的模型，顶部按钮返回网格；
   *      行本身的选中逻辑完全交给原生（这里不监听行的点击）。行内原生的
   *      `xxx（供应商）` 标签隐藏并退出无障碍树，由我们插入的裸模型名顶替。
   *   3. 行没有被搬动，React 的 child 对账不受影响；折叠用 CSS 类隐藏，
   *      不增删原生节点，所以不会把原生列表搞崩。
   *
   * 只在"确实有多个供应商分组"时接管；单供应商（直连供应商）时完全不动原生列表。
   * 分组标题带 `data-codex-plus-ext`，扫描调度器会把它排除在自喂循环之外
   * （issue #1960）。供应商归属存 localStorage，导航状态只在本次菜单内保留。
   */
  registerCodexPlusExtensionSelector('[data-codex-plus-ext="builtin-model-picker-groups"]');
  // 重注入时先与上一个实例交接：停掉它的轮询与观察器，但**不**让它清理 DOM 补丁。
  // 旧实例的清理会先把标题/标签拆掉、再等自己下一轮扫描重建，而它已经被 dispose，
  // 结果是"标题一闪一闪"。DOM 由下面这个新实例在同一个 list 上重建，幂等。
  window.__codexPlusModelPickerGroupsRuntime?.dispose?.();

  /** 沿用旧键中的供应商归属缓存，不再读取旧折叠状态。 */
  const codexPlusModelPickerGroupsStoreKey = "codexPlus.modelPicker.collapsedGroups.v1";
  /** 分组标题与工具条节点上的归属标记。 */
  const codexPlusModelPickerGroupsExtMark = "builtin-model-picker-groups";
  /** 样式表节点 id。 */
  const codexPlusModelPickerGroupsStyleId = "codex-plus-model-picker-groups-style";
  /** 原生行上由我们添加的类名，卸载时要摘干净。 */
  const codexPlusModelPickerItemClass = "codex-plus-model-group-item";
  /** 行内被我们替换掉的原生标签类名（恢复时要用）。 */
  const codexPlusModelPickerNativeLabelClass = "codex-plus-model-group-native-label";
  /** 我们自己插入的模型标签类名。 */
  const codexPlusModelPickerOwnLabelClass = "codex-plus-model-group-own-label";
  /** 在 Codex++ 里收藏过的模型，行尾的星标。 */
  const codexPlusModelPickerFavoriteClass = "codex-plus-model-group-favorite";
  /** 行上的分组标记属性。 */
  const codexPlusModelPickerGroupAttr = "data-codex-plus-model-group";
  /** 分组标题上的分组 id 属性。 */
  const codexPlusModelPickerGroupIdAttr = "data-codex-plus-model-group-id";
  /** 一行最多解析多少字符的供应商名；超长的一律当成模型名不拆。 */
  const codexPlusModelPickerGroupNameMaxLength = 24;
  /** 列表 DOM 变化后的合并重扫延迟。 */
  const codexPlusModelPickerRescanDelayMs = 60;
  /**
   * 菜单开着的兜底轮询间隔。
   *
   * React 在菜单里做一次重渲染就可能顺手回收我们插的标题节点，而这类删除不会产生
   * 我们关心的 childList 变更（节点带 data-codex-plus-ext）。所以只在菜单打开期间
   * 以这个间隔自检一次标题是否还在，坏了就地重建；菜单关掉立刻停。
   */
  const codexPlusModelPickerWatchIntervalMs = 250;

  const codexPlusModelPickerGroupsState = {
    disposed: false,
    /** 已打补丁的列表根节点（WeakSet，不缓存 DOM 生命周期）。 */
    patchedLists: new WeakSet(),
    /** 上一次打补丁时列表的指纹，用来跳过多余重建。 */
    signature: "",
    /** 轮询用的菜单节点；菜单关闭后置空。 */
    activeList: null,
    timer: 0,
    rescanTimer: 0,
    observer: null,
    /** 空串表示供应商网格；每次重新打开菜单都从网格开始。 */
    activeGroupId: "",
    /** 会话内累积的"归一化模型名 -> 供应商"。目录快照会被整体替换，这里做加法不丢已知归属。 */
    providerNames: new Map(),
    /** 默认成员的分组 id（聚合的"默认供应商"），用于打「默认」标志。 */
    defaultGroupId: "",
    /** 目录快照的单飞标记与冷却时间。 */
    catalogPending: false,
    catalogRetryAt: 0,
    stored: false,
  };

  function codexPlusModelPickerText() {
    const language = (document.documentElement?.lang || navigator.language || "").toLowerCase();
    return language.startsWith("zh")
      ? {
          defaultBadge: "默认",
          defaultTitle: "默认供应商：未指定模型时走这一家",
          providers: "供应商",
          back: "返回供应商列表",
          unnamed: "其他供应商",
          current: "当前使用",
          modelCount: (count) => `${count} 个模型`,
          groupAria: (name, count, current, isDefault) => `${name}${isDefault ? "（默认供应商）" : ""}，${count} 个模型${current ? "，当前使用" : ""}`,
          groupTitle: (name, count) => `${name} · ${count}`,
          favorite: "已收藏（在 Codex++ 里收藏的模型）",
        }
      : {
          defaultBadge: "Default",
          defaultTitle: "Default provider: used when no model is pinned",
          providers: "Providers",
          back: "Back to providers",
          unnamed: "Other providers",
          current: "Currently selected",
          modelCount: (count) => `${count} models`,
          groupAria: (name, count, current, isDefault) => `${name}${isDefault ? " (default provider)" : ""}, ${count} models${current ? ", currently selected" : ""}`,
          groupTitle: (name, count) => `${name} · ${count}`,
          favorite: "Favorite (starred in Codex++)",
        };
  }

  function codexPlusModelPickerStore() {
    if (codexPlusModelPickerGroupsState.stored) return;
    codexPlusModelPickerGroupsState.stored = true;
    try {
      const raw = window.localStorage?.getItem(codexPlusModelPickerGroupsStoreKey);
      const parsed = raw ? JSON.parse(raw) : null;
      if (typeof parsed?.defaultGroupId === "string") {
        codexPlusModelPickerGroupsState.defaultGroupId = parsed.defaultGroupId;
      }
      if (parsed?.providers && typeof parsed.providers === "object") {
        for (const [key, value] of Object.entries(parsed.providers)) {
          if (typeof key === "string" && typeof value === "string" && key && value) {
            codexPlusModelPickerGroupsState.providerNames.set(key, { name: value, parsed: false });
          } else if (key && typeof value?.name === "string" && value.name) {
            codexPlusModelPickerGroupsState.providerNames.set(key, { name: value.name, parsed: value.parsed === true });
          }
        }
      }
    } catch {}
  }

  function codexPlusModelPickerPersist() {
    try {
      const providers = {};
      codexPlusModelPickerGroupsState.providerNames.forEach((value, key) => {
        providers[key] = value;
      });
      window.localStorage?.setItem(
        codexPlusModelPickerGroupsStoreKey,
        JSON.stringify({
          providers,
          defaultGroupId: codexPlusModelPickerGroupsState.defaultGroupId || "",
        }),
      );
    } catch {}
  }

  /**
   * 记一条"模型名 -> 供应商"。
   *
   * `parsed` 表示这条是从行末尾 `（供应商）` 猜出来的；权威来源（目录侧
   * `codex_plus_group` / `description`）可以覆盖它，反过来不行。
   */
  function codexPlusModelPickerRememberProvider(displayName, provider, parsed) {
    const key = codexPlusModelPickerNormalize(displayName);
    if (!key || !provider) return;
    const existing = codexPlusModelPickerGroupsState.providerNames.get(key);
    if (existing && !existing.parsed && parsed) return;
    if (existing && existing.name === provider && existing.parsed === !!parsed) return;
    codexPlusModelPickerGroupsState.providerNames.set(key, { name: provider, parsed: !!parsed });
    codexPlusModelPickerGroupsState.providerDirty = true;
  }

  /**
   * 需要时补一次目录快照。
   *
   * 默认身份（`codex_plus_default_group_id` / 条目上的 `codex_plus_default`）只在
   * 目录快照里，而这份快照不一定已经被别的分片拉过（用户可能一直没打开过相关页面）。
   * 拿到后下一次扫描就会补上「默认」标志。单飞 + 冷却，避免菜单开着时反复打后端。
   */
  function codexPlusModelPickerEnsureCatalog() {
    const state = codexPlusModelPickerGroupsState;
    if (state.defaultGroupId) return;
    if (state.catalogPending) return;
    if (Date.now() < state.catalogRetryAt) return;
    const loaded = !!(codexModelCatalog?.models?.length || codexModelCatalog?.modelMetadata);
    if (loaded && typeof loadCodexModelCatalog !== "function") return;
    if (typeof loadCodexModelCatalog !== "function") return;
    state.catalogPending = true;
    state.catalogRetryAt = Date.now() + 5000;
    Promise.resolve()
      .then(() => loadCodexModelCatalog(false))
      .catch(() => {})
      .then(() => {
        state.catalogPending = false;
      });
  }

  /**
   * 默认成员（聚合里的"默认供应商"）是哪一个分组。
   *
   * 三个来源，按可靠性排序：
   *   1. `/codex-model-catalog` 的 `codex_plus_default_group_id`（聚合配置里的
   *      `default_relay_id`，最权威）；
   *   2. 目录里标了 `codex_plus_default` 的模型所属分组（老版本后端的兜底）；
   *   3. 会话内记下来的结果 —— 目录快照会被整体替换，记下来才不会丢标志。
   */
  function codexPlusModelPickerDefaultGroupId() {
    const state = codexPlusModelPickerGroupsState;
    const direct = codexModelCatalog?.codex_plus_default_group_id;
    if (typeof direct === "string" && direct.trim()) {
      if (state.defaultGroupId !== direct.trim()) {
        state.defaultGroupId = direct.trim();
        state.providerDirty = true;
      }
      return state.defaultGroupId;
    }
    const modelMetadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    if (modelMetadata && typeof modelMetadata === "object") {
      for (const entry of Object.values(modelMetadata)) {
        if (!entry || typeof entry !== "object") continue;
        if (entry.codex_plus_default !== true) continue;
        const groupId = typeof entry.codex_plus_group_id === "string" ? entry.codex_plus_group_id.trim() : "";
        if (!groupId) continue;
        const displayName = entry.displayName || entry.display_name || "";
        codexPlusModelPickerRememberProvider(displayName, providerNameOf(entry), false);
        if (state.defaultGroupId !== groupId) {
          state.defaultGroupId = groupId;
          state.providerDirty = true;
        }
        return groupId;
      }
    }
    return state.defaultGroupId || "";
  }

  /** 目录条目里的供应商展示名；没有分组键就退回 description 的「上游 · 供应商」。 */
  function providerNameOf(entry) {
    const direct = typeof entry.codex_plus_group === "string" ? entry.codex_plus_group.trim() : "";
    if (direct) return direct;
    const description = typeof entry.description === "string" ? entry.description : "";
    const separator = description.lastIndexOf(" · ");
    return separator > 0 ? description.slice(separator + 3).trim() : "";
  }

  /**
   * 原生模型选择器的面板根。
   *
   * 不绑触发按钮的属性（`data-codex-intelligence-trigger` 只是 composer 上那一个），
   * 直接找挂在 popper 里、可见的 `[data-model-picker-view]`：菜单一关它就从 DOM 里
   * 消失，天然就是"开着/关着"的判据。同时排除宫格视图——宫格下折叠没有意义。
   */
  function codexPlusModelPickerMenu() {
    const roots = document.querySelectorAll("[data-model-picker-view]");
    for (const root of roots) {
      // 实测列表视图的值是 `simple`；只排除宫格。
      if (root.getAttribute("data-model-picker-view") === "grid") continue;
      // 菜单关闭后节点会被摘掉。切到「高级/推理强度」子面板时列表只是被隐藏
      // （display:none），仍然连着 —— 那种情况不能当成关闭，否则回来时补丁已经
      // 被拆掉、要等一轮才重建，看上去就是"分组闪一下没了"。
      if (!root.isConnected) continue;
      if (!root.querySelector('[class*="_ModelList_"]')) continue;
      return root;
    }
    return null;
  }

  function codexPlusModelPickerList(menu) {
    const root = menu?.matches?.("[data-model-picker-view]") ? menu : menu?.querySelector?.("[data-model-picker-view]");
    if (!root) return null;
    const list = root.querySelector('[class*="_ModelList_"]');
    if (!list) return null;
    if (root.getAttribute("data-model-picker-view") === "grid") return null;
    return list;
  }

  /**
   * 拆 `DeepSeek-V4-Pro（DeepSeek）` 这种"展示名（供应商）"。
   * 返回 `{ name, provider }`；没有后缀或后缀不像供应商名时 `provider` 为空。
   */
  function codexPlusModelPickerSplitLabel(text) {
    const value = String(text || "").replace(/\s+/g, " ").trim();
    if (!value) return { name: "", provider: "" };
    const match = value.match(/^(.*\S)\s*[（(]([^（()）]{1,64})[)）]$/u);
    if (!match) return { name: value, provider: "" };
    const name = match[1].trim();
    const provider = match[2].trim();
    if (!name || !provider || provider.length > codexPlusModelPickerGroupNameMaxLength) {
      return { name: value, provider: "" };
    }
    // 分组标题也是我们自己插的按钮，文本里可能带 `· 计数`，别把它当成模型名。
    return { name, provider };
  }

  /** 严格归一化：只去空白 + 小写，保留 `gpt-` 前缀。 */
  function codexPlusModelPickerNormalizeStrict(value) {
    return String(value || "").replace(/\s+/g, "").trim().toLowerCase();
  }

  /** 归一化：比较时忽略大小写、`gpt-` 前缀和空白。 */
  function codexPlusModelPickerNormalize(value) {
    return codexPlusModelPickerNormalizeStrict(value).replace(/^gpt-/u, "");
  }

  /** 宽松归一化：忽略大小写、`gpt-` 前缀、空白以及 `-`/`_` 等分隔符。
   * 目录里的 slug 是 `deepseek-flash`，原生行文本是 `DeepSeek Flash`，
   * 先用严格归一化会漏掉这些别名，行就退化成 legacy 分组，同一家被切成好几段。 */
  function codexPlusModelPickerNormalizeLoose(value) {
    return String(value || "")
      .toLowerCase()
      .replace(/^gpt[\s_-]*/u, "")
      .replace(/[^\p{L}\p{N}]+/gu, "");
  }

  /**
   * 目录里的键和选择器里的名字不一定逐字相同。
   *
   * 目录条目是这个模型的完整身份（`displayName` 为 `GPT-6.1-Sol`，slug 为
   * `gpt-6.1-sol`），而选择器里的行会去掉 `gpt-` 前缀、空格也有差异（`6.1 Sol`）。
   * 所以给每个目录条目登记它所有已知写法（去掉前缀的 displayName、slug、去掉
   * `cpx-<成员>/` 之后的裸 slug），前端才可能把"行"和"目录归属"对上。
   */
  function codexPlusModelPickerCatalogAliases(displayName, entry, slug) {
    const aliases = new Set();
    const add = (value) => {
      const key = codexPlusModelPickerNormalize(value);
      if (key) aliases.add(key);
      const strict = codexPlusModelPickerNormalizeStrict(value);
      if (strict && strict !== key) aliases.add(strict);
    };
    add(displayName);
    // `GPT-6.1-Sol` 与 `6.1 Sol` 只差前缀：两种写法都登记。
    add(String(displayName || "").replace(/^gpt[-\s]*/iu, ""));
    if (typeof slug === "string" && slug) {
      add(slug);
      const bare = slug.includes("/") ? slug.slice(slug.lastIndexOf("/") + 1) : slug;
      add(bare);
    }
    return aliases;
  }

  /**
   * 权威归属：聚合目录里每个模型的 `description` 是 `上游 · 供应商`。
   * 命中后可以拿到聚合成员的真名（而不是从展示名尾部猜出来的供应商名）。
   */
  function codexPlusModelPickerCatalogProviderMap() {
    const modelMetadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    // 会话内已记下的归属先铺一层：目录快照可能被整体替换（重连 / 重新生成），
    // 只认当前快照会让默认供应商的分组名在某一刻变空。
    const map = new Map(codexPlusModelPickerGroupsState.providerNames);
    if (!modelMetadata || typeof modelMetadata !== "object") return map;
    const slugList = Array.isArray(codexModelCatalog?.models)
      ? codexModelCatalog.models.filter((slug) => typeof slug === "string" && slug)
      : [];
    const entries = Object.entries(modelMetadata);
    // `models` 是权威切片：优先按它逐个取元数据（键能对上就用真实 slug 做别名）。
    // 取不到（切片缺失 / 键不一致）时退回遍历全部元数据，保证不漏分组。
    const seenEntries = new Set();
    const registerEntry = (entry, slug) => {
      if (!entry || typeof entry !== "object" || seenEntries.has(entry)) return;
      seenEntries.add(entry);
      const displayName = entry.displayName || entry.display_name;
      if (typeof displayName !== "string" || !displayName) return;
      const provider = providerNameOf(entry);
      const groupId = typeof entry.codex_plus_group_id === "string" ? entry.codex_plus_group_id.trim() : "";
      if (!provider || !groupId) return;
      for (const key of codexPlusModelPickerCatalogAliases(displayName, entry, slug)) {
        const remembered = map.get(key);
        // 权威来源可以覆盖行文本猜出来的；已经有权威值就不动。
        if (!remembered || remembered.parsed) map.set(key, { provider, groupId, parsed: false });
      }
      codexPlusModelPickerRememberProvider(displayName, provider, false);
    };
    const bySlug = new Map(entries);
    for (const slug of slugList) registerEntry(bySlug.get(slug), slug);
    for (const [key, entry] of entries) registerEntry(entry, key);
    return map;
  }

  /**
   * 目录里每一行“完整展示名 -> 权威归属”的精确索引。
   *
   * 行的原生文本就是目录的 `displayName`（带 `（供应商）` 后缀）。拿完整文本
   * 匹配才能区分“多个成员都发布了同名上游模型”（例如三家都有
   * `DeepSeek-V4-Pro`）的情况；只按裸模型名匹配会命中目录里第一个同名成员，
   * 把另外两家的行并进错误的组。
   */
  function codexPlusModelPickerCatalogExactMap() {
    const map = new Map();
    const modelMetadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    if (!modelMetadata || typeof modelMetadata !== "object") return map;
    const register = (entry) => {
      if (!entry || typeof entry !== "object") return;
      const displayName = entry.displayName || entry.display_name;
      if (typeof displayName !== "string" || !displayName) return;
      const provider = providerNameOf(entry);
      const groupId =
        typeof entry.codex_plus_group_id === "string" ? entry.codex_plus_group_id.trim() : "";
      if (!provider || !groupId) return;
      // 在 Codex++ 里收藏过的模型（网关写的 `codex_plus_favorite`）：行尾画星标。
      const value = { provider, groupId, favorite: entry.codex_plus_favorite === true };
      // 原文键：网关关掉「模型名后带供应商」时，同名的几份只差末尾看不见的零宽字符，
      // 宽松归一化会把它们抹掉，必须先按原文比对。
      const rawKey = `raw\u0000${String(displayName).replace(/\s+/g, " ").trim()}`;
      if (!map.has(rawKey)) map.set(rawKey, value);
      const fullKey = codexPlusModelPickerNormalizeLoose(displayName);
      if (fullKey && !map.has(fullKey)) map.set(fullKey, value);
      const split = codexPlusModelPickerSplitLabel(displayName);
      if (split.name && split.provider) {
        const pairKey = `${codexPlusModelPickerNormalizeLoose(split.name)}\u0000${codexPlusModelPickerNormalizeLoose(split.provider)}`;
        if (!map.has(pairKey)) map.set(pairKey, value);
      }
    };
    const slugList = Array.isArray(codexModelCatalog?.models)
      ? codexModelCatalog.models.filter((slug) => typeof slug === "string" && slug)
      : [];
    const bySlug = new Map(Object.entries(modelMetadata));
    for (const slug of slugList) register(bySlug.get(slug));
    for (const entry of Object.values(modelMetadata)) register(entry);
    return map;
  }

  /**
   * 一行的可分组身份。
   *
   * 原生结构（实测 2026-10 Codex 桌面版）：
   *   [role=menuitemradio] > [data-menu-row-content] > span.flex-1 > span[.truncate] > span
   * "默认 / 推荐模型集"这类入口是同一个骨架多一个副标题 span，必须排除，
   * 否则会被当成一个没有供应商的模型。
   */
  function codexPlusModelPickerRowIdentity(row) {
    const content = row.querySelector("[data-menu-row-content]");
    if (!content) return null;
    const native = content.querySelector(`.${codexPlusModelPickerNativeLabelClass}`);
    const own = content.querySelector(`.${codexPlusModelPickerOwnLabelClass}`);
    const outer = content.querySelector("span.flex-1, span[class*='flex-1']");
    const inner = outer?.firstElementChild;
    // 原始标签优先；没打过补丁时就是 inner 里的第一个 span。
    const labelElement = native || own || inner?.firstElementChild || inner;
    const text = (labelElement?.textContent || "").replace(/\s+/g, " ").trim();
    // 副标题判定：只数原生 span，别把自己插进去的标签算成第二个 span。
    // 早期版本没排除自己，补丁装完后每一行都被判成"双行入口"，于是每次扫描都
    // 判定分组只剩 0 组、重建一遍 —— 表现出来就是标题一闪一闪。
    const nativeSpans = inner
      ? [...inner.children].filter((node) => !node.classList.contains(codexPlusModelPickerOwnLabelClass)
        && node.getAttribute("data-codex-plus-ext") !== codexPlusModelPickerGroupsExtMark)
      : [];
    // 已经在补丁里的行直接认账，不再重复判定结构。
    const patched = row.getAttribute("data-codex-plus-ext") === codexPlusModelPickerGroupsExtMark;
    return {
      content,
      inner,
      labelElement,
      text,
      spanCount: nativeSpans.length,
      groupable: !!text && (patched || nativeSpans.length <= 1),
    };
  }

  function codexPlusModelPickerRowLabel(row) {
    return codexPlusModelPickerRowIdentity(row)?.text || "";
  }

  /**
   * 把行切成 `{ row, label, name, provider, groupId }`。
   * 已打过补丁的行直接读我们留下的标记，避免反复解析。
   */
  function codexPlusModelPickerRows(list) {
    const providers = codexPlusModelPickerCatalogProviderMap();
    const exact = codexPlusModelPickerCatalogExactMap();
    const defaultGroupId = codexPlusModelPickerDefaultGroupId();
    const rows = [];
    const items = list.querySelectorAll('[role="menuitemradio"]');
    items.forEach((row) => {
      const identity = codexPlusModelPickerRowIdentity(row);
      if (!identity?.groupable) return;
      const split = codexPlusModelPickerSplitLabel(identity.text);
      if (split.provider) codexPlusModelPickerRememberProvider(split.name, split.provider, true);
      // 分组键：优先用「完整展示名」或「模型名 + 后缀供应商」做精确匹配。
      // 有后缀的行成员归属由后缀决定，不能再拿裸模型名去查目录——三家都发布
      // `DeepSeek-V4-Pro` 时，裸名别名只会命中目录里第一个成员，把别人的
      // 模型并进错误的组（同一家被切成好几段 / 同名分组出现多次）。
      const pairKey = split.provider
        ? `${codexPlusModelPickerNormalizeLoose(split.name)}\u0000${codexPlusModelPickerNormalizeLoose(split.provider)}`
        : "";
      const exactKnown = exact.get(`raw\u0000${identity.text}`)
        || exact.get(codexPlusModelPickerNormalizeLoose(identity.text))
        || (pairKey ? exact.get(pairKey) : null);
      const known = exactKnown
        || (!split.provider
          ? providers.get(codexPlusModelPickerNormalizeStrict(split.name))
            || providers.get(codexPlusModelPickerNormalize(split.name))
          : null);
      const groupId = known?.groupId
        || (split.provider ? `provider:${split.provider}` : defaultGroupId);
      // 显示名：整行精确命中目录时用目录里的分组名（网关写的是打过码的完整账号名，后缀可能只是
      // 供应商 id）；否则有后缀就是那一家；裸名（默认成员的模型）用默认供应商的名字。
      const provider = exactKnown?.provider || split.provider || known?.provider || codexPlusModelPickerDefaultProviderName();
      const authoritativeGroupId = !!known?.groupId
        || (split.provider ? true : !!defaultGroupId);
      rows.push({ row, label: identity.text, name: split.name, provider, groupId, authoritativeGroupId, favorite: exactKnown?.favorite === true });
    });
    return rows;
  }

  /**
   * 无后缀模型（聚合里的默认成员）该叫什么。
   *
   * 默认成员的模型保持裸名发布，所以行上没有供应商；这里从 catalog 里挑一个
   * 同样没有后缀的模型，读它的分组归属（`codex_plus_group` 或 `description`
   * 里的 `上游 · 供应商`）。挑不到就返回空串，调用方用兜底文案。
   */
  function codexPlusModelPickerDefaultProviderName() {
    const modelMetadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    if (!modelMetadata || typeof modelMetadata !== "object") return "";
    // 取"裸名模型里出现最多的那个分组"：聚合的默认成员可能有几百个模型，
    // 偶尔混进一两个别的成员的裸名条目也不该把整组改姓。
    const counts = new Map();
    for (const entry of Object.values(modelMetadata)) {
      if (!entry || typeof entry !== "object") continue;
      const displayName = entry.displayName || entry.display_name;
      if (typeof displayName !== "string" || !displayName) continue;
      if (codexPlusModelPickerSplitLabel(displayName).provider) continue;
      const direct = typeof entry.codex_plus_group === "string" ? entry.codex_plus_group.trim() : "";
      let provider = direct;
      if (!provider) {
        const description = typeof entry.description === "string" ? entry.description : "";
        const separator = description.lastIndexOf(" · ");
        if (separator > 0) provider = description.slice(separator + 3).trim();
      }
      if (!provider) continue;
      counts.set(provider, (counts.get(provider) || 0) + 1);
      codexPlusModelPickerRememberProvider(displayName, provider, false);
    }
    let best = "";
    let bestCount = 0;
    counts.forEach((count, provider) => {
      if (count > bestCount) {
        best = provider;
        bestCount = count;
      }
    });
    return best;
  }

  /** 权威 groupId 全局合组；无 ID 的旧行只做相邻回退，避免同名误合并。 */
  function codexPlusModelPickerGroups(rows, defaultProviderName = "") {
    const groups = [];
    const identified = new Map();
    let legacy = null;
    for (const entry of rows) {
      const key = entry.provider || "";
      const id = entry.authoritativeGroupId !== false && typeof entry.groupId === "string" ? entry.groupId.trim() : "";
      let group;
      if (id) {
        legacy = null;
        group = identified.get(id);
        if (!group) {
          group = { key, id, name: entry.provider || defaultProviderName, entries: [] };
          identified.set(id, group);
          groups.push(group);
        }
      } else {
        if (!legacy || legacy.key !== key) {
          legacy = { key, id: `legacy:${groups.length}:${key}`, name: entry.provider || defaultProviderName, entries: [] };
          groups.push(legacy);
        }
        group = legacy;
      }
      group.entries.push(entry);
    }
    return groups;
  }

  /**
   * 分组 id 撞车保护：不同分组名共用同一个 id（或都没有 id）时补上序号，
   * 否则折叠状态会串组。
   */
  function codexPlusModelPickerUniqueGroupIds(groups) {
    const seen = new Map();
    const defaultGroupId = codexPlusModelPickerDefaultGroupId();
    for (const group of groups) {
      // 默认成员的分组在 id 改写之前先认出来。同一组的行里，有的能查到权威分组 id
      // （裸名发布、行上没有后缀的模型），有的只有显示名，
      // 所以按"组内任意一行命中默认分组"判定，而不是只看组首那一行。
      group.isDefault = !!defaultGroupId
        && group.entries.some((entry) => entry.groupId === defaultGroupId || entry.provider === defaultGroupId);
      if (!group.name) {
        group.id = "__ungrouped__";
        continue;
      }
      const base = group.id || group.name;
      const count = (seen.get(base) || 0) + 1;
      seen.set(base, count);
      group.id = count > 1 ? `${base}#${count}` : base;
    }
    return groups;
  }

  function codexPlusModelPickerFingerprint(groups) {
    return groups
      .map((group) => `${group.id}:${group.name}:${group.entries.map((entry) => entry.label).join("|")}`)
      .join("||");
  }

  function codexPlusModelPickerSelectedRow(group) {
    return group.entries.find((entry) => entry.row.matches('[data-model-selected], [aria-checked="true"], [data-state="checked"]'))?.row;
  }

  /** 自定义供应商没有品牌资源时，用稳定的名称头像区分不同账号。 */
  function codexPlusModelPickerProviderIcon(group) {
    const name = (group.name || codexPlusModelPickerText().unnamed).split(/[·（(]/u)[0].trim();
    const words = name.match(/[\p{L}\p{N}]+/gu) || [];
    const letters = words.length > 1
      ? [Array.from(words[0])[0], Array.from(words[1])[0]]
      : Array.from(words[0] || "?").slice(0, 2);
    let hash = 0;
    for (const character of group.id) hash = (Math.imul(hash, 31) + character.codePointAt(0)) | 0;
    const colors = ["#64ad94", "#809cdb", "#c78e63", "#c681a5", "#9baa5e", "#6ba5b8"];
    return { label: letters.join("").toLocaleUpperCase(), color: colors[(hash >>> 0) % colors.length] };
  }

  function codexPlusModelPickerElement(tag, className) {
    const node = document.createElement(tag);
    node.className = className;
    node.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
    return node;
  }

  /**
   * 「默认」徽标：默认身份可能晚于标题创建才知道（目录快照刷新、切分组），
   * 所以每次同步都按 isDefault 补齐或摘掉，而不是只在建标题时决定一次。
   */
  function codexPlusModelPickerSyncDefaultBadge(header, group, text) {
    let badge = header.querySelector(".codex-plus-model-group-default");
    if (!group.isDefault) {
      badge?.remove();
      return;
    }
    if (!badge) {
      badge = codexPlusModelPickerElement("span", "codex-plus-model-group-default");
      const count = header.querySelector(".codex-plus-model-group-count");
      header.insertBefore(badge, count || null);
    }
    if (badge.textContent !== text.defaultBadge) badge.textContent = text.defaultBadge;
    badge.title = text.defaultTitle;
  }

  function codexPlusModelPickerSyncGroup(list, group) {
    const text = codexPlusModelPickerText();
    const expanded = codexPlusModelPickerGroupsState.activeGroupId === group.id;
    const header = list.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(group.id)}"]`);
    group.entries.forEach((entry) => {
      entry.row.setAttribute(codexPlusModelPickerGroupAttr, group.id);
      entry.row.classList.toggle("codex-plus-model-group-collapsed", !expanded);
    });
    if (header) {
      const current = !!codexPlusModelPickerSelectedRow(group);
      header.setAttribute("data-current", current ? "true" : "false");
      const label = header.querySelector(".codex-plus-model-group-title");
      const name = group.name || text.unnamed;
      if (label && label.textContent !== name) label.textContent = name;
      const count = header.querySelector(".codex-plus-model-group-count");
      const countText = text.modelCount(group.entries.length);
      if (count && count.textContent !== countText) count.textContent = countText;
      codexPlusModelPickerSyncDefaultBadge(header, group, text);
      header.setAttribute("aria-label", text.groupAria(name, group.entries.length, current, group.isDefault));
      header.title = `${text.groupTitle(name, group.entries.length)}${current ? ` · ${text.current}` : ""}`;
    }
  }

  /** 自己插的标签节点；原生标签留着（隐藏），这样无障碍名称仍然完整。 */
  function codexPlusModelPickerInstallLabel(entry) {
    const identity = codexPlusModelPickerRowIdentity(entry.row);
    const inner = identity?.inner;
    if (!inner) return;
    // 行本身带归属标记：补丁装完后第二次扫描能直接认出这一行，不再重新解析结构。
    entry.row.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
    const isOurs = (node) => node.classList.contains(codexPlusModelPickerOwnLabelClass)
      || node.classList.contains(codexPlusModelPickerNativeLabelClass)
      || node.classList.contains(codexPlusModelPickerFavoriteClass);
    const native = [...inner.children].find((node) => node.classList.contains(codexPlusModelPickerNativeLabelClass))
      // 第一次进来时装的是原生标签（单个 span）
      || [...inner.children].find((node) => !isOurs(node));
    if (native && !native.classList.contains(codexPlusModelPickerNativeLabelClass)) {
      native.classList.add(codexPlusModelPickerNativeLabelClass);
      native.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
      // 视觉上由我们自己的标签顶上；隐藏原节点时必须同时退出无障碍树，
      // 否则读屏会把同一行念两遍。
      native.setAttribute("aria-hidden", "true");
    }
    let own = [...inner.children].find((node) => node.classList.contains(codexPlusModelPickerOwnLabelClass));
    if (!own) {
      own = document.createElement("span");
      own.className = `${codexPlusModelPickerOwnLabelClass} truncate whitespace-nowrap`;
      own.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
      inner.insertBefore(own, native || null);
    }
    if (own.textContent !== entry.name) own.textContent = entry.name;
    // 收藏星标跟在模型名后面；取消收藏后下一轮扫描拿掉。
    let star = [...inner.children].find((node) => node.classList.contains(codexPlusModelPickerFavoriteClass));
    if (entry.favorite && !star) {
      star = document.createElement("span");
      star.className = codexPlusModelPickerFavoriteClass;
      star.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
      star.setAttribute("role", "img");
      star.setAttribute("aria-label", codexPlusModelPickerText().favorite);
      star.title = codexPlusModelPickerText().favorite;
      star.textContent = "★";
      inner.insertBefore(star, own.nextSibling);
    } else if (!entry.favorite && star) {
      star.remove();
    }
    entry.row.classList.add(codexPlusModelPickerItemClass);
  }

  function codexPlusModelPickerRestoreLabel(row) {
    const content = row.querySelector("[data-menu-row-content]") || row;
    content.querySelector(`.${codexPlusModelPickerOwnLabelClass}`)?.remove();
    content.querySelector(`.${codexPlusModelPickerFavoriteClass}`)?.remove();
    const native = content.querySelector(`.${codexPlusModelPickerNativeLabelClass}`);
    if (native) {
      native.classList.remove(codexPlusModelPickerNativeLabelClass);
      native.removeAttribute("data-codex-plus-ext");
      native.removeAttribute("aria-hidden");
    }
    row.classList.remove(codexPlusModelPickerItemClass, "codex-plus-model-group-collapsed");
    row.removeAttribute(codexPlusModelPickerGroupAttr);
    if (row.getAttribute("data-codex-plus-ext") === codexPlusModelPickerGroupsExtMark) {
      row.removeAttribute("data-codex-plus-ext");
    }
  }

  function codexPlusModelPickerNavigate(list, groupId) {
    if (!list?.isConnected) return;
    const state = codexPlusModelPickerGroupsState;
    const previous = state.activeGroupId;
    // React 可能刚换过行，点击时必须重新读取，不能使用创建按钮时的 DOM 快照。
    const groups = codexPlusModelPickerUniqueGroupIds(
      codexPlusModelPickerGroups(codexPlusModelPickerRows(list), codexPlusModelPickerDefaultProviderName()),
    );
    const group = groups.find((item) => item.id === groupId);
    state.activeGroupId = group?.id || "";
    codexPlusModelPickerSyncNavigation(list, groups);
    const focus = group
      ? codexPlusModelPickerSelectedRow(group) || group.entries[0]?.row
      : list.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(previous)}"]`)
        || list.querySelector(".codex-plus-model-group-header");
    list.scrollTop = 0;
    focus?.focus({ preventScroll: true });
  }

  function codexPlusModelPickerSyncNavigation(list, groups) {
    const state = codexPlusModelPickerGroupsState;
    const active = groups.find((group) => group.id === state.activeGroupId);
    const lostGroup = !!state.activeGroupId && !active;
    if (!active) state.activeGroupId = "";
    list.setAttribute("data-codex-plus-model-provider-view", active ? "models" : "providers");
    const grid = list.querySelector(".codex-plus-model-provider-grid");
    const toolbar = list.querySelector(".codex-plus-model-group-toolbar");
    if (grid) grid.hidden = !!active;
    if (toolbar) {
      toolbar.hidden = !active;
      if (active) {
        const text = codexPlusModelPickerText();
        const label = toolbar.querySelector(".codex-plus-model-group-toolbar-label");
        const name = active.name || text.unnamed;
        if (label && label.textContent !== name) label.textContent = name;
        if (label) label.title = name;
        const count = toolbar.querySelector(".codex-plus-model-group-count");
        const countText = text.modelCount(active.entries.length);
        if (count && count.textContent !== countText) count.textContent = countText;
        codexPlusModelPickerSyncDefaultBadge(toolbar, active, text);
      }
    }
    groups.forEach((group) => codexPlusModelPickerSyncGroup(list, group));
    if (lostGroup) grid?.querySelector("button")?.focus({ preventScroll: true });
  }

  function codexPlusModelPickerNavigationKeydown(event) {
    const list = event.currentTarget;
    if (event.isComposing) return;
    if (codexPlusModelPickerGroupsState.activeGroupId) {
      if (event.key === "ArrowLeft" || event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        codexPlusModelPickerNavigate(list, "");
      }
      return;
    }
    const buttons = [...list.querySelectorAll(".codex-plus-model-group-header")];
    if (!buttons.length) return;
    const button = event.target.closest?.(".codex-plus-model-group-header");
    const index = buttons.indexOf(button);
    const arrows = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"];
    if (!arrows.includes(event.key)) {
      if (button && (event.key === "Enter" || event.key === " ")) event.stopPropagation();
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const columns = Math.max(1, getComputedStyle(buttons[0].parentElement).gridTemplateColumns.split(" ").length);
    const step = event.key === "ArrowUp" ? -columns : event.key === "ArrowDown" ? columns : event.key === "ArrowLeft" ? -1 : 1;
    const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1
      : index < 0 ? (step < 0 ? buttons.length - 1 : 0) : Math.max(0, Math.min(buttons.length - 1, index + step));
    buttons[next].focus();
  }

  /** 原生弹层在 document 捕获 Escape，供应商子页要在更早的 window 阶段返回。 */
  function codexPlusModelPickerEscapeKeydown(event) {
    const state = codexPlusModelPickerGroupsState;
    const list = state.activeList;
    if (event.key !== "Escape" || event.isComposing || !state.activeGroupId || !list?.isConnected || !list.contains(event.target)) return;
    event.preventDefault();
    event.stopPropagation();
    codexPlusModelPickerNavigate(list, "");
  }

  function codexPlusModelPickerGroupHeader(list, group) {
    const header = codexPlusModelPickerElement("button", "codex-plus-model-group-header");
    header.type = "button";
    header.setAttribute(codexPlusModelPickerGroupIdAttr, group.id);
    const avatar = codexPlusModelPickerProviderIcon(group);
    const icon = codexPlusModelPickerElement("span", "codex-plus-model-provider-icon");
    icon.setAttribute("aria-hidden", "true");
    icon.textContent = avatar.label;
    icon.style.setProperty("--codex-plus-provider-color", avatar.color);
    const title = codexPlusModelPickerElement("span", "codex-plus-model-group-title");
    const count = codexPlusModelPickerElement("span", "codex-plus-model-group-count");
    header.append(icon, title, count);
    const groupId = group.id;
    header.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      codexPlusModelPickerNavigate(event.currentTarget.closest("[data-codex-plus-model-groups]"), groupId);
    });
    header.addEventListener("pointerdown", (event) => {
      // 别让指针事件冒到菜单外，否则菜单会被关掉。
      event.stopPropagation();
    });
    return header;
  }

  function codexPlusModelPickerToolbar() {
    const text = codexPlusModelPickerText();
    const toolbar = codexPlusModelPickerElement("div", "codex-plus-model-group-toolbar");
    const button = codexPlusModelPickerElement("button", "codex-plus-model-group-toolbar-button");
    button.type = "button";
    button.title = text.back;
    button.setAttribute("aria-label", text.back);
    const arrow = codexPlusModelPickerElement("span", "codex-plus-model-group-back-icon");
    arrow.setAttribute("aria-hidden", "true");
    arrow.textContent = "←";
    button.appendChild(arrow);
    const label = codexPlusModelPickerElement("span", "codex-plus-model-group-toolbar-label");
    const count = codexPlusModelPickerElement("span", "codex-plus-model-group-count");
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      codexPlusModelPickerNavigate(event.currentTarget.closest("[data-codex-plus-model-groups]"), "");
    });
    button.addEventListener("pointerdown", (event) => event.stopPropagation());
    button.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") event.stopPropagation();
    });
    toolbar.append(button, label, count);
    return toolbar;
  }

  function codexPlusModelPickerRemoveOwnNodes(list) {
    list.querySelectorAll(`[data-codex-plus-ext="${codexPlusModelPickerGroupsExtMark}"]`).forEach((node) => {
      if (node.closest('[role="menuitemradio"]')) return;
      node.remove();
    });
  }

  function codexPlusModelPickerClear(list) {
    if (!list) return;
    codexPlusModelPickerRemoveOwnNodes(list);
    list.querySelectorAll('[role="menuitemradio"]').forEach((row) => codexPlusModelPickerRestoreLabel(row));
    list.querySelectorAll(".codex-plus-model-provider-overview-only").forEach((node) => node.classList.remove("codex-plus-model-provider-overview-only"));
    list.removeEventListener("keydown", codexPlusModelPickerNavigationKeydown, true);
    window.removeEventListener("keydown", codexPlusModelPickerEscapeKeydown, true);
    list.removeAttribute("data-codex-plus-model-groups");
    list.removeAttribute("data-codex-plus-model-provider-view");
    codexPlusModelPickerGroupsState.activeGroupId = "";
    codexPlusModelPickerGroupsState.signature = "";
    codexPlusModelPickerGroupsState.patchedLists.delete(list);
  }

  /**
   * 分组标题与工具条是否还在原位。
   *
   * React 在菜单里做任何一次重渲染都可能顺手回收我们插进去的兄弟节点，
   * 此时指纹没变、但标题已经没了，只靠指纹判等会留下一个"点了没用"的死状态。
   */
  function codexPlusModelPickerHeadersIntact(list, groups) {
    const toolbar = list.querySelector(".codex-plus-model-group-toolbar");
    const grid = list.querySelector(".codex-plus-model-provider-grid");
    if (!toolbar || !grid || grid.children.length !== groups.length) return false;
    return groups.every((group) => {
      const header = list.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(group.id)}"]`);
      return !!header && header.parentElement === grid;
    });
  }

  /**
   * 打补丁主流程。返回是否改动了原生列表。
   */
  function codexPlusModelPickerPatch(list) {
    if (codexPlusModelPickerGroupsState.disposed) return false;
    codexPlusModelPickerStore();
    const rows = codexPlusModelPickerRows(list);
    const groups = codexPlusModelPickerUniqueGroupIds(
      codexPlusModelPickerGroups(rows, codexPlusModelPickerDefaultProviderName()),
    );
    // 只有一个供应商时直接选模型，省去没有选择意义的供应商层。
    if (groups.length < 2) {
      if (codexPlusModelPickerGroupsState.patchedLists.has(list) || list.hasAttribute("data-codex-plus-model-groups")) codexPlusModelPickerClear(list);
      else codexPlusModelPickerRemoveOwnNodes(list);
      return false;
    }
    const signature = codexPlusModelPickerFingerprint(groups);
    const patched = codexPlusModelPickerGroupsState.patchedLists.has(list);
    if (
      patched
      && signature === codexPlusModelPickerGroupsState.signature
      && list.isConnected
      && codexPlusModelPickerHeadersIntact(list, groups)
    ) {
      groups.forEach((group) => group.entries.forEach((entry) => codexPlusModelPickerInstallLabel(entry)));
      codexPlusModelPickerSyncNavigation(list, groups);
      return false;
    }
    codexPlusModelPickerGroupsState.patchedLists.add(list);
    codexPlusModelPickerGroupsState.signature = signature;
    list.setAttribute("data-codex-plus-model-groups", "true");
    const focusedGroupId = document.activeElement?.closest?.(`[${codexPlusModelPickerGroupIdAttr}]`)?.getAttribute(codexPlusModelPickerGroupIdAttr)
      || groups.find((group) => group.entries.some((entry) => entry.row.contains(document.activeElement)))?.id;
    codexPlusModelPickerRemoveOwnNodes(list);
    groups.forEach((group) => group.entries.forEach((entry) => codexPlusModelPickerInstallLabel(entry)));
    const groupedRows = new Set(rows.map((entry) => entry.row));
    list.querySelectorAll('[role="menuitemradio"], [class*="_sectionLabel_"]').forEach((node) => {
      node.classList.toggle("codex-plus-model-provider-overview-only", !groupedRows.has(node));
    });
    const grid = codexPlusModelPickerElement("div", "codex-plus-model-provider-grid");
    grid.setAttribute("role", "group");
    grid.setAttribute("aria-label", codexPlusModelPickerText().providers);
    groups.forEach((group) => grid.appendChild(codexPlusModelPickerGroupHeader(list, group)));
    const anchor = groups[0]?.entries[0]?.row;
    if (anchor?.parentElement) anchor.parentElement.insertBefore(grid, anchor);
    list.insertBefore(codexPlusModelPickerToolbar(), list.firstChild);
    list.addEventListener("keydown", codexPlusModelPickerNavigationKeydown, true);
    window.addEventListener("keydown", codexPlusModelPickerEscapeKeydown, true);
    codexPlusModelPickerSyncNavigation(list, groups);
    if (focusedGroupId && !codexPlusModelPickerGroupsState.activeGroupId) {
      grid.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(focusedGroupId)}"]`)?.focus({ preventScroll: true });
    }
    if (codexPlusModelPickerGroupsState.providerDirty) {
      codexPlusModelPickerGroupsState.providerDirty = false;
      codexPlusModelPickerPersist();
    }
    return true;
  }

  /** CSS.escape 兜底：分组 id 里可能有中文、空格与 `#`。 */
  function cssEscapeValue(value) {
    if (typeof CSS !== "undefined" && typeof CSS.escape === "function") return CSS.escape(value);
    return String(value).replace(/["\\]/g, "\\$&");
  }

  function codexPlusModelPickerInstallStyle() {
    const style = document.getElementById(codexPlusModelPickerGroupsStyleId) || document.createElement("style");
    style.id = codexPlusModelPickerGroupsStyleId;
    style.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
    style.textContent = `
      .codex-plus-model-provider-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:4px;padding:6px 4px;min-width:0}
      .codex-plus-model-group-header{position:relative;display:flex;flex-direction:column;align-items:center;justify-content:flex-start;gap:4px;min-width:0;width:100%;box-sizing:border-box;margin:0;padding:10px 4px 8px;border:1px solid transparent;border-radius:8px;background:none;color:var(--color-token-text-primary,#dfdfdf);font:inherit;font-size:11px;line-height:16px;font-weight:500;letter-spacing:0;text-align:center;cursor:pointer;user-select:none;appearance:none}
      .codex-plus-model-group-header:hover{background:var(--color-token-bg-secondary,rgba(255,255,255,.04));color:var(--color-token-text-primary,#dfdfdf)}
      .codex-plus-model-group-header:focus-visible{outline:1px solid color-mix(in srgb,var(--color-token-text-primary,#dfdfdf) 35%,transparent);outline-offset:1px}
      .codex-plus-model-provider-icon{display:grid;place-items:center;flex:none;width:40px;height:40px;margin-bottom:2px;border:1px solid color-mix(in srgb,var(--codex-plus-provider-color) 38%,transparent);border-radius:8px;background:color-mix(in srgb,var(--codex-plus-provider-color) 18%,transparent);color:var(--color-token-text-primary,#dfdfdf);font-size:16px;font-weight:600;line-height:1}
      .codex-plus-model-group-header[data-current="true"] .codex-plus-model-provider-icon{outline:2px solid var(--codex-plus-provider-color);outline-offset:2px}
      .codex-plus-model-group-header[data-current="true"]::after{content:"";position:absolute;top:10px;left:calc(50% + 14px);width:7px;height:7px;border:2px solid var(--color-token-bg-primary,#292929);border-radius:50%;background:var(--color-token-text-primary,#dfdfdf)}
      .codex-plus-model-group-title{flex:1 1 auto;min-width:0;max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
      .codex-plus-model-group-count{flex:0 0 auto;color:var(--color-token-text-secondary,rgba(205,205,215,.75));font-size:10px;font-weight:400;line-height:14px;font-variant-numeric:tabular-nums;white-space:nowrap}
      .codex-plus-model-group-default{flex:0 0 auto;padding:0 4px;border:1px solid color-mix(in srgb,var(--color-token-text-primary,#dfdfdf) 20%,transparent);border-radius:4px;color:var(--color-token-text-secondary,#aaa);font-size:9px;line-height:13px;font-weight:500;letter-spacing:0}
      .codex-plus-model-group-header .codex-plus-model-group-default{position:absolute;top:42px;left:50%;transform:translateX(-50%);background:var(--color-token-bg-primary,#292929)}
      .codex-plus-model-group-toolbar{display:flex;align-items:center;gap:6px;min-width:0;box-sizing:border-box;padding:4px 4px 8px;margin-bottom:4px;border-bottom:1px solid color-mix(in srgb,var(--color-token-text-primary,#dfdfdf) 10%,transparent)}
      .codex-plus-model-group-toolbar-label{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px;font-weight:600}
      .codex-plus-model-group-toolbar-button{display:grid;place-items:center;flex:none;width:28px;height:28px;border:0;border-radius:6px;background:none;padding:0;color:var(--color-token-text-secondary,rgba(205,205,215,.8));font:inherit;font-size:20px;line-height:1;cursor:pointer;appearance:none}
      .codex-plus-model-group-toolbar-button:hover{background:var(--color-token-bg-secondary,rgba(255,255,255,.04));color:var(--color-token-text-primary,#dfdfdf)}
      .codex-plus-model-group-toolbar-button:focus-visible{outline:1px solid currentColor;outline-offset:1px}
      .codex-plus-model-provider-grid[hidden],.codex-plus-model-group-toolbar[hidden],[data-codex-plus-model-provider-view="models"] .codex-plus-model-provider-overview-only{display:none!important}
      /* 只隐藏行，不移动行，React 的 child 对账不受影响。 */
      [role="menuitemradio"].codex-plus-model-group-collapsed{display:none!important}
      .${codexPlusModelPickerNativeLabelClass}{display:none!important}
      .${codexPlusModelPickerOwnLabelClass}{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
      .${codexPlusModelPickerFavoriteClass}{flex:none;margin-left:6px;color:#f5b94a;font-size:12px;line-height:1}
    `;
    if (!style.isConnected) document.head?.appendChild(style);
  }

  function codexPlusModelPickerNeedsRescan(records) {
    const changed = records.some((record) => {
      const nodes = [...record.addedNodes, ...record.removedNodes];
      return nodes.some((node) => node.nodeType === 1 && !node.closest?.(`[data-codex-plus-ext="${codexPlusModelPickerGroupsExtMark}"]`));
    });
    if (!changed) return false;
    const list = codexPlusModelPickerList(codexPlusModelPickerMenu());
    if (!list) return false;
    const rows = codexPlusModelPickerRows(list);
    const groups = codexPlusModelPickerUniqueGroupIds(
      codexPlusModelPickerGroups(rows, codexPlusModelPickerDefaultProviderName()),
    );
    if (groups.length < 2) return codexPlusModelPickerGroupsState.patchedLists.has(list);
    return codexPlusModelPickerFingerprint(groups) !== codexPlusModelPickerGroupsState.signature;
  }

  function codexPlusModelPickerScheduleRescan() {
    clearTimeout(codexPlusModelPickerGroupsState.rescanTimer);
    codexPlusModelPickerGroupsState.rescanTimer = setTimeout(() => {
      codexPlusModelPickerGroupsState.rescanTimer = 0;
      codexPlusModelPickerScan();
    }, codexPlusModelPickerRescanDelayMs);
  }

  function codexPlusModelPickerObserveList(list) {
    const state = codexPlusModelPickerGroupsState;
    if (state.activeList === list) return;
    if (state.activeList) codexPlusModelPickerClear(state.activeList);
    state.activeGroupId = "";
    state.activeList = list;
    state.observer?.disconnect();
    state.observer = new MutationObserver((records) => {
      if (!codexPlusModelPickerNeedsRescan(records)) return;
      codexPlusModelPickerScheduleRescan();
    });
    state.observer.observe(list, { childList: true, subtree: true });
  }

  /**
   * 一次扫描：菜单开着就确保分组已打好；关了就清理并停止轮询。
   *
   * 为什么还要轮询：React 复用行节点时不会产生 childList 变更，只靠 MutationObserver
   * 会漏掉"标签变了"这种情况。600ms 一次、且只在菜单开着时跑，开销可以忽略。
   */
  /**
   * 模型菜单开着却认不出来时（#2753：旧版 Codex 上分组不出现），记一次结构快照，
   * 好据此补上旧版的识别规则。只在页面上真有一个装着目录里模型的原生单选菜单时记，
   * 每个页面最多一次；只记标记数量和结构特征，不记模型名等文本。
   */
  function codexPlusModelPickerReportUnrecognized() {
    const state = codexPlusModelPickerGroupsState;
    if (state.unrecognizedReported) return;
    let exact = null;
    for (const menu of document.querySelectorAll('[role="menu"]')) {
      if (!visibleElement(menu) || menu.closest("[data-codex-plus-ext]")) continue;
      const radios = [...menu.querySelectorAll('[role="menuitemradio"]')];
      if (radios.length < 2) continue;
      // 先过廉价的 DOM 条件，真有候选菜单时才建目录映射（扫描每 600ms 一次）。
      exact ||= codexPlusModelPickerCatalogExactMap();
      if (!exact.size) return;
      const known = radios.filter((row) => {
        const text = (row.textContent || "").replace(/\s+/g, " ").trim();
        return exact.has(`raw\u0000${text}`) || exact.has(codexPlusModelPickerNormalizeLoose(text));
      }).length;
      if (!known) continue;
      state.unrecognizedReported = true;
      const count = (selector) => {
        try { return menu.querySelectorAll(selector).length; } catch (_) { return -1; }
      };
      const modulePrefixes = new Set();
      for (const node of [menu, ...menu.querySelectorAll("[class]")].slice(0, 400)) {
        for (const token of String(node.getAttribute("class") || "").split(/\s+/)) {
          const match = /^_([A-Za-z]+)_/.exec(token);
          if (match) modulePrefixes.add(match[1]);
        }
      }
      const dataAttributes = (node) => (node ? [...node.attributes].map((item) => item.name).filter((name) => name.startsWith("data-")) : []);
      try {
        Promise.resolve(sendCodexPlusDiagnostic("model_picker_groups_menu_unrecognized", {
          radios: radios.length,
          catalogMatches: known,
          pickerView: document.querySelectorAll("[data-model-picker-view]").length,
          modelList: count('[class*="_ModelList_"]'),
          rowContent: count("[data-menu-row-content]"),
          flex1: count("span.flex-1, span[class*='flex-1']"),
          menuData: dataAttributes(menu),
          rowData: dataAttributes(radios[0]),
          modulePrefixes: [...modulePrefixes].slice(0, 16),
        })).catch(() => {});
      } catch (_) {
        // 诊断失败不能影响原生菜单。
      }
      return;
    }
  }

  function codexPlusModelPickerScan() {
    const state = codexPlusModelPickerGroupsState;
    if (state.disposed) return;
    // 内置拓展「模型分组」停用时当作没有列表：已分组的列表还原。
    const menu = globalThis.__codexPlusBuiltinInactive?.has("model-picker-groups") ? null : codexPlusModelPickerMenu();
    const list = menu ? codexPlusModelPickerList(menu) : null;
    if (!list) {
      codexPlusModelPickerReportUnrecognized();
      if (state.activeList) {
        codexPlusModelPickerClear(state.activeList);
        state.activeList = null;
        state.observer?.disconnect();
        state.observer = null;
      }
      return;
    }
    codexPlusModelPickerEnsureCatalog();
    codexPlusModelPickerObserveList(list);
    // 列表被隐藏（用户在推理强度子面板里）时不动结构，只让轮询继续跑；
    // 回到列表立即是分组后的样子。
    if (list.getClientRects().length === 0 && codexPlusModelPickerGroupsState.patchedLists.has(list)) return;
    codexPlusModelPickerPatch(list);
  }

  function codexPlusModelPickerSchedule(delay) {
    const state = codexPlusModelPickerGroupsState;
    if (state.disposed) return;
    clearTimeout(state.timer);
    state.timer = setTimeout(() => {
      codexPlusModelPickerScan();
      // 内置拓展「模型分组」停用 / 被顶替时，扫描清理完就停下轮询；装回来时由它重新开始。
      if (!globalThis?.__codexPlusBuiltinInactive?.has("model-picker-groups")) codexPlusModelPickerSchedule(codexPlusModelPickerWatchIntervalMs);
    }, delay);
  }

  codexPlusModelPickerInstallStyle();
  // 轮询由内置拓展「模型分组」装上时启动（96-builtin-extensions.js），这里不再自启。
  window.__codexPlusModelPickerGroupsRuntime = {
    version: 2,
    sync: () => codexPlusModelPickerSchedule(0),
    scan: () => codexPlusModelPickerScan(),
    /** 测试与调试用：不打开菜单也能验证分组解析。 */
    splitLabel: (text) => codexPlusModelPickerSplitLabel(text),
    /** 测试与调试用：内部状态快照。 */
    state: () => ({
      signature: codexPlusModelPickerGroupsState.signature,
      hasActiveList: !!codexPlusModelPickerGroupsState.activeList,
      activeGroupId: codexPlusModelPickerGroupsState.activeGroupId,
      disposed: codexPlusModelPickerGroupsState.disposed,
    }),
    /** 测试与调试用：默认身份解析结果（徽标取决于它）。 */
    defaultGroup: () => ({
      id: codexPlusModelPickerGroupsState.defaultGroupId,
      resolved: codexPlusModelPickerDefaultGroupId(),
      catalogStatus: codexModelCatalog?.status || "",
      catalogModels: codexModelCatalog?.models?.length || 0,
      catalogDefaultId: codexModelCatalog?.codex_plus_default_group_id ?? null,
      metaEntries: codexModelCatalog?.modelMetadata ? Object.keys(codexModelCatalog.modelMetadata).length : 0,
      loaderVisible: typeof loadCodexModelCatalog === "function",
    }),
    /** 测试与调试用：看某一行被解析成了什么。 */
    describeRow: (row) => {
      const identity = codexPlusModelPickerRowIdentity(row);
      const rows = codexPlusModelPickerRows(row.parentElement || row);
      const entry = rows.find((item) => item.row === row);
      return {
        identity: identity
          ? { text: identity.text, spanCount: identity.spanCount, groupable: identity.groupable }
          : null,
        entry: entry ? { name: entry.name, provider: entry.provider, groupId: entry.groupId } : null,
        siblings: rows.length,
      };
    },
    groupsFor: (container = document) => {
      const list = codexPlusModelPickerList(container.querySelector?.('[data-model-picker-view]') ? container : codexPlusModelPickerMenu() || container);
      if (!list) return [];
      return codexPlusModelPickerUniqueGroupIds(
        codexPlusModelPickerGroups(codexPlusModelPickerRows(list), codexPlusModelPickerDefaultProviderName()),
      ).map((group) => ({ id: group.id, name: group.name, count: group.entries.length, isDefault: !!group.isDefault, entryGroupIds: [...new Set(group.entries.map((entry) => entry.groupId))].slice(0, 2) }));
    },
    dispose() {
      const state = codexPlusModelPickerGroupsState;
      state.disposed = true;
      clearTimeout(state.timer);
      clearTimeout(state.rescanTimer);
      state.observer?.disconnect();
      state.activeList?.removeEventListener("keydown", codexPlusModelPickerNavigationKeydown, true);
      window.removeEventListener("keydown", codexPlusModelPickerEscapeKeydown, true);
      state.observer = null;
      state.activeList = null;
      state.timer = 0;
      state.rescanTimer = 0;
      // 注意：这里不摘 DOM。注入重跑时新实例会接着用同一个 list 重建补丁，
      // 先摘后建只会闪一下；真要彻底还原（例如关闭该功能）走 releaseDom()。
    },
    releaseDom() {
      const state = codexPlusModelPickerGroupsState;
      if (state.activeList) codexPlusModelPickerClear(state.activeList);
      state.activeList = null;
      document.getElementById(codexPlusModelPickerGroupsStyleId)?.remove();
    },
  };
