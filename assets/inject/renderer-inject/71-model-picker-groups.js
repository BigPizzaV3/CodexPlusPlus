  /**
   * 模型选择器按供应商分组折叠。
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
   *   2. 在每组第一行前插一个自己的分组标题按钮，点它只折叠/展开这一组；
   *      行本身的选中逻辑完全交给原生（这里不监听行的点击）。行内原生的
   *      `xxx（供应商）` 标签隐藏并退出无障碍树，由我们插入的裸模型名顶替。
   *   3. 行没有被搬动，React 的 child 对账不受影响；折叠用 CSS 类隐藏，
   *      不增删原生节点，所以不会把原生列表搞崩。
   *
   * 只在"确实有多个供应商分组"时接管；单供应商（直连供应商）时完全不动原生列表。
   * 分组标题带 `data-codex-plus-ext`，扫描调度器会把它排除在自喂循环之外
   * （issue #1960）。折叠状态与供应商归属存 localStorage。
   */
  registerCodexPlusExtensionSelector('[data-codex-plus-ext="builtin-model-picker-groups"]');
  // 重注入时先与上一个实例交接：停掉它的轮询与观察器，但**不**让它清理 DOM 补丁。
  // 旧实例的清理会先把标题/标签拆掉、再等自己下一轮扫描重建，而它已经被 dispose，
  // 结果是"标题一闪一闪"。DOM 由下面这个新实例在同一个 list 上重建，幂等。
  window.__codexPlusModelPickerGroupsRuntime?.dispose?.();

  /** 折叠状态存储键，改结构要换版本号。 */
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
    /** 本次菜单打开期间是否已经做过"自动只留当前分组展开"。 */
    autoCollapsed: false,
    /** 用户手动点过标题的分组 id。 */
    touched: new Set(),
    /** 已折叠的分组 id。 */
    collapsed: new Set(),
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
          expandAll: "全部展开",
          collapseAll: "全部折叠",
          groupAria: (name, count, expanded, isDefault) => `${name}${isDefault ? "（默认供应商）" : ""}，${count} 个模型，${expanded ? "已展开" : "已折叠"}`,
          groupTitle: (name, count) => `${name} · ${count}`,
        }
      : {
          defaultBadge: "Default",
          defaultTitle: "Default provider: used when no model is pinned",
          expandAll: "Expand all",
          collapseAll: "Collapse all",
          groupAria: (name, count, expanded, isDefault) => `${name}${isDefault ? " (default provider)" : ""}, ${count} models, ${expanded ? "expanded" : "collapsed"}`,
          groupTitle: (name, count) => `${name} · ${count}`,
        };
  }

  function codexPlusModelPickerStore() {
    if (codexPlusModelPickerGroupsState.stored) return;
    codexPlusModelPickerGroupsState.stored = true;
    try {
      const raw = window.localStorage?.getItem(codexPlusModelPickerGroupsStoreKey);
      const parsed = raw ? JSON.parse(raw) : null;
      if (Array.isArray(parsed?.collapsed)) {
        parsed.collapsed.filter((id) => typeof id === "string").forEach((id) => codexPlusModelPickerGroupsState.collapsed.add(id));
      }
      if (Array.isArray(parsed?.touched)) {
        parsed.touched.filter((id) => typeof id === "string").forEach((id) => codexPlusModelPickerGroupsState.touched.add(id));
      }
      if (typeof parsed?.defaultGroupId === "string") {
        codexPlusModelPickerGroupsState.defaultGroupId = parsed.defaultGroupId;
      }
      if (parsed?.providers && typeof parsed.providers === "object") {
        for (const [key, value] of Object.entries(parsed.providers)) {
          if (typeof key === "string" && typeof value === "string" && key && value) {
            codexPlusModelPickerGroupsState.providerNames.set(key, value);
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
          collapsed: [...codexPlusModelPickerGroupsState.collapsed],
          touched: [...codexPlusModelPickerGroupsState.touched],
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
      const value = { provider, groupId };
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
      const known = exact.get(codexPlusModelPickerNormalizeLoose(identity.text))
        || (pairKey ? exact.get(pairKey) : null)
        || (!split.provider
          ? providers.get(codexPlusModelPickerNormalizeStrict(split.name))
            || providers.get(codexPlusModelPickerNormalize(split.name))
          : null);
      const groupId = known?.groupId
        || (split.provider ? `provider:${split.provider}` : defaultGroupId);
      // 显示名按行文本：有后缀就是那一家；裸名（默认成员的模型）用默认供应商的名字。
      const provider = split.provider || known?.provider || codexPlusModelPickerDefaultProviderName();
      const authoritativeGroupId = !!known?.groupId
        || (split.provider ? true : !!defaultGroupId);
      rows.push({ row, label: identity.text, name: split.name, provider, groupId, authoritativeGroupId });
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
      .map((group) => `${group.id}:${group.entries.map((entry) => entry.label).join("|")}`)
      .join("||");
  }

  /**
   * 首次打开时的默认形态：永远**至少有一组是展开的**。
   *
   *   - 用户在这之前存过折叠状态（`touched` 非空）：完全尊重，不再自动决策，
   *     否则每次打开都会把用户特意展开的组又收回去。
   *   - 当前有选中模型，且能定位到它属于哪个分组：只展开那一组，其余收起——
   *     用户下次打开选择器，看到的就是自己正在用的供应商。
   *   - 其他情况（还没有选中模型 / 定位不到）：全部保持展开。信息完整比"整齐"重要，
   *     看不到模型比看到一长串更糟。
   */
  function codexPlusModelPickerAutoCollapse(list, groups) {
    const state = codexPlusModelPickerGroupsState;
    if (state.autoCollapsed) return;
    state.autoCollapsed = true;
    if (groups.length < 2) return;
    if (state.touched.size > 0) return;
    let keep = "";
    const selected = list.querySelector("[data-model-selected]");
    if (selected) {
      const split = codexPlusModelPickerSplitLabel(codexPlusModelPickerRowLabel(selected));
      const desired = split.provider || "";
      const found = groups.find((group) => (group.name || "") === desired);
      if (found) keep = found.id;
    }
    // 没有选中模型（或定位不到所属分组）时展开默认供应商：目录里裸名发布的那些
    // 模型本来就归它，用户最可能找的就是它。
    if (!keep) keep = groups.find((group) => group.isDefault)?.id || "";
    if (!keep) return;
    let changed = false;
    for (const group of groups) {
      if (group.id === keep) {
        if (state.collapsed.delete(group.id)) changed = true;
      } else if (!state.collapsed.has(group.id)) {
        // 默认成员那一组（裸名、没有供应商后缀）跟着一起收，避免"默认 7 个 + 当前供应商"
        // 两段都散着；它有 7 个模型，收起来不影响选型。
        state.collapsed.add(group.id);
        changed = true;
      }
    }
    if (changed) codexPlusModelPickerPersist();
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
      badge = document.createElement("span");
      badge.className = "codex-plus-model-group-default";
      const count = header.querySelector(".codex-plus-model-group-count");
      header.insertBefore(badge, count || null);
    }
    if (badge.textContent !== text.defaultBadge) badge.textContent = text.defaultBadge;
    badge.title = text.defaultTitle;
  }

  function codexPlusModelPickerGroupExpanded(group) {
    return !codexPlusModelPickerGroupsState.collapsed.has(group.id);
  }

  function codexPlusModelPickerSyncGroup(list, group) {
    const text = codexPlusModelPickerText();
    const expanded = codexPlusModelPickerGroupExpanded(group);
    const header = list.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(group.id)}"]`);
    group.entries.forEach((entry) => {
      entry.row.setAttribute(codexPlusModelPickerGroupAttr, group.id);
      entry.row.classList.toggle("codex-plus-model-group-collapsed", !expanded);
    });
    if (header) {
      header.setAttribute("aria-expanded", expanded ? "true" : "false");
      header.classList.toggle("codex-plus-model-group-header-collapsed", !expanded);
      const label = header.querySelector(".codex-plus-model-group-title");
      // 标题只放名字：数量右边有徽标，标题里再来一次就是重复。
      if (label) label.textContent = group.name;
      codexPlusModelPickerSyncDefaultBadge(header, group, text);
      header.setAttribute("aria-label", text.groupAria(group.name, group.entries.length, expanded, group.isDefault));
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
      || node.classList.contains(codexPlusModelPickerNativeLabelClass);
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
    entry.row.classList.add(codexPlusModelPickerItemClass);
  }

  function codexPlusModelPickerRestoreLabel(row) {
    const content = row.querySelector("[data-menu-row-content]") || row;
    content.querySelector(`.${codexPlusModelPickerOwnLabelClass}`)?.remove();
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

  function codexPlusModelPickerToggleGroup(list, group) {
    const state = codexPlusModelPickerGroupsState;
    const expanded = codexPlusModelPickerGroupExpanded(group);
    if (expanded) state.collapsed.add(group.id);
    else state.collapsed.delete(group.id);
    state.touched.add(group.id);
    codexPlusModelPickerPersist();
    codexPlusModelPickerSyncGroup(list, group);
    codexPlusModelPickerSyncToolbar(list);
  }

  function codexPlusModelPickerSetAll(list, groups, collapsed) {
    const state = codexPlusModelPickerGroupsState;
    for (const group of groups) {
      if (collapsed) state.collapsed.add(group.id);
      else state.collapsed.delete(group.id);
      state.touched.add(group.id);
    }
    codexPlusModelPickerPersist();
    groups.forEach((group) => codexPlusModelPickerSyncGroup(list, group));
    codexPlusModelPickerSyncToolbar(list);
  }

  function codexPlusModelPickerGroupHeader(list, group, groups) {
    const text = codexPlusModelPickerText();
    const header = document.createElement("button");
    header.type = "button";
    header.className = "codex-plus-model-group-header";
    header.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
    header.setAttribute(codexPlusModelPickerGroupIdAttr, group.id);
    header.setAttribute("aria-expanded", "true");
    const chevron = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    chevron.setAttribute("viewBox", "0 0 12 12");
    chevron.setAttribute("aria-hidden", "true");
    chevron.setAttribute("class", "codex-plus-model-group-chevron");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M4 2.5 8 6 4 9.5");
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", "currentColor");
    path.setAttribute("stroke-width", "1.4");
    path.setAttribute("stroke-linecap", "round");
    path.setAttribute("stroke-linejoin", "round");
    chevron.appendChild(path);
    const title = document.createElement("span");
    title.className = "codex-plus-model-group-title";
    header.append(chevron, title);
    if (group.isDefault) {
      const badge = document.createElement("span");
      badge.className = "codex-plus-model-group-default";
      badge.textContent = text.defaultBadge;
      badge.title = text.defaultTitle;
      header.appendChild(badge);
    }
    const count = document.createElement("span");
    count.className = "codex-plus-model-group-count";
    count.textContent = String(group.entries.length);
    header.appendChild(count);
    header.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      const current = groups.find((item) => item.id === group.id);
      if (current) codexPlusModelPickerToggleGroup(list, current);
    });
    header.addEventListener("pointerdown", (event) => {
      // 别让指针事件冒到菜单外，否则菜单会被关掉。
      event.stopPropagation();
    });
    header.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") event.stopPropagation();
    });
    // 悬浮提示保留数量：表头文字只有名字，数量用右侧徽标表示。
    header.title = text.groupTitle(group.name, group.entries.length);
    return header;
  }

  function codexPlusModelPickerSyncToolbar(list) {
    const toolbar = list.querySelector(".codex-plus-model-group-toolbar");
    if (!toolbar) return;
    const groups = [...list.querySelectorAll(`[${codexPlusModelPickerGroupIdAttr}]`)]
      .map((node) => node.getAttribute(codexPlusModelPickerGroupIdAttr));
    const anyExpanded = groups.some((id) => !codexPlusModelPickerGroupsState.collapsed.has(id));
    toolbar.querySelector("button")?.setAttribute("data-all-expanded", anyExpanded ? "true" : "false");
    const label = toolbar.querySelector(".codex-plus-model-group-toolbar-label");
    if (label) label.textContent = anyExpanded ? codexPlusModelPickerText().collapseAll : codexPlusModelPickerText().expandAll;
  }

  function codexPlusModelPickerToolbar(list, groups) {
    const text = codexPlusModelPickerText();
    const toolbar = document.createElement("div");
    toolbar.className = "codex-plus-model-group-toolbar";
    toolbar.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
    const spacer = document.createElement("span");
    spacer.className = "codex-plus-model-group-toolbar-title";
    spacer.textContent = "";
    const button = document.createElement("button");
    button.type = "button";
    button.className = "codex-plus-model-group-toolbar-button";
    button.setAttribute("data-all-expanded", "true");
    const label = document.createElement("span");
    label.className = "codex-plus-model-group-toolbar-label";
    label.textContent = text.collapseAll;
    button.appendChild(label);
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      codexPlusModelPickerSetAll(list, groups, button.getAttribute("data-all-expanded") === "true");
    });
    button.addEventListener("pointerdown", (event) => event.stopPropagation());
    toolbar.append(spacer, button);
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
    list.removeAttribute("data-codex-plus-model-groups");
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
    if (!toolbar) return false;
    return groups.every((group) => {
      const header = list.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(group.id)}"]`);
      const first = group.entries[0]?.row;
      return !!header && !!first && header.nextElementSibling === first;
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
    // 只有一个供应商时不折叠：直连供应商本来就是一组，插标题只是多一层噪音。
    if (groups.length < 2) {
      if (codexPlusModelPickerGroupsState.patchedLists.has(list)) codexPlusModelPickerClear(list);
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
      groups.forEach((group) => codexPlusModelPickerSyncGroup(list, group));
      return false;
    }
    codexPlusModelPickerGroupsState.patchedLists.add(list);
    codexPlusModelPickerGroupsState.signature = signature;
    list.setAttribute("data-codex-plus-model-groups", "true");
    codexPlusModelPickerRemoveOwnNodes(list);
    groups.forEach((group) => group.entries.forEach((entry) => codexPlusModelPickerInstallLabel(entry)));
    groups.forEach((group) => {
      const first = group.entries[0]?.row;
      if (!first?.parentElement) return;
      first.parentElement.insertBefore(codexPlusModelPickerGroupHeader(list, group, groups), first);
    });
    codexPlusModelPickerAutoCollapse(list, groups);
    groups.forEach((group) => codexPlusModelPickerSyncGroup(list, group));
    const anchor = groups[0]?.entries[0]?.row;
    if (anchor?.parentElement && !list.querySelector(".codex-plus-model-group-toolbar")) {
      anchor.parentElement.insertBefore(codexPlusModelPickerToolbar(list, groups), anchor.parentElement.firstChild);
    }
    codexPlusModelPickerSyncToolbar(list);
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
    if (document.getElementById(codexPlusModelPickerGroupsStyleId)) return;
    const style = document.createElement("style");
    style.id = codexPlusModelPickerGroupsStyleId;
    style.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
    style.textContent = `
      /* 分组标题贴着原生行的左右内边距，视觉上属于同一列。 */
      .codex-plus-model-group-header{display:flex;align-items:center;gap:6px;width:100%;box-sizing:border-box;margin:2px 0 0;padding:4px 8px;border:0;border-radius:8px;background:none;color:var(--color-token-text-secondary,rgba(205,205,215,.8));font:inherit;font-size:11px;line-height:16px;font-weight:600;letter-spacing:.02em;text-align:left;cursor:pointer;user-select:none;appearance:none}
      .codex-plus-model-group-header:hover{background:var(--color-token-bg-secondary,rgba(255,255,255,.04));color:var(--color-token-text-primary,#dfdfdf)}
      .codex-plus-model-group-header:focus-visible{outline:1px solid color-mix(in srgb,var(--color-token-text-primary,#dfdfdf) 35%,transparent);outline-offset:1px}
      .codex-plus-model-group-chevron{flex:0 0 auto;width:12px;height:12px;opacity:.75;transform:rotate(90deg);transition:transform .15s ease}
      .codex-plus-model-group-header-collapsed .codex-plus-model-group-chevron{transform:rotate(0deg)}
      /* 标题撑满剩余空间，把「默认」徽标与计数挤到右端。不要用 margin-left:auto 推计数：
         那一行多出徽标时 auto 外边距会被吃掉，数字就跟在徽标后面，与其它行不对齐。 */
      .codex-plus-model-group-title{flex:1 1 auto;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
      .codex-plus-model-group-count{flex:0 0 auto;padding:0 6px;border-radius:999px;background:var(--color-token-bg-secondary,rgba(255,255,255,.06));color:var(--color-token-text-secondary,rgba(205,205,215,.75));font-size:10px;font-weight:500;font-variant-numeric:tabular-nums}
      /* 「默认」标志：默认成员的模型是裸名发布的，跟"另一家没起名的成员"看名字分不出来。
         用主题强调色描边小标签，比再加一句解释文字省地方。 */
      .codex-plus-model-group-default{flex:0 0 auto;padding:0 5px;border:1px solid color-mix(in srgb,var(--color-token-text-primary,#dfdfdf) 26%,transparent);border-radius:999px;color:var(--color-token-text-primary,#dfdfdf);font-size:10px;line-height:14px;font-weight:500;letter-spacing:.02em}
      /* 徽标紧贴计数左侧时留一点缝（默认 gap 是 6px，这里收紧一点看起来更像一组）。 */
      .codex-plus-model-group-default + .codex-plus-model-group-count{margin-left:-2px}
      .codex-plus-model-group-toolbar{display:flex;align-items:center;gap:8px;box-sizing:border-box;padding:2px 8px 4px}
      .codex-plus-model-group-toolbar-button{margin-left:auto;border:0;border-radius:6px;background:none;padding:2px 6px;color:var(--color-token-text-secondary,rgba(205,205,215,.8));font:inherit;font-size:11px;cursor:pointer;appearance:none}
      .codex-plus-model-group-toolbar-button:hover{background:var(--color-token-bg-secondary,rgba(255,255,255,.04));color:var(--color-token-text-primary,#dfdfdf)}
      /* 折叠：只隐藏行，不移动行，React 的 child 对账不受影响。 */
      [role="menuitemradio"].codex-plus-model-group-collapsed{display:none!important}
      .${codexPlusModelPickerNativeLabelClass}{display:none!important}
      .${codexPlusModelPickerOwnLabelClass}{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    `;
    document.head?.appendChild(style);
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
  function codexPlusModelPickerScan() {
    const state = codexPlusModelPickerGroupsState;
    if (state.disposed) return;
    const menu = codexPlusModelPickerMenu();
    const list = codexPlusModelPickerList(menu);
    if (!list) {
      if (state.activeList) {
        codexPlusModelPickerClear(state.activeList);
        state.activeList = null;
        state.observer?.disconnect();
        state.observer = null;
        state.autoCollapsed = false;
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
      codexPlusModelPickerSchedule(codexPlusModelPickerWatchIntervalMs);
    }, delay);
  }

  codexPlusModelPickerInstallStyle();
  codexPlusModelPickerSchedule(300);
  window.__codexPlusModelPickerGroupsRuntime = {
    version: 1,
    sync: () => codexPlusModelPickerSchedule(0),
    scan: () => codexPlusModelPickerScan(),
    /** 测试与调试用：不打开菜单也能验证分组解析。 */
    splitLabel: (text) => codexPlusModelPickerSplitLabel(text),
    /** 测试与调试用：内部状态快照。 */
    state: () => ({
      signature: codexPlusModelPickerGroupsState.signature,
      hasActiveList: !!codexPlusModelPickerGroupsState.activeList,
      autoCollapsed: codexPlusModelPickerGroupsState.autoCollapsed,
      collapsed: [...codexPlusModelPickerGroupsState.collapsed],
      touched: [...codexPlusModelPickerGroupsState.touched],
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
