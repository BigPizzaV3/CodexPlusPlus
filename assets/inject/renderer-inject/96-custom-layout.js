  /**
   * 自定义布局只接管原生节点的样式，不移动 React 子树。
   * 布局数据持久、节点引用瞬态；节点重建后重新识别并应用同一份数据。
   */
  const codexPlusCustomLayoutKey = "codexPlus.customLayout.v1";
  const codexPlusCustomLayoutOwner = "custom-layout";
  const codexPlusCustomLayoutPanels = {
    rail: { label: "图标栏", minWidth: 44, resizableHeight: true },
    sidebar: { label: "侧边栏", minWidth: 180, resizableHeight: true },
    summary: { label: "详情卡", minWidth: 220, resizableHeight: false },
    composer: { label: "输入框", minWidth: 280, resizableHeight: false },
  };

  function codexPlusCustomLayoutFinite(value, fallback = 0) {
    return typeof value === "number" && Number.isFinite(value) ? value : fallback;
  }

  function codexPlusCustomLayoutClamp(value, minimum, maximum) {
    return Math.max(minimum, Math.min(maximum, value));
  }

  function normalizeLayout(value) {
    const result = { version: 1, panels: {} };
    if (!value || value.version !== 1 || !value.panels || typeof value.panels !== "object") return result;
    for (const id of Object.keys(codexPlusCustomLayoutPanels)) {
      const record = value.panels[id];
      if (!record || !Number.isFinite(record.x) || !Number.isFinite(record.y)) continue;
      const next = {
        x: codexPlusCustomLayoutClamp(record.x, 0, 1),
        y: codexPlusCustomLayoutClamp(record.y, 0, 1),
        snapX: ["left", "right"].includes(record.snapX) ? record.snapX : null,
        snapY: ["top", "bottom"].includes(record.snapY) ? record.snapY : null,
      };
      if (id === "sidebar" && record.grouped === true) next.grouped = true;
      for (const dimension of ["width", "height"]) {
        if (typeof record[dimension] === "number" && Number.isFinite(record[dimension]) && record[dimension] > 0) {
          next[dimension] = codexPlusCustomLayoutClamp(record[dimension], 1, 10000);
        }
      }
      result.panels[id] = next;
    }
    return result;
  }

  function codexPlusCustomLayoutBoundsValue(bounds) {
    return {
      left: codexPlusCustomLayoutFinite(bounds?.left),
      top: codexPlusCustomLayoutFinite(bounds?.top),
      width: Math.max(1, codexPlusCustomLayoutFinite(bounds?.width, 1)),
      height: Math.max(1, codexPlusCustomLayoutFinite(bounds?.height, 1)),
    };
  }

  function clampRect(rect, bounds, minWidth = 40, minHeight = 24) {
    const area = codexPlusCustomLayoutBoundsValue(bounds);
    const width = codexPlusCustomLayoutClamp(
      codexPlusCustomLayoutFinite(rect?.width, minWidth), Math.min(minWidth, area.width), area.width,
    );
    const height = codexPlusCustomLayoutClamp(
      codexPlusCustomLayoutFinite(rect?.height, minHeight), Math.min(minHeight, area.height), area.height,
    );
    return {
      left: codexPlusCustomLayoutClamp(codexPlusCustomLayoutFinite(rect?.left, area.left), area.left, area.left + area.width - width),
      top: codexPlusCustomLayoutClamp(codexPlusCustomLayoutFinite(rect?.top, area.top), area.top, area.top + area.height - height),
      width,
      height,
    };
  }

  function snapRect(rect, bounds, otherRects = [], threshold = 16) {
    const area = codexPlusCustomLayoutBoundsValue(bounds);
    const next = clampRect(rect, area);
    const distance = Math.max(0, codexPlusCustomLayoutFinite(threshold, 16));
    const xCandidates = [
      { value: area.left, edge: "left" },
      { value: area.left + area.width - next.width, edge: "right" },
    ];
    const yCandidates = [
      { value: area.top, edge: "top" },
      { value: area.top + area.height - next.height, edge: "bottom" },
    ];
    for (const other of otherRects) {
      if (![other?.left, other?.top, other?.width, other?.height].every(Number.isFinite)) continue;
      // 只吸附同一水平/垂直带内的面板，避免隔着整个窗口出现意外跳动。
      if (next.top < other.top + other.height + distance && next.top + next.height > other.top - distance) {
        for (const value of [other.left, other.left + other.width, other.left - next.width, other.left + other.width - next.width]) {
          xCandidates.push({ value, edge: null });
        }
      }
      if (next.left < other.left + other.width + distance && next.left + next.width > other.left - distance) {
        for (const value of [other.top, other.top + other.height, other.top - next.height, other.top + other.height - next.height]) {
          yCandidates.push({ value, edge: null });
        }
      }
    }
    const choose = (candidates, current, minimum, maximum) => candidates
      .filter((candidate) => candidate.value >= minimum && candidate.value <= maximum && Math.abs(candidate.value - current) <= distance)
      .sort((a, b) => Math.abs(a.value - current) - Math.abs(b.value - current))[0];
    const x = choose(xCandidates, next.left, area.left, area.left + area.width - next.width);
    const y = choose(yCandidates, next.top, area.top, area.top + area.height - next.height);
    if (x) next.left = x.value;
    if (y) next.top = y.value;
    return { rect: next, snapX: x?.edge || null, snapY: y?.edge || null };
  }

  function rectToRecord(rect, bounds, snaps = {}) {
    const area = codexPlusCustomLayoutBoundsValue(bounds);
    const next = clampRect(rect, area);
    return {
      x: (next.left - area.left) / Math.max(1, area.width - next.width),
      y: (next.top - area.top) / Math.max(1, area.height - next.height),
      width: next.width,
      height: next.height,
      snapX: ["left", "right"].includes(snaps.snapX) ? snaps.snapX : null,
      snapY: ["top", "bottom"].includes(snaps.snapY) ? snaps.snapY : null,
    };
  }

  function recordToRect(record, nativeRect, bounds) {
    const area = codexPlusCustomLayoutBoundsValue(bounds);
    const size = clampRect({ ...nativeRect, width: record.width ?? nativeRect.width, height: record.height ?? nativeRect.height }, area);
    const left = record.snapX === "left" ? area.left
      : record.snapX === "right" ? area.left + area.width - size.width
      : area.left + codexPlusCustomLayoutClamp(codexPlusCustomLayoutFinite(record.x), 0, 1) * (area.width - size.width);
    const top = record.snapY === "top" ? area.top
      : record.snapY === "bottom" ? area.top + area.height - size.height
      : area.top + codexPlusCustomLayoutClamp(codexPlusCustomLayoutFinite(record.y), 0, 1) * (area.height - size.height);
    return clampRect({ ...size, left, top }, area);
  }

  try { window.__codexPlusCustomLayoutRuntime?.cleanup?.(false); } catch {}
  const codexPlusCustomLayoutState = {
    enabled: false,
    editing: false,
    layout: { version: 1, panels: {} },
    entries: new Map(),
    styled: new Map(),
    ancestors: new Map(),
    root: null,
    toolbar: null,
    notice: "",
    observer: null,
    resizeObserver: null,
    raf: 0,
    retryTimer: 0,
    drag: null,
    motions: new Map(),
    motionRaf: 0,
    motionTime: null,
    motionGeneration: 0,
    projectedRects: null,
    projectionSignature: "",
    lastActiveId: null,
    listeners: [],
    storageLoaded: false,
    registered: false,
  };

  function codexPlusCustomLayoutEnabled() {
    try { return codexPlusSettings().customLayout === true; } catch { return false; }
  }

  function codexPlusCustomLayoutRead() {
    const state = codexPlusCustomLayoutState;
    if (state.storageLoaded) return;
    state.storageLoaded = true;
    try { state.layout = normalizeLayout(JSON.parse(localStorage.getItem(codexPlusCustomLayoutKey) || "null")); } catch {}
  }

  function codexPlusCustomLayoutSave() {
    const state = codexPlusCustomLayoutState;
    try {
      localStorage.setItem(codexPlusCustomLayoutKey, JSON.stringify(state.layout));
      state.notice = "";
    } catch {
      state.notice = "保存失败，当前布局仅在本次打开期间有效";
    }
    codexPlusCustomLayoutRenderToolbar();
  }

  function codexPlusCustomLayoutVisible(element) {
    if (!element?.isConnected || typeof element.getBoundingClientRect !== "function") return false;
    if (element.closest?.('[inert], [hidden], [aria-hidden="true"]')) return false;
    const rect = element.getBoundingClientRect();
    if (!(rect.width > 0 && rect.height > 0)) return false;
    const style = getComputedStyle(element);
    return style.display !== "none" && style.visibility !== "hidden";
  }

  function codexPlusCustomLayoutUnionRect(rects) {
    const left = Math.min(...rects.map((rect) => rect.left));
    const top = Math.min(...rects.map((rect) => rect.top));
    return {
      left, top,
      width: Math.max(...rects.map((rect) => rect.left + rect.width)) - left,
      height: Math.max(...rects.map((rect) => rect.top + rect.height)) - top,
    };
  }

  function codexPlusCustomLayoutElements(entry) {
    return entry.elements || [entry.element];
  }

  function codexPlusCustomLayoutStyledElements(entry) {
    const elements = codexPlusCustomLayoutElements(entry);
    return entry.grouped && elements.length === 1
      ? [...new Set([...elements, entry.railElement, entry.sidebarElement].filter(Boolean))]
      : elements;
  }

  function codexPlusCustomLayoutEntryRect(entry) {
    return codexPlusCustomLayoutUnionRect(codexPlusCustomLayoutElements(entry).map((element) => element.getBoundingClientRect()));
  }

  function codexPlusCustomLayoutEntryConnected(entry) {
    return codexPlusCustomLayoutElements(entry).every((element) => element.isConnected);
  }

  function codexPlusCustomLayoutMinWidth(entry) {
    return codexPlusCustomLayoutPanels[entry.id].minWidth + (entry.grouped ? entry.railWidth : 0);
  }

  function codexPlusCustomLayoutRestoreEntry(entry) {
    for (const element of codexPlusCustomLayoutStyledElements(entry)) {
      codexPlusCustomLayoutRestoreStyle(codexPlusCustomLayoutState.styled, element);
    }
  }

  /** 只选导航自己的外壳，不能把包含主内容或输入框的祖先一起拖走。 */
  function codexPlusCustomLayoutNavigationShellSafe(element, rect, protectedElements = [], ownedShell = false) {
    if (!element || element === document.body || element === document.documentElement || !codexPlusCustomLayoutVisible(element)) return false;
    if (element.matches?.('main, [data-app-shell-main-surface]')
      || element.querySelector?.('main, [data-app-shell-main-surface], [data-codex-composer-root], [data-summary-panel-variant]')
      || protectedElements.some((node) => node && (element === node || element.contains?.(node)))) return false;
    // 已接管的共同壳不会因一次 flex 尺寸重算就被误拆成两个 fixed 面板。
    if (ownedShell) return true;
    const measured = element.getBoundingClientRect();
    // 允许边框和内边距，拒绝覆盖整张工作区的背景容器。
    return measured.width <= rect.width + 24 && measured.height <= rect.height + 24
      && measured.left >= rect.left - 12 && measured.top >= rect.top - 12
      && measured.left <= rect.left + 12 && measured.top <= rect.top + 12
      && measured.left + measured.width <= rect.left + rect.width + 12
      && measured.top + measured.height <= rect.top + rect.height + 12
      && measured.left + measured.width >= rect.left + rect.width - 12
      && measured.top + measured.height >= rect.top + rect.height - 12;
  }

  function codexPlusCustomLayoutNavigationPart(element, protectedElements) {
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    let shell = element;
    for (let node = element.parentElement; node && node !== document.body && node !== document.documentElement; node = node.parentElement) {
      if (codexPlusCustomLayoutNavigationShellSafe(node, rect, protectedElements)) shell = node;
    }
    return shell;
  }

  function codexPlusCustomLayoutFindTargets() {
    const firstVisible = (selector) => Array.from(document.querySelectorAll(selector)).find(codexPlusCustomLayoutVisible) || null;
    const rail = firstVisible("nav[data-app-navigation-rail]");
    const sidebar = Array.from(document.querySelectorAll("#app-shell-sidebar .sidebar-navigation, aside.app-shell-left-panel .sidebar-navigation, aside.app-shell-left-panel nav"))
      .find((node) => node !== rail && !node.matches?.("[data-app-navigation-rail]")
        && !node.closest?.("nav[data-app-navigation-rail]") && !node.querySelector?.("nav[data-app-navigation-rail]")
        && codexPlusCustomLayoutVisible(node)) || null;
    const summary = firstVisible("[data-summary-panel-variant]");
    let composer = null;
    try { composer = conversationViewFindComposerEl(); } catch {}
    if (!codexPlusCustomLayoutVisible(composer)) composer = null;
    const protectedElements = [summary, composer];
    const railPart = codexPlusCustomLayoutNavigationPart(rail, [...protectedElements, sidebar]);
    const sidebarPart = codexPlusCustomLayoutNavigationPart(sidebar, [...protectedElements, rail]);
    if (rail && sidebar) {
      const union = codexPlusCustomLayoutUnionRect([railPart.getBoundingClientRect(), sidebarPart.getBoundingClientRect()]);
      const current = codexPlusCustomLayoutState.entries.get("sidebar");
      let shell = current?.grouped && current.elements.length === 1 && codexPlusCustomLayoutState.styled.has(current.element)
        && current.element.contains?.(rail) && current.element.contains?.(sidebar)
        && codexPlusCustomLayoutNavigationShellSafe(current.element, union, protectedElements, true) ? current.element : null;
      if (!shell) {
        for (let node = rail.parentElement; node && node !== document.body && node !== document.documentElement; node = node.parentElement) {
          if (node.contains?.(sidebar) && codexPlusCustomLayoutNavigationShellSafe(node, union, protectedElements)) shell = node;
        }
      }
      return {
        rail: null,
        sidebar: {
          element: shell || sidebarPart,
          elements: shell ? [shell] : [railPart, sidebarPart],
          grouped: true,
          railElement: railPart, sidebarElement: sidebarPart,
          railRect: railPart.getBoundingClientRect(),
          sidebarRect: sidebarPart.getBoundingClientRect(),
        },
        summary, composer,
      };
    }
    return { rail: railPart, sidebar: sidebarPart, summary, composer };
  }

  function codexPlusCustomLayoutBounds() {
    const width = Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1);
    const height = Math.max(1, window.innerHeight || document.documentElement.clientHeight || 1);
    let top = 8;
    for (const header of document.querySelectorAll('.app-header-tint, [class*="ApplicationMenuTopBar"]')) {
      const rect = header.getBoundingClientRect();
      if (rect.height >= 20 && rect.height <= 96 && rect.top < 32 && rect.width > width * 0.4) top = Math.max(top, rect.bottom + 4);
    }
    top = Math.min(top, Math.max(0, height - 1));
    const margin = Math.min(8, Math.max(0, (width - 1) / 2));
    return { left: margin, top, width: Math.max(1, width - margin * 2), height: Math.max(1, height - top - Math.min(8, height - top - 1)) };
  }

  function codexPlusCustomLayoutScale(element) {
    let scale = 1;
    for (let node = element; node; node = node.parentElement) {
      const raw = getComputedStyle(node).zoom;
      const value = Number.parseFloat(raw);
      if (Number.isFinite(value) && value > 0) scale *= String(raw).endsWith("%") ? value / 100 : value;
    }
    return scale > 0 && Number.isFinite(scale) ? scale : 1;
  }

  function codexPlusCustomLayoutStyleRecord(map, element) {
    let record = map.get(element);
    if (!record) {
      record = { original: new Map(), applied: new Map(), priorities: new Map(), requested: new Map(), lastStyle: "" };
      map.set(element, record);
    }
    return record;
  }

  function codexPlusCustomLayoutWriteStyle(map, element, values) {
    const record = codexPlusCustomLayoutStyleRecord(map, element);
    for (const [property, value] of Object.entries(values)) {
      if (!record.original.has(property)) {
        record.original.set(property, { value: element.style.getPropertyValue(property), priority: element.style.getPropertyPriority(property) });
      }
      // React/动画可能写回样式。保留它的新值，让关闭功能后回到最新原生状态。
      const previous = record.applied.get(property);
      const nativeChanged = record.applied.has(property)
        && (element.style.getPropertyValue(property) !== previous || element.style.getPropertyPriority(property) !== record.priorities.get(property));
      if (nativeChanged) {
        record.original.set(property, { value: element.style.getPropertyValue(property), priority: element.style.getPropertyPriority(property) });
      }
      // CSSOM 会将 38.400px 规范化为 38.4px。比较上次请求可以避开重复写入，
      // 归属判断则必须比较浏览器实际存下来的规范化值与优先级。
      if (nativeChanged || !record.requested.has(property) || record.requested.get(property) !== value) {
        element.style.setProperty(property, value, "important");
      }
      record.requested.set(property, value);
      record.applied.set(property, element.style.getPropertyValue(property));
      record.priorities.set(property, element.style.getPropertyPriority(property));
    }
    record.lastStyle = element.getAttribute("style") || "";
  }

  function codexPlusCustomLayoutRestoreStyle(map, element) {
    const record = map.get(element);
    if (!record) return;
    for (const [property, original] of record.original) {
      // 原生在我们停止期间写入的更新也不能被旧快照覆盖。
      if (element.style.getPropertyValue(property) !== record.applied.get(property)
        || element.style.getPropertyPriority(property) !== record.priorities.get(property)) continue;
      if (original.value) element.style.setProperty(property, original.value, original.priority);
      else element.style.removeProperty(property);
    }
    map.delete(element);
  }

  function codexPlusCustomLayoutReleaseAncestors(elements) {
    const state = codexPlusCustomLayoutState;
    const needed = new Set();
    for (const element of elements) {
      for (let node = element.parentElement; node && node !== document.documentElement; node = node.parentElement) needed.add(node);
    }
    for (const element of state.ancestors.keys()) {
      if (!needed.has(element)) codexPlusCustomLayoutRestoreStyle(state.ancestors, element);
    }
    for (const element of needed) {
      const computed = getComputedStyle(element);
      const existing = state.ancestors.get(element);
      const values = existing ? Object.fromEntries(existing.applied) : {};
      if (computed.contain && computed.contain !== "none" && computed.contain !== "style") values.contain = "none";
      for (const property of ["transform", "translate", "rotate", "scale", "perspective", "filter", "backdrop-filter", "clip-path"]) {
        const value = computed.getPropertyValue(property);
        if (value && value !== "none") values[property] = "none";
      }
      if (/transform|filter|perspective|contain/.test(computed.willChange || "")) values["will-change"] = "auto";
      if (computed.contentVisibility === "auto") values["content-visibility"] = "visible";
      for (const property of ["overflow-x", "overflow-y"]) {
        if (["hidden", "clip"].includes(computed.getPropertyValue(property))) values[property] = "visible";
      }
      if (Object.keys(values).length) codexPlusCustomLayoutWriteStyle(state.ancestors, element, values);
    }
  }

  function codexPlusCustomLayoutOwnedNode(node) {
    return !!node?.closest?.('[data-codex-plus-ext="custom-layout"]');
  }

  function codexPlusCustomLayoutUiNode(tag, className = "") {
    const node = document.createElement(tag);
    node.setAttribute("data-codex-plus-ext", codexPlusCustomLayoutOwner);
    node.className = className;
    return node;
  }

  function codexPlusCustomLayoutEnsureUi() {
    const state = codexPlusCustomLayoutState;
    if (state.root?.isConnected) return;
    const root = codexPlusCustomLayoutUiNode("div", "codex-plus-custom-layout-ui");
    root.style.cssText = "position:fixed;inset:0;pointer-events:none;z-index:1000;font:12px system-ui,sans-serif;color:var(--color-text-primary,#eee);";
    const style = codexPlusCustomLayoutUiNode("style");
    style.textContent = `
      .codex-plus-custom-layout-toolbar { position:absolute; display:flex; flex-wrap:wrap; align-items:center; justify-content:center; gap:6px; padding:7px 9px; width:max-content; max-width:calc(100% - 16px); box-sizing:border-box; border:1px solid rgba(128,128,128,.45); border-radius:12px; background:var(--color-surface,#242424); box-shadow:0 4px 20px #0004; pointer-events:auto; }
      .codex-plus-custom-layout-toolbar button,.codex-plus-custom-layout-grip { border:1px solid rgba(128,128,128,.45); border-radius:7px; background:var(--color-surface-secondary,#353535); color:inherit; padding:5px 8px; font:inherit; cursor:pointer; -webkit-app-region:no-drag; }
      .codex-plus-custom-layout-toolbar button:focus-visible,.codex-plus-custom-layout-grip:focus-visible { outline:2px solid #60a5fa; outline-offset:2px; }
      .codex-plus-custom-layout-toolbar button:disabled { opacity:.5; cursor:default; }
      .codex-plus-custom-layout-handle { position:absolute; display:flex; align-items:center; gap:3px; pointer-events:none; }
      .codex-plus-custom-layout-grip { pointer-events:auto; touch-action:none; user-select:none; cursor:grab; padding:3px 7px; font-size:11px; line-height:16px; box-shadow:0 2px 8px #0003; }
      .codex-plus-custom-layout-grip[data-resize] { cursor:nwse-resize; }
      .codex-plus-custom-layout-grip[data-dragging] { cursor:grabbing; background:#2563eb; color:white; }
      .codex-plus-custom-layout-outline { position:absolute; box-sizing:border-box; border:1px dashed #60a5fa; border-radius:10px; pointer-events:none; }
      .codex-plus-custom-layout-notice { flex-basis:100%; text-align:center; color:#fbbf24; }
    `;
    root.appendChild(style);
    const toolbar = codexPlusCustomLayoutUiNode("div", "codex-plus-custom-layout-toolbar");
    toolbar.setAttribute("role", "toolbar");
    toolbar.setAttribute("aria-label", "编辑自定义布局");
    root.appendChild(toolbar);
    (document.body || document.documentElement).appendChild(root);
    state.root = root;
    state.toolbar = toolbar;
  }

  function codexPlusCustomLayoutRenderToolbar() {
    const state = codexPlusCustomLayoutState;
    if (!state.editing || !state.toolbar) return;
    const toolbar = state.toolbar;
    toolbar.replaceChildren();
    const label = codexPlusCustomLayoutUiNode("span");
    label.textContent = "拖动让位 · 弹簧回弹 · Alt 暂停吸附";
    toolbar.appendChild(label);
    for (const [id, definition] of Object.entries(codexPlusCustomLayoutPanels)) {
      if (id === "rail" && state.entries.get("sidebar")?.grouped) continue;
      const button = codexPlusCustomLayoutUiNode("button");
      button.type = "button";
      const entry = state.entries.get(id);
      const saved = state.layout.panels[id];
      const status = !entry ? "未出现" : saved ? (saved.snapX || saved.snapY ? "已吸附" : "悬浮") : "原位";
      button.textContent = `${definition.label}·${status}`;
      button.disabled = !entry;
      button.title = entry ? "定位此面板的拖动手柄；双击手柄恢复该面板" : "面板再次出现时会恢复保存的位置";
      button.addEventListener("click", () => entry?.move?.focus());
      toolbar.appendChild(button);
    }
    for (const [text, action] of [["恢复默认", codexPlusCustomLayoutReset], ["完成", () => codexPlusCustomLayoutSetEditing(false)]]) {
      const button = codexPlusCustomLayoutUiNode("button");
      button.type = "button";
      button.textContent = text;
      button.addEventListener("click", action);
      toolbar.appendChild(button);
    }
    if (state.notice) {
      const notice = codexPlusCustomLayoutUiNode("span", "codex-plus-custom-layout-notice");
      notice.setAttribute("role", "status");
      notice.textContent = state.notice;
      toolbar.appendChild(notice);
    }
    const bounds = codexPlusCustomLayoutBounds();
    const scale = codexPlusCustomLayoutScale(state.root);
    toolbar.style.left = `${(bounds.left + bounds.width / 2) / scale}px`;
    toolbar.style.top = `${(bounds.top + 4) / scale}px`;
    toolbar.style.transform = "translateX(-50%)";
  }

  function codexPlusCustomLayoutPreferredRect(entry, bounds) {
    const record = codexPlusCustomLayoutState.layout.panels[entry.id];
    const measured = codexPlusCustomLayoutEntryRect(entry);
    const native = { left: measured.left, top: measured.top, width: measured.width, height: measured.height };
    if (!record) return native;
    // 输入框/详情卡的内容决定高度，持久数据只约束其宽度。
    const preferred = { ...record };
    if (!codexPlusCustomLayoutPanels[entry.id].resizableHeight) delete preferred.height;
    return clampRect(recordToRect(preferred, native, bounds), bounds,
      codexPlusCustomLayoutMinWidth(entry),
      codexPlusCustomLayoutPanels[entry.id].resizableHeight ? 80 : 24);
  }

  function codexPlusCustomLayoutCreateEntry(id, target) {
    const element = target.element || target;
    const elements = Array.isArray(target.elements) ? target.elements : [element];
    const partRects = elements.map((part) => part.getBoundingClientRect());
    return {
      id, element, elements, grouped: target.grouped === true,
      railWidth: target.grouped ? target.railRect.width : 0,
      railElement: target.railElement, sidebarElement: target.sidebarElement,
      railRect: target.railRect, sidebarRect: target.sidebarRect,
      nativeGroupRect: codexPlusCustomLayoutUnionRect(partRects), partRects,
      handle: null, outline: null, move: null, justAdded: true,
    };
  }

  /** v1 的聊天栏/图标栏位置升级为整组位置；正式交互保存时再写回。 */
  function codexPlusCustomLayoutMigrateNavigation(entry, bounds) {
    if (!entry?.grouped) return;
    const panels = codexPlusCustomLayoutState.layout.panels;
    const sidebarRecord = panels.sidebar;
    const railRecord = panels.rail;
    if (sidebarRecord?.grouped !== true && (sidebarRecord || railRecord)) {
      const native = entry.nativeGroupRect;
      const part = sidebarRecord ? entry.sidebarRect : entry.railRect;
      const previous = recordToRect(sidebarRecord || railRecord, part, bounds);
      const rect = {
        left: previous.left - (part.left - native.left),
        top: previous.top - (part.top - native.top),
        width: previous.width + native.width - part.width,
        height: previous.height + native.height - part.height,
      };
      panels.sidebar = { ...rectToRecord(rect, bounds, sidebarRecord || railRecord), grouped: true };
    }
    // 新版只有一个求解单位，旧图标栏记录不能继续与侧边栏竞争位置。
    delete panels.rail;
  }

  function codexPlusCustomLayoutNavigationPartRect(entry, rect, part) {
    const native = entry.nativeGroupRect;
    const railStart = entry.railRect.left - native.left;
    const railEnd = railStart + entry.railWidth;
    const horizontalScale = Math.max(0, (rect.width - entry.railWidth) / Math.max(1, native.width - entry.railWidth));
    // 调整组宽时保留图标栏宽度，剩余空间交给聊天栏；两块仍是同一个碰撞单位。
    const x = (value) => value <= railStart ? value * horizontalScale
      : value < railEnd ? railStart * horizontalScale + value - railStart
      : railStart * horizontalScale + entry.railWidth + (value - railEnd) * horizontalScale;
    const left = x(part.left - native.left);
    const right = x(part.left + part.width - native.left);
    const verticalScale = rect.height / Math.max(1, native.height);
    return {
      left: rect.left + left, top: rect.top + (part.top - native.top) * verticalScale,
      width: Math.max(1, right - left), height: Math.max(1, part.height * verticalScale),
    };
  }

  function codexPlusCustomLayoutPartRect(entry, rect, index) {
    return entry.elements.length === 1 ? rect : codexPlusCustomLayoutNavigationPartRect(entry, rect, entry.partRects[index]);
  }

  function codexPlusCustomLayoutApplyNavigationWidths(entry, rect) {
    if (!entry.grouped || entry.elements.length !== 1) return;
    for (const [element, native] of [[entry.railElement, entry.railRect], [entry.sidebarElement, entry.sidebarRect]]) {
      if (!element?.isConnected || element === entry.element) continue;
      const part = codexPlusCustomLayoutNavigationPartRect(entry, rect, native);
      const width = `${(part.width / codexPlusCustomLayoutScale(element)).toFixed(3)}px`;
      // 只约束共同壳里的正常流子面板，不设置 position，不更换滚动/裁剪规则。
      codexPlusCustomLayoutWriteStyle(codexPlusCustomLayoutState.styled, element, {
        width, "min-width": "0", "max-width": width, "flex-basis": width,
        "flex-grow": "0", "flex-shrink": "0", "box-sizing": "border-box",
      });
    }
  }

  function codexPlusCustomLayoutRefreshNavigationParts(entry, target) {
    if (!entry.grouped || entry.elements.length !== 1) return;
    const next = [target.railElement, target.sidebarElement];
    for (const element of [entry.railElement, entry.sidebarElement]) {
      if (element && !next.includes(element)) codexPlusCustomLayoutRestoreStyle(codexPlusCustomLayoutState.styled, element);
    }
    entry.railElement = target.railElement;
    entry.sidebarElement = target.sidebarElement;
  }

  function codexPlusCustomLayoutApplyEntry(entry, bounds) {
    const state = codexPlusCustomLayoutState;
    const record = state.layout.panels[entry.id];
    const motion = state.motions.get(entry.id);
    const projection = state.projectedRects?.[entry.id];
    if (!record && !motion && !projection) {
      codexPlusCustomLayoutRestoreEntry(entry);
      return;
    }
    if (entry.id === "composer" && !state.styled.has(entry.element)
      && typeof conversationViewRestoreElement === "function"
      && typeof conversationViewState !== "undefined" && conversationViewState.elements?.has(entry.element)) {
      // 居中功能的旧快照不能在稍后 cleanup 时覆盖我们接管后的尺寸和位置。
      conversationViewRestoreElement(entry.element);
      conversationViewState.elements.delete(entry.element);
    }
    const rect = motion?.current || projection || codexPlusCustomLayoutPreferredRect(entry, bounds);
    for (const [index, element] of entry.elements.entries()) {
      const part = codexPlusCustomLayoutPartRect(entry, rect, index);
      const scale = codexPlusCustomLayoutScale(element);
      const px = (value) => `${(value / scale).toFixed(3)}px`;
      const values = {
        position: "fixed", left: px(part.left), top: px(part.top), right: "auto", bottom: "auto",
        width: px(part.width), "min-width": "0", "max-width": px(bounds.width),
        "max-height": px(bounds.height), "box-sizing": "border-box", margin: "0",
        transform: "none", translate: "none", rotate: "none", scale: "none", "z-index": "120",
      };
      if (codexPlusCustomLayoutPanels[entry.id].resizableHeight) {
        values.height = px(part.height);
        // 整壳保持宿主自己的滚动分工，不能再给 rail/chat 外面套一层滚动条。
        if (!entry.grouped) {
          values["overflow-y"] = "auto";
          values["overflow-x"] = "hidden";
        }
      }
      codexPlusCustomLayoutWriteStyle(state.styled, element, values);
    }
    codexPlusCustomLayoutApplyNavigationWidths(entry, rect);
  }

  function codexPlusCustomLayoutInstallHandle(entry) {
    const state = codexPlusCustomLayoutState;
    if (!state.editing || !state.root) return;
    if (entry.handle?.isConnected) return;
    const handle = codexPlusCustomLayoutUiNode("div", "codex-plus-custom-layout-handle");
    const outline = codexPlusCustomLayoutUiNode("div", "codex-plus-custom-layout-outline");
    const move = codexPlusCustomLayoutUiNode("button", "codex-plus-custom-layout-grip");
    move.type = "button";
    move.textContent = entry.id === "rail" ? "⠿" : `⠿ ${codexPlusCustomLayoutPanels[entry.id].label}`;
    move.setAttribute("aria-label", `拖动${codexPlusCustomLayoutPanels[entry.id].label}，方向键移动，双击恢复原位`);
    move.title = "拖动时其他面板会弹性让位；Alt 暂停吸附；方向键移动，Shift 加速；双击恢复原位";
    move.addEventListener("pointerdown", (event) => codexPlusCustomLayoutBeginDrag(event, entry.id, false));
    move.addEventListener("keydown", (event) => codexPlusCustomLayoutKeyboard(event, entry.id, false));
    move.addEventListener("dblclick", () => codexPlusCustomLayoutResetPanel(entry.id));
    const resize = codexPlusCustomLayoutUiNode("button", "codex-plus-custom-layout-grip");
    resize.type = "button";
    resize.textContent = "↔";
    resize.setAttribute("data-resize", "true");
    resize.setAttribute("aria-label", `调整${codexPlusCustomLayoutPanels[entry.id].label}尺寸`);
    resize.title = codexPlusCustomLayoutPanels[entry.id].resizableHeight ? "拖动调整宽高，方向键也可调整" : "拖动调整宽度，左右键也可调整";
    if (entry.id === "rail") handle.style.flexDirection = "column";
    resize.addEventListener("pointerdown", (event) => codexPlusCustomLayoutBeginDrag(event, entry.id, true));
    resize.addEventListener("keydown", (event) => codexPlusCustomLayoutKeyboard(event, entry.id, true));
    handle.appendChild(move);
    handle.appendChild(resize);
    state.root.appendChild(outline);
    state.root.appendChild(handle);
    Object.assign(entry, { handle, outline, move });
  }

  function codexPlusCustomLayoutPositionHandle(entry) {
    if (!entry.handle || !entry.outline) return;
    const bounds = codexPlusCustomLayoutBounds();
    const rect = codexPlusCustomLayoutEntryRect(entry);
    const scale = codexPlusCustomLayoutScale(codexPlusCustomLayoutState.root);
    const left = codexPlusCustomLayoutClamp(rect.left, bounds.left, bounds.left + bounds.width - 24);
    const top = codexPlusCustomLayoutClamp(rect.top, bounds.top, bounds.top + bounds.height - 24);
    entry.handle.style.left = `${left / scale}px`;
    entry.handle.style.top = `${top / scale}px`;
    entry.outline.style.left = `${rect.left / scale}px`;
    entry.outline.style.top = `${rect.top / scale}px`;
    entry.outline.style.width = `${rect.width / scale}px`;
    entry.outline.style.height = `${rect.height / scale}px`;
  }

  function codexPlusCustomLayoutOtherRects(id) {
    return [...codexPlusCustomLayoutState.entries.values()]
      .filter((entry) => entry.id !== id && codexPlusCustomLayoutElements(entry).every(codexPlusCustomLayoutVisible))
      .map(codexPlusCustomLayoutEntryRect);
  }

  function codexPlusCustomLayoutRecordFor(id, rect, snaps = {}) {
    const record = rectToRecord(rect, codexPlusCustomLayoutBounds(), snaps);
    if (id === "sidebar" && codexPlusCustomLayoutState.entries.get(id)?.grouped) record.grouped = true;
    if (!codexPlusCustomLayoutPanels[id].resizableHeight) delete record.height;
    return record;
  }

  function codexPlusCustomLayoutClone(layout) {
    return { version: 1, panels: Object.fromEntries(Object.entries(layout.panels).map(([id, record]) => [id, { ...record }])) };
  }

  function codexPlusCustomLayoutRectEqual(a, b, tolerance = 0.15) {
    return !!a && !!b && ["left", "top", "width", "height"].every((key) => Math.abs(a[key] - b[key]) <= tolerance);
  }

  function codexPlusCustomLayoutReducedMotion() {
    return window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches === true;
  }

  function codexPlusCustomLayoutCaptureRects() {
    const state = codexPlusCustomLayoutState;
    return Object.fromEntries([...state.entries.values()].map((entry) => {
      const measured = state.motions.get(entry.id)?.target || state.projectedRects?.[entry.id] || codexPlusCustomLayoutEntryRect(entry);
      const definition = codexPlusCustomLayoutPanels[entry.id];
      return [entry.id, {
        left: measured.left, top: measured.top, width: measured.width, height: measured.height,
        minWidth: codexPlusCustomLayoutMinWidth(entry), minHeight: definition.resizableHeight ? 80 : measured.height,
        canResizeHeight: definition.resizableHeight,
      }];
    }));
  }

  function codexPlusCustomLayoutStopMotion(settle = false) {
    const state = codexPlusCustomLayoutState;
    state.motionGeneration += 1;
    if (state.motionRaf) cancelAnimationFrame(state.motionRaf);
    state.motionRaf = 0;
    state.motionTime = null;
    if (settle && state.enabled) {
      const bounds = codexPlusCustomLayoutBounds();
      for (const [id, motion] of state.motions) {
        motion.current = { ...motion.target };
        const entry = state.entries.get(id);
        if (entry) codexPlusCustomLayoutApplyEntry(entry, bounds);
      }
    }
    state.motions.clear();
  }

  function codexPlusCustomLayoutMotionFrame(time, generation) {
    const state = codexPlusCustomLayoutState;
    if (!state.enabled || generation !== state.motionGeneration) return;
    state.motionRaf = 0;
    const now = Number.isFinite(time) ? time : performance.now();
    const dt = state.motionTime == null ? 1 / 60 : Math.max(0.001, Math.min(0.05, (now - state.motionTime) / 1000));
    state.motionTime = now;
    const bounds = codexPlusCustomLayoutBounds();
    const reducedMotion = codexPlusCustomLayoutReducedMotion();
    const finished = [];
    // 碰撞求解只写目标；中间帧独立推进弹簧，不能拿中间帧反过来求解。
    for (const [id, motion] of state.motions) {
      const entry = state.entries.get(id);
      if (!entry || !codexPlusCustomLayoutEntryConnected(entry)) { finished.push(id); continue; }
      let settled = true;
      const restoring = !state.layout.panels[id] && !state.projectedRects?.[id];
      const area = restoring
        ? { left: 0, top: 0, width: window.innerWidth, height: window.innerHeight }
        : bounds;
      for (const key of ["width", "height", "left", "top"]) {
        const minimum = key === "left" ? area.left : key === "top" ? area.top : 1;
        const maximum = key === "left" ? area.left + area.width - motion.current.width
          : key === "top" ? area.top + area.height - motion.current.height
          : key === "width" ? area.width : area.height;
        const next = springStep(motion.current[key], motion.velocity[key], motion.target[key], dt,
          { stiffness: 290, damping: 23, mass: 1, min: minimum, max: maximum, reducedMotion });
        motion.current[key] = next.position;
        motion.velocity[key] = next.velocity;
        settled &&= next.settled;
      }
      if (settled) { motion.current = { ...motion.target }; finished.push(id); }
      codexPlusCustomLayoutApplyEntry(entry, bounds);
    }
    for (const id of finished) {
      state.motions.delete(id);
      const entry = state.entries.get(id);
      if (entry) codexPlusCustomLayoutApplyEntry(entry, bounds);
    }
    if (state.editing) for (const entry of state.entries.values()) codexPlusCustomLayoutPositionHandle(entry);
    if (state.motions.size) {
      state.motionRaf = requestAnimationFrame((timestamp) => codexPlusCustomLayoutMotionFrame(timestamp, generation));
    } else {
      state.motionTime = null;
      codexPlusCustomLayoutSchedule();
    }
  }

  function codexPlusCustomLayoutAnimate(rects, immediateId = null, releaseVelocity = null) {
    const state = codexPlusCustomLayoutState;
    const reducedMotion = codexPlusCustomLayoutReducedMotion();
    for (const [id, target] of Object.entries(rects)) {
      const entry = state.entries.get(id);
      if (!entry || !codexPlusCustomLayoutEntryConnected(entry)) continue;
      if (id === immediateId || reducedMotion || entry.justAdded) { state.motions.delete(id); continue; }
      const old = state.motions.get(id);
      if (old && codexPlusCustomLayoutRectEqual(old.target, target)) continue;
      const measured = old?.current || codexPlusCustomLayoutEntryRect(entry);
      const current = { left: measured.left, top: measured.top, width: measured.width, height: measured.height };
      const velocity = old?.velocity || { left: 0, top: 0, width: 0, height: 0 };
      if (releaseVelocity?.id === id) {
        velocity.left = releaseVelocity.left;
        velocity.top = releaseVelocity.top;
      }
      if (codexPlusCustomLayoutRectEqual(current, target) && Math.abs(velocity.left) + Math.abs(velocity.top) < 1) continue;
      state.motions.set(id, { current, velocity: { ...velocity }, target: { ...target } });
    }
    if (state.motions.size && !state.motionRaf) {
      const generation = state.motionGeneration;
      state.motionRaf = requestAnimationFrame((timestamp) => codexPlusCustomLayoutMotionFrame(timestamp, generation));
    }
  }

  function codexPlusCustomLayoutApplySolution(solution, baseline, previousLayout, activeId, snaps = {}, immediate = true) {
    const state = codexPlusCustomLayoutState;
    const next = codexPlusCustomLayoutClone(previousLayout);
    for (const [id, rect] of Object.entries(solution.rects)) {
      if (id === activeId || !codexPlusCustomLayoutRectEqual(rect, baseline[id])) {
        // 让位后的面板清掉原吸附边；不能被旧 snap 再拉回碰撞区。
        next.panels[id] = codexPlusCustomLayoutRecordFor(id, rect, id === activeId && !solution.blocked ? snaps : {});
      }
    }
    state.layout = next;
    state.projectedRects = solution.rects;
    state.projectionSignature = "";
    state.lastActiveId = activeId;
    state.notice = solution.blocked ? "空间不足，拖动已限制在可用位置" : "";
    codexPlusCustomLayoutAnimate(solution.rects, immediate ? activeId : null);
    codexPlusCustomLayoutRefresh();
  }

  function codexPlusCustomLayoutBeginDrag(event, id, resize) {
    const state = codexPlusCustomLayoutState;
    const entry = state.entries.get(id);
    if (!state.enabled || !state.editing || !entry || event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    codexPlusCustomLayoutFinishDrag(true);
    codexPlusCustomLayoutStopMotion(true);
    const origin = codexPlusCustomLayoutEntryRect(entry);
    const handle = event.currentTarget;
    const drag = {
      id, resize, handle, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY,
      origin: { left: origin.left, top: origin.top, width: origin.width, height: origin.height },
      previousLayout: codexPlusCustomLayoutClone(state.layout), baseline: codexPlusCustomLayoutCaptureRects(),
      previousProjection: state.projectedRects, moved: false,
      velocityX: 0, velocityY: 0, moveTime: performance.now(), lastX: event.clientX, lastY: event.clientY,
    };
    state.drag = drag;
    try { handle.setPointerCapture(event.pointerId); } catch {}
    const onMove = (move) => {
      if (state.drag !== drag || move.pointerId !== drag.pointerId) return;
      const dx = move.clientX - drag.startX;
      const dy = move.clientY - drag.startY;
      if (!drag.moved && Math.hypot(dx, dy) < 3) return;
      drag.moved = true;
      move.preventDefault();
      handle.setAttribute("data-dragging", "true");
      const area = codexPlusCustomLayoutBounds();
      const candidate = resize
        ? { ...drag.origin, width: drag.origin.width + dx, height: drag.origin.height + (codexPlusCustomLayoutPanels[id].resizableHeight ? dy : 0) }
        : { ...drag.origin, left: drag.origin.left + dx, top: drag.origin.top + dy };
      const clamped = clampRect(candidate, area, codexPlusCustomLayoutMinWidth(entry),
        codexPlusCustomLayoutPanels[id].resizableHeight ? 80 : 24);
      // 吸附候选与碰撞输入使用起始快照；动画中的实际坐标不会改变下一帧解算。
      const stableSnap = resize || move.altKey ? { rect: clamped }
        : snapRect(clamped, area, Object.entries(drag.baseline).filter(([otherId]) => otherId !== id).map(([, rect]) => rect));
      const solution = codexPlusLayoutSolve(drag.baseline, id, stableSnap.rect, area, drag.origin);
      if (solution.feasible === false) {
        state.notice = "空间不足，请先缩小面板再拖动";
        codexPlusCustomLayoutRenderToolbar();
        return;
      }
      const now = Number.isFinite(move.timeStamp) ? move.timeStamp : performance.now();
      const elapsed = Math.max(0.016, Math.min(0.1, (now - drag.moveTime) / 1000));
      drag.velocityX = codexPlusCustomLayoutClamp((move.clientX - drag.lastX) / elapsed, -1200, 1200);
      drag.velocityY = codexPlusCustomLayoutClamp((move.clientY - drag.lastY) / elapsed, -1200, 1200);
      drag.lastX = move.clientX; drag.lastY = move.clientY; drag.moveTime = now;
      codexPlusCustomLayoutApplySolution(solution, drag.baseline, drag.previousLayout, id, stableSnap);
    };
    const onEnd = (end) => {
      if (end.pointerId !== drag.pointerId) return;
      codexPlusCustomLayoutFinishDrag(end.type !== "pointercancel");
    };
    drag.cleanup = () => {
      window.removeEventListener("pointermove", onMove, true);
      window.removeEventListener("pointerup", onEnd, true);
      window.removeEventListener("pointercancel", onEnd, true);
      handle.removeAttribute("data-dragging");
      try { handle.releasePointerCapture(drag.pointerId); } catch {}
    };
    window.addEventListener("pointermove", onMove, { capture: true, passive: false });
    window.addEventListener("pointerup", onEnd, true);
    window.addEventListener("pointercancel", onEnd, true);
  }

  function codexPlusCustomLayoutFinishDrag(commit) {
    const state = codexPlusCustomLayoutState;
    const drag = state.drag;
    if (!drag) return;
    state.drag = null;
    drag.cleanup?.();
    if (!commit) {
      state.layout = codexPlusCustomLayoutClone(drag.previousLayout);
      state.projectedRects = drag.previousProjection;
      state.projectionSignature = "";
      if (state.enabled && drag.moved) codexPlusCustomLayoutAnimate(drag.baseline);
    } else if (drag.moved) {
      // 保存最终整组目标，不把弹簧中间帧写入磁盘。
      codexPlusCustomLayoutSave();
      if (state.enabled && state.projectedRects) codexPlusCustomLayoutAnimate(state.projectedRects, null,
        { id: drag.id, left: drag.velocityX * 0.12, top: drag.velocityY * 0.12 });
    }
    if (state.enabled) codexPlusCustomLayoutSchedule();
  }

  function codexPlusCustomLayoutKeyboard(event, id, resize) {
    const state = codexPlusCustomLayoutState;
    const entry = state.entries.get(id);
    if (!entry || !state.editing || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
    event.preventDefault();
    event.stopPropagation();
    const delta = event.shiftKey ? 24 : 4;
    const dx = event.key === "ArrowLeft" ? -delta : event.key === "ArrowRight" ? delta : 0;
    const dy = event.key === "ArrowUp" ? -delta : event.key === "ArrowDown" ? delta : 0;
    codexPlusCustomLayoutStopMotion(true);
    const baseline = codexPlusCustomLayoutCaptureRects();
    const previousLayout = codexPlusCustomLayoutClone(state.layout);
    const native = baseline[id];
    const candidate = resize
      ? { left: native.left, top: native.top, width: native.width + dx, height: native.height + (codexPlusCustomLayoutPanels[id].resizableHeight ? dy : 0) }
      : { left: native.left + dx, top: native.top + dy, width: native.width, height: native.height };
    const bounds = codexPlusCustomLayoutBounds();
    const clamped = clampRect(candidate, bounds, codexPlusCustomLayoutMinWidth(entry),
      codexPlusCustomLayoutPanels[id].resizableHeight ? 80 : 24);
    const snapped = resize || event.altKey ? { rect: clamped } : snapRect(clamped, bounds, codexPlusCustomLayoutOtherRects(id));
    const solution = codexPlusLayoutSolve(baseline, id, snapped.rect, bounds, native);
    if (solution.feasible === false) return;
    codexPlusCustomLayoutApplySolution(solution, baseline, previousLayout, id, snapped, false);
    codexPlusCustomLayoutSave();
  }

  function codexPlusCustomLayoutDropEntry(entry) {
    const state = codexPlusCustomLayoutState;
    if (state.drag?.id === entry.id) codexPlusCustomLayoutFinishDrag(false);
    state.motions.delete(entry.id);
    for (const element of codexPlusCustomLayoutElements(entry)) state.resizeObserver?.unobserve?.(element);
    codexPlusCustomLayoutRestoreEntry(entry);
    entry.handle?.remove();
    entry.outline?.remove();
    state.entries.delete(entry.id);
  }

  function codexPlusCustomLayoutSchedule() {
    const state = codexPlusCustomLayoutState;
    if (!state.enabled || state.raf) return;
    state.raf = requestAnimationFrame(() => {
      state.raf = 0;
      installCodexPlusCustomLayout();
    });
  }

  function codexPlusCustomLayoutBind(target, name, handler, options) {
    target.addEventListener(name, handler, options);
    codexPlusCustomLayoutState.listeners.push(() => target.removeEventListener(name, handler, options));
  }

  function codexPlusCustomLayoutStart() {
    const state = codexPlusCustomLayoutState;
    if (state.enabled) return;
    state.enabled = true;
    codexPlusCustomLayoutRead();
    if (!state.registered && typeof registerCodexPlusExtensionSelector === "function") {
      state.registered = registerCodexPlusExtensionSelector('[data-codex-plus-ext="custom-layout"]');
    }
    if (typeof MutationObserver === "function") {
      state.observer = new MutationObserver((mutations) => {
        const relevant = mutations.some((mutation) => {
          if (codexPlusCustomLayoutOwnedNode(mutation.target)) return false;
          if (mutation.type === "attributes" && mutation.attributeName === "style") {
            const record = state.styled.get(mutation.target) || state.ancestors.get(mutation.target);
            if (record && record.lastStyle === (mutation.target.getAttribute("style") || "")) return false;
          }
          const nodes = [...(mutation.addedNodes || []), ...(mutation.removedNodes || [])];
          return !nodes.length || !nodes.every(codexPlusCustomLayoutOwnedNode);
        });
        if (relevant) codexPlusCustomLayoutSchedule();
      });
      state.observer.observe(document.body || document.documentElement, {
        childList: true, subtree: true, attributes: true,
        attributeFilter: ["class", "style", "hidden", "inert", "aria-hidden", "data-summary-panel-variant", "data-codex-composer-root"],
      });
    }
    if (typeof ResizeObserver === "function") state.resizeObserver = new ResizeObserver(() => {
      if (!state.drag && !state.motions.size) codexPlusCustomLayoutSchedule();
    });
    codexPlusCustomLayoutBind(window, "resize", codexPlusCustomLayoutSchedule);
    codexPlusCustomLayoutBind(window, "blur", () => codexPlusCustomLayoutFinishDrag(true));
    codexPlusCustomLayoutBind(document, "scroll", () => { if (state.editing) codexPlusCustomLayoutSchedule(); }, true);
    codexPlusCustomLayoutBind(document, "keydown", (event) => {
      if (event.key === "Escape" && state.drag) {
        event.preventDefault();
        codexPlusCustomLayoutFinishDrag(false);
      } else if (event.key === "Escape" && state.editing) codexPlusCustomLayoutSetEditing(false);
    }, true);
    codexPlusCustomLayoutBind(window, "storage", (event) => {
      if (event.key !== codexPlusCustomLayoutKey) return;
      codexPlusCustomLayoutFinishDrag(false);
      codexPlusCustomLayoutStopMotion();
      state.projectedRects = null;
      state.projectionSignature = "";
      try { state.layout = normalizeLayout(JSON.parse(event.newValue || "null")); } catch { state.layout = normalizeLayout(null); }
      codexPlusCustomLayoutSchedule();
    });
    let attempts = 0;
    // 原生侧栏常晚于注入数秒出现；只保留有界启动重试。
    state.retryTimer = setInterval(() => {
      codexPlusCustomLayoutSchedule();
      if (++attempts >= 20 || state.entries.size === (state.entries.get("sidebar")?.grouped ? 3 : 4)) {
        clearInterval(state.retryTimer);
        state.retryTimer = 0;
      }
    }, 300);
  }

  function codexPlusCustomLayoutRefresh() {
    const state = codexPlusCustomLayoutState;
    if (!state.enabled) return;
    const targets = codexPlusCustomLayoutFindTargets();
    for (const entry of [...state.entries.values()]) {
      const target = targets[entry.id];
      const elements = target ? Array.isArray(target.elements) ? target.elements : [target] : [];
      if (elements.length !== entry.elements.length || elements.some((element, index) => element !== entry.elements[index])
        || (target?.grouped === true) !== entry.grouped) codexPlusCustomLayoutDropEntry(entry);
      else codexPlusCustomLayoutRefreshNavigationParts(entry, target);
    }
    for (const [id, target] of Object.entries(targets)) {
      if (!target || state.entries.has(id)) continue;
      const entry = codexPlusCustomLayoutCreateEntry(id, target);
      state.entries.set(id, entry);
      for (const element of entry.elements) state.resizeObserver?.observe(element);
    }
    const bounds = codexPlusCustomLayoutBounds();
    codexPlusCustomLayoutMigrateNavigation(state.entries.get("sidebar"), bounds);
    codexPlusCustomLayoutProject(bounds);
    const floated = [...state.entries.values()].filter((entry) => !!state.layout.panels[entry.id] || state.motions.has(entry.id) || state.projectedRects?.[entry.id]);
    codexPlusCustomLayoutReleaseAncestors(floated.flatMap(codexPlusCustomLayoutElements));
    for (const entry of state.entries.values()) {
      codexPlusCustomLayoutApplyEntry(entry, bounds);
      entry.justAdded = false;
    }
    if (state.editing) {
      codexPlusCustomLayoutEnsureUi();
      for (const entry of state.entries.values()) {
        codexPlusCustomLayoutInstallHandle(entry);
        codexPlusCustomLayoutPositionHandle(entry);
      }
      codexPlusCustomLayoutRenderToolbar();
    }
  }

  function codexPlusCustomLayoutProject(bounds) {
    const state = codexPlusCustomLayoutState;
    if (state.drag || state.motions.size) return;
    if (!Object.keys(state.layout.panels).length) {
      state.projectedRects = null;
      state.projectionSignature = "";
      return;
    }
    const desired = {};
    for (const entry of state.entries.values()) {
      // 没有用户位置的面板先读回原生几何，不能把上次让位投影当首选位置。
      if (!state.layout.panels[entry.id]) codexPlusCustomLayoutRestoreEntry(entry);
      const rect = codexPlusCustomLayoutPreferredRect(entry, bounds);
      const definition = codexPlusCustomLayoutPanels[entry.id];
      desired[entry.id] = { ...rect, minWidth: codexPlusCustomLayoutMinWidth(entry),
        minHeight: definition.resizableHeight ? 80 : rect.height, canResizeHeight: definition.resizableHeight };
    }
    const signature = JSON.stringify([bounds, desired]);
    if (signature === state.projectionSignature) return;
    const activeId = desired[state.lastActiveId] ? state.lastActiveId
      : Object.keys(desired).find((id) => !!state.layout.panels[id]);
    if (!activeId) return;
    const solution = codexPlusLayoutSolve(desired, activeId, desired[activeId], bounds, desired[activeId]);
    if (solution.feasible === false) {
      state.notice = "空间不足，请先缩小面板再拖动";
      // 极小窗口连最小尺寸都放不下时，仍保证全部手柄可达，避免沿用大窗口的越界坐标。
      state.projectedRects = Object.fromEntries(Object.entries(desired).map(([id, rect]) => [id, clampRect(rect, bounds)]));
      state.projectionSignature = signature;
      codexPlusCustomLayoutStopMotion();
      return;
    }
    const animate = !!state.projectionSignature;
    state.projectedRects = solution.rects;
    state.projectionSignature = signature;
    // 窗口变小或内容高度变化只调整显示投影，不覆盖用户在大窗口下的首选布局。
    if (animate) codexPlusCustomLayoutAnimate(solution.rects);
  }

  function codexPlusCustomLayoutSetEditing(editing) {
    installCodexPlusCustomLayout();
    const state = codexPlusCustomLayoutState;
    if (!state.enabled) return false;
    codexPlusCustomLayoutFinishDrag(true);
    state.editing = !!editing;
    if (!state.editing) {
      state.root?.remove();
      state.root = null;
      state.toolbar = null;
      for (const entry of state.entries.values()) Object.assign(entry, { handle: null, outline: null, move: null });
    }
    codexPlusCustomLayoutRefresh();
    return state.editing;
  }

  function codexPlusCustomLayoutResetPanel(id) {
    if (!codexPlusCustomLayoutPanels[id]) return;
    codexPlusCustomLayoutFinishDrag(false);
    codexPlusCustomLayoutStopMotion();
    codexPlusCustomLayoutState.projectedRects = null;
    codexPlusCustomLayoutState.projectionSignature = "";
    delete codexPlusCustomLayoutState.layout.panels[id];
    codexPlusCustomLayoutRefresh();
    codexPlusCustomLayoutSave();
  }

  function codexPlusCustomLayoutReset() {
    codexPlusCustomLayoutFinishDrag(false);
    codexPlusCustomLayoutStopMotion();
    codexPlusCustomLayoutState.projectedRects = null;
    codexPlusCustomLayoutState.projectionSignature = "";
    codexPlusCustomLayoutState.layout = { version: 1, panels: {} };
    codexPlusCustomLayoutRefresh();
    codexPlusCustomLayoutSave();
  }

  function codexPlusCustomLayoutCleanup(restoreConversation = true) {
    const state = codexPlusCustomLayoutState;
    state.enabled = false;
    state.editing = false;
    codexPlusCustomLayoutFinishDrag(false);
    codexPlusCustomLayoutStopMotion();
    state.projectedRects = null;
    state.projectionSignature = "";
    if (state.raf) cancelAnimationFrame(state.raf);
    if (state.retryTimer) clearInterval(state.retryTimer);
    state.raf = 0;
    state.retryTimer = 0;
    state.observer?.disconnect();
    state.resizeObserver?.disconnect();
    state.observer = null;
    state.resizeObserver = null;
    for (const cleanup of state.listeners.splice(0)) cleanup();
    for (const element of [...state.styled.keys()]) codexPlusCustomLayoutRestoreStyle(state.styled, element);
    for (const element of [...state.ancestors.keys()]) codexPlusCustomLayoutRestoreStyle(state.ancestors, element);
    state.entries.clear();
    state.root?.remove();
    state.root = null;
    state.toolbar = null;
    if (restoreConversation && typeof refreshConversationView === "function") refreshConversationView();
  }

  function codexPlusCustomLayoutOwnsElement(element) {
    const state = codexPlusCustomLayoutState;
    if (!state.enabled || !element) return false;
    return [...state.entries.values()].some((entry) => (!!state.layout.panels[entry.id] || state.motions.has(entry.id) || !!state.projectedRects?.[entry.id])
      && codexPlusCustomLayoutElements(entry).some((part) => part === element || part.contains?.(element)));
  }

  function installCodexPlusCustomLayout() {
    // 后端设置尚未读取时沿用已有运行态，不用短暂的默认值拆掉布局。
    if (typeof codexPlusBackendSettingsLoaded === "boolean" && !codexPlusBackendSettingsLoaded) return;
    if (!codexPlusCustomLayoutEnabled()) {
      if (codexPlusCustomLayoutState.enabled) codexPlusCustomLayoutCleanup();
      return;
    }
    codexPlusCustomLayoutStart();
    codexPlusCustomLayoutRefresh();
  }

  window.__codexPlusCustomLayoutRuntime = {
    setEditing: codexPlusCustomLayoutSetEditing,
    toggleEditing: () => codexPlusCustomLayoutSetEditing(!codexPlusCustomLayoutState.editing),
    reset: codexPlusCustomLayoutReset,
    refresh: installCodexPlusCustomLayout,
    cleanup: codexPlusCustomLayoutCleanup,
    get editing() { return codexPlusCustomLayoutState.editing; },
    get enabled() { return codexPlusCustomLayoutState.enabled; },
  };

  if (window.__CODEX_PLUS_TEST_CUSTOM_LAYOUT__) {
    Object.assign(window.__CODEX_PLUS_TEST_CUSTOM_LAYOUT__, { normalizeLayout, clampRect, snapRect, rectToRecord, recordToRect });
  }
