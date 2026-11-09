  /**
   * 模型选择器：保留供应商图标导航，并提供可切换的折叠列表。
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
   * 图标模式仅在多供应商时接管导航；单供应商保留原生列表，也能切换视图。
   * 分组标题带 `data-codex-plus-ext`，扫描调度器会把它排除在自喂循环之外
   * （issue #1960）。供应商归属存 localStorage，导航状态只在本次菜单内保留。
   */
  registerCodexPlusExtensionSelector('[data-codex-plus-ext="builtin-model-picker-groups"]');
  // 重注入时先与上一个实例交接：停掉它的轮询与观察器，但**不**让它清理 DOM 补丁。
  // 旧实例的清理会先把标题/标签拆掉、再等自己下一轮扫描重建，而它已经被 dispose，
  // 结果是"标题一闪一闪"。DOM 由下面这个新实例在同一个 list 上重建，幂等。
  const codexPlusModelPickerDomState = window.__codexPlusModelPickerGroupsRuntime?.domState?.() || new WeakMap();
  const codexPlusModelPickerPreviousView = window.__codexPlusModelPickerGroupsRuntime?.viewState?.();
  window.__codexPlusModelPickerGroupsRuntime?.dispose?.();

  /** 沿用旧键中的供应商归属缓存，不再读取旧折叠状态。 */
  const codexPlusModelPickerGroupsStoreKey = "codexPlus.modelPicker.collapsedGroups.v1";
  const codexPlusModelPickerViewStoreKey = "codexPlus.modelPicker.viewMode.v1";
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
    viewMode: codexPlusModelPickerPreviousView?.mode === "list" ? "list" : "icons",
    viewLoaded: false,
    collapsed: new Set(codexPlusModelPickerPreviousView?.collapsed || []),
    /** 会话内累积的"归一化模型名 -> 供应商"。目录快照会被整体替换，这里做加法不丢已知归属。 */
    providerNames: new Map(),
    /** 默认成员的分组 id（聚合的"默认供应商"），用于打「默认」标志。 */
    defaultGroupId: "",
    /** 上次从目录算出的默认供应商名：快照暂缺元数据时沿用（#2860）。 */
    defaultProviderName: "",
    /** 目录快照的单飞标记与冷却时间。 */
    catalogPending: false,
    catalogRetryAt: 0,
    /** 目录还没有价格字段时重拉目录的冷却时间。 */
    priceRetryAt: 0,
    /** 最近使用区块上次渲染的指纹，没变就不重建。 */
    recentsSignature: "",
    /** 归属表在本会话里新写过，退出补丁流程时落盘。 */
    providerDirty: false,
    stored: false,
  };

  function codexPlusModelPickerCatalogChanged(options = {}) {
    const state = codexPlusModelPickerGroupsState;
    if (options.sourceChanged) {
      state.providerNames.clear();
      state.stored = true;
      state.providerDirty = false;
    }
    state.defaultGroupId = "";
    state.defaultProviderName = "";
    state.signature = "";
    state.catalogRetryAt = 0;
    if (options.sourceChanged) codexPlusModelPickerPersist();
    if (!state.disposed && state.timer) codexPlusModelPickerSchedule(0);
  }

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
          listView: "切换为列表视图",
          iconsView: "切换为图标视图",
          expandAll: "全部展开",
          collapseAll: "全部折叠",
          defaultUnavailable: "未获取到默认模型",
          recents: "最近使用",
          free: "免费",
          priceTitle: (input, output) => `价格（每百万 token）：输入 $${input} · 输出 $${output}（models.dev）`,
          pickRecent: (name, provider) => `选择 ${name}${provider ? `（${provider}）` : ""}`,
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
          listView: "Switch to list view",
          iconsView: "Switch to icon view",
          expandAll: "Expand all",
          collapseAll: "Collapse all",
          defaultUnavailable: "Default model unavailable",
          recents: "Recently used",
          free: "Free",
          priceTitle: (input, output) => `Price per 1M tokens: in $${input} · out $${output} (models.dev)`,
          pickRecent: (name, provider) => `Pick ${name}${provider ? ` (${provider})` : ""}`,
        };
  }

  function codexPlusModelPickerStore() {
    if (codexPlusModelPickerGroupsState.stored) return;
    const runtime = window.__codexPlusModelCatalogRuntime;
    if (runtime && !runtime.source) return;
    codexPlusModelPickerGroupsState.stored = true;
    if (runtime) {
      codexPlusModelPickerGroupsState.providerNames.clear();
      codexPlusModelPickerGroupsState.defaultGroupId = "";
      codexPlusModelPickerGroupsState.defaultProviderName = "";
    }
    try {
      const raw = window.localStorage?.getItem(codexPlusModelPickerGroupsStoreKey);
      const parsed = raw ? JSON.parse(raw) : null;
      if (runtime && parsed?.source !== runtime.source) return;
      if (typeof parsed?.defaultGroupId === "string") {
        codexPlusModelPickerGroupsState.defaultGroupId = parsed.defaultGroupId;
      }
      if (parsed?.providers && typeof parsed.providers === "object") {
        for (const [key, value] of Object.entries(parsed.providers)) {
          if (typeof key === "string" && typeof value === "string" && key && value) {
            codexPlusModelPickerGroupsState.providerNames.set(key, { name: value, parsed: false });
          } else if (key && typeof value?.name === "string" && value.name) {
            codexPlusModelPickerGroupsState.providerNames.set(key, {
              name: value.name,
              parsed: value.parsed === true,
              groupId: typeof value.groupId === "string" ? value.groupId : "",
              ...(typeof value.slug === "string" && value.slug ? { slug: value.slug } : {}),
              ...(typeof value.displayName === "string" && value.displayName ? { displayName: value.displayName } : {}),
            });
          }
        }
      }
    } catch {}
  }

  function codexPlusModelPickerPersist() {
    try {
      const runtime = window.__codexPlusModelCatalogRuntime;
      if (runtime && !runtime.source) return;
      const providers = {};
      codexPlusModelPickerGroupsState.providerNames.forEach((value, key) => {
        providers[key] = value;
      });
      window.localStorage?.setItem(
        codexPlusModelPickerGroupsStoreKey,
        JSON.stringify({
          providers,
          defaultGroupId: codexPlusModelPickerGroupsState.defaultGroupId || "",
          ...(runtime ? { source: runtime.source } : {}),
        }),
      );
    } catch {}
  }

  function codexPlusModelPickerLoadView() {
    const state = codexPlusModelPickerGroupsState;
    if (state.viewLoaded) return;
    state.viewLoaded = true;
    try {
      const mode = window.localStorage?.getItem(codexPlusModelPickerViewStoreKey);
      if (mode === "icons" || mode === "list") state.viewMode = mode;
    } catch {}
  }

  /** 只保存自己覆盖的属性；WeakMap 随重注入交接，不保留已关闭菜单。 */
  function codexPlusModelPickerRememberAttribute(node, name, value) {
    let saved = codexPlusModelPickerDomState.get(node);
    if (!saved) {
      saved = new Map();
      codexPlusModelPickerDomState.set(node, saved);
    }
    if (!saved.has(name)) saved.set(name, node.getAttribute(name));
    if (node.getAttribute(name) !== value) node.setAttribute(name, value);
  }

  function codexPlusModelPickerRestoreAttribute(node, name) {
    const saved = codexPlusModelPickerDomState.get(node);
    if (!saved?.has(name)) return;
    const value = saved.get(name);
    if (value === null) node.removeAttribute(name);
    else node.setAttribute(name, value);
    saved.delete(name);
  }

  /** 完整默认身份只读有效配置字段；缺失时不按供应商或目录顺序猜测。 */
  function codexPlusModelPickerDefaultModelName() {
    const model = typeof codexModelCatalog?.effective_config_model === "string"
      ? codexModelCatalog.effective_config_model.trim() : "";
    if (!model) return codexPlusModelPickerText().defaultUnavailable;
    const metadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    const entry = metadata?.[model];
    const display = entry?.displayName || entry?.display_name;
    return typeof display === "string" && display.trim() ? display.trim() : model;
  }

  function codexPlusModelPickerDefaultRows(list) {
    return [...list.querySelectorAll('[role="menuitemradio"]')].filter((row) => {
      const identity = codexPlusModelPickerRowIdentity(row);
      return identity?.spanCount > 1 && /^(默认|default)$/iu.test(identity.text);
    });
  }

  function codexPlusModelPickerSyncDefaultRow(list) {
    const name = codexPlusModelPickerDefaultModelName();
    codexPlusModelPickerDefaultRows(list).forEach((row) => {
      const identity = codexPlusModelPickerRowIdentity(row);
      const inner = identity?.inner;
      if (!inner) return;
      row.setAttribute("data-codex-plus-model-default-row", "true");
      inner.setAttribute("data-codex-plus-model-default-content", "true");
      [...inner.children].forEach((node) => {
        if (node === identity.labelElement || node.getAttribute("data-codex-plus-ext") === codexPlusModelPickerGroupsExtMark) return;
        node.setAttribute("data-codex-plus-model-default-note", "true");
        codexPlusModelPickerRememberAttribute(node, "aria-hidden", "true");
      });
      let label = inner.querySelector(".codex-plus-model-default-name");
      if (!label) {
        label = codexPlusModelPickerElement("span", "codex-plus-model-default-name");
        inner.appendChild(label);
      }
      if (label.textContent !== name) label.textContent = name;
      label.title = name;
    });
  }

  function codexPlusModelPickerRestoreDefaultRow(list) {
    list.querySelectorAll(".codex-plus-model-default-name").forEach((node) => node.remove());
    list.querySelectorAll("[data-codex-plus-model-default-note]").forEach((node) => {
      codexPlusModelPickerRestoreAttribute(node, "aria-hidden");
      node.removeAttribute("data-codex-plus-model-default-note");
    });
    list.querySelectorAll("[data-codex-plus-model-default-content]").forEach((node) => node.removeAttribute("data-codex-plus-model-default-content"));
    list.querySelectorAll("[data-codex-plus-model-default-row]").forEach((node) => node.removeAttribute("data-codex-plus-model-default-row"));
  }

  function codexPlusModelPickerHeading(list) {
    return [...list.querySelectorAll('[data-menu-section-label], [class*="_sectionLabel_"], [class^="sectionLabel-"], [class*=" sectionLabel-"]')]
      .find((node) => !node.closest('[role="menuitemradio"], [data-codex-plus-ext]')) || null;
  }

  /** Lucide 的标准路径；注入环境没有模块加载器，不额外下载图标包。 */
  function codexPlusModelPickerActionIcon(action) {
    const paths = {
      list: ["M8 6h13", "M8 12h13", "M8 18h13", "M3 6h.01", "M3 12h.01", "M3 18h.01"],
      icons: ["M3 3h7v7H3z", "M14 3h7v7h-7z", "M3 14h7v7H3z", "M14 14h7v7h-7z"],
      expand: ["m7 10 5-5 5 5", "m7 14 5 5 5-5"],
      collapse: ["m7 5 5 5 5-5", "m7 19 5-5 5 5"],
      chevron: ["m9 6 6 6-6 6"],
    };
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("width", "16");
    svg.setAttribute("height", "16");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "1.75");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark);
    for (const value of paths[action] || []) {
      const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
      path.setAttribute("d", value);
      svg.appendChild(path);
    }
    return svg;
  }

  function codexPlusModelPickerViewAction(list, action) {
    const state = codexPlusModelPickerGroupsState;
    if (!list?.isConnected) return;
    if (action === "list" || action === "icons") {
      state.viewMode = action;
      try { window.localStorage?.setItem(codexPlusModelPickerViewStoreKey, action); } catch {}
      codexPlusModelPickerClear(list);
    } else {
      const groups = codexPlusModelPickerUniqueGroupIds(
        codexPlusModelPickerGroups(codexPlusModelPickerRows(list), codexPlusModelPickerDefaultProviderName()),
      );
      groups.forEach((group) => {
        if (action === "collapse") state.collapsed.add(group.id);
        else state.collapsed.delete(group.id);
      });
    }
    codexPlusModelPickerPatch(list);
    list.querySelector(`[data-codex-plus-model-action="${action === "list" ? "icons" : action === "icons" ? "list" : action}"]`)?.focus({ preventScroll: true });
  }

  function codexPlusModelPickerViewControls(list) {
    const heading = codexPlusModelPickerHeading(list);
    if (!heading) return;
    const state = codexPlusModelPickerGroupsState;
    const text = codexPlusModelPickerText();
    heading.setAttribute("data-codex-plus-model-heading", "true");
    const controls = codexPlusModelPickerElement("span", "codex-plus-model-view-controls");
    const actions = state.viewMode === "list"
      ? [["expand", text.expandAll], ["collapse", text.collapseAll], ["icons", text.iconsView]]
      : [["list", text.listView]];
    for (const [action, label] of actions) {
      const button = codexPlusModelPickerElement("button", "codex-plus-model-view-button");
      button.type = "button";
      button.title = label;
      button.setAttribute("aria-label", label);
      button.setAttribute("data-codex-plus-model-action", action);
      button.appendChild(codexPlusModelPickerActionIcon(action));
      button.addEventListener("pointerdown", (event) => event.stopPropagation());
      button.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") event.stopPropagation();
      });
      button.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        codexPlusModelPickerViewAction(event.currentTarget.closest("[data-codex-plus-model-groups]"), action);
      });
      controls.appendChild(button);
    }
    heading.appendChild(controls);
  }

  /** 所有直接子项共享连续顺序；只写 CSS，不搬动 React 管理的行。 */
  function codexPlusModelPickerOrderPlan(list, groups) {
    const members = new Set(groups.flatMap((group) => group.entries.map((entry) => entry.row)));
    const plan = [...list.children].filter((node) => !members.has(node)
      && node.getAttribute("data-codex-plus-ext") !== codexPlusModelPickerGroupsExtMark);
    // 「最近使用」排在原生前缀（标题/默认入口）之后、第一组标题之前。
    const recents = list.querySelector(`:scope > .${codexPlusModelPickerRecentsClass}`);
    if (recents) plan.push(recents);
    for (const group of groups) {
      const header = list.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(group.id)}"]`);
      if (header) plan.push(header);
      group.entries.forEach((entry) => plan.push(entry.row));
    }
    return plan;
  }

  function codexPlusModelPickerApplyOrder(list, groups) {
    list.setAttribute("data-codex-plus-model-ordering", "true");
    codexPlusModelPickerOrderPlan(list, groups).forEach((node, index) => {
      let saved = codexPlusModelPickerDomState.get(node);
      if (!saved) {
        saved = new Map();
        codexPlusModelPickerDomState.set(node, saved);
      }
      const property = "--codex-plus-model-order";
      if (!saved.has(property)) saved.set(property, {
        value: node.style.getPropertyValue(property),
        priority: node.style.getPropertyPriority(property),
        hadStyle: node.hasAttribute("style"),
      });
      const order = String(index + 1);
      node.setAttribute("data-codex-plus-model-order", order);
      if (node.style.getPropertyValue(property) !== order) node.style.setProperty(property, order);
    });
  }

  function codexPlusModelPickerRestoreOrder(list) {
    list.querySelectorAll('[data-codex-plus-model-order], [style*="--codex-plus-model-order"]').forEach((node) => {
      const saved = codexPlusModelPickerDomState.get(node);
      const property = "--codex-plus-model-order";
      const original = saved?.get(property);
      if (original) {
        if (original.value) node.style.setProperty(property, original.value, original.priority);
        else node.style.removeProperty(property);
        if (!original.hadStyle && !node.getAttribute("style")) node.removeAttribute("style");
        saved.delete(property);
      }
      node.removeAttribute("data-codex-plus-model-order");
    });
    list.removeAttribute("data-codex-plus-model-ordering");
  }

  function codexPlusModelPickerSyncList(list, groups) {
    codexPlusModelPickerApplyOrder(list, groups);
    groups.forEach((group) => codexPlusModelPickerSyncGroup(list, group));
    codexPlusModelPickerSyncDefaultRow(list);
  }

  /**
   * 记一条"模型名 -> 供应商"。
   *
   * `parsed` 表示这条是从行末尾 `（供应商）` 猜出来的；权威来源（目录侧
   * `codex_plus_group` / `description`）可以覆盖它，反过来不行。
   */
  function codexPlusModelPickerRememberProvider(displayName, provider, parsed, groupId = "", slug = "") {
    const key = codexPlusModelPickerNormalize(displayName);
    if (!key || !provider) return;
    if (!parsed && groupId && slug) {
      const modelKey = `model\u0000${slug}`;
      const previous = codexPlusModelPickerGroupsState.providerNames.get(modelKey);
      if (!previous || previous.name !== provider || previous.groupId !== groupId || previous.displayName !== displayName) {
        codexPlusModelPickerGroupsState.providerNames.set(modelKey, {
          name: provider, parsed: false, groupId, slug, displayName,
        });
        codexPlusModelPickerGroupsState.providerDirty = true;
      }
    }
    const existing = codexPlusModelPickerGroupsState.providerNames.get(key);
    if (existing && !existing.parsed && parsed) return;
    const id = groupId || (existing?.name === provider ? existing.groupId || "" : "");
    const model = slug || existing?.slug || "";
    if (existing && existing.name === provider && existing.parsed === !!parsed && (existing.groupId || "") === id && (existing.slug || "") === model) return;
    codexPlusModelPickerGroupsState.providerNames.set(key, { name: provider, parsed: !!parsed, groupId: id, slug: model });
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
    if (state.catalogPending) return;
    if (Date.now() < state.catalogRetryAt) return;
    const loaded = !!(codexModelCatalog?.models?.length || codexModelCatalog?.modelMetadata);
    if (loaded && typeof loadCodexModelCatalog !== "function") return;
    if (typeof loadCodexModelCatalog !== "function") return;
    state.catalogPending = true;
    state.catalogRetryAt = Date.now() + 10000;
    Promise.resolve()
      .then(() => loadCodexModelCatalog(false))
      .catch(() => {})
      .then(() => {
        state.catalogPending = false;
      });
  }

  /**
   * 目录还没有价格字段时强刷一次目录快照。
   *
   * 后端只在本地有价格表缓存时下发 `codex_plus_price`；首次缺缓存时它后台预热，
   * 这里隔一段时间强刷一次目录（绕过 10s 快照缓存），价格表就绪后行尾的
   * 价格条自然出现。单飞 + 15s 冷却，避免菜单开着时反复打后端。
   */
  function codexPlusModelPickerEnsurePrices() {
    const state = codexPlusModelPickerGroupsState;
    if (Date.now() < state.priceRetryAt) return;
    if (typeof loadCodexModelCatalog !== "function") return;
    if (codexModelCatalog?.status !== "ok") return;
    const modelMetadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    const hasPrice = modelMetadata && typeof modelMetadata === "object"
      && Object.values(modelMetadata).some((entry) => entry && typeof entry === "object" && entry.codex_plus_price);
    if (hasPrice) return;
    state.priceRetryAt = Date.now() + 15000;
    Promise.resolve()
      .then(() => loadCodexModelCatalog(true))
      .catch(() => {});
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
    // 成功目录的显式空值代表取消默认成员；缺字段或请求失败仍沿用上次结果。
    if (typeof direct === "string"
        && (direct.trim() || ["ok", "not_applied"].includes(codexModelCatalog?.status))) {
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
        codexPlusModelPickerRememberProvider(displayName, providerNameOf(entry), false, groupId);
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
      if (!codexPlusModelPickerListIn(root)) continue;
      return root;
    }
    return null;
  }

  /**
   * 面板根里的模型列表。按结构找、不认类名（#2893）：模型行（`role="menuitemradio"`）的共同父节点
   * 就是列表。实测 26.1029：菜单里 41 个模型行都直接挂在列表下，列表外（含推理强度子面板）没有别的
   * 模型行；按结构找到的和按类名找到的是同一个节点。`role` 是 Codex 做菜单交互和无障碍要用的，
   * 比 CSS Modules 的哈希类名稳定——26.1029 就把 `_ModelList_哈希` 改成了 `ModelList-哈希`。
   *
   * 还没有模型行时（列表加载中）回落到类名，新旧两种格式都认；新格式按类名开头匹配，不会误中
   * `ModelListMotion-`。共同父节点退到面板根本身时（行分散在几个面板里）说明结构变了，同样回落。
   */
  function codexPlusModelPickerListIn(root) {
    const rows = [...root.querySelectorAll('[role="menuitemradio"]')];
    let list = rows[0]?.parentElement || null;
    while (list && list !== root && !rows.every((row) => list.contains(row))) list = list.parentElement;
    if (list && list !== root) return list;
    return root.querySelector('[class*="_ModelList_"], [class^="ModelList-"], [class*=" ModelList-"]');
  }

  function codexPlusModelPickerList(menu) {
    const root = menu?.matches?.("[data-model-picker-view]") ? menu : menu?.querySelector?.("[data-model-picker-view]");
    if (!root) return null;
    if (root.getAttribute("data-model-picker-view") === "grid") return null;
    return codexPlusModelPickerListIn(root);
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

  function codexPlusModelPickerNativeLabelKey(value) {
    return `label\u0000${String(value || "").toLowerCase().replace(/^gpt[\s_-]*/u, "")
      .replace(/[^\p{L}\p{N}\u200b-\u200d\u2060-\u206f]+/gu, "")}`;
  }

  function codexPlusModelPickerCatalogEntries() {
    const metadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    if (!metadata || typeof metadata !== "object") return [];
    return Array.isArray(codexModelCatalog?.models)
      ? codexModelCatalog.models.map((slug) => [slug, metadata[slug]])
      : Object.entries(metadata);
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

  function codexPlusModelPickerCanRestoreProvider(entry, remembered) {
    if (!entry || typeof entry !== "object") return true;
    if (Object.hasOwn(entry, "codex_plus_group_id") || Object.hasOwn(entry, "codex_plus_group")) return false;
    const displayName = entry.displayName || entry.display_name;
    const provider = providerNameOf(entry);
    return !!remembered?.displayName && displayName === remembered.displayName
      && (!provider || provider === remembered.name);
  }

  /**
   * 权威归属：聚合目录里每个模型的 `description` 是 `上游 · 供应商`。
   * 命中后可以拿到聚合成员的真名（而不是从展示名尾部猜出来的供应商名）。
   */
  function codexPlusModelPickerCatalogProviderMap() {
    const modelMetadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    const models = ["ok", "not_applied", "not_configured"].includes(codexModelCatalog?.status)
      && Array.isArray(codexModelCatalog?.models) ? codexModelCatalog.models : null;
    const map = new Map();
    // 记下的归属只补当前快照缺的键：目录快照会被整体替换（重连 / 拉取超时返回空元数据），
    // 只认当前快照会让裸名模型在某一刻全部失去归属，落进一串空名分组（#2860）。
    // 缓存里的形状是 `{ name, parsed, groupId }`，这里换成读取方要的 `{ provider, groupId }`；
    // 从行尾后缀猜的（parsed）不补：那是别家同名模型的归属，套给裸名行会串组。
    const fillFromMemory = () => {
      const complete = codexModelCatalog?.status === "ok" && modelMetadata
        && Array.isArray(codexModelCatalog?.models)
        && codexModelCatalog.models.every((slug) => modelMetadata?.[slug]
          && (Object.hasOwn(modelMetadata[slug], "codex_plus_group_id")
            || Object.hasOwn(modelMetadata[slug], "codex_plus_group")));
      if (complete) return map;
      const rememberedAliases = new Map();
      codexPlusModelPickerGroupsState.providerNames.forEach((value, key) => {
        if (!key.startsWith("model\u0000") || !value?.displayName || !value.groupId || value.parsed) return;
        if (models && !models.includes(value.slug)) return;
        if (!codexPlusModelPickerCanRestoreProvider(modelMetadata?.[value.slug], value)) return;
        for (const alias of codexPlusModelPickerCatalogAliases(value.displayName, {}, value.slug)) {
          if (!rememberedAliases.has(alias)) rememberedAliases.set(alias, value);
          else if (rememberedAliases.get(alias)?.groupId !== value.groupId) rememberedAliases.set(alias, null);
        }
      });
      rememberedAliases.forEach((value, key) => {
        if (!map.has(key)) map.set(key, value ? { provider: value.name, groupId: value.groupId, parsed: false } : null);
        else if (!value || map.get(key)?.groupId !== value.groupId) map.set(key, null);
      });
      codexPlusModelPickerGroupsState.providerNames.forEach((value, key) => {
        if (key.startsWith("model\u0000") || map.has(key) || !value?.name || value.parsed) return;
        if (models && value.slug && !models.includes(value.slug)) return;
        const current = value.slug && codexPlusModelPickerGroupsState.providerNames.get(`model\u0000${value.slug}`);
        if (value.slug && !codexPlusModelPickerCanRestoreProvider(modelMetadata?.[value.slug], current)) return;
        if (current?.displayName && !codexPlusModelPickerCatalogAliases(current.displayName, {}, value.slug).has(key)) return;
        map.set(key, { provider: value.name, groupId: value.groupId || "", parsed: false });
      });
      return map;
    };
    if (!modelMetadata || typeof modelMetadata !== "object") return fillFromMemory();
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
        if (!map.has(key) || remembered?.groupId === groupId) map.set(key, { provider, groupId, parsed: false });
        else map.set(key, null);
      }
      codexPlusModelPickerRememberProvider(displayName, provider, false, groupId, slug);
    };
    const bySlug = new Map(entries);
    for (const slug of slugList) registerEntry(bySlug.get(slug), slug);
    if (!Array.isArray(codexModelCatalog?.models)) {
      for (const [key, entry] of entries) registerEntry(entry, key);
    }
    return fillFromMemory();
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
    const candidate = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    const modelMetadata = candidate && typeof candidate === "object" ? candidate : {};
    const models = ["ok", "not_applied", "not_configured"].includes(codexModelCatalog?.status)
      && Array.isArray(codexModelCatalog?.models) ? codexModelCatalog.models : null;
    const setUnique = (key, value) => {
      if (!key) return;
      if (!map.has(key)) map.set(key, value);
      else if (map.get(key)?.slug !== value.slug) map.set(key, null);
    };
    const register = (entry, slug) => {
      if (!entry || typeof entry !== "object") return;
      const displayName = entry.displayName || entry.display_name;
      if (typeof displayName !== "string" || !displayName) return;
      const provider = providerNameOf(entry);
      const groupId =
        typeof entry.codex_plus_group_id === "string" ? entry.codex_plus_group_id.trim() : "";
      if (!provider || !groupId) return;
      // 在 Codex++ 里收藏过的模型（网关写的 `codex_plus_favorite`）：行尾画星标。
      // slug 留给图标识别：原生行会去掉 `GPT-` 前缀，只看行文本认不出 OpenAI。
      const value = {
        provider, groupId, favorite: entry.codex_plus_favorite === true, price: codexPlusModelPickerPrice(entry),
        slug: typeof slug === "string" ? slug : String(displayName), displayName,
      };
      map.set(`model\u0000${slug}`, value);
      map.set(`slug\u0000${slug}`, value);
      // 原文键：网关关掉「模型名后带供应商」时，同名的几份只差末尾看不见的零宽字符，
      // 宽松归一化会把它们抹掉，必须先按原文比对。
      const rawKey = `raw\u0000${String(displayName).replace(/\s+/g, " ").trim()}`;
      setUnique(rawKey, value);
      setUnique(codexPlusModelPickerNativeLabelKey(displayName), value);
      const fullKey = codexPlusModelPickerNormalizeLoose(displayName);
      setUnique(fullKey, value);
      const split = codexPlusModelPickerSplitLabel(displayName);
      if (split.name && split.provider) {
        const pairKey = `${codexPlusModelPickerNormalizeLoose(split.name)}\u0000${codexPlusModelPickerNormalizeLoose(split.provider)}`;
        setUnique(pairKey, value);
      }
    };
    for (const [slug, entry] of codexPlusModelPickerCatalogEntries()) register(entry, slug);
    codexPlusModelPickerGroupsState.providerNames.forEach((value, key) => {
      if (!key.startsWith("model\u0000") || !value?.slug || !value.groupId || value.parsed) return;
      if (models && !models.includes(value.slug)) return;
      if (!codexPlusModelPickerCanRestoreProvider(modelMetadata[value.slug], value)) return;
      if (map.has(`model\u0000${value.slug}`)) return;
      register({ displayName: value.displayName, codex_plus_group: value.name, codex_plus_group_id: value.groupId }, value.slug);
    });
    return map;
  }

  /** 仅接受目录里存在的行 key，不读取上层选择器的当前选中模型。 */
  function codexPlusModelPickerRowSlug(row) {
    const key = Object.getOwnPropertyNames(row).find((name) => name.startsWith("__reactFiber$") || name.startsWith("__reactInternalInstance$"));
    let fiber = key ? row[key] : null;
    const metadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    for (let depth = 0; fiber && depth < 20; depth += 1, fiber = fiber.return) {
      const props = fiber.memoizedProps || fiber.pendingProps || {};
      if (Array.isArray(props.models) || typeof props.onSelectModel === "function") break;
      if (typeof fiber.key === "string" && metadata?.[fiber.key]) return fiber.key;
    }
    return "";
  }

  /** 目录条目上的价格字段；不是 `{input, output}` 形状就当没有。 */
  function codexPlusModelPickerPrice(entry) {
    const price = entry?.codex_plus_price;
    if (!price || typeof price !== "object") return null;
    const input = Number(price.input);
    const output = Number(price.output);
    if (!Number.isFinite(input) && !Number.isFinite(output)) return null;
    return {
      input: Number.isFinite(input) ? input : 0,
      output: Number.isFinite(output) ? output : 0,
      currency: typeof price.currency === "string" && price.currency ? price.currency : "USD",
    };
  }

  /**
   * 目录里"各种写法 -> 价格"的索引。与归属映射不同：价格只按模型名找，
   * 不区分供应商后缀（同一个上游模型在哪一家都是这份公开价目）。
   * `（供应商）` 后缀要先拆掉再归一化，否则后缀文本会并进键里。
   */
  function codexPlusModelPickerCatalogPriceMap() {
    const map = new Map();
    const modelMetadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata;
    if (!modelMetadata || typeof modelMetadata !== "object") return map;
    const put = (key, price) => {
      if (key && !map.has(key)) map.set(key, price);
    };
    const register = (entry, slug) => {
      if (!entry || typeof entry !== "object") return;
      const price = codexPlusModelPickerPrice(entry);
      if (!price) return;
      const displayName = entry.displayName || entry.display_name;
      if (typeof displayName === "string" && displayName) {
        put(`raw\u0000${displayName.replace(/\s+/g, " ").trim()}`, price);
        put(codexPlusModelPickerNormalizeLoose(displayName), price);
        const bare = codexPlusModelPickerSplitLabel(displayName).name;
        if (bare) put(codexPlusModelPickerNormalizeLoose(bare), price);
      }
      if (typeof slug === "string" && slug) {
        put(codexPlusModelPickerNormalizeLoose(slug), price);
        const bare = slug.includes("/") ? slug.slice(slug.lastIndexOf("/") + 1) : slug;
        put(codexPlusModelPickerNormalizeLoose(bare), price);
      }
    };
    const slugList = Array.isArray(codexModelCatalog?.models)
      ? codexModelCatalog.models.filter((slug) => typeof slug === "string" && slug)
      : [];
    const bySlug = new Map(Object.entries(modelMetadata));
    for (const slug of slugList) register(bySlug.get(slug), slug);
    for (const [key, entry] of Object.entries(modelMetadata)) register(entry, key);
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
    const outer = content.querySelector("span.flex-1, span[class*='flex-1'], div.flex-1, div[class*='flex-1']");
    // 原生副标题行使用 div.flex-1，普通单行及旧版骨架使用 span.flex-1。
    const inner = outer?.tagName === "DIV" ? outer : outer?.firstElementChild;
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
      outer,
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

  function codexPlusModelPickerRowModel(row) {
    const metadata = codexModelCatalog?.modelMetadata || codexModelCatalog?.model_metadata || {};
    const models = codexModelCatalog?.models;
    const key = Object.keys(row).find((name) => name.startsWith("__reactFiber$"));
    for (let fiber = key && row[key], depth = 0; fiber && depth < 16; depth++, fiber = fiber.return) {
      const props = fiber.memoizedProps || {};
      if (typeof props.onSelectModel === "function" && Array.isArray(props.models)) break;
      for (const value of [fiber.key, props.model, props.model?.model, props.model?.id, props.value, props.item?.model]) {
        if (typeof value === "string" && (Object.hasOwn(metadata, value)
            || (Array.isArray(models) && models.includes(value))
            || codexPlusModelPickerGroupsState.providerNames.has(`model\u0000${value}`))) return value;
      }
    }
    return "";
  }

  /**
   * 把行切成 `{ row, label, name, provider, groupId }`。
   * 已打过补丁的行直接读我们留下的标记，避免反复解析。
   */
  function codexPlusModelPickerRows(list) {
    const providers = codexPlusModelPickerCatalogProviderMap();
    const exact = codexPlusModelPickerCatalogExactMap();
    const providerIds = new Map();
    const rememberProviderId = (entry) => {
      if (!entry?.provider || !entry.groupId) return;
      if (!providerIds.has(entry.provider)) providerIds.set(entry.provider, entry.groupId);
      else if (providerIds.get(entry.provider) !== entry.groupId) providerIds.set(entry.provider, null);
    };
    exact.forEach(rememberProviderId);
    providers.forEach(rememberProviderId);
    const prices = codexPlusModelPickerCatalogPriceMap();
    const rows = [];
    const items = list.querySelectorAll('[role="menuitemradio"]');
    items.forEach((row) => {
      const identity = codexPlusModelPickerRowIdentity(row);
      if (!identity?.groupable) return;
      const saved = codexPlusModelPickerDomState.get(row);
      const remembered = saved?.get("model-identity");
      const model = codexPlusModelPickerRowModel(row) || codexPlusModelPickerRowSlug(row)
        || (remembered?.label === identity.text ? remembered.slug : "");
      const current = model ? exact.get(`model\u0000${model}`) : null;
      const label = model.startsWith("group/") && current ? current.displayName : identity.text;
      let split = codexPlusModelPickerSplitLabel(label);
      if (split.provider && !providerIds.has(split.provider)
          && ![...codexPlusModelPickerGroupsState.providerNames.values()]
            .some((value) => !value.parsed && value.name === split.provider)) {
        split = { name: label, provider: "" };
      }
      if (split.provider) codexPlusModelPickerRememberProvider(split.name, split.provider, true);
      // 分组键：优先用「完整展示名」或「模型名 + 后缀供应商」做精确匹配。
      // 有后缀的行成员归属由后缀决定，不能再拿裸模型名去查目录——三家都发布
      // `DeepSeek-V4-Pro` 时，裸名别名只会命中目录里第一个成员，把别人的
      // 模型并进错误的组（同一家被切成好几段 / 同名分组出现多次）。
      const pairKey = split.provider
        ? `${codexPlusModelPickerNormalizeLoose(split.name)}\u0000${codexPlusModelPickerNormalizeLoose(split.provider)}`
        : "";
      const exactKnown = model ? current
        : exact.get(`raw\u0000${identity.text}`)
          || exact.get(codexPlusModelPickerNativeLabelKey(identity.text))
          || exact.get(codexPlusModelPickerNormalizeLoose(identity.text))
          || (pairKey ? exact.get(pairKey) : null);
      const known = exactKnown
        || (!model && !split.provider
          ? providers.get(codexPlusModelPickerNormalizeStrict(split.name))
            || providers.get(codexPlusModelPickerNormalize(split.name))
          : null);
      const groupId = known?.groupId
        || (split.provider ? providerIds.get(split.provider) || "" : "");
      // 显示名：整行精确命中目录时用目录里的分组名（网关写的是打过码的完整账号名，后缀可能只是
      // 供应商 id）；否则有后缀就是那一家；裸名（默认成员的模型）用默认供应商的名字。
      const provider = exactKnown?.provider || split.provider || known?.provider || "";
      const authoritativeGroupId = !!groupId;
      const price = exactKnown?.price
        || prices.get(`raw\u0000${identity.text}`)
        || prices.get(codexPlusModelPickerNormalizeLoose(identity.text))
        || prices.get(codexPlusModelPickerNormalizeLoose(split.name))
        || null;
      const entry = { row, label, name: groupId ? split.name : label, provider, groupId, authoritativeGroupId, favorite: exactKnown?.favorite === true, price, slug: exactKnown?.slug || "" };
      if (entry.slug) {
        if (!saved) codexPlusModelPickerDomState.set(row, new Map());
        codexPlusModelPickerDomState.get(row).set("model-identity", { label: identity.text, slug: entry.slug });
      }
      rows.push(entry);
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
    // 快照暂时没有元数据（拉取超时 / 重连）时沿用上次算出的名字，裸名行不至于变成无名分组（#2860）。
    if (!modelMetadata || typeof modelMetadata !== "object") return codexPlusModelPickerGroupsState.defaultProviderName || "";
    // 取"裸名模型里出现最多的那个分组"：聚合的默认成员可能有几百个模型，
    // 偶尔混进一两个别的成员的裸名条目也不该把整组改姓。
    const counts = new Map();
    for (const [, entry] of codexPlusModelPickerCatalogEntries()) {
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
      const groupId = typeof entry.codex_plus_group_id === "string" ? entry.codex_plus_group_id.trim() : "";
      codexPlusModelPickerRememberProvider(displayName, provider, false, groupId);
    }
    let best = "";
    let bestCount = 0;
    counts.forEach((count, provider) => {
      if (count > bestCount) {
        best = provider;
        bestCount = count;
      }
    });
    if (best) codexPlusModelPickerGroupsState.defaultProviderName = best;
    return best || codexPlusModelPickerGroupsState.defaultProviderName || "";
  }

  /** 按权威 ID 合组；归属不明的行集中展示，保留原始标签。 */
  function codexPlusModelPickerGroups(rows, defaultProviderName = "") {
    const groups = [];
    const identified = new Map();
    let unnamed = null;
    for (const entry of rows) {
      const key = entry.provider || "";
      const id = entry.authoritativeGroupId !== false && typeof entry.groupId === "string" ? entry.groupId.trim() : "";
      let group;
      if (id) {
        group = identified.get(id);
        if (!group) {
          group = { key, id, name: entry.provider || defaultProviderName, entries: [] };
          identified.set(id, group);
          groups.push(group);
        }
      } else {
        if (!unnamed) {
          unnamed = { key: "", id: "", name: "", entries: [] };
          groups.push(unnamed);
        }
        group = unnamed;
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

  // ---------- 行装饰：厂商图标 + 价格条 ----------

  /** 行头厂商图标类名。 */
  const codexPlusModelPickerRowIconClass = "codex-plus-model-row-icon";
  /** 行尾价格条类名。 */
  const codexPlusModelPickerRowMeterClass = "codex-plus-model-row-meter";
  /** 行尾「免费」标签类名。 */
  const codexPlusModelPickerRowFreeClass = "codex-plus-model-row-free";
  /** 「最近使用」区块类名。 */
  const codexPlusModelPickerRecentsClass = "codex-plus-model-recents";
  /** 最近使用的本地存储键。 */
  const codexPlusModelPickerRecentsStoreKey = "codexPlus.modelPicker.recents.v1";
  /** 存多少条最近使用（渲染时再截到 2）。 */
  const codexPlusModelPickerRecentsMax = 5;
  /** 列表上已挂最近使用点击监听的标记。 */
  const codexPlusModelPickerRecentsHookAttr = "data-codex-plus-model-recents-hook";

  /**
   * 厂商字形：按模型名认家族，图标取自管理端同款 lobehub 品牌图标
   * （apps/codex-plus-manager/src/assets/providers/）。单色图标跟随文字色，
   * 彩色图标自带渐变；认不出的回退为供应商配色 + 首字母。
   * OpenAI 那条额外认「6.1 Sol / 5.3 Codex Spark」：原生选择器会去掉 `GPT-` 前缀。
   */
  const codexPlusModelPickerGlyphs = [
    { re: /claude|anthropic|sonnet|opus|haiku/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z\" fill=\"#D97757\" fill-rule=\"nonzero\"></path></svg>" },
    { re: /(^|[\s_/.-])(gpt|o[0-9]|chatgpt|openai|codex)|^\d+(\.\d+)?[\s-]+(sol|terra|luna|astra|codex|mini|nano|pro)\b/i, mono: true, svg: "<svg viewBox=\"0 0 24 24\" fill=\"currentColor\" fill-rule=\"evenodd\" aria-hidden=\"true\"><path d=\"M9.205 8.658v-2.26c0-.19.072-.333.238-.428l4.543-2.616c.619-.357 1.356-.523 2.117-.523 2.854 0 4.662 2.212 4.662 4.566 0 .167 0 .357-.024.547l-4.71-2.759a.797.797 0 00-.856 0l-5.97 3.473zm10.609 8.8V12.06c0-.333-.143-.57-.429-.737l-5.97-3.473 1.95-1.118a.433.433 0 01.476 0l4.543 2.617c1.309.76 2.189 2.378 2.189 3.948 0 1.808-1.07 3.473-2.76 4.163zM7.802 12.703l-1.95-1.142c-.167-.095-.239-.238-.239-.428V5.899c0-2.545 1.95-4.472 4.591-4.472 1 0 1.927.333 2.712.928L8.23 5.067c-.285.166-.428.404-.428.737v6.898zM12 15.128l-2.795-1.57v-3.33L12 8.658l2.795 1.57v3.33L12 15.128zm1.796 7.23c-1 0-1.927-.332-2.712-.927l4.686-2.712c.285-.166.428-.404.428-.737v-6.898l1.974 1.142c.167.095.238.238.238.428v5.233c0 2.545-1.974 4.472-4.614 4.472zm-5.637-5.303l-4.544-2.617c-1.308-.761-2.188-2.378-2.188-3.948A4.482 4.482 0 014.21 6.327v5.423c0 .333.143.571.428.738l5.947 3.449-1.95 1.118a.432.432 0 01-.476 0zm-.262 3.9c-2.688 0-4.662-2.021-4.662-4.519 0-.19.024-.38.047-.57l4.686 2.71c.286.167.571.167.856 0l5.97-3.448v2.26c0 .19-.07.333-.237.428l-4.543 2.616c-.619.357-1.356.523-2.117.523zm5.899 2.83a5.947 5.947 0 005.827-4.756C22.287 18.339 24 15.84 24 13.296c0-1.665-.713-3.282-1.998-4.448.119-.5.19-.999.19-1.498 0-3.401-2.759-5.947-5.946-5.947-.642 0-1.26.095-1.88.31A5.962 5.962 0 0010.205 0a5.947 5.947 0 00-5.827 4.757C1.713 5.447 0 7.945 0 10.49c0 1.666.713 3.283 1.998 4.448-.119.5-.19 1-.19 1.499 0 3.401 2.759 5.946 5.946 5.946.642 0 1.26-.095 1.88-.309a5.96 5.96 0 004.162 1.713z\"></path></svg>" },
    { re: /gemini|bard|gemma/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M20.616 10.835a14.147 14.147 0 01-4.45-3.001 14.111 14.111 0 01-3.678-6.452.503.503 0 00-.975 0 14.134 14.134 0 01-3.679 6.452 14.155 14.155 0 01-4.45 3.001c-.65.28-1.318.505-2.002.678a.502.502 0 000 .975c.684.172 1.35.397 2.002.677a14.147 14.147 0 014.45 3.001 14.112 14.112 0 013.679 6.453.502.502 0 00.975 0c.172-.685.397-1.351.677-2.003a14.145 14.145 0 013.001-4.45 14.113 14.113 0 016.453-3.678.503.503 0 000-.975 13.245 13.245 0 01-2.003-.678z\" fill=\"#3186FF\"></path><path d=\"M20.616 10.835a14.147 14.147 0 01-4.45-3.001 14.111 14.111 0 01-3.678-6.452.503.503 0 00-.975 0 14.134 14.134 0 01-3.679 6.452 14.155 14.155 0 01-4.45 3.001c-.65.28-1.318.505-2.002.678a.502.502 0 000 .975c.684.172 1.35.397 2.002.677a14.147 14.147 0 014.45 3.001 14.112 14.112 0 013.679 6.453.502.502 0 00.975 0c.172-.685.397-1.351.677-2.003a14.145 14.145 0 013.001-4.45 14.113 14.113 0 016.453-3.678.503.503 0 000-.975 13.245 13.245 0 01-2.003-.678z\" fill=\"url(#lobe-icons-gemini-0-_R_0_)\"></path><path d=\"M20.616 10.835a14.147 14.147 0 01-4.45-3.001 14.111 14.111 0 01-3.678-6.452.503.503 0 00-.975 0 14.134 14.134 0 01-3.679 6.452 14.155 14.155 0 01-4.45 3.001c-.65.28-1.318.505-2.002.678a.502.502 0 000 .975c.684.172 1.35.397 2.002.677a14.147 14.147 0 014.45 3.001 14.112 14.112 0 013.679 6.453.502.502 0 00.975 0c.172-.685.397-1.351.677-2.003a14.145 14.145 0 013.001-4.45 14.113 14.113 0 016.453-3.678.503.503 0 000-.975 13.245 13.245 0 01-2.003-.678z\" fill=\"url(#lobe-icons-gemini-1-_R_0_)\"></path><path d=\"M20.616 10.835a14.147 14.147 0 01-4.45-3.001 14.111 14.111 0 01-3.678-6.452.503.503 0 00-.975 0 14.134 14.134 0 01-3.679 6.452 14.155 14.155 0 01-4.45 3.001c-.65.28-1.318.505-2.002.678a.502.502 0 000 .975c.684.172 1.35.397 2.002.677a14.147 14.147 0 014.45 3.001 14.112 14.112 0 013.679 6.453.502.502 0 00.975 0c.172-.685.397-1.351.677-2.003a14.145 14.145 0 013.001-4.45 14.113 14.113 0 016.453-3.678.503.503 0 000-.975 13.245 13.245 0 01-2.003-.678z\" fill=\"url(#lobe-icons-gemini-2-_R_0_)\"></path><defs><linearGradient gradientUnits=\"userSpaceOnUse\" id=\"lobe-icons-gemini-0-_R_0_\" x1=\"7\" x2=\"11\" y1=\"15.5\" y2=\"12\"><stop stop-color=\"#08B962\"></stop><stop offset=\"1\" stop-color=\"#08B962\" stop-opacity=\"0\"></stop></linearGradient><linearGradient gradientUnits=\"userSpaceOnUse\" id=\"lobe-icons-gemini-1-_R_0_\" x1=\"8\" x2=\"11.5\" y1=\"5.5\" y2=\"11\"><stop stop-color=\"#F94543\"></stop><stop offset=\"1\" stop-color=\"#F94543\" stop-opacity=\"0\"></stop></linearGradient><linearGradient gradientUnits=\"userSpaceOnUse\" id=\"lobe-icons-gemini-2-_R_0_\" x1=\"3.5\" x2=\"17.5\" y1=\"13.5\" y2=\"12\"><stop stop-color=\"#FABC12\"></stop><stop offset=\".46\" stop-color=\"#FABC12\" stop-opacity=\"0\"></stop></linearGradient></defs></svg>" },
    { re: /deepseek/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M23.748 4.482c-.254-.124-.364.113-.512.234-.051.039-.094.09-.137.136-.372.397-.806.657-1.373.626-.829-.046-1.537.214-2.163.848-.133-.782-.575-1.248-1.247-1.548-.352-.156-.708-.311-.955-.65-.172-.241-.219-.51-.305-.774-.055-.16-.11-.323-.293-.35-.2-.031-.278.136-.356.276-.313.572-.434 1.202-.422 1.84.027 1.436.633 2.58 1.838 3.393.137.093.172.187.129.323-.082.28-.18.552-.266.833-.055.179-.137.217-.329.14a5.526 5.526 0 01-1.736-1.18c-.857-.828-1.631-1.742-2.597-2.458a11.365 11.365 0 00-.689-.471c-.985-.957.13-1.743.388-1.836.27-.098.093-.432-.779-.428-.872.004-1.67.295-2.687.684a3.055 3.055 0 01-.465.137 9.597 9.597 0 00-2.883-.102c-1.885.21-3.39 1.102-4.497 2.623C.082 8.606-.231 10.684.152 12.85c.403 2.284 1.569 4.175 3.36 5.653 1.858 1.533 3.997 2.284 6.438 2.14 1.482-.085 3.133-.284 4.994-1.86.47.234.962.327 1.78.397.63.059 1.236-.03 1.705-.128.735-.156.684-.837.419-.961-2.155-1.004-1.682-.595-2.113-.926 1.096-1.296 2.746-2.642 3.392-7.003.05-.347.007-.565 0-.845-.004-.17.035-.237.23-.256a4.173 4.173 0 001.545-.475c1.396-.763 1.96-2.015 2.093-3.517.02-.23-.004-.467-.247-.588zM11.581 18c-2.089-1.642-3.102-2.183-3.52-2.16-.392.024-.321.471-.235.763.09.288.207.486.371.739.114.167.192.416-.113.603-.673.416-1.842-.14-1.897-.167-1.361-.802-2.5-1.86-3.301-3.307-.774-1.393-1.224-2.887-1.298-4.482-.02-.386.093-.522.477-.592a4.696 4.696 0 011.529-.039c2.132.312 3.946 1.265 5.468 2.774.868.86 1.525 1.887 2.202 2.891.72 1.066 1.494 2.082 2.48 2.914.348.292.625.514.891.677-.802.09-2.14.11-3.054-.614zm1-6.44a.306.306 0 01.415-.287.302.302 0 01.2.288.306.306 0 01-.31.307.303.303 0 01-.304-.308zm3.11 1.596c-.2.081-.399.151-.59.16a1.245 1.245 0 01-.798-.254c-.274-.23-.47-.358-.552-.758a1.73 1.73 0 01.016-.588c.07-.327-.008-.537-.239-.727-.187-.156-.426-.199-.688-.199a.559.559 0 01-.254-.078c-.11-.054-.2-.19-.114-.358.028-.054.16-.186.192-.21.356-.202.767-.136 1.146.016.352.144.618.408 1.001.782.391.451.462.576.685.914.176.265.336.537.445.848.067.195-.019.354-.25.452z\" fill=\"#4D6BFE\"></path></svg>" },
    { re: /kimi|moonshot/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M21.846 0a1.923 1.923 0 110 3.846H20.15a.226.226 0 01-.227-.226V1.923C19.923.861 20.784 0 21.846 0z\" fill=\"#1783FF\"></path><path d=\"M11.065 11.199l7.257-7.2c.137-.136.06-.41-.116-.41H14.3a.164.164 0 00-.117.051l-7.82 7.756c-.122.12-.302.013-.302-.179V3.82c0-.127-.083-.23-.185-.23H3.186c-.103 0-.186.103-.186.23V19.77c0 .128.083.23.186.23h2.69c.103 0 .186-.102.186-.23v-3.25c0-.069.025-.135.069-.178l2.424-2.406a.158.158 0 01.205-.023l6.484 4.772a7.677 7.677 0 003.453 1.283c.108.012.2-.095.2-.23v-3.06c0-.117-.07-.212-.164-.227a5.028 5.028 0 01-2.027-.807l-5.613-4.064c-.117-.078-.132-.279-.028-.381z\" fill=\"currentColor\"></path></svg>" },
    { re: /grok|xai/i, mono: true, svg: "<svg viewBox=\"0 0 24 24\" fill=\"currentColor\" fill-rule=\"evenodd\" aria-hidden=\"true\"><path d=\"M9.27 15.29l7.978-5.897c.391-.29.95-.177 1.137.272.98 2.369.542 5.215-1.41 7.169-1.951 1.954-4.667 2.382-7.149 1.406l-2.711 1.257c3.889 2.661 8.611 2.003 11.562-.953 2.341-2.344 3.066-5.539 2.388-8.42l.006.007c-.983-4.232.242-5.924 2.75-9.383.06-.082.12-.164.179-.248l-3.301 3.305v-.01L9.267 15.292M7.623 16.723c-2.792-2.67-2.31-6.801.071-9.184 1.761-1.763 4.647-2.483 7.166-1.425l2.705-1.25a7.808 7.808 0 00-1.829-1A8.975 8.975 0 005.984 5.83c-2.533 2.536-3.33 6.436-1.962 9.764 1.022 2.487-.653 4.246-2.34 6.022-.599.63-1.199 1.259-1.682 1.925l7.62-6.815\"></path></svg>" },
    { re: /mistral|mixtral|codestral|pixtral|magistral|devstral/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M3.428 3.4h3.429v3.428H3.428V3.4zm13.714 0h3.43v3.428h-3.43V3.4z\" fill=\"gold\"></path><path d=\"M3.428 6.828h6.857v3.429H3.429V6.828zm10.286 0h6.857v3.429h-6.857V6.828z\" fill=\"#FFAF00\"></path><path d=\"M3.428 10.258h17.144v3.428H3.428v-3.428z\" fill=\"#FF8205\"></path><path d=\"M3.428 13.686h3.429v3.428H3.428v-3.428zm6.858 0h3.429v3.428h-3.429v-3.428zm6.856 0h3.43v3.428h-3.43v-3.428z\" fill=\"#FA500F\"></path><path d=\"M0 17.114h10.286v3.429H0v-3.429zm13.714 0H24v3.429H13.714v-3.429z\" fill=\"#E10500\"></path></svg>" },
    { re: /qwen|tongyi|qwq/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M19.713 2.825h-5.907c0-.251-.066-.503-.196-.728L12.819.728A1.46 1.46 0 0 0 11.559 0h-1.58c-.52 0-1 .275-1.26.728l-2.42 4.19a1.45 1.45 0 0 0 0 1.453l.791 1.37c.26.448.74.727 1.26.727h11.36c.52 0 1-.275 1.26-.728l.79-1.37a1.45 1.45 0 0 0 0-1.451l-.79-1.37a1.46 1.46 0 0 0-1.26-.728zM.195 9.905l2.951 5.115a1.46 1.46 0 0 0-.531.531l-.791 1.37a1.45 1.45 0 0 0 0 1.452l.79 1.37c.26.448.74.727 1.26.727h4.84c.52 0 1-.275 1.26-.728l.79-1.37a1.45 1.45 0 0 0 0-1.451L5.086 7.083a1.46 1.46 0 0 0-1.26-.728h-1.58c-.52 0-1 .276-1.26.728l-.79 1.37a1.45 1.45 0 0 0 0 1.452m15.888 13.367 2.951-5.115c.217.125.469.196.728.196h1.582c.52 0 1-.275 1.26-.728l.79-1.37a1.45 1.45 0 0 0 0-1.451l-2.42-4.191a1.46 1.46 0 0 0-1.259-.728h-1.582c-.52 0-1 .276-1.259.728l-5.678 9.838a1.45 1.45 0 0 0 0 1.452l.79 1.37c.26.448.74.727 1.26.727h1.582c.52 0 1-.275 1.26-.728z\" fill=\"#082dff\"></path></svg>" },
    { re: /glm|chatglm|zhipu/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M11.991 23.503a.24.24 0 00-.244.248.24.24 0 00.244.249.24.24 0 00.245-.249.24.24 0 00-.22-.247l-.025-.001zM9.671 5.365a1.697 1.697 0 011.099 2.132l-.071.172-.016.04-.018.054c-.07.16-.104.32-.104.498-.035.71.47 1.279 1.186 1.314h.366c1.309.053 2.338 1.173 2.286 2.523-.052 1.332-1.152 2.38-2.478 2.327h-.174c-.715.018-1.274.64-1.239 1.368 0 .124.018.23.053.337.209.373.54.658.96.8.75.23 1.517-.125 1.9-.782l.018-.035c.402-.64 1.17-.96 1.92-.711.854.284 1.378 1.226 1.099 2.167a1.661 1.661 0 01-2.077 1.102 1.711 1.711 0 01-.907-.711l-.017-.035c-.2-.323-.463-.58-.851-.711l-.056-.018a1.646 1.646 0 00-1.954.746 1.66 1.66 0 01-1.065.764 1.677 1.677 0 01-1.989-1.279c-.209-.906.332-1.83 1.257-2.043a1.51 1.51 0 01.296-.035h.018c.68-.071 1.151-.622 1.116-1.333a1.307 1.307 0 00-.227-.693 2.515 2.515 0 01-.366-1.403 2.39 2.39 0 01.366-1.208c.14-.195.21-.444.227-.693.018-.71-.506-1.261-1.186-1.332l-.07-.018a1.43 1.43 0 01-.299-.07l-.05-.019a1.7 1.7 0 01-1.047-2.114 1.68 1.68 0 012.094-1.101zm-5.575 10.11c.26-.264.639-.367.994-.27.355.096.633.379.728.74.095.362-.007.748-.267 1.013-.402.41-1.053.41-1.455 0a1.062 1.062 0 010-1.482zm14.845-.294c.359-.09.738.024.992.297.254.274.344.665.237 1.025-.107.36-.396.634-.756.718-.551.128-1.1-.22-1.23-.781a1.05 1.05 0 01.757-1.26zm-.064-4.39c.314.32.49.753.49 1.206 0 .452-.176.886-.49 1.206-.315.32-.74.5-1.185.5-.444 0-.87-.18-1.184-.5a1.727 1.727 0 010-2.412 1.654 1.654 0 012.369 0zm-11.243.163c.364.484.447 1.128.218 1.691a1.665 1.665 0 01-2.188.923c-.855-.36-1.26-1.358-.907-2.228a1.68 1.68 0 011.33-1.038c.593-.08 1.183.169 1.547.652zm11.545-4.221c.368 0 .708.2.892.524.184.324.184.724 0 1.048a1.026 1.026 0 01-.892.524c-.568 0-1.03-.47-1.03-1.048 0-.579.462-1.048 1.03-1.048zm-14.358 0c.368 0 .707.2.891.524.184.324.184.724 0 1.048a1.026 1.026 0 01-.891.524c-.569 0-1.03-.47-1.03-1.048 0-.579.461-1.048 1.03-1.048zm10.031-1.475c.925 0 1.675.764 1.675 1.706s-.75 1.705-1.675 1.705-1.674-.763-1.674-1.705c0-.942.75-1.706 1.674-1.706zm-2.626-.684c.362-.082.653-.356.761-.718a1.062 1.062 0 00-.238-1.028 1.017 1.017 0 00-.996-.294c-.547.14-.881.7-.752 1.257.13.558.675.907 1.225.783zm0 16.876c.359-.087.644-.36.75-.72a1.062 1.062 0 00-.237-1.019 1.018 1.018 0 00-.985-.301 1.037 1.037 0 00-.762.717c-.108.361-.017.754.239 1.028.245.263.606.377.953.305l.043-.01zM17.19 3.5a.631.631 0 00.628-.64c0-.355-.279-.64-.628-.64a.631.631 0 00-.628.64c0 .355.28.64.628.64zm-10.38 0a.631.631 0 00.628-.64c0-.355-.28-.64-.628-.64a.631.631 0 00-.628.64c0 .355.279.64.628.64zm-5.182 7.852a.631.631 0 00-.628.64c0 .354.28.639.628.639a.63.63 0 00.627-.606l.001-.034a.62.62 0 00-.628-.64zm5.182 9.13a.631.631 0 00-.628.64c0 .355.279.64.628.64a.631.631 0 00.628-.64c0-.355-.28-.64-.628-.64zm10.38.018a.631.631 0 00-.628.64c0 .355.28.64.628.64a.631.631 0 00.628-.64c0-.355-.279-.64-.628-.64zm5.182-9.148a.631.631 0 00-.628.64c0 .354.279.639.628.639a.631.631 0 00.628-.64c0-.355-.28-.64-.628-.64zm-.384-4.992a.24.24 0 00.244-.249.24.24 0 00-.244-.249.24.24 0 00-.244.249c0 .142.122.249.244.249zM11.991.497a.24.24 0 00.245-.248A.24.24 0 0011.99 0a.24.24 0 00-.244.249c0 .133.108.236.223.247l.021.001zM2.011 6.36a.24.24 0 00.245-.249.24.24 0 00-.244-.249.24.24 0 00-.244.249.24.24 0 00.244.249zm0 11.263a.24.24 0 00-.243.248.24.24 0 00.244.249.24.24 0 00.244-.249.252.252 0 00-.244-.248zm19.995-.018a.24.24 0 00-.245.248.24.24 0 00.245.25.24.24 0 00.244-.25.252.252 0 00-.244-.248z\" fill=\"#3859FF\" fill-rule=\"nonzero\"></path></svg>" },
    { re: /doubao|seed/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M5.31 15.756c.172-3.75 1.883-5.999 2.549-6.739-3.26 2.058-5.425 5.658-6.358 8.308v1.12C1.501 21.513 4.226 24 7.59 24a6.59 6.59 0 002.2-.375c.353-.12.7-.248 1.039-.378.913-.899 1.65-1.91 2.243-2.992-4.877 2.431-7.974.072-7.763-4.5l.002.001z\" fill=\"#1E37FC\"></path><path d=\"M22.57 10.283c-1.212-.901-4.109-2.404-7.397-2.8.295 3.792.093 8.766-2.1 12.773a12.782 12.782 0 01-2.244 2.992c3.764-1.448 6.746-3.457 8.596-5.219 2.82-2.683 3.353-5.178 3.361-6.66a2.737 2.737 0 00-.216-1.084v-.002z\" fill=\"#37E1BE\"></path><path d=\"M14.303 1.867C12.955.7 11.248 0 9.39 0 7.532 0 5.883.677 4.545 1.807 2.791 3.29 1.627 5.557 1.5 8.125v9.201c.932-2.65 3.097-6.25 6.357-8.307.5-.318 1.025-.595 1.569-.829 1.883-.801 3.878-.932 5.746-.706-.222-2.83-.718-5.002-.87-5.617h.001z\" fill=\"#A569FF\"></path><path d=\"M17.305 4.961a199.47 199.47 0 01-1.08-1.094c-.202-.213-.398-.419-.586-.622l-1.333-1.378c.151.615.648 2.786.869 5.617 3.288.395 6.185 1.898 7.396 2.8-1.306-1.275-3.475-3.487-5.266-5.323z\" fill=\"#1E37FC\"></path></svg>" },
    { re: /hunyuan/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><circle cx=\"12\" cy=\"12\" fill=\"#0055E9\" r=\"12\"></circle><path d=\"M12 0c.518 0 1.028.033 1.528.096A6.188 6.188 0 0112.12 12.28l-.12.001c-2.99 0-5.242 2.179-5.554 5.11-.223 2.086.353 4.412 2.242 6.146C3.672 22.1 0 17.479 0 12 0 5.373 5.373 0 12 0z\" fill=\"#A8DFF5\"></path><path d=\"M5.286 5a2.438 2.438 0 01.682 3.38c-3.962 5.966-3.215 10.743 2.648 15.136C3.636 22.056 0 17.452 0 12c0-1.787.39-3.482 1.09-5.006.253-.435.525-.872.817-1.311A2.438 2.438 0 015.286 5z\" fill=\"#0055E9\"></path><path d=\"M12.98.04c.272.021.543.053.81.093.583.106 1.117.254 1.538.44 6.638 2.927 8.07 10.052 1.748 15.642a4.125 4.125 0 01-5.822-.358c-1.51-1.706-1.3-4.184.357-5.822.858-.848 3.108-1.223 4.045-2.441 1.257-1.634 2.122-6.009-2.523-7.506L12.98.039z\" fill=\"#00BCFF\"></path><path d=\"M13.528.096A6.187 6.187 0 0112 12.281a5.75 5.75 0 00-1.71.255c.147-.905.595-1.784 1.321-2.501.858-.848 3.108-1.223 4.045-2.441 1.27-1.651 2.14-6.104-2.676-7.554.184.014.367.033.548.056z\" fill=\"#ECECEE\"></path></svg>" },
    { re: /minimax|abab/i, mono: false, svg: "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><defs><linearGradient id=\"lobe-icons-minimax-_R_0_\" x1=\"0%\" x2=\"100.182%\" y1=\"50.057%\" y2=\"50.057%\"><stop offset=\"0%\" stop-color=\"#E2167E\"></stop><stop offset=\"100%\" stop-color=\"#FE603C\"></stop></linearGradient></defs><path d=\"M16.278 2c1.156 0 2.093.927 2.093 2.07v12.501a.74.74 0 00.744.709.74.74 0 00.743-.709V9.099a2.06 2.06 0 012.071-2.049A2.06 2.06 0 0124 9.1v6.561a.649.649 0 01-.652.645.649.649 0 01-.653-.645V9.1a.762.762 0 00-.766-.758.762.762 0 00-.766.758v7.472a2.037 2.037 0 01-2.048 2.026 2.037 2.037 0 01-2.048-2.026v-12.5a.785.785 0 00-.788-.753.785.785 0 00-.789.752l-.001 15.904A2.037 2.037 0 0113.441 22a2.037 2.037 0 01-2.048-2.026V18.04c0-.356.292-.645.652-.645.36 0 .652.289.652.645v1.934c0 .263.142.506.372.638.23.131.514.131.744 0a.734.734 0 00.372-.638V4.07c0-1.143.937-2.07 2.093-2.07zm-5.674 0c1.156 0 2.093.927 2.093 2.07v11.523a.648.648 0 01-.652.645.648.648 0 01-.652-.645V4.07a.785.785 0 00-.789-.78.785.785 0 00-.789.78v14.013a2.06 2.06 0 01-2.07 2.048 2.06 2.06 0 01-2.071-2.048V9.1a.762.762 0 00-.766-.758.762.762 0 00-.766.758v3.8a2.06 2.06 0 01-2.071 2.049A2.06 2.06 0 010 12.9v-1.378c0-.357.292-.646.652-.646.36 0 .653.29.653.646V12.9c0 .418.343.757.766.757s.766-.339.766-.757V9.099a2.06 2.06 0 012.07-2.048 2.06 2.06 0 012.071 2.048v8.984c0 .419.343.758.767.758.423 0 .766-.339.766-.758V4.07c0-1.143.937-2.07 2.093-2.07z\" fill=\"url(#lobe-icons-minimax-_R_0_)\" fill-rule=\"nonzero\"></path></svg>" },
  ];

  function codexPlusModelPickerGlyphHit(...names) {
    return codexPlusModelPickerGlyphs.find((glyph) => names.some((name) => name && glyph.re.test(String(name)))) || null;
  }

  /** 一个模型行该画什么图标：`{ key, svg }` 或 `{ key, color, letter }`。 */
  function codexPlusModelPickerGlyph(entry) {
    const name = String(entry?.name || entry?.label || "");
    const hit = codexPlusModelPickerGlyphHit(entry?.name, entry?.label, entry?.slug, entry?.slug?.split("/").pop());
    if (hit) return { key: `svg:${hit.re.source}`, svg: hit.svg };
    const letter = (Array.from(name.replace(/^[^\p{L}\p{N}]+/u, ""))[0] || "?").toUpperCase();
    // 认不出的家族：用供应商头像的配色，保证和供应商行/头像视觉一致。
    const avatar = codexPlusModelPickerProviderIcon({
      id: entry?.groupId || entry?.provider || name || "?",
      name: entry?.provider || name || "?",
    });
    return { key: `letter:${avatar.color}:${letter}`, color: avatar.color, letter };
  }

  function codexPlusModelPickerGlyphHtml(glyph) {
    return glyph.svg
      ? glyph.svg
      : `<span class="codex-plus-model-row-icon-letter" style="color:${glyph.color}">${glyph.letter}</span>`;
  }

  /**
   * 供应商图标只按组名认家族（"ChatGPT Pro" → OpenAI、"SuperGrok" → Grok）。
   * 不拿组内模型名兜底：中转账号里装的什么模型都有，按模型打品牌会把
   * 「jojocodemax / a哥」这类中转错标成 OpenAI / Claude。认不出返回 null，用字母头像。
   */
  function codexPlusModelPickerProviderGlyph(group) {
    return codexPlusModelPickerGlyphHit(group?.name || "");
  }

  function codexPlusModelPickerFormatPrice(value) {
    if (!Number.isFinite(value)) return "0";
    const text = value >= 10 ? value.toFixed(1) : value >= 1 ? value.toFixed(2) : value.toFixed(3);
    return text.replace(/\.?0+$/, "") || "0";
  }

  /**
   * 价格档位：输入输出各取一半混合（每百万 token 美元）。
   * 0 = 免费（画「免费」标签），-1 = 无价目（什么都不画），1~5 = 彩条段数。
   */
  function codexPlusModelPickerPriceTier(price) {
    if (!price) return -1;
    const score = price.input * 0.5 + price.output * 0.5;
    if (!(score > 0)) return 0;
    if (score < 0.35) return 1;
    if (score < 1.2) return 2;
    if (score < 4) return 3;
    if (score < 12) return 4;
    return 5;
  }

  /** 行尾价格节点：计量条或「免费」标签；无价目返回 null。最近使用行也用它。 */
  function codexPlusModelPickerPriceNode(price) {
    const tier = codexPlusModelPickerPriceTier(price);
    if (tier < 0) return null;
    const text = codexPlusModelPickerText();
    const title = price ? text.priceTitle(codexPlusModelPickerFormatPrice(price.input), codexPlusModelPickerFormatPrice(price.output)) : "";
    if (tier === 0) {
      const node = codexPlusModelPickerElement("span", codexPlusModelPickerRowFreeClass);
      node.textContent = text.free;
      if (title) node.title = title;
      return node;
    }
    const node = codexPlusModelPickerElement("span", codexPlusModelPickerRowMeterClass);
    node.setAttribute("aria-hidden", "true");
    node.dataset.tier = String(tier);
    for (let index = 0; index < 5; index += 1) {
      const segment = document.createElement("i");
      if (index < tier) segment.className = "on";
      node.appendChild(segment);
    }
    if (title) node.title = title;
    return node;
  }

  /**
   * 行装饰：厂商图标插在文本列之前，价格条跟在文本列之后（原生勾选仍在最右）。
   * 幂等：glyph/tier 没变就不动 DOM，React 重排行结构时扫描会补回来。
   */
  function codexPlusModelPickerInstallDecor(entry) {
    const identity = codexPlusModelPickerRowIdentity(entry.row);
    const content = identity?.content;
    const host = identity?.outer?.parentElement;
    if (!content || !identity?.outer || !host) return;
    // 图标 / 价格条插到「文本列的父容器」里（不一定是 content 的直接子节点），
    // 图标在文本列之前、价格条紧随其后，原生勾选仍在最右。
    let icon = content.querySelector(`.${codexPlusModelPickerRowIconClass}`);
    if (!icon) {
      icon = codexPlusModelPickerElement("span", codexPlusModelPickerRowIconClass);
      icon.setAttribute("aria-hidden", "true");
      host.insertBefore(icon, identity.outer);
    }
    const glyph = codexPlusModelPickerGlyph(entry);
    if (icon.dataset.glyphKey !== glyph.key) {
      icon.dataset.glyphKey = glyph.key;
      icon.innerHTML = codexPlusModelPickerGlyphHtml(glyph);
    }
    codexPlusModelPickerMarkCheck(entry.row, content, host, identity.outer);
    let meter = content.querySelector(`.${codexPlusModelPickerRowMeterClass}, .${codexPlusModelPickerRowFreeClass}`);
    const wanted = codexPlusModelPickerPriceNode(entry.price);
    if (!wanted) {
      meter?.remove();
      return;
    }
    if (meter && meter.className !== wanted.className) {
      meter.remove();
      meter = null;
    }
    if (!meter) {
      meter = wanted;
      host.insertBefore(meter, identity.outer.nextSibling);
      return;
    }
    if (meter.dataset.tier !== wanted.dataset.tier) {
      meter.dataset.tier = wanted.dataset.tier;
      [...meter.children].forEach((segment, index) => {
        segment.classList.toggle("on", index < Number(wanted.dataset.tier));
      });
    }
    if (meter.title !== wanted.title) meter.title = wanted.title;
  }

  /**
   * 原生选中勾：文本列之后的原生兄弟节点（只在选中行出现）。打上标记后 CSS 用
   * flex order 把它排到价格条左边，价格条始终贴行尾——否则选中行的价格条会被勾往左挤。
   * 只加属性、不搬节点；勾随选中态被 React 增删时，下一轮扫描重新标记。
   */
  function codexPlusModelPickerMarkCheck(row, content, host, outer) {
    row.classList.add("codex-plus-model-decorated");
    const candidates = [...host.children];
    if (content.parentElement === row) candidates.push(...row.children);
    for (const node of candidates) {
      if (node === outer || node === content || node.contains(outer)) continue;
      if (node.getAttribute("data-codex-plus-ext") === codexPlusModelPickerGroupsExtMark) continue;
      if (!node.hasAttribute("data-codex-plus-model-check")) node.setAttribute("data-codex-plus-model-check", "true");
    }
  }

  function codexPlusModelPickerRemoveDecor(row) {
    row.classList.remove("codex-plus-model-decorated");
    row.querySelectorAll("[data-codex-plus-model-check]").forEach((node) => node.removeAttribute("data-codex-plus-model-check"));
    const content = row.querySelector("[data-menu-row-content]") || row;
    content.querySelector(`.${codexPlusModelPickerRowIconClass}`)?.remove();
    content.querySelector(`.${codexPlusModelPickerRowMeterClass}`)?.remove();
    content.querySelector(`.${codexPlusModelPickerRowFreeClass}`)?.remove();
  }

  // ---------- 最近使用 ----------

  /**
   * 每次现读 localStorage：重注入后旧实例留在列表上的点击监听仍在写同一个键，
   * 会话内缓存会看不到它写入的记录。最多 5 条，每次读的代价可以忽略。
   */
  function codexPlusModelPickerRecents() {
    try {
      const raw = window.localStorage?.getItem(codexPlusModelPickerRecentsStoreKey);
      const parsed = raw ? JSON.parse(raw) : null;
      const items = Array.isArray(parsed) ? parsed : parsed?.items;
      if (!Array.isArray(items)) return [];
      return items
        .filter((item) => item && typeof item === "object" && typeof item.label === "string" && item.label)
        .slice(0, codexPlusModelPickerRecentsMax);
    } catch {
      return [];
    }
  }

  /** 用替换前的目录给旧格式记录补稳定身份，重名记录不能按展示名猜到别的组。 */
  function codexPlusModelPickerMigrateRecents(previous, next) {
    const before = previous?.modelMetadata || previous?.model_metadata || {};
    const after = next?.modelMetadata || next?.model_metadata || {};
    const previousItems = codexPlusModelPickerRecents();
    const items = previousItems.flatMap((item) => {
      let slug = item.slug;
      if (!slug) {
        const matches = Object.entries(before).filter(([, entry]) => {
          const display = entry.displayName || entry.display_name;
          if (typeof display !== "string") return false;
          const split = codexPlusModelPickerSplitLabel(display);
          return (!item.groupId || item.groupId === entry.codex_plus_group_id)
            && (!item.provider || item.provider === providerNameOf(entry))
            && (codexPlusModelPickerNormalizeLoose(display) === item.k
              || codexPlusModelPickerNormalizeLoose(split.name) === codexPlusModelPickerNormalizeLoose(item.name));
        });
        if (matches.length !== 1) return [item];
        slug = matches[0][0];
      }
      const entry = after[slug];
      const display = entry?.displayName || entry?.display_name;
      if (!slug || typeof display !== "string") return [item];
      return [{
        ...item, slug, k: codexPlusModelPickerNormalizeLoose(display),
        label: display, name: codexPlusModelPickerSplitLabel(display).name,
        provider: providerNameOf(entry), groupId: entry.codex_plus_group_id || "",
      }];
    });
    if (JSON.stringify(items) === JSON.stringify(previousItems)) return;
    codexPlusModelPickerGroupsState.recentsSignature = "";
    try {
      window.localStorage?.setItem(codexPlusModelPickerRecentsStoreKey, JSON.stringify({ items }));
    } catch {}
  }

  /** 点中某一行后记进最近使用（只观察、不干预原生点击）。 */
  function codexPlusModelPickerRecordRecent(entry) {
    if (!entry?.label) return;
    const items = codexPlusModelPickerRecents();
    const key = codexPlusModelPickerNormalizeLoose(entry.label) || entry.label;
    const next = {
      k: key,
      label: entry.label,
      name: entry.name || entry.label,
      provider: entry.provider || "",
      groupId: entry.groupId || "",
      slug: entry.slug || "",
    };
    const index = items.findIndex((item) => next.slug ? item.slug === next.slug || (!item.slug && item.k === key) : item.k === key);
    if (index >= 0) items.splice(index, 1);
    items.unshift(next);
    if (items.length > codexPlusModelPickerRecentsMax) items.length = codexPlusModelPickerRecentsMax;
    codexPlusModelPickerGroupsState.recentsSignature = "";
    try {
      window.localStorage?.setItem(codexPlusModelPickerRecentsStoreKey, JSON.stringify({ items }));
    } catch {}
  }

  /** 列表上的捕获期点击监听：选模型时把它记进最近使用。 */
  function codexPlusModelPickerRecordClick(event) {
    const list = event.currentTarget;
    const row = event.target?.closest?.('[role="menuitemradio"]');
    if (!row || !list.contains(row)) return;
    const entry = codexPlusModelPickerRows(list).find((item) => item.row === row);
    if (entry) codexPlusModelPickerRecordRecent(entry);
  }

  /**
   * 最近使用行：我们自己的按钮（不接管点击）——列表模式下先展开对应分组，
   * 图标模式下先下钻到对应分组，再把点击转发给真实原生行，选中逻辑完全交给原生。
   */
  function codexPlusModelPickerRecentRow(list, entry) {
    const text = codexPlusModelPickerText();
    const button = codexPlusModelPickerElement("button", "codex-plus-model-recent-row");
    button.type = "button";
    button.title = text.pickRecent(entry.name || entry.label, entry.provider);
    button.setAttribute("aria-label", button.title);
    const icon = codexPlusModelPickerElement("span", codexPlusModelPickerRowIconClass);
    icon.setAttribute("aria-hidden", "true");
    icon.innerHTML = codexPlusModelPickerGlyphHtml(codexPlusModelPickerGlyph(entry));
    const name = codexPlusModelPickerElement("span", "codex-plus-model-recent-name");
    name.textContent = entry.name || entry.label;
    const provider = codexPlusModelPickerElement("span", "codex-plus-model-recent-provider");
    provider.textContent = entry.provider || "";
    button.append(icon, name, provider);
    const meter = codexPlusModelPickerPriceNode(entry.price);
    if (meter) button.appendChild(meter);
    button.addEventListener("pointerdown", (event) => event.stopPropagation());
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (!entry.row.isConnected || !list.isConnected) return;
      const state = codexPlusModelPickerGroupsState;
      if (state.viewMode === "list") {
        if (entry.groupId && state.collapsed.has(entry.groupId)) {
          state.collapsed.delete(entry.groupId);
          codexPlusModelPickerPatch(list);
        }
      } else if (entry.groupId && state.activeGroupId !== entry.groupId) {
        codexPlusModelPickerNavigate(list, entry.groupId);
      }
      entry.row.click();
    });
    return button;
  }

  /**
   * 同步「最近使用」区块。始终是 list 的直接子节点：
   *   - 列表模式由 CSS order 摆到「原生前缀之后、第一组标题之前」（见 OrderPlan）；
   *   - 图标模式插在供应商网格之前（网格隐藏时自然贴在该组模型上方）。
   * 存的记录按当前行重新解析——组 id 变了也不碍事，点击时用当前行的归属。
   */
  function codexPlusModelPickerSyncRecents(list, rows, groups) {
    const state = codexPlusModelPickerGroupsState;
    const block = list.querySelector(`:scope > .${codexPlusModelPickerRecentsClass}`);
    const byKey = new Map();
    const bySlug = new Map();
    for (const entry of rows) {
      if (entry.slug) bySlug.set(entry.slug, entry);
      const key = codexPlusModelPickerNormalizeLoose(entry.label);
      byKey.set(key, byKey.has(key) ? null : entry);
      const nameKey = codexPlusModelPickerNormalizeLoose(entry.name);
      if (nameKey && nameKey !== key) byKey.set(nameKey, byKey.has(nameKey) ? null : entry);
    }
    const matched = [];
    for (const item of codexPlusModelPickerRecents()) {
      const entry = item.slug ? bySlug.get(item.slug)
        : byKey.get(item.k) || byKey.get(codexPlusModelPickerNormalizeLoose(item.name));
      // 旧记录没存路由身份时，同名路由组可能已被删除重建，不能猜成当前这一组。
      if (!item.slug && entry?.slug.startsWith("group/")) continue;
      if (entry && !matched.includes(entry)) matched.push(entry);
      if (matched.length >= 2) break;
    }
    if (!matched.length) {
      block?.remove();
      state.recentsSignature = "";
      return;
    }
    const signature = `${state.viewMode}|${matched.map((entry) => entry.label).join("|")}`;
    if (block && state.recentsSignature === signature && block.parentElement === list) return;
    state.recentsSignature = signature;
    block?.remove();
    const next = codexPlusModelPickerElement("div", codexPlusModelPickerRecentsClass);
    const label = codexPlusModelPickerElement("div", "codex-plus-model-section-label");
    label.textContent = codexPlusModelPickerText().recents;
    next.appendChild(label);
    for (const entry of matched) next.appendChild(codexPlusModelPickerRecentRow(list, entry));
    const grid = list.querySelector(":scope > .codex-plus-model-provider-grid");
    let anchor = grid;
    if (!anchor) {
      anchor = rows[0]?.row || null;
      while (anchor && anchor.parentElement !== list) anchor = anchor.parentElement;
    }
    list.insertBefore(next, anchor || null);
    // 列表模式靠 CSS order 排序：插入后重跑一次排序，让新区块拿到自己的槽位。
    if (state.viewMode === "list" && list.getAttribute("data-codex-plus-model-ordering") === "true") {
      codexPlusModelPickerApplyOrder(list, groups);
    }
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
    const state = codexPlusModelPickerGroupsState;
    const expanded = state.viewMode === "list" ? !state.collapsed.has(group.id) : state.activeGroupId === group.id;
    const header = list.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(group.id)}"]`);
    group.entries.forEach((entry) => {
      entry.row.setAttribute(codexPlusModelPickerGroupAttr, group.id);
      entry.row.classList.toggle("codex-plus-model-group-collapsed", !expanded);
    });
    if (header) {
      if (state.viewMode === "list") header.setAttribute("aria-expanded", String(expanded));
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
    codexPlusModelPickerInstallDecor(entry);
  }

  function codexPlusModelPickerRestoreLabel(row) {
    const content = row.querySelector("[data-menu-row-content]") || row;
    codexPlusModelPickerRemoveDecor(row);
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
      codexPlusModelPickerGroups(codexPlusModelPickerRows(list)),
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
    const controls = list.querySelector(".codex-plus-model-view-controls");
    const heading = list.querySelector("[data-codex-plus-model-heading]");
    const controlsParent = active ? toolbar : heading;
    if (controls && controlsParent && controls.parentElement !== controlsParent) controlsParent.appendChild(controls);
    groups.forEach((group) => codexPlusModelPickerSyncGroup(list, group));
    if (lostGroup) grid?.querySelector("button")?.focus({ preventScroll: true });
  }

  function codexPlusModelPickerNavigationKeydown(event) {
    const list = event.currentTarget;
    if (event.isComposing) return;
    if (codexPlusModelPickerGroupsState.viewMode === "list") {
      const header = event.target.closest?.(".codex-plus-model-group-header");
      if (header && (event.key === "Enter" || event.key === " ")) {
        event.stopPropagation();
        return;
      }
      if (header && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
        event.preventDefault();
        event.stopPropagation();
        const id = header.getAttribute(codexPlusModelPickerGroupIdAttr);
        if (event.key === "ArrowLeft") codexPlusModelPickerGroupsState.collapsed.add(id);
        else codexPlusModelPickerGroupsState.collapsed.delete(id);
        codexPlusModelPickerPatch(list);
        return;
      }
      const target = event.target.closest?.('[role="menuitemradio"], .codex-plus-model-group-header');
      if (!target || !["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
      const items = [...list.querySelectorAll('[role="menuitemradio"], .codex-plus-model-group-header')]
        .filter((node) => node.getClientRects().length && !node.classList.contains("codex-plus-model-group-collapsed"))
        .sort((a, b) => Number(a.getAttribute("data-codex-plus-model-order")) - Number(b.getAttribute("data-codex-plus-model-order")));
      const index = items.indexOf(target);
      const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
        : Math.max(0, Math.min(items.length - 1, index + (event.key === "ArrowUp" ? -1 : 1)));
      event.preventDefault();
      event.stopPropagation();
      items[next]?.focus({ preventScroll: true });
      return;
    }
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
    const glyph = codexPlusModelPickerProviderGlyph(group);
    const icon = codexPlusModelPickerElement("span", "codex-plus-model-provider-icon");
    icon.setAttribute("aria-hidden", "true");
    if (glyph) {
      icon.innerHTML = glyph.svg;
      icon.setAttribute("data-brand", "true");
    } else {
      // 列表模式图标位只有 16px：含中日韩等宽字时只放首字（「自动」→「自」、「A哥」→「A」）。
      const compact = codexPlusModelPickerGroupsState.viewMode === "list" && /[^\x00-\x7f]/u.test(avatar.label);
      icon.textContent = compact ? Array.from(avatar.label)[0] : avatar.label;
    }
    icon.style.setProperty("--codex-plus-provider-color", glyph ? "#8a8a8a" : avatar.color);
    const title = codexPlusModelPickerElement("span", "codex-plus-model-group-title");
    const count = codexPlusModelPickerElement("span", "codex-plus-model-group-count");
    header.append(icon, title, count);
    const groupId = group.id;
    if (codexPlusModelPickerGroupsState.viewMode === "list") {
      const chevron = codexPlusModelPickerElement("span", "codex-plus-model-group-chevron");
      chevron.appendChild(codexPlusModelPickerActionIcon("chevron"));
      header.insertBefore(chevron, icon);
    }
    header.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      const currentList = event.currentTarget.closest("[data-codex-plus-model-groups]");
      if (codexPlusModelPickerGroupsState.viewMode === "list") {
        const collapsed = codexPlusModelPickerGroupsState.collapsed;
        if (collapsed.has(groupId)) collapsed.delete(groupId);
        else collapsed.add(groupId);
        codexPlusModelPickerPatch(currentList);
      } else {
        codexPlusModelPickerNavigate(currentList, groupId);
      }
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
    codexPlusModelPickerRestoreOrder(list);
    codexPlusModelPickerRestoreDefaultRow(list);
    codexPlusModelPickerRemoveOwnNodes(list);
    list.querySelectorAll('[role="menuitemradio"]').forEach((row) => codexPlusModelPickerRestoreLabel(row));
    list.querySelectorAll(".codex-plus-model-provider-overview-only").forEach((node) => node.classList.remove("codex-plus-model-provider-overview-only"));
    list.removeEventListener("keydown", codexPlusModelPickerNavigationKeydown, true);
    window.removeEventListener("keydown", codexPlusModelPickerEscapeKeydown, true);
    list.removeEventListener("click", codexPlusModelPickerRecordClick, true);
    list.removeAttribute(codexPlusModelPickerRecentsHookAttr);
    list.removeAttribute("data-codex-plus-model-groups");
    list.removeAttribute("data-codex-plus-model-provider-view");
    list.removeAttribute("data-codex-plus-model-view-mode");
    list.querySelectorAll("[data-codex-plus-model-heading]").forEach((node) => node.removeAttribute("data-codex-plus-model-heading"));
    codexPlusModelPickerGroupsState.activeGroupId = "";
    codexPlusModelPickerGroupsState.signature = "";
    codexPlusModelPickerGroupsState.recentsSignature = "";
    codexPlusModelPickerGroupsState.patchedLists.delete(list);
  }

  /**
   * 分组标题与工具条是否还在原位。
   *
   * React 在菜单里做任何一次重渲染都可能顺手回收我们插进去的兄弟节点，
   * 此时指纹没变、但标题已经没了，只靠指纹判等会留下一个"点了没用"的死状态。
   */
  function codexPlusModelPickerHeadersIntact(list, groups) {
    const mode = codexPlusModelPickerGroupsState.viewMode;
    if (list.getAttribute("data-codex-plus-model-view-mode") !== mode) return false;
    const heading = codexPlusModelPickerHeading(list);
    const controls = list.querySelector(".codex-plus-model-view-controls");
    const actions = mode === "list" ? ["expand", "collapse", "icons"] : ["list"];
    if (heading) {
      if (!heading.hasAttribute("data-codex-plus-model-heading") || !controls || controls.children.length !== actions.length) return false;
      if (!actions.every((action, index) => {
        const button = controls.children[index];
        return button.getAttribute("data-codex-plus-model-action") === action
          && button.hasAttribute("aria-label") && button.hasAttribute("title") && !!button.querySelector("svg");
      })) return false;
    }
    if (mode === "list") {
      if (controls && controls.parentElement !== heading) return false;
      if (list.getAttribute("data-codex-plus-model-ordering") !== "true") return false;
      if (!groups.every((group) => {
        const header = list.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(group.id)}"]`);
        return header?.parentElement === list && header.hasAttribute("aria-expanded") && !!header.querySelector(".codex-plus-model-group-chevron");
      })) return false;
      return codexPlusModelPickerOrderPlan(list, groups).every((node, index) => {
        const order = String(index + 1);
        return node.getAttribute("data-codex-plus-model-order") === order
          && node.style.getPropertyValue("--codex-plus-model-order") === order;
      });
    }
    if (groups.length < 2) return !list.querySelector(".codex-plus-model-provider-grid, .codex-plus-model-group-header");
    const toolbar = list.querySelector(".codex-plus-model-group-toolbar");
    const grid = list.querySelector(".codex-plus-model-provider-grid");
    if (!toolbar || !grid || grid.children.length !== groups.length) return false;
    if (controls && controls.parentElement !== (codexPlusModelPickerGroupsState.activeGroupId ? toolbar : heading)) return false;
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
    codexPlusModelPickerLoadView();
    const rows = codexPlusModelPickerRows(list);
    const groups = codexPlusModelPickerUniqueGroupIds(
      codexPlusModelPickerGroups(rows),
    );
    if (!groups.length) {
      if (codexPlusModelPickerGroupsState.patchedLists.has(list) || list.hasAttribute("data-codex-plus-model-groups")) codexPlusModelPickerClear(list);
      else codexPlusModelPickerRemoveOwnNodes(list);
      return false;
    }
    const mode = codexPlusModelPickerGroupsState.viewMode;
    const grouped = mode === "list" || groups.length >= 2;
    const signature = `${mode}:${codexPlusModelPickerFingerprint(groups)}`;
    const patched = codexPlusModelPickerGroupsState.patchedLists.has(list);
    if (
      patched
      && signature === codexPlusModelPickerGroupsState.signature
      && list.isConnected
      && codexPlusModelPickerHeadersIntact(list, groups)
    ) {
      if (grouped) groups.forEach((group) => group.entries.forEach((entry) => codexPlusModelPickerInstallLabel(entry)));
      else rows.forEach((entry) => entry.slug.startsWith("group/")
        ? codexPlusModelPickerInstallLabel(entry) : codexPlusModelPickerInstallDecor(entry));
      if (mode === "list") codexPlusModelPickerSyncList(list, groups);
      else if (grouped) codexPlusModelPickerSyncNavigation(list, groups);
      codexPlusModelPickerSyncRecents(list, rows, groups);
      return false;
    }
    codexPlusModelPickerGroupsState.patchedLists.add(list);
    codexPlusModelPickerGroupsState.signature = signature;
    list.setAttribute("data-codex-plus-model-groups", "true");
    list.setAttribute("data-codex-plus-model-view-mode", mode);
    const focusedGroupId = document.activeElement?.closest?.(`[${codexPlusModelPickerGroupIdAttr}]`)?.getAttribute(codexPlusModelPickerGroupIdAttr)
      || groups.find((group) => group.entries.some((entry) => entry.row.contains(document.activeElement)))?.id;
    codexPlusModelPickerRestoreOrder(list);
    codexPlusModelPickerRemoveOwnNodes(list);
    if (!grouped) {
      list.querySelectorAll('[role="menuitemradio"]').forEach((row) => codexPlusModelPickerRestoreLabel(row));
      list.removeAttribute("data-codex-plus-model-provider-view");
      codexPlusModelPickerGroupsState.activeGroupId = "";
      // 普通模型保留原生标签；路由组仍以当前目录名称显示。
      rows.forEach((entry) => entry.slug.startsWith("group/")
        ? codexPlusModelPickerInstallLabel(entry) : codexPlusModelPickerInstallDecor(entry));
    }
    if (mode === "icons") codexPlusModelPickerRestoreDefaultRow(list);
    if (grouped) groups.forEach((group) => group.entries.forEach((entry) => codexPlusModelPickerInstallLabel(entry)));
    codexPlusModelPickerViewControls(list);
    const groupedRows = new Set(rows.map((entry) => entry.row));
    list.querySelectorAll('[role="menuitemradio"], [class*="_sectionLabel_"], [class^="sectionLabel-"], [class*=" sectionLabel-"]').forEach((node) => {
      node.classList.toggle("codex-plus-model-provider-overview-only", mode === "icons" && grouped && !groupedRows.has(node));
    });
    if (mode === "list") {
      list.removeAttribute("data-codex-plus-model-provider-view");
      groups.forEach((group) => {
        const anchor = group.entries[0]?.row;
        anchor?.parentElement?.insertBefore(codexPlusModelPickerGroupHeader(list, group), anchor);
      });
      codexPlusModelPickerSyncList(list, groups);
    } else if (grouped) {
      const grid = codexPlusModelPickerElement("div", "codex-plus-model-provider-grid");
      grid.setAttribute("role", "group");
      grid.setAttribute("aria-label", codexPlusModelPickerText().providers);
      groups.forEach((group) => grid.appendChild(codexPlusModelPickerGroupHeader(list, group)));
      const anchor = groups[0]?.entries[0]?.row;
      if (anchor?.parentElement) anchor.parentElement.insertBefore(grid, anchor);
      list.insertBefore(codexPlusModelPickerToolbar(), list.firstChild);
      codexPlusModelPickerSyncNavigation(list, groups);
    }
    list.addEventListener("keydown", codexPlusModelPickerNavigationKeydown, true);
    window.addEventListener("keydown", codexPlusModelPickerEscapeKeydown, true);
    if (!list.hasAttribute(codexPlusModelPickerRecentsHookAttr)) {
      list.addEventListener("click", codexPlusModelPickerRecordClick, true);
      list.setAttribute(codexPlusModelPickerRecentsHookAttr, "true");
    }
    codexPlusModelPickerSyncRecents(list, rows, groups);
    if (focusedGroupId && (mode === "list" || !codexPlusModelPickerGroupsState.activeGroupId)) {
      list.querySelector(`[${codexPlusModelPickerGroupIdAttr}="${cssEscapeValue(focusedGroupId)}"]`)?.focus({ preventScroll: true });
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
      .codex-plus-model-provider-icon{display:grid;place-items:center;flex:none;width:40px;height:40px;margin-bottom:2px;border:1px solid color-mix(in srgb,var(--codex-plus-provider-color) 38%,transparent);border-radius:8px;background:color-mix(in srgb,var(--codex-plus-provider-color) 18%,transparent);color:var(--color-token-text-primary,#dfdfdf);font-size:16px;font-weight:600;line-height:1;white-space:nowrap}
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
      [data-codex-plus-model-heading]{position:relative;padding-inline-end:40px!important}
      [data-codex-plus-model-view-mode="list"] [data-codex-plus-model-heading]{padding-inline-end:100px!important}
      .codex-plus-model-view-controls{position:absolute;inset-inline-end:4px;top:50%;transform:translateY(-50%);display:flex;align-items:center;gap:2px}
      .codex-plus-model-view-button{display:grid;place-items:center;flex:none;width:28px;height:28px;box-sizing:border-box;border:0;border-radius:5px;background:none;padding:0;color:var(--color-token-text-secondary,#aaa);cursor:pointer;appearance:none}
      .codex-plus-model-view-button:hover{background:var(--color-token-bg-secondary,rgba(255,255,255,.04));color:var(--color-token-text-primary,#dfdfdf)}
      .codex-plus-model-view-button:focus-visible{outline:1px solid currentColor;outline-offset:1px}
      .codex-plus-model-group-toolbar:has(>.codex-plus-model-view-controls){position:relative;padding-inline-end:40px}
      [data-codex-plus-model-ordering="true"]{display:flex!important;flex-direction:column!important}
      [data-codex-plus-model-ordering="true"]>[data-codex-plus-model-order]{order:var(--codex-plus-model-order)!important;flex-shrink:0}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-group-header{flex-direction:row;align-items:center;justify-content:flex-start;gap:6px;padding:8px;border:0;border-radius:4px;font-size:12px;line-height:18px;text-align:start}
      .codex-plus-model-provider-icon svg{display:block;width:22px;height:22px}
      .codex-plus-model-provider-icon[data-brand="true"]{color:var(--color-token-text-primary,#dfdfdf)}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-provider-icon{display:grid;place-items:center;flex:none;width:16px;height:16px;margin:0;border:0;outline:none!important;border-radius:0;background:none;color:var(--codex-plus-provider-color);font-size:10px;font-weight:700;line-height:1;white-space:nowrap;overflow:hidden}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-provider-icon[data-brand="true"]{color:var(--color-token-text-primary,#dfdfdf)}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-provider-icon svg{width:15px;height:15px}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-group-header::after{display:none}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-group-title{order:1;flex:0 1 auto}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-group-header[data-current="true"]::after{display:block;position:static;order:2;flex:none;width:6px;height:6px;border:0;background:var(--color-token-text-primary,#dfdfdf)}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-group-header .codex-plus-model-group-default{position:static;order:3;margin-inline-start:auto;transform:none;background:none}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-group-count{order:4}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-group-header:not(:has(.codex-plus-model-group-default)) .codex-plus-model-group-count{margin-inline-start:auto}
      [data-codex-plus-model-view-mode="list"] .codex-plus-model-group-item{padding-inline-start:28px!important}
      .codex-plus-model-group-chevron{display:grid;place-items:center;flex:none;width:14px;height:16px;color:var(--color-token-text-secondary,#aaa)}
      .codex-plus-model-group-header[aria-expanded="true"] .codex-plus-model-group-chevron{transform:rotate(90deg)}
      [data-codex-plus-model-default-content]{display:flex!important;flex-direction:row!important;align-items:center!important;gap:8px;width:100%;min-width:0}
      [data-codex-plus-model-default-note]{display:none!important}
      .codex-plus-model-default-name{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:end;color:var(--color-token-text-secondary,#aaa);font-size:12px}
      .codex-plus-model-section-label{padding:7px 8px 3px;color:var(--color-token-text-secondary,rgba(205,205,215,.68));font-size:11px;line-height:15px;font-weight:400;letter-spacing:.01em;user-select:none}
      .codex-plus-model-recents{display:flex;flex-direction:column;gap:1px;padding:0 2px 4px;min-width:0}
      .codex-plus-model-recent-row{display:flex;flex-direction:row;align-items:center;gap:10px;min-width:0;width:100%;box-sizing:border-box;margin:0;padding:7px 8px;border:1px solid transparent;border-radius:8px;background:none;color:var(--color-token-text-primary,#dfdfdf);font:inherit;font-size:13px;line-height:18px;text-align:left;cursor:pointer;user-select:none;appearance:none}
      .codex-plus-model-recent-row:hover{background:var(--color-token-bg-secondary,rgba(255,255,255,.04))}
      .codex-plus-model-recent-row:focus-visible{outline:1px solid color-mix(in srgb,var(--color-token-text-primary,#dfdfdf) 35%,transparent);outline-offset:1px}
      .codex-plus-model-recent-name{flex:0 1 auto;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:500}
      .codex-plus-model-recent-provider{flex:1 1 auto;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--color-token-text-secondary,rgba(205,205,215,.68));font-size:11px;font-weight:400}
      .codex-plus-model-decorated [data-codex-plus-model-check]{order:1}
      .codex-plus-model-decorated .codex-plus-model-row-meter,.codex-plus-model-decorated .codex-plus-model-row-free{order:2;margin-inline-start:6px!important}
      .codex-plus-model-row-icon{display:grid;place-items:center;flex:0 0 auto;width:16px;height:16px;margin-right:6px;color:var(--color-token-text-primary,#dfdfdf)}
      .codex-plus-model-row-icon svg{display:block;width:16px;height:16px}
      .codex-plus-model-row-icon-letter{font-size:11px;font-weight:700;line-height:1}
      .codex-plus-model-row-meter{display:inline-flex;align-items:center;gap:2px;flex:0 0 auto;margin-left:8px}
      .codex-plus-model-row-meter i{display:block;width:4px;height:8px;border-radius:2px;background:color-mix(in srgb,currentColor 18%,transparent)}
      .codex-plus-model-row-meter i.on:nth-child(1){background:#58c26f}
      .codex-plus-model-row-meter i.on:nth-child(2){background:#a5c95b}
      .codex-plus-model-row-meter i.on:nth-child(3){background:#e3b34d}
      .codex-plus-model-row-meter i.on:nth-child(4){background:#e07f63}
      .codex-plus-model-row-meter i.on:nth-child(5){background:#b579dd}
      .codex-plus-model-row-free{flex:0 0 auto;margin-left:8px;color:#6ea8ff;font-size:11px;line-height:1;font-weight:500;white-space:nowrap}
    `;
    if (!style.isConnected) document.head?.appendChild(style);
  }

  function codexPlusModelPickerNeedsRescan(records) {
    if (records.some((record) => record.type === "attributes"
      && ["aria-checked", "data-state", "data-model-selected"].includes(record.attributeName)
      && record.target.matches?.('[role="menuitemradio"]'))) return true;
    const changed = records.some((record) => {
      if (record.type === "attributes") return true;
      const nodes = [...record.addedNodes, ...record.removedNodes];
      return nodes.some((node) => node.nodeType === 1 && !node.closest?.(`[data-codex-plus-ext="${codexPlusModelPickerGroupsExtMark}"]`));
    });
    if (!changed) return false;
    const list = codexPlusModelPickerList(codexPlusModelPickerMenu());
    if (!list) return false;
    const rows = codexPlusModelPickerRows(list);
    const groups = codexPlusModelPickerUniqueGroupIds(
      codexPlusModelPickerGroups(rows),
    );
    if (!groups.length) return codexPlusModelPickerGroupsState.patchedLists.has(list);
    return `${codexPlusModelPickerGroupsState.viewMode}:${codexPlusModelPickerFingerprint(groups)}` !== codexPlusModelPickerGroupsState.signature
      || !codexPlusModelPickerHeadersIntact(list, groups);
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
    if (state.activeList) {
      codexPlusModelPickerClear(state.activeList);
      state.collapsed?.clear();
    }
    state.activeGroupId = "";
    state.activeList = list;
    state.observer?.disconnect();
    state.observer = new MutationObserver((records) => {
      if (!codexPlusModelPickerNeedsRescan(records)) return;
      codexPlusModelPickerScheduleRescan();
    });
    state.observer.observe(list, {
      childList: true, subtree: true, attributes: true,
      attributeFilter: ["aria-checked", "data-state", "data-model-selected"],
    });
  }

  /**
   * 一次扫描：菜单开着就确保分组已打好；关了就清理并停止轮询。
   *
   * 为什么还要轮询：React 复用行节点时不会产生 childList 变更，只靠 MutationObserver
   * 会漏掉"标签变了"这种情况。250ms 一次、且只在菜单开着时跑。
   */
  /**
   * 模型菜单开着却认不出来时（#2753：旧版 Codex 上分组不出现），记一次结构快照，
   * 好据此补上旧版的识别规则。只在页面上真有一个装着目录里模型的原生单选菜单时记，
   * 每个页面最多一次；只记标记数量和结构特征，不记模型名等文本。
   */
  function codexPlusModelPickerReportUnrecognized() {
    const state = codexPlusModelPickerGroupsState;
    if (state.unrecognizedReported) return;
    // #2883：诊断也节流到每 2 秒最多一次——菜单关着时每 250ms 的扫描轮
    // 都会走到这里，对每个 [role=menu] 读布局是白做工。
    const now = Date.now();
    if (state.lastUnrecognizedProbe && now - state.lastUnrecognizedProbe < 2000) return;
    state.lastUnrecognizedProbe = now;
    let exact = null;
    for (const menu of document.querySelectorAll('[role="menu"]')) {
      // 先过廉价的 DOM 过滤，通过了才碰 visibleElement 的强制布局。
      if (menu.closest("[data-codex-plus-ext]")) continue;
      const radios = [...menu.querySelectorAll('[role="menuitemradio"]')];
      if (radios.length < 2 || !visibleElement(menu)) continue;
      // 先过廉价的 DOM 条件，真有候选菜单时才建目录映射。
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
          modelList: count('[class*="_ModelList_"], [class^="ModelList-"], [class*=" ModelList-"]'),
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
    if (globalThis.__codexPlusBuiltinInactive?.has("model-picker-groups")) {
      if (state.activeList) codexPlusModelPickerClear(state.activeList);
      document.querySelectorAll("[data-codex-plus-model-groups]").forEach((list) => codexPlusModelPickerClear(list));
      state.activeList = null;
      state.observer?.disconnect();
      state.observer = null;
      return;
    }
    const menu = codexPlusModelPickerMenu();
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
    codexPlusModelPickerEnsurePrices();
    if (!document.getElementById(codexPlusModelPickerGroupsStyleId)) codexPlusModelPickerInstallStyle();
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
    version: 3,
    catalogChanged: (previous, next) => {
      if (next) codexPlusModelPickerMigrateRecents(previous, next);
      codexPlusModelPickerCatalogChanged(next ? {} : previous);
    },
    domState: () => codexPlusModelPickerDomState,
    viewState: () => ({
      mode: codexPlusModelPickerGroupsState.viewMode,
      collapsed: [...codexPlusModelPickerGroupsState.collapsed],
    }),
    sync: () => codexPlusModelPickerSchedule(0),
    scan: () => codexPlusModelPickerScan(),
    /** 测试与调试用：不打开菜单也能验证分组解析。 */
    splitLabel: (text) => codexPlusModelPickerSplitLabel(text),
    /** 测试与调试用：价格条档位与行图标解析。 */
    priceTier: (price) => codexPlusModelPickerPriceTier(price),
    glyphFor: (entry) => codexPlusModelPickerGlyph(entry),
    /** 测试与调试用：内部状态快照。 */
    state: () => ({
      signature: codexPlusModelPickerGroupsState.signature,
      hasActiveList: !!codexPlusModelPickerGroupsState.activeList,
      activeGroupId: codexPlusModelPickerGroupsState.activeGroupId,
      viewMode: codexPlusModelPickerGroupsState.viewMode,
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
        codexPlusModelPickerGroups(codexPlusModelPickerRows(list)),
      ).map((group) => ({ id: group.id, name: group.name, count: group.entries.length, isDefault: !!group.isDefault, entryGroupIds: [...new Set(group.entries.map((entry) => entry.groupId))].slice(0, 2) }));
    },
    dispose() {
      const state = codexPlusModelPickerGroupsState;
      state.disposed = true;
      clearTimeout(state.timer);
      clearTimeout(state.rescanTimer);
      state.observer?.disconnect();
      state.activeList?.removeEventListener("keydown", codexPlusModelPickerNavigationKeydown, true);
      state.activeList?.removeEventListener("click", codexPlusModelPickerRecordClick, true);
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
      document.querySelectorAll("[data-codex-plus-model-groups]").forEach((list) => codexPlusModelPickerClear(list));
      state.activeList = null;
      document.getElementById(codexPlusModelPickerGroupsStyleId)?.remove();
    },
  };
  if (globalThis.__codexPlusBuiltinInactive?.has("model-picker-groups")) codexPlusModelPickerScan();
