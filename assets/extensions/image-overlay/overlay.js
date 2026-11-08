  // 背景图引擎：在 Codex 窗口里铺一张背景图，随界面变化跟踪需要处理的原生表面。
  // config = { enabled, dataUrl, opacity: 0~1, fitMode: fill | fit | stretch | tile | center }；
  // enabled 为假或没有图片时只做清理，所以停用时以 { enabled: false } 再调一次即可。
  // report(event, data) 可选，用来写诊断日志。
  const codexPlusImageOverlayId = "codex-plus-image-overlay";

  function installCodexPlusImageOverlay(config = {}, report = null) {
    const source = typeof config.dataUrl === "string" ? config.dataUrl : "";
    const cached = window.__codexPlusImageOverlayResource;
    const configuredOpacity = Number(config.opacity);
    const opacity = Number.isFinite(configuredOpacity)
      ? Math.min(1, Math.max(0.01, configuredOpacity)) : 0.35;
    const fitMode = ["fill", "fit", "stretch", "tile", "center"].includes(config.fitMode)
      ? config.fitMode : "fit";
    const reuse = !!(config.enabled && source && cached?.version === 37 && cached.ready && cached.source === source);
    if (reuse && cached.opacity === opacity && cached.fitMode === fitMode &&
      document.getElementById(codexPlusImageOverlayId)?.getAttribute("data-codex-plus-ext") === "image-overlay") return;
    window.__codexPlusImageOverlayGeometryCleanup?.();
    window.__codexPlusImageOverlayGeometryCleanup = null;
    window.__codexPlusImageOverlayCleanup?.(reuse);
    window.__codexPlusImageOverlayCleanup = null;
    document.getElementById(codexPlusImageOverlayId)?.remove();
    const root = document.documentElement;
    window.__codexPlusImageOverlayReady = Promise.resolve(false);
    if (!config.enabled || !source || !root) return;

    const fitStyles = {
      fill: { size: "cover", position: "center center", repeat: "no-repeat" },
      fit: { size: "contain", position: "center center", repeat: "no-repeat" },
      stretch: { size: "100% 100%", position: "center center", repeat: "no-repeat" },
      tile: { size: "auto", position: "left top", repeat: "repeat" },
      center: { size: "auto", position: "center center", repeat: "no-repeat" },
    }[fitMode];
    const viewerSelector = [
      '[role="dialog"]:has([data-testid="image-preview-dismiss-area"])',
      '[data-testid="image-preview-editor"]',
      '[data-testid="image-preview-dismiss-area"]',
      '.codex-dialog-overlay[class*="_imageOverlay_"]',
      '.codex-dialog:has([data-image-preview-toolbar])',
      '.codex-dialog:has([data-image-preview-composer])',
      "[data-browser-sidebar-webview]",
      "[data-browser-sidebar-toolbar]",
    ].join(", ");
    const toolbarSelector = "[data-browser-sidebar-toolbar]";
    const toolbarSurfaceSelector = `${toolbarSelector} :is([data-menu-overlay-surface], [class*="controlSurface"])`;
    // 胶囊内的按钮共用外壳，只补画最外层材质，避免嵌套控件重复叠图。
    const isToolbarSurface = node => node.matches(toolbarSurfaceSelector) &&
      !node.parentElement?.closest(toolbarSurfaceSelector);
    const browserSelector = `[data-browser-sidebar-webview], ${toolbarSelector}`;
    const sidebarSelector = "aside.app-shell-left-panel, [data-app-action-sidebar-scroll], nav[data-app-navigation-rail]";
    const sidebarRowSelector = "[data-app-action-sidebar-thread-row], [data-app-action-sidebar-project-row]";
    const pageSelector = ".codex-plus-page-overlay";
    const timelineSelector = "[data-app-action-timeline-scroll], .thread-scroll-container";
    const reviewHeaderSelector = '[class~="group/turn-diff-header"]';
    const rightPanelSelector = '[data-app-shell-focus-area="right-panel"]';
    const inactivePanel = node => node.closest?.(
      `${rightPanelSelector}[inert], ${rightPanelSelector}[aria-hidden="true"]`);
    const mediaSelector = `img, video, canvas, ${viewerSelector}, ${reviewHeaderSelector}`;
    const nativeRangeSelector = 'input[type="range"], [role="slider"]';
    const composerSelector = ':is([data-composer-surface-variant][data-composer-radius-variant], [class*="_ComposerLayoutRoot_"])';
    const headerSelector = '[data-app-shell-header-layout], [data-sticky]:is([class*="_shell_"], [class^="shell-"], [class*=" shell-"]), .sticky.z-30.bg-surface';
    const editorSelector = '[contenteditable="true"], .ProseMirror';
    const composerOverlaySelector = "[data-composer-overlay-floating-ui]";
    const diffPreviewSelector = '[data-testid="diff-preview-scroll"]';
    const controlSelector = 'button, [role="button"], input, textarea, select, a';
    const mediaAttribute = "data-codex-plus-image-native";
    const scopeAttribute = "data-codex-plus-image-scope";
    const containmentAttribute = "data-codex-plus-image-contain";
    const browserPortalAttribute = "data-codex-plus-image-browser-portal";
    const popupMaterialAttribute = "data-codex-plus-image-popup-material";
    const popupBoundaryAttribute = "data-codex-plus-image-popup-boundary";
    const popupPortalAttribute = "data-codex-plus-image-popup-portal";
    const popupPortalSelector = ".fixed.inset-0.pointer-events-none.zoom-adjusted-viewport.contain-layout";
    const controlAttribute = "data-codex-plus-image-control";
    const paintAttribute = "data-codex-plus-image-paint";
    const topControlAttribute = "data-codex-plus-image-top-control";
    const footerAttribute = "data-codex-plus-image-footer-gutter";
    const footerSelector = "[data-thread-scroll-footer]";
    // 原生共享卡片没有全部暴露 data 属性（例如旧 optionPicker），保留其外壳组合匹配。
    const nativeTooltipSelector = '[role="tooltip"]:is([class^="Tooltip-"], [class*=" Tooltip-"])';
    const surfaceSelector = [
      'button[class~="group/scroll-to-bottom"]',
      "[data-composer-attachment-pill]", ".codex-toast-area [data-sonner-toast] .alert-root", nativeTooltipSelector,
      "[data-codex-composer-request-navigation]", "[data-codex-approval-surface]", "[data-composer-rail-item]",
      ".flex.flex-col.overflow-hidden.rounded-3xl.border.bg-surface-elevated-secondary",
      "[data-in-progress-fixed-content] .rounded-3xl.border.bg-surface-elevated-secondary",
      ".group.relative.flex.flex-wrap.rounded-2xl.bg-surface-elevated-secondary",
      ".tortilla-gallery.bg-surface-elevated-secondary", ".tortilla-gallery .bg-surface-elevated-secondary",
    ].join(", ");
    const surfaceBoundarySelector = `[${mediaAttribute}^="surface"]`;
    const imageViewerSelector = '.codex-dialog:has([data-image-preview-toolbar], [data-image-preview-composer]), [role="dialog"]:has([data-testid="image-preview-dismiss-area"])';
    const viewerSurfaceSelector = `:is(${imageViewerSelector}) :is(` +
      `[class*="controlSurface"], [role="toolbar"][class*="annotationTheme"], ` +
      `.pointer-events-auto.rounded-full:has([role="slider"]), [data-image-preview-composer] ${composerSelector})`;
    const isViewerSurface = node => node.matches(viewerSurfaceSelector) &&
      !node.parentElement?.closest(viewerSurfaceSelector);
    const topControlSelector = [
      '[role="menu"]', '[role="listbox"]', '[role="dialog"]', '[role="tooltip"]', '[class*="_Tooltip_"]', '[class^="Tooltip-"]', '[class*=" Tooltip-"]',
      // Sonner 的内层 z-index 无法越过这个原生 z60 外壳；只抬外壳，不改通知本身的颜色与动画。
      ".codex-toast-area",
      "[data-radix-popper-content-wrapper]", '[data-slot="popover-content"]', "[popover]", composerOverlaySelector,
    ].join(", ");
    // Extensions keep their own layers; their full-page shell is the one wallpaper surface.
    const owned = node => !!node?.closest?.('[data-codex-plus-ext]') && !node.matches(pageSelector);
    const wallpaperLayer = 1000;
    const overlay = document.createElement("div");
    overlay.id = codexPlusImageOverlayId;
    overlay.setAttribute("aria-hidden", "true");
    overlay.setAttribute("data-codex-plus-ext", "image-overlay");
    Object.assign(overlay.style, {
      position: "fixed", inset: "0", width: "100vw", height: "100vh",
      opacity: String(opacity), pointerEvents: "none", zIndex: String(wallpaperLayer),
      userSelect: "none", backgroundSize: fitStyles.size,
      backgroundPosition: fitStyles.position, backgroundRepeat: fitStyles.repeat,
      transition: "none", animation: "none",
    });
    const style = document.createElement("style");
    style.setAttribute("data-codex-plus-ext", "image-overlay");
    const nativeRules = `
      [${scopeAttribute}] { isolation:auto!important;z-index:auto!important;will-change:auto!important; }
      [${browserPortalAttribute}] { z-index:${wallpaperLayer + 1}!important; }
      ${popupPortalSelector}[${browserPortalAttribute}] { z-index:${wallpaperLayer + 2}!important; }
      [${scopeAttribute}^="identity"] { transform:none!important; }
      [${scopeAttribute}="identity-flow"] { position:relative!important; }
      [${containmentAttribute}] { contain:none!important; }
      [${containmentAttribute}^="flow"] { position:relative!important; }
      [${containmentAttribute}$="-clip"] { overflow:clip!important; }
      [${mediaAttribute}]:where(:not([${mediaAttribute}="review"], [${mediaAttribute}="composer"], [${mediaAttribute}="page"], [${mediaAttribute}^="toolbar-surface"], ${surfaceBoundarySelector})) { z-index:${wallpaperLayer}!important; }
      [${mediaAttribute}="surface-flow"] { position:relative!important; }
      [${mediaAttribute}="flow"], [${mediaAttribute}="island-flow"] { position:relative!important; }
      [${mediaAttribute}="viewer"] { z-index:${wallpaperLayer + 3}!important; }
      [${mediaAttribute}="footer"] { z-index:${wallpaperLayer + 1}!important; }
      [${mediaAttribute}^="header"] { z-index:${wallpaperLayer + 1}!important; }
      [${mediaAttribute}="header-flow"] { position:relative!important; }
      [${mediaAttribute}^="header"][class~="after:bg-linear-to-b"][class~="after:from-surface"][class~="after:to-transparent"]::after { background-image:none!important; }
      ${composerSelector} .top-full [data-composer-placement="home"].bg-background-composer-action-bar { background:none!important; }
      [${mediaAttribute}="page"], [${mediaAttribute}="composer"] { isolation:isolate!important; }
      [${mediaAttribute}="composer"]:not([${mediaAttribute}="footer"] *) { z-index:${wallpaperLayer + 1}!important; }
      [${mediaAttribute}="toolbar"], [${mediaAttribute}="toolbar-flow"] { background:none!important;box-shadow:none!important;filter:none!important;backdrop-filter:none!important; }
      [${mediaAttribute}="toolbar-flow"], [${mediaAttribute}="toolbar-surface-flow"] { position:relative!important; }
      [${mediaAttribute}^="toolbar-surface"] { isolation:isolate!important;backdrop-filter:none!important; }
      ${timelineSelector} { overscroll-behavior-y:none!important; }
      [${popupMaterialAttribute}] { background-image:none!important;filter:none!important;backdrop-filter:none!important; }
      [${paintAttribute}] { isolation:isolate!important; }
      [${paintAttribute}]:not([${paintAttribute}^="before-"], [${paintAttribute}^="background-"])::after, [${paintAttribute}^="before-"]::before {
        content:"";position:absolute;inset:0;pointer-events:none;border-radius:inherit;
        corner-shape:inherit;z-index:-1;opacity:${opacity};
        background-size:${fitStyles.size};background-position:${fitStyles.position};
        background-repeat:${fitStyles.repeat};background-attachment:fixed;
      }
      [${controlAttribute}] { z-index:${wallpaperLayer}!important; }
      [${topControlAttribute}] { z-index:${wallpaperLayer + 2}!important; }
      [${popupPortalAttribute}] { z-index:${wallpaperLayer + 2}!important; }
      [${topControlAttribute}="flow"] { position:relative!important; }
    `;
    const ns = "http://www.w3.org/2000/svg";
    const svgNode = (tag, attributes = {}) => {
      const node = document.createElementNS(ns, tag);
      for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, String(value));
      return node;
    };
    const svg = svgNode("svg", { width: 0, height: 0, "aria-hidden": "true", "data-codex-plus-ext": "image-overlay" });
    Object.assign(svg.style, { position: "fixed", width: "0", height: "0", pointerEvents: "none" });
    const defs = svgNode("defs");
    svg.appendChild(defs);
    let resource = reuse ? cached : null;
    let disposed = false;
    let raf = 0;
    let nextControl = 0;
    let nextSource = 0;
    let previewVisible = false;
    let sidebarScrollUntil = 0;
    let finishReady;
    const image = new Image();
    const records = new Map();
    const scopes = new Map();
    const browserPortals = new Map();
    const controls = new Map();
    const popups = new Map();
    const visibleMedia = new Set();
    const visibleControls = new Set();
    const activeControls = new Set();
    const paintedNodes = new Set();
    const viewers = new Set();
    const scopeKeys = new Map();
    const footers = new Map();
    const themeInputs = new Map();
    // 受变换影响的壁纸表面：待重算集合、当前按视口换算的集合、为其挂的 ResizeObserver（表面 -> 记录节点）。
    const paintsDirty = new Set();
    const geometryPaints = new Set();
    const paintObservers = new Map();
    let geometryDirty = true;
    const originalAttributes = new Map();
    const stats = { frames: 0, rectReads: 0, styleReads: 0, queries: 0, maskWrites: 0, layerWrites: 0 };
    window.__codexPlusImageOverlayStats = stats;
    window.__codexPlusImageOverlayReady = new Promise(resolve => { finishReady = resolve; });
    const mark = (node, name, value) => {
      let attributes = originalAttributes.get(node);
      if (!attributes) originalAttributes.set(node, attributes = new Map());
      if (!attributes.has(name)) attributes.set(name, node.getAttribute(name));
      if (node.getAttribute(name) !== value) node.setAttribute(name, value);
    };
    const unmark = (node, name) => {
      const attributes = originalAttributes.get(node);
      if (!attributes?.has(name)) return;
      const previous = attributes.get(name);
      if (previous === null) node.removeAttribute(name);
      else node.setAttribute(name, previous);
      attributes.delete(name);
      if (!attributes.size) originalAttributes.delete(node);
    };
    const computed = (node, pseudo) => { stats.styleReads++; return getComputedStyle(node, pseudo); };
    const bounds = node => { stats.rectReads++; return node.getBoundingClientRect(); };
    const queryAll = (node, selector) => { stats.queries++; return node.querySelectorAll(selector); };
    // 按 CSS Transforms 规范，自身或祖先带变换时 background-attachment:fixed 退化为元素局部坐标；
    // Chromium 里 translate:0、scale:1、will-change:transform 同样会触发（#2813：侧栏打开时 footer 带 translate）。
    const transformedStyle = paint => [paint.transform, paint.translate, paint.scale, paint.rotate]
      .some(value => value && value !== "none") || /transform|translate|scale|rotate/.test(paint.willChange || "");
    // 全局壁纸层在视口里的绘制矩形，与 overlay 的 background-size / background-position 等价。
    const wallpaperRect = (mode, width, height, imageWidth, imageHeight) => {
      if (mode === "stretch") return [0, 0, width, height];
      if (!imageWidth || !imageHeight) return null;
      if (mode === "tile") return [0, 0, imageWidth, imageHeight];
      const scale = mode === "fill" ? Math.max(width / imageWidth, height / imageHeight)
        : mode === "fit" ? Math.min(width / imageWidth, height / imageHeight) : 1;
      return [(width - imageWidth * scale) / 2, (height - imageHeight * scale) / 2, imageWidth * scale, imageHeight * scale];
    };
    // 把视口坐标的壁纸矩形换算到背景定位区域（视口原点 originX/originY，祖先缩放 sx/sy）的局部坐标。
    const localWallpaper = (wall, originX, originY, sx, sy) => {
      const px = value => `${Math.round(value * 1000) / 1000 || 0}px`;
      return { size: `${px(wall[2] / sx)} ${px(wall[3] / sy)}`,
        position: `${px((wall[0] - originX) / sx)} ${px((wall[1] - originY) / sy)}` };
    };
    const layoutScale = (size, layout) => !layout || Math.abs(size - layout) <= 1 ? 1 : size / layout;
    const scopeKey = node => [node.getAttribute("class"), ...[
      "position", "z-index", "isolation", "contain", "transform", "translate", "scale", "rotate",
      "will-change", "overflow", "overflow-x", "overflow-y", "animation", "transition",
    ].map(name => node.style.getPropertyValue(name))].join("|");
    const themeKey = node => [
      [...node.classList].filter(name => node !== root ||
        !/^_(?:active|opening|closing)_[a-zA-Z0-9]+_\d+$/.test(name)).sort().join(" "),
      [...node.style].filter(name => node !== root || name !== "--image-preview-thumbnail-radius")
        .sort().map(name => `${name}:${node.style.getPropertyValue(name)}:${node.style.getPropertyPriority(name)}`).join(";"),
    ].join("|");
    const viewerKey = node => [node.hidden, node.getAttribute("class"), node.getAttribute("role"),
      node.getAttribute("data-testid"),
      node.style.display, node.style.visibility, node.style.contentVisibility].join("|");
    const release = record => {
      if (record.browserPortal) {
        const count = browserPortals.get(record.browserPortal) - 1;
        if (count) browserPortals.set(record.browserPortal, count);
        else {
          browserPortals.delete(record.browserPortal);
          unmark(record.browserPortal, browserPortalAttribute);
        }
      }
      for (const ancestor of record.ancestors) {
        const count = scopes.get(ancestor) - 1;
        if (count) scopes.set(ancestor, count);
        else {
          scopes.delete(ancestor);
          scopeKeys.delete(ancestor);
          unmark(ancestor, scopeAttribute);
          unmark(ancestor, containmentAttribute);
        }
      }
    };
    const refreshScope = node => {
      unmark(node, scopeAttribute);
      unmark(node, containmentAttribute);
      const originalStyle = computed(node);
      if (originalStyle.contain.includes("paint")) {
        const flow = originalStyle.position === "static" ? "flow" : "positioned";
        const clip = originalStyle.overflowX === "visible" && originalStyle.overflowY === "visible" ? "-clip" : "";
        mark(node, containmentAttribute, flow + clip);
      }
      const transform = originalStyle.transform;
      const identity = transform !== "none" && new DOMMatrix(transform).isIdentity &&
        !node.getAnimations().some(animation => animation.playState === "running");
      mark(node, scopeAttribute, identity ? originalStyle.position === "static" ? "identity-flow" : "identity" : "stack");
      scopeKeys.set(node, scopeKey(node));
    };
    const nativeKind = node => records.get(node)?.localSurface ? computed(node).position === "static" ? "surface-flow" : "surface" :
      node.matches(reviewHeaderSelector) ? "review" :
      node.matches(toolbarSelector) ? computed(node).position === "static" ? "toolbar-flow" : "toolbar" :
      isToolbarSurface(node) ? computed(node).position === "static" ? "toolbar-surface-flow" : "toolbar-surface" :
      node.matches(pageSelector) ? "page" : node.matches(composerSelector) ? "composer" :
      node.matches(headerSelector) ? computed(node).position === "static" ? "header-flow" : "header" :
      node.matches(footerSelector) ? "footer" :
      node.matches(viewerSelector) && !node.matches("[data-browser-sidebar-webview]") ? "viewer" :
      records.get(node)?.mediaIsland ? computed(node).position === "static" ? "island-flow" : "island" :
      computed(node).position === "static" ? "flow" : "positioned";
    const add = (node, localSurface = false) => {
      if (owned(node) || inactivePanel(node) || node.closest(editorSelector)) return;
      const previous = records.get(node);
      localSurface ||= !!previous?.localSurface;
      const toolbarSurface = isToolbarSurface(node);
      if (!toolbarSurface && !isViewerSurface(node) && !node.matches(viewerSelector) && node.closest(viewerSelector) ||
        !node.matches(reviewHeaderSelector) && node.closest(reviewHeaderSelector)) {
        if (previous) removeMedia(node, previous);
        return;
      }
      const nativeBoundary = toolbarSurface ? node.closest(toolbarSelector) : (localSurface && node.closest(viewerSelector)) ||
        node.parentElement?.closest(`[${popupMaterialAttribute}], [${popupBoundaryAttribute}], [${mediaAttribute}^="island"], ${surfaceBoundarySelector}, ${pageSelector}, ${composerSelector}, ${footerSelector}, ${headerSelector}`);
      if (nativeBoundary && !localSurface && !toolbarSurface && !node.matches(`${composerSelector}, ${headerSelector}`)) {
        if (previous) removeMedia(node, previous);
        return;
      }
      if (node.matches("img, video, canvas")) {
        let host = node;
        // Keep an icon's badges and overlapping logo stack in native DOM order.
        for (let parent = node.parentElement; parent && parent !== document.body && !owned(parent) &&
          !parent.matches(`${topControlSelector}, ${controlSelector}`) && !parent.textContent.trim(); parent = parent.parentElement) {
          const children = [...parent.children];
          const sameBox = parent.offsetWidth > 0 && parent.offsetWidth === host.offsetWidth && parent.offsetHeight === host.offsetHeight;
          const stack = children.length > 1 && children.every(child => child.matches("img,svg") || child.querySelector("img,svg")) &&
            ["flex", "inline-flex"].includes(computed(parent).display) &&
            parent.offsetWidth <= children.reduce((sum, child) => sum + child.offsetWidth, 0) &&
            parent.offsetHeight <= Math.max(...children.map(child => child.offsetHeight));
          if (!sameBox && !stack) break;
          host = parent;
        }
        if (host !== node) {
          if (previous) removeMedia(node, previous);
          removeWithin(records, host, removeMedia, false);
          removeWithin(controls, host, removeControl);
          add(host);
          const record = records.get(host);
          if (record) {
            record.mediaIsland = true;
            unmark(host, mediaAttribute);
            mark(host, mediaAttribute, nativeKind(host));
          }
          return;
        }
      }
      let browserPortal = node.matches(nativeTooltipSelector) ? node.closest(popupPortalSelector)
        : node.matches(browserSelector) ? node.closest("[data-embedded-frame-host-root]") : null;
      if (node.matches(browserSelector)) {
        for (let parent = node.parentElement; !browserPortal && parent && parent !== root; parent = parent.parentElement) {
          const paint = computed(parent);
          if (paint.position === "fixed" && paint.pointerEvents === "none") { browserPortal = parent; break; }
        }
      }
      const ancestors = [];
      for (let parent = nativeBoundary || node.matches(reviewHeaderSelector) ||
        node.matches(viewerSelector) && !node.matches(browserSelector)
        ? null : browserPortal ? browserPortal.parentElement : node.parentElement; parent && parent !== root && !parent.matches(pageSelector) &&
          parent !== nativeBoundary; parent = parent.parentElement) {
        ancestors.push(parent);
      }
      if (previous?.parent === node.parentElement && previous.localSurface === localSurface && previous.browserPortal === browserPortal && previous.nativeBoundary === nativeBoundary &&
        previous.ancestors.length === ancestors.length &&
        ancestors.every((parent, index) => previous.ancestors[index] === parent)) return;
      if (previous) release(previous);
      for (const parent of ancestors) {
        const count = scopes.get(parent) || 0;
        scopes.set(parent, count + 1);
        if (!count) refreshScope(parent);
      }
      if (browserPortal) {
        browserPortals.set(browserPortal, (browserPortals.get(browserPortal) || 0) + 1);
        mark(browserPortal, browserPortalAttribute, "");
      }
      records.set(node, { ...previous, localSurface, parent: node.parentElement, ancestors, browserPortal, nativeBoundary, box: null });
      unmark(node, mediaAttribute);
      mark(node, mediaAttribute, nativeKind(node));
      if (node.getAttribute(mediaAttribute) === "viewer") {
        viewers.add(node);
        records.get(node).viewerKey = viewerKey(node);
      } else viewers.delete(node);
      if (nativeBoundary || node.matches(`${browserSelector}, ${reviewHeaderSelector}, ${composerSelector}, ${footerSelector}, ${pageSelector}, ${headerSelector}`)) {
        intersectionObserver.unobserve(node);
        visibleMedia.delete(node);
      } else if (!previous || previous.nativeBoundary) intersectionObserver.observe(node);
      stats.layerWrites++;
    };
    const addControl = (node, origin = null) => {
      if (owned(node) || inactivePanel(node) || node.closest(reviewHeaderSelector) ||
        node.matches(nativeRangeSelector) || node.closest(browserSelector) ||
        node.closest(sidebarSelector) || node.closest(pageSelector) || node.closest(viewerSelector) ||
        node.closest(topControlSelector) || node.closest(`[${popupBoundaryAttribute}]`) ||
          node.closest(`${composerSelector}, ${footerSelector}, ${editorSelector}, ${surfaceBoundarySelector}, [${mediaAttribute}^="island"], ${headerSelector}`)) {
        const previous = controls.get(node);
        if (previous) removeControl(node, previous);
        return;
      }
      let record = controls.get(node);
      if (!record) {
        record = { id: ++nextControl, origin, box: null, paintDirty: false,
          filter: null, backdrop: null, rule: "", key: "", backdropKey: "" };
        controls.set(node, record);
        intersectionObserver.observe(node);
        resizeObserver.observe(node);
      }
      return record;
    };
    const controlSurface = node => {
      let surface = node;
      for (let parent = node.parentElement; parent && parent !== root && !scopes.has(parent); parent = parent.parentElement) {
        if (parent.querySelector(mediaSelector)) break;
        const paint = computed(parent);
        if (paint.zIndex !== "auto" || paint.transform !== "none" ||
          paint.filter !== "none" || paint.backdropFilter !== "none" ||
          paint.isolation === "isolate" || Number(paint.opacity) < 1)
          surface = parent;
      }
      return surface;
    };
    const nativeSurfacePaint = surface => ({ id: ++nextControl, surface,
      pseudo: ["after", "before"].find(pseudo => ["none", "normal"].includes(computed(surface, `::${pseudo}`).content)) });
    // 未受变换影响时沿用 fixed；受影响时 paint.geometry 记录每个绘制目标按视口换算出的尺寸与偏移。
    const wallRule = paint => {
      const image = JSON.stringify(resource.imageSource);
      const selector = `[${paintAttribute}="${paint.ruleId}"]`;
      if (paint.tints) return paint.tints.map(([pseudo, tint]) => {
        const geometry = paint.geometry?.get(pseudo);
        return `${selector}${pseudo}{background-image:linear-gradient(${tint},${tint}),url(${image})!important;background-size:auto,${geometry?.size || fitStyles.size}!important;background-position:0 0,${geometry?.position || fitStyles.position}!important;background-repeat:no-repeat,${fitStyles.repeat}!important;background-attachment:${geometry ? "scroll" : "fixed"}!important;}`;
      }).join("\n");
      const pseudo = `::${paint.pseudo}`;
      const geometry = paint.geometry?.get(pseudo);
      return `${selector}${pseudo}{background-image:url(${image});${geometry
        ? `background-attachment:scroll!important;background-size:${geometry.size}!important;background-position:${geometry.position}!important;` : ""}}`;
    };
    const measurePaint = paint => {
      const surface = paint.surface;
      const geometry = new Map();
      const wall = wallpaperRect(fitMode, innerWidth, innerHeight, resource.width, resource.height);
      if (!wall || !surface.isConnected) return geometry;
      const surfaceStyle = computed(surface);
      // 绘制用的伪元素以表面为包含块时才能换算；否则保持 fixed，与改动前一致。
      const containing = surfaceStyle.position !== "static" || transformedStyle(surfaceStyle);
      const box = bounds(surface);
      const sx = layoutScale(box.width, surface.offsetWidth), sy = layoutScale(box.height, surface.offsetHeight);
      for (const pseudo of paint.tints ? paint.tints.map(([target]) => target) : [`::${paint.pseudo}`]) {
        let x = surface.clientLeft, y = surface.clientTop;
        if (pseudo) {
          if (!containing) continue;
          const own = computed(surface, pseudo);
          const left = parseFloat(own.left), top = parseFloat(own.top);
          if (own.position !== "absolute" || !Number.isFinite(left) || !Number.isFinite(top)) continue;
          x += left + (parseFloat(own.borderLeftWidth) || 0);
          y += top + (parseFloat(own.borderTopWidth) || 0);
        }
        geometry.set(pseudo, localWallpaper(wall, box.left + x * sx, box.top + y * sy, sx, sy));
      }
      return geometry;
    };
    const releasePaintGeometry = (node, paint) => {
      geometryPaints.delete(node);
      if (!paint) return;
      paint.geometry = null;
      const surface = paint.surface;
      if (paintObservers.get(surface) !== node) return;
      paintObservers.delete(surface);
      if (!controls.has(surface) && ![...footers.values()].some(footer => footer.scroller === surface))
        resizeObserver.unobserve(surface);
    };
    // 祖先样式变化（class/style 突变、过渡/动画结束）后，只标记待查，统一在 render 里读样式。
    const invalidatePaintChains = (target, viewerOnly = false) => {
      let found = false;
      for (const node of paintedNodes) {
        if (viewerOnly && !isViewerSurface(node)) continue;
        const paint = records.get(node)?.paint;
        if (!paint || target !== paint.surface && !target.contains(paint.surface)) continue;
        paint.chainDirty = true;
        paintsDirty.add(node);
        found = true;
      }
      return found;
    };
    const refreshPaints = (viewerOnly = false) => {
      const nodes = geometryDirty ? new Set([...paintsDirty, ...geometryPaints]) : paintsDirty;
      for (const node of nodes) {
        if (viewerOnly && !isViewerSurface(node)) continue;
        const record = records.get(node);
        const paint = record?.paint;
        if (!paint || !paintedNodes.has(node) || !paint.surface.isConnected) continue;
        if (paint.chainDirty) {
          paint.chainDirty = false;
          paint.transformed = false;
          for (let parent = paint.surface; parent; parent = parent === root ? null : parent.parentElement)
            if (transformedStyle(computed(parent))) { paint.transformed = true; break; }
        }
        const geometry = paint.transformed ? measurePaint(paint) : null;
        if (geometry?.size) {
          paint.geometry = geometry;
          geometryPaints.add(node);
          if (!paintObservers.has(paint.surface)) {
            paintObservers.set(paint.surface, node);
            resizeObserver.observe(paint.surface);
          }
        } else releasePaintGeometry(node, paint);
        const rule = wallRule(paint);
        if (record.wallRule !== rule) { record.wallRule = rule; writeRules(); }
      }
      if (viewerOnly) {
        for (const node of nodes) if (isViewerSurface(node)) paintsDirty.delete(node);
      } else paintsDirty.clear();
    };
    const registerPaint = (node, paint = records.get(node).paint || nativeSurfacePaint(node)) => {
      const record = records.get(node);
      const previous = record.paint;
      if (previous && previous.surface !== paint.surface) {
        unmark(previous.surface, paintAttribute);
        releasePaintGeometry(node, previous);
      }
      record.paint = paint;
      if (node.matches(headerSelector)) {
        unmark(paint.surface, paintAttribute);
        paint.ruleId = `background-${paint.id}`;
        // Paint only existing opaque backing, including native top-edge extensions.
        // Transparent headers already see the single global wallpaper underneath.
        paint.tints = ["", "::before", "::after"].map(pseudo => {
          const native = computed(paint.surface, pseudo || undefined);
          const color = native.backgroundColor;
          if (color === "transparent" || color.startsWith("rgba(") || color.includes("/") || native.backgroundImage !== "none") return null;
          return [pseudo, `color-mix(in srgb, ${color} ${(1 - opacity) * 100}%, transparent)`];
        }).filter(Boolean);
        mark(paint.surface, paintAttribute, paint.ruleId);
      } else if (!paint.pseudo) {
        // Leave a surface's two native pseudo-elements intact when both are used.
        unmark(paint.surface, paintAttribute);
        paintedNodes.delete(node);
        releasePaintGeometry(node, paint);
        record.wallRule = "";
        writeRules();
        return;
      } else {
        paint.ruleId = (paint.pseudo === "before" ? "before-" : "") + paint.id;
        paint.tints = null;
        mark(paint.surface, paintAttribute, paint.ruleId);
      }
      paintedNodes.add(node);
      record.wallRule = wallRule(paint);
      paint.chainDirty = true;
      paintsDirty.add(node);
      schedule();
      writeRules();
    };
    const removeWithin = (registry, tree, remove, includeRoot = true) => {
      for (const [node, record] of registry)
        if ((includeRoot || node !== tree) && tree.contains(node)) remove(node, record);
    };
    const popupBackgroundKey = node => Array.from(node.style)
      .filter(name => name.startsWith("background") || name === "color")
      .sort().map(name => `${name}:${node.style.getPropertyValue(name)}:${node.style.getPropertyPriority(name)}`).join(";");
    // 与普通宿主卡片保持一致：只补画全覆盖不透明材质，避免给透出的父层再叠一张壁纸。
    const opaqueColor = value => value !== "transparent" && !/rgba\(|hsla\(|\//.test(value);
    const hasLocalMaterial = paint => {
      if (paint.backgroundClip === "text") return false;
      if (paint.backgroundImage === "none") return opaqueColor(paint.backgroundColor);
      if (!paint.backgroundImage.includes("gradient(") || /url\(/.test(paint.backgroundImage)) return false;
      if (opaqueColor(paint.backgroundColor)) return true;
      return !/transparent|rgba\(|hsla\(|\//.test(paint.backgroundImage) &&
        ["auto", "auto auto", "100% 100%"].includes(paint.backgroundSize) &&
        paint.backgroundRepeat === "repeat";
    };
    // 弱引用记录候选表面的祖先，包括当前透明/尚未获得宿主身份的候选。
    // 无关流式文字的 class/style 变化不需要查询子树，也不遍历 records。
    const localSurfaceAncestors = new WeakSet();
    const trackLocalSurfaceAncestors = node => {
      for (let parent = node.parentElement; parent; parent = parent.parentElement)
        localSurfaceAncestors.add(parent);
    };
    const isLocalSurface = node => {
      const viewerSurface = isViewerSurface(node);
      const nativeTooltip = node.matches(nativeTooltipSelector);
      if (!(viewerSurface || node.matches(surfaceSelector)) || owned(node) || inactivePanel(node) ||
        node.closest(`${editorSelector}, ${pageSelector}`) || (!nativeTooltip && node.matches(topControlSelector)) ||
        (!viewerSurface && (node.closest(viewerSelector) || !nativeTooltip &&
          !node.parentElement?.closest(`${composerSelector}, ${footerSelector}, ${topControlSelector}`)))) return false;
      // 即使 tooltip 本身半透明，也要保留前景层；只决定是否补画，不让文字落回全屏壁纸下面。
      return nativeTooltip || hasLocalMaterial(computed(node));
    };
    const addSurface = node => {
      trackLocalSurfaceAncestors(node);
      // 同一节点可能先是普通 popup，稍后才挂上原生 tooltip 类名；先释放旧 alpha 规则再读材质。
      if (node.matches(nativeTooltipSelector) && popups.has(node)) removePopup(node, popups.get(node));
      if (!isLocalSurface(node)) {
        const previous = records.get(node);
        if (previous?.localSurface) { removeMedia(node, previous); visit(node); }
        return;
      }
      if (node.matches(nativeTooltipSelector) && !hasLocalMaterial(computed(node))) {
        const previous = records.get(node);
        if (previous?.paint) removeMedia(node, previous);
        add(node, true);
        return;
      }
      add(node, true);
      if (!records.has(node)) return;
      // 外壳上补画，子按钮、图标和编辑器继续使用原生堆叠与事件。
      removeWithin(controls, node, removeControl);
      for (const [child, record] of records)
        if (child !== node && node.contains(child) && !record.localSurface &&
          !child.matches(`${composerSelector}, ${headerSelector}, ${topControlSelector}`)) removeMedia(child, record);
      registerPaint(node);
    };
    // 宿主身份和继承的主题变量都会改变后代表面资格；仅扫描变更子树，已登记且资格不变的表面不重建。
    const refreshLocalSurfaces = tree => {
      const candidates = new Set([tree, ...(tree.childElementCount
        ? queryAll(tree, `${surfaceSelector}, ${viewerSurfaceSelector}`) : [])]);
      for (const [node, record] of records)
        if (record.localSurface && tree.childElementCount && tree.contains(node)) candidates.add(node);
      let changed = false;
      for (const node of candidates) {
        // 新发现的透明候选也要登记；后续祖先主题变化才有机会使它获得补画资格。
        if (node.matches(surfaceSelector) || isViewerSurface(node)) trackLocalSurfaceAncestors(node);
        const localSurface = isLocalSurface(node);
        const tooltipMaterialChanged = localSurface && node.matches(nativeTooltipSelector) &&
          !!records.get(node)?.paint !== hasLocalMaterial(computed(node));
        if (localSurface === !!records.get(node)?.localSurface && !tooltipMaterialChanged) continue;
        addSurface(node);
        changed = true;
      }
      return changed;
    };
    const addComposer = node => {
      if (isViewerSurface(node)) { addSurface(node); return; }
      if (owned(node) || inactivePanel(node) || node.closest(viewerSelector)) return;
      // Paint below the native children. Never annotate editor-owned node views.
      const surface = node.matches('[data-composer-utility-bar-variant="home"]')
        ? node.querySelector('[class*="ComposerLayoutBody"]') || node : node;
      let surfacePaint = records.get(node)?.paint;
      if (!surfacePaint) {
        unmark(node, mediaAttribute);
        surfacePaint = nativeSurfacePaint(surface);
      }
      add(node);
      registerPaint(node, surfacePaint);
    };
    const addToolbarSurface = node => {
      if (!isToolbarSurface(node) || owned(node) || inactivePanel(node)) return;
      unmark(node, mediaAttribute);
      const kind = nativeKind(node);
      add(node);
      // 保留原生渐变、阴影和交互伪元素，沿用输入框的空闲伪元素及视口坐标补画。
      if (records.has(node)) {
        mark(node, mediaAttribute, kind);
        registerPaint(node);
      }
    };
    const visitToolbarSurfaces = node => {
      if (isToolbarSurface(node)) addToolbarSurface(node);
      queryAll(node, toolbarSurfaceSelector).forEach(addToolbarSurface);
    };
    const addPopupSurface = (boundary, surface) => {
      if (owned(surface) || inactivePanel(surface) || surface.closest(reviewHeaderSelector) ||
        surface.closest(viewerSelector) || surface.closest(pageSelector)) return;
      const previous = popups.get(surface);
      if (previous?.boundary === boundary) return;
      if (previous) removePopup(surface, previous);
      removeWithin(controls, boundary, removeControl);
      const paint = computed(surface);
      const record = { boundary, portal: boundary.closest(popupPortalSelector), owner: surface.closest(composerOverlaySelector) || boundary,
        id: ++nextControl, backgroundKey: popupBackgroundKey(surface),
        nativeBackground: paint.backgroundColor,
        className: surface.getAttribute("class"), rule: "" };
      // Only the background is translucent; native contents and positioning stay intact.
      record.rule = `[${popupMaterialAttribute}="${record.id}"]{background-color:color-mix(in srgb, ${paint.backgroundColor} ${(1 - opacity) * 100}%, transparent)!important;}`;
      popups.set(surface, record);
      // Native tooltips share a fixed portal; lifting only its children cannot clear the composer.
      if (record.portal) mark(record.portal, popupPortalAttribute, "");
      mark(boundary, popupBoundaryAttribute, "");
      mark(boundary, topControlAttribute, computed(boundary).position === "static" ? "flow" : "positioned");
      mark(surface, popupMaterialAttribute, String(record.id));
      queryAll(boundary, mediaSelector).forEach(add);
      writeRules();
    };
    const refreshPopupTheme = () => {
      for (const [surface, popup] of popups) {
        unmark(surface, popupMaterialAttribute);
        const background = computed(surface).backgroundColor;
        mark(surface, popupMaterialAttribute, String(popup.id));
        if (background === popup.nativeBackground) continue;
        popup.nativeBackground = background;
        popup.rule = `[${popupMaterialAttribute}="${popup.id}"]{background-color:color-mix(in srgb, ${background} ${(1 - opacity) * 100}%, transparent)!important;}`;
      }
      writeRules();
    };
    const addDiffPreview = scroll => {
      const surface = scroll.parentElement;
      const tooltip = scroll.closest('[class~="group/diff-preview"]');
      if (!surface || !tooltip || owned(scroll)) return;
      addPopupSurface(tooltip.closest("[data-radix-popper-content-wrapper]") || tooltip, surface);
    };
    const addNativePopup = node => {
      if (owned(node) || node.closest(viewerSelector) || node.closest(pageSelector)) return;
      // 原生 tooltip 的不透明底色单独补画；仅降低底色 alpha 会把底下已混合的壁纸再次衰减。
      const tooltip = node.matches(nativeTooltipSelector) ? node :
        node.matches('[data-radix-popper-content-wrapper]') ? node.querySelector(nativeTooltipSelector) : null;
      if (tooltip) { addSurface(tooltip); return; }
      const composerOverlay = node.matches(composerOverlaySelector);
      const parent = node.parentElement;
      // The roleless composer menu uses a translated positioning wrapper.
      const boundary = composerOverlay && parent?.childElementCount === 1 &&
        ["fixed", "absolute"].includes(computed(parent).position)
        ? parent : node.closest("[data-radix-popper-content-wrapper]") || node;
      if (popups.has(boundary)) return;
      if (node === boundary && [...popups.values()].some(popup => popup.boundary === boundary)) return;
      const surface = composerOverlay ? node.firstElementChild :
        node.matches("[data-radix-popper-content-wrapper]") &&
        computed(node).backgroundColor === "rgba(0, 0, 0, 0)"
        ? node.querySelector(topControlSelector) : node;
      if (!surface) return;
      addPopupSurface(boundary, surface);
    };
    const isPopupSurface = surface => surface.matches(topControlSelector) ||
      surface.closest(`[class~="group/diff-preview"], ${composerOverlaySelector}`);
    const removePopup = (surface, record) => {
      popups.delete(surface);
      unmark(surface, popupMaterialAttribute);
      if (record.portal && ![...popups.values()].some(other => other.portal === record.portal))
        unmark(record.portal, popupPortalAttribute);
      if (![...popups.values()].some(other => other.boundary === record.boundary)) {
        unmark(record.boundary, popupBoundaryAttribute);
        unmark(record.boundary, topControlAttribute);
      }
    };
    const addPage = node => {
      removeWithin(popups, node, removePopup);
      removeWithin(controls, node, removeControl);
      removeWithin(records, node, removeMedia, false);
      add(node);
      registerPaint(node);
    };
    const addHeader = node => {
      if (owned(node) || inactivePanel(node) || node.closest(`${viewerSelector}, ${editorSelector}`)) return;
      removeWithin(controls, node, removeControl);
      removeWithin(records, node, removeMedia, false);
      add(node);
      registerPaint(node);
    };
    const updateFooter = (footer, record) => {
      const scroller = record.scroller;
      const parent = footer.offsetParent;
      if (!parent) return false;
      let left = 0, right = 0;
      // The native footer is absolute in the outer frame, not in the scroller.
      if (parent !== scroller) {
        const scrollBox = bounds(scroller), parentBox = bounds(parent);
        const scale = parentBox.width / (parent.offsetWidth || parentBox.width);
        const scrollScale = scrollBox.width / (scroller.offsetWidth || scrollBox.width);
        const origin = parentBox.left + parent.clientLeft * scale;
        left = Math.max(0, (scrollBox.left + scroller.clientLeft * scrollScale - origin) / scale);
        right = Math.max(0, parent.clientWidth - left - scroller.clientWidth * scrollScale / scale);
      }
      const rule = `[${footerAttribute}="${record.id}"]{left:${left}px!important;right:${right}px!important;}`;
      if (record.rule === rule) return false;
      record.rule = rule;
      return true;
    };
    const removeFooter = (footer, record) => {
      footers.delete(footer);
      unmark(footer, footerAttribute);
      if (![...footers.values()].some(other => other.scroller === record.scroller) &&
        !records.has(record.scroller) && !controls.has(record.scroller) && !paintObservers.has(record.scroller))
        resizeObserver.unobserve(record.scroller);
    };
    const addFooter = footer => {
      if (owned(footer) || inactivePanel(footer) || footer.closest(reviewHeaderSelector)) return;
      add(footer);
      const surface = footer.querySelector(':scope > [aria-hidden="true"]');
      if (surface && records.get(footer)?.paint?.surface !== surface) registerPaint(footer, nativeSurfacePaint(surface));
      const scroller = footer.closest(timelineSelector);
      if (!scroller) return;
      const previous = footers.get(footer);
      if (previous?.scroller === scroller) {
        if (updateFooter(footer, previous)) writeRules();
        return;
      }
      if (previous) removeFooter(footer, previous);
      const paint = computed(footer);
      // Only the full-width native footer can cover the scrollbar gutter.
      if (!["absolute", "fixed"].includes(paint.position) ||
        paint.left !== "0px" || paint.right !== "0px") return;
      const record = { id: ++nextControl, scroller, rule: "" };
      footers.set(footer, record);
      mark(footer, footerAttribute, String(record.id));
      updateFooter(footer, record);
      resizeObserver.observe(scroller);
      writeRules();
    };
    const addReviewHeader = node => {
      removeWithin(popups, node, removePopup);
      removeWithin(controls, node, removeControl);
      removeWithin(records, node, removeMedia, false);
      add(node);
    };
    const discoverySelector = [pageSelector, mediaSelector, controlSelector, composerSelector, surfaceSelector, viewerSurfaceSelector, toolbarSurfaceSelector, footerSelector, headerSelector,
      diffPreviewSelector, topControlSelector].join(", ");
    const visit = node => {
      if (node?.nodeType !== 1 || !node.isConnected || owned(node) || inactivePanel(node) ||
        node.closest(editorSelector)) return;
      if (node.matches(browserSelector)) {
        add(node);
        if (node.matches(toolbarSelector)) visitToolbarSurfaces(node);
        return;
      }
      if (node.closest(browserSelector)) {
        if (node.closest(toolbarSelector)) visitToolbarSurfaces(node);
        return;
      }
      const targets = [node, ...(node.childElementCount ? queryAll(node, discoverySelector) : [])]
        .filter(target => !inactivePanel(target));
      // Establish native boundaries before registering their media descendants.
      for (const target of targets) if (target.matches(pageSelector)) addPage(target);
      for (const target of targets) if (target.matches(footerSelector)) addFooter(target);
      for (const target of targets) if (target.matches(composerSelector)) addComposer(target);
      for (const target of targets) if (isToolbarSurface(target)) addToolbarSurface(target);
      for (const target of targets) if (target.matches(headerSelector)) addHeader(target);
      for (const target of targets) if (target.matches(reviewHeaderSelector)) addReviewHeader(target);
      for (const target of targets) {
        if (target.matches(diffPreviewSelector)) addDiffPreview(target);
        if (target.matches(topControlSelector)) addNativePopup(target);
      }
      for (const target of targets) if (target.matches(surfaceSelector) || isViewerSurface(target)) addSurface(target);
      for (const target of targets) {
        if (target.matches(mediaSelector)) add(target);
        if (target.matches(controlSelector)) addControl(target);
      }
      const popup = node.closest(topControlSelector);
      if (popup && popup !== node) {
        if (popup.matches(viewerSelector)) add(popup);
        else addNativePopup(popup);
      }
      if (node.parentElement?.matches(footerSelector)) addFooter(node.parentElement);
    };
    const intersectionObserver = new IntersectionObserver(entries => {
      let changed = false;
      const regions = new Set();
      for (const entry of entries) {
        if (inactivePanel(entry.target)) continue;
        if (records.has(entry.target)) {
          if (records.get(entry.target).nativeBoundary || entry.target.matches(browserSelector) || entry.target.matches(reviewHeaderSelector)) {
            visibleMedia.delete(entry.target);
            continue;
          }
          records.get(entry.target).box = null;
          if (entry.isIntersecting) visibleMedia.add(entry.target);
          else visibleMedia.delete(entry.target);
          const region = entry.target.closest(`${timelineSelector}, ${composerSelector}, ${sidebarSelector}`);
          if (region) regions.add(region);
        }
        if (controls.has(entry.target)) {
          controls.get(entry.target).box = null;
          if (entry.isIntersecting) visibleControls.add(entry.target);
          else visibleControls.delete(entry.target);
        }
        changed ||= records.has(entry.target) || controls.has(entry.target);
      }
      for (const region of regions) invalidateRegion(region, false);
      if (changed && !previewVisible) schedule();
    });
    const resizeObserver = new ResizeObserver(entries => {
      let changed = false;
      let viewerChanged = false;
      for (const { target } of entries) {
        for (const [footer, record] of footers)
          if (record.scroller === target && updateFooter(footer, record)) writeRules();
        const control = controls.get(target);
        if (control) control.box = null;
        const record = records.get(target);
        if (record) record.box = null;
        const painted = paintObservers.get(target);
        if (painted) paintsDirty.add(painted);
        viewerChanged ||= !!painted && isViewerSurface(painted);
        changed ||= !!(control || record || painted);
      }
      if (changed && (!previewVisible || viewerChanged)) schedule();
    });
    const removeControl = (node, control) => {
      control.filter?.remove();
      control.backdrop?.remove();
      unmark(node, controlAttribute);
      unmark(node, topControlAttribute);
      if (!records.has(node)) {
        intersectionObserver.unobserve(node);
        if (!paintObservers.has(node)) resizeObserver.unobserve(node);
      }
      visibleControls.delete(node);
      activeControls.delete(node);
      controls.delete(node);
      stats.layerWrites++;
    };
    const removeMedia = (node, record) => {
      release(record);
      releasePaintGeometry(node, record.paint);
      paintsDirty.delete(node);
      record.paint?.filter?.remove();
      unmark(node, mediaAttribute);
      unmark(record.paint?.surface || node, paintAttribute);
      if (!controls.has(node)) {
        intersectionObserver.unobserve(node);
        resizeObserver.unobserve(node);
      }
      visibleMedia.delete(node);
      viewers.delete(node);
      paintedNodes.delete(node);
      records.delete(node);
      stats.layerWrites++;
    };
    const prune = () => {
      for (const [footer, record] of footers) {
        if (footer.isConnected && !inactivePanel(footer) && !footer.closest(reviewHeaderSelector) &&
          record.scroller.isConnected && record.scroller.contains(footer)) continue;
        removeFooter(footer, record);
      }
      for (const [surface, popup] of popups) {
        if (surface.isConnected && !inactivePanel(surface) && !surface.closest(reviewHeaderSelector) &&
          popup.boundary.isConnected && popup.boundary.contains(surface) &&
          !surface.closest(viewerSelector) && !surface.closest(pageSelector) &&
          isPopupSurface(surface)) continue;
        removePopup(surface, popup);
      }
      for (const [node, record] of records) {
        if (record.localSurface) {
          if (node.isConnected && isLocalSurface(node)) { add(node, true); continue; }
          removeMedia(node, record);
          if (node.isConnected) visit(node);
          continue;
        }
        if (node.isConnected && !inactivePanel(node) &&
          (node.matches(viewerSelector) || isToolbarSurface(node) || !node.closest(viewerSelector)) &&
          (node.matches(reviewHeaderSelector) || !node.closest(reviewHeaderSelector)) &&
          (node.matches(mediaSelector) || isToolbarSurface(node) || node.matches(`${composerSelector}, ${footerSelector}, ${pageSelector}, ${headerSelector}`) ||
            record.mediaIsland && node.querySelector("img, video, canvas"))) {
          if (record.nativeBoundary &&
            !record.nativeBoundary.matches(`[${popupMaterialAttribute}], [${popupBoundaryAttribute}], ${pageSelector}, ${footerSelector}, ${composerSelector}, ${headerSelector}, ${toolbarSelector}`)) add(node);
          continue;
        }
        removeMedia(node, record);
        if (node.isConnected && !node.closest(viewerSelector)) visit(node);
      }
      for (const [node, control] of controls) {
        if (node.isConnected && !inactivePanel(node) && !node.closest(reviewHeaderSelector) &&
          !node.closest(viewerSelector) && (node.matches(controlSelector) ||
          control.origin?.isConnected && node.contains(control.origin))) continue;
        removeControl(node, control);
      }
    };
    const retireTree = tree => {
      removeWithin(popups, tree, removePopup);
      removeWithin(controls, tree, removeControl);
      removeWithin(records, tree, removeMedia);
      removeWithin(footers, tree, removeFooter);
    };
    const mutationObserver = new MutationObserver(mutations => {
      let changed = false;
      let pruneNeeded = false;
      const added = new Set();
      const attributes = new Set();
      const semanticAttributes = new Set();
      const panels = new Set();
      const movedInactive = new Set();
      for (const mutation of mutations) {
        if (owned(mutation.target) || mutation.target.closest?.(editorSelector)) continue;
        if (mutation.type === "childList") {
          for (const node of mutation.removedNodes)
            if (node.nodeType === 1 && node.isConnected && inactivePanel(node)) movedInactive.add(node);
        }
        if (mutation.type === "attributes" && mutation.target.matches(rightPanelSelector) &&
          ["inert", "aria-hidden", "data-app-shell-focus-area"].includes(mutation.attributeName)) {
          panels.add(mutation.target);
          continue;
        }
        if (inactivePanel(mutation.target)) continue;
        if (mutation.type === "attributes" && mutation.attributeName === "data-composer-overlay-floating-ui") {
          for (const [surface, popup] of popups)
            if (popup.owner === mutation.target) removePopup(surface, popup);
          added.add(mutation.target);
          changed = true;
          pruneNeeded = true;
          continue;
        }
        if (mutation.type === "childList") {
          if (mutation.target.closest?.(browserSelector) && !mutation.target.closest(toolbarSelector)) continue;
          const nodes = [...mutation.addedNodes, ...mutation.removedNodes];
          if (!nodes.some(node => node.nodeType === 1 && !owned(node))) continue;
          pruneNeeded = true;
          for (const node of mutation.addedNodes)
            if (node.nodeType === 1 && node.isConnected && !owned(node)) added.add(node);
          if ([...mutation.removedNodes].some(node =>
            node.nodeType === 1 && !owned(node) && (node.matches(`[${mediaAttribute}],[${controlAttribute}],[${footerAttribute}]`) ||
              node.querySelector(`[${mediaAttribute}],[${controlAttribute}],[${popupMaterialAttribute}],[${footerAttribute}]`) ||
              node.matches(`[${popupMaterialAttribute}]`)))) changed = true;
        } else {
          attributes.add(mutation.target);
          if (mutation.attributeName !== "style") semanticAttributes.add(mutation.target);
        }
      }
      const before = stats.layerWrites;
      const count = records.size + controls.size + popups.size;
      for (const tree of movedInactive) { retireTree(tree); changed = true; }
      for (const panel of panels) {
        if (inactivePanel(panel)) retireTree(panel);
        else visit(panel);
        changed = true;
      }
      for (const node of added) {
        let parent = node.parentElement;
        while (parent && !added.has(parent)) parent = parent.parentElement;
        if (!parent) visit(node);
      }
      changed ||= stats.layerWrites !== before || records.size + controls.size + popups.size !== count;
      for (const node of attributes) {
        if (!node.isConnected || inactivePanel(node)) continue;
        if (node !== root && node !== document.body &&
          (localSurfaceAncestors.has(node) || node.matches(`${surfaceSelector}, ${viewerSurfaceSelector}, ${footerSelector}, ${topControlSelector}, [data-in-progress-fixed-content], [data-sonner-toast], .tortilla-gallery`)) &&
          refreshLocalSurfaces(node)) changed = true;
        if (node.matches(toolbarSelector)) {
          // 原生容器可能在子控件已挂载后才声明工具栏身份。
          if (semanticAttributes.has(node)) { visitToolbarSurfaces(node); pruneNeeded = true; changed = true; }
          // 工具栏自身由浏览器定位，但其新补画表面仍须跟踪 none → transform。
          if (invalidatePaintChains(node)) changed = true;
        }
        if (node.closest?.(browserSelector) && !node.matches(browserSelector)) {
          if (node.closest(toolbarSelector)) {
            if (semanticAttributes.has(node)) { visitToolbarSurfaces(node); changed = true; }
            else if (isToolbarSurface(node)) { addToolbarSurface(node); changed = true; }
            if (invalidatePaintChains(node)) changed = true;
            pruneNeeded ||= semanticAttributes.has(node);
          }
          continue;
        }
        // Native hosts and painted surfaces own their animated geometry.
        if (node.matches(`${browserSelector}, ${headerSelector}`) && records.has(node) && !semanticAttributes.has(node)) continue;
        if (node === root || node === document.body) {
          const key = themeKey(node);
          if (themeInputs.get(node) !== key) {
            themeInputs.set(node, key);
            if (refreshLocalSurfaces(node)) changed = true;
            refreshPopupTheme();
            for (const surface of paintedNodes) if (surface.matches(headerSelector)) registerPaint(surface);
          }
        }
        if (viewers.has(node)) {
          const record = records.get(node);
          if (!node.matches(viewerSelector)) {
            removeMedia(node, record);
            visit(node);
            changed = true;
            pruneNeeded = true;
            continue;
          }
          const key = viewerKey(node);
          if (semanticAttributes.has(node)) {
            queryAll(node, viewerSurfaceSelector).forEach(addSurface);
            pruneNeeded = true; changed = true;
          }
          if (invalidatePaintChains(node)) changed = true;
          if (record.viewerKey === key) continue;
          record.viewerKey = key;
          changed ||= isPreviewOpen() !== previewVisible;
          continue;
        } else if (node.closest(viewerSelector) && !node.matches(browserSelector)) {
          if (isViewerSurface(node) || records.get(node)?.localSurface) { addSurface(node); changed = true; }
          if (semanticAttributes.has(node)) { queryAll(node, viewerSurfaceSelector).forEach(addSurface); pruneNeeded = true; changed = true; }
          if (invalidatePaintChains(node)) changed = true;
          continue;
        }
        if ([...viewers].some(viewer => node.contains(viewer)))
          changed ||= isPreviewOpen() !== previewVisible;
        pruneNeeded ||= semanticAttributes.has(node);
        if (node.matches(pageSelector) && !records.get(node)?.paint) visit(node);
        if (node.matches(composerSelector) && (!records.get(node)?.paint || semanticAttributes.has(node))) { addComposer(node); changed = true; }
        if (node.matches(surfaceSelector) || records.get(node)?.localSurface) { addSurface(node); changed = true; }
        if (node.matches(headerSelector) && (!records.get(node)?.paint || semanticAttributes.has(node))) { addHeader(node); changed = true; }
        if (node.matches(reviewHeaderSelector)) { addReviewHeader(node); continue; }
        const popup = popups.get(node);
        if (popup && !node.closest(viewerSelector) && !node.closest(pageSelector) &&
          isPopupSurface(node)) {
          const key = popupBackgroundKey(node);
          const className = node.getAttribute("class");
          if (key !== popup.backgroundKey || className !== popup.className) {
            const boundary = popup.boundary;
            removePopup(node, popup);
            addPopupSurface(boundary, node);
            changed = true;
          }
          continue;
        }
        if (popup) { removePopup(node, popup); visit(node); changed = true; }
        if (node.matches(topControlSelector)) {
          const count = popups.size;
          addNativePopup(node);
          changed ||= popups.size !== count;
        }
        if (node.closest(`[${popupBoundaryAttribute}]`) && !records.has(node) && !scopes.has(node)) continue;
        if (previewVisible) {
          continue;
        }
        // 祖先 class/style 变化可能加上或撤掉 translate（如侧栏开合），重新判断其下壁纸表面。
        if (invalidatePaintChains(node)) changed = true;
        if (footers.has(node) || node.matches(footerSelector)) {
          const footer = footers.get(node);
          if (footer) removeFooter(node, footer);
          if (node.matches(footerSelector)) addFooter(node);
          if (footer) writeRules();
        }
        if (records.has(node)) {
          records.get(node).box = null;
          unmark(node, mediaAttribute);
          add(node);
          if (!records.has(node)) { changed = true; continue; }
          mark(node, mediaAttribute, nativeKind(node));
          if (records.get(node).nativeBoundary) continue;
        } else {
          if (node.matches(mediaSelector)) add(node);
          if (node.matches(controlSelector)) addControl(node);
          if (!scopes.has(node) && !controls.has(node) && !records.has(node)) continue;
        }
        if (scopes.has(node)) {
          if (scopeKeys.get(node) !== scopeKey(node)) {
            refreshScope(node);
            geometryDirty = true;
            pruneNeeded = true;
            changed = true;
          } else changed = invalidateRegion(node.closest(sidebarSelector) ? document.body : node, false) || changed;
          if (!records.has(node) && !controls.has(node)) continue;
        }
        const control = controls.get(node);
        if (control) { control.box = null; control.surface = null; control.candidate = undefined; }
        if (control) control.paintDirty = true;
        changed = true;
      }
      changed ||= stats.layerWrites !== before;
      if (pruneNeeded && [...viewers].some(node => !node.isConnected || !node.matches(viewerSelector)))
        changed = true;
      if (changed) {
        if (pruneNeeded) prune();
        if (isPreviewOpen() !== previewVisible) {
          if (raf) cancelAnimationFrame(raf);
          render();
        } else schedule();
      }
    });
    const isPreviewOpen = () => [...viewers].some(node =>
      node.isConnected && node.matches(viewerSelector) && node.checkVisibility({ checkVisibilityCSS: true }));
    const inViewport = box => box.width > 0 && box.height > 0 && box.right > 0 &&
      box.bottom > 0 && box.left < innerWidth && box.top < innerHeight;
    const overlap = (a, b) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
    const wallpaperInput = (box, sx, sy) => {
      const width = innerWidth / sx, height = innerHeight / sy;
      if (fitMode === "tile") {
        const key = `${innerWidth}|${innerHeight}`;
        if (resource.tileInputKey !== key) {
          if (resource.tileInputUrl) URL.revokeObjectURL(resource.tileInputUrl);
          const tiled = svgNode("svg", { width: innerWidth, height: innerHeight });
          const pattern = svgNode("pattern", { id: "wall", patternUnits: "userSpaceOnUse",
            width: resource.width, height: resource.height });
          pattern.appendChild(svgNode("image", { href: source, width: resource.width, height: resource.height }));
          const patterns = svgNode("defs");
          patterns.appendChild(pattern);
          tiled.append(patterns, svgNode("rect", { width: innerWidth, height: innerHeight, fill: "url(#wall)" }));
          resource.tileInputUrl = URL.createObjectURL(new Blob(
            [new XMLSerializer().serializeToString(tiled)], { type: "image/svg+xml" }));
          resource.tileInputKey = key;
        }
        return { href: resource.tileInputUrl, x: -box.x / sx, y: -box.y / sy,
          width, height, preserveAspectRatio: "none", result: "wall" };
      }
      let x = -box.x / sx, y = -box.y / sy, w = width, h = height;
      let aspect = fitMode === "fill" ? "xMidYMid slice" : fitMode === "fit" ? "xMidYMid meet" : "none";
      if (fitMode === "center") {
        w = resource.width / sx;
        h = resource.height / sy;
        x += (width - w) / 2;
        y += (height - h) / 2;
      }
      return { href: resource.imageSource, x, y, width: w, height: h, preserveAspectRatio: aspect, result: "wall" };
    };
    const backdropInput = (item, box, sx, sy, index) => {
      const target = item.node, paint = computed(target), mediaBox = item.box;
      const x = (mediaBox.x - box.x) / sx, y = (mediaBox.y - box.y) / sy;
      const width = mediaBox.width / sx, height = mediaBox.height / sy;
      const input = `media-${index}`, shape = `shape-${index}`;
      const clipped = `clipped-${index}`, painted = `painted-${index}`;
      const primitives = [];
      if (target instanceof HTMLImageElement && target.complete && target.naturalWidth) {
        const sourceUrl = target.currentSrc || target.src;
        const mediaRecord = records.get(target);
        if (mediaRecord.sourceUrl !== sourceUrl || mediaRecord.sourceWidth !== target.naturalWidth ||
          mediaRecord.sourceHeight !== target.naturalHeight) {
          mediaRecord.sourceUrl = sourceUrl;
          mediaRecord.sourceWidth = target.naturalWidth;
          mediaRecord.sourceHeight = target.naturalHeight;
          mediaRecord.inputRevision = ++nextSource;
          mediaRecord.inputSource = sourceUrl;
          if (sourceUrl.startsWith("data:image/")) {
            const sourceSvg = svgNode("svg", { width: target.naturalWidth, height: target.naturalHeight });
            sourceSvg.appendChild(svgNode("image", { href: sourceUrl,
              width: target.naturalWidth, height: target.naturalHeight, preserveAspectRatio: "none" }));
            mediaRecord.inputSource = "data:" + "image/svg+xml;charset=utf-8," +
              encodeURIComponent(new XMLSerializer().serializeToString(sourceSvg));
          }
        }
        let w = width, h = height, left = x, top = y;
        const mediaScaleX = mediaBox.width / (target.offsetWidth || mediaBox.width);
        const mediaScaleY = mediaBox.height / (target.offsetHeight || mediaBox.height);
        const naturalWidth = target.naturalWidth * mediaScaleX / sx;
        const naturalHeight = target.naturalHeight * mediaScaleY / sy;
        const fit = paint.objectFit;
        if (fit !== "fill") {
          let scale = fit === "cover" ? Math.max(width / naturalWidth, height / naturalHeight)
            : fit === "none" ? 1 : Math.min(width / naturalWidth, height / naturalHeight);
          if (fit === "scale-down") scale = Math.min(1, scale);
          w = naturalWidth * scale;
          h = naturalHeight * scale;
          const positions = paint.objectPosition.split(/\s+/);
          const offset = (value, available) => value?.endsWith("%")
            ? available * parseFloat(value) / 100 : parseFloat(value) || 0;
          left += offset(positions[0], width - w);
          top += offset(positions[1] || positions[0], height - h);
        }
        primitives.push(["feImage", { href: mediaRecord.inputSource,
          x: left, y: top, width: w, height: h, preserveAspectRatio: "none", result: input }]);
      } else {
        // Opaque live surfaces use the existing backdrop pixels, never a capture.
        primitives.push(["feComposite", { in: "SourceGraphic", in2: "SourceGraphic",
          operator: "in", result: input }]);
      }
      const radius = paint.borderTopLeftRadius.split(/\s+/);
      const length = (value, size, scale) => value?.endsWith("%")
        ? size * parseFloat(value) / 100 : (parseFloat(value) || 0) / scale;
      const rx = Math.min(width / 2, length(radius[0], width, sx));
      const ry = Math.min(height / 2, length(radius[1] || radius[0], height, sy));
      const clip = `<svg xmlns="${ns}" width="${width}" height="${height}"><rect width="${width}" height="${height}" rx="${rx}" ry="${ry}" fill="white"/></svg>`;
      primitives.push(
        ["feImage", { href: `data:image/svg+xml,${encodeURIComponent(clip)}`,
          x, y, width, height, preserveAspectRatio: "none", result: shape }],
        ["feComposite", { in: input, in2: shape, operator: "in", result: clipped }],
      );
      if (Number(paint.opacity) !== 1) primitives.push(["feColorMatrix", {
        in: clipped, type: "matrix", values: `1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 ${paint.opacity} 0`, result: painted,
      }]);
      return { primitives, input: Number(paint.opacity) === 1 ? clipped : painted, key: [x, y, width, height,
        records.get(target).inputRevision || 0, paint.objectFit,
        paint.objectPosition, paint.borderTopLeftRadius, paint.opacity].join("|") };
    };
    const restoreBackdrop = (record, box, sx, sy, mediaBoxes) => {
      if (record.nativeBackdrop === "none" || opacity === 1) return "";
      const padding = (record.nativeBackdrop.match(/[\d.]+px/g) || [])
        .reduce((sum, value) => sum + parseFloat(value) * 3, 0);
      const area = { left: box.left - padding * sx, right: box.right + padding * sx,
        top: box.top - padding * sy, bottom: box.bottom + padding * sy };
      const inputs = mediaBoxes.filter(item => overlap(area, item.box))
        .map((item, index) => backdropInput(item, box, sx, sy, index));
      const key = [box.x, box.y, box.width, box.height, sx, sy, innerWidth, innerHeight,
        record.nativeBackdrop, ...inputs.map(input => input.key)].join("|");
      const id = `${codexPlusImageOverlayId}-backdrop-${record.id}`;
      if (!record.backdrop) {
        record.backdrop = svgNode("filter", { id, filterUnits: "userSpaceOnUse", "color-interpolation-filters": "sRGB" });
        defs.appendChild(record.backdrop);
      }
      if (key !== record.backdropKey) {
        record.backdropKey = key;
        const primitives = [svgNode("feImage", wallpaperInput(box, sx, sy))];
        let compensation = "wall";
        for (const [index, { primitives: sourcePrimitives, input }] of inputs.entries()) {
          const next = `compensation-${index}`;
          primitives.push(...sourcePrimitives.map(([tag, attributes]) => svgNode(tag, attributes)),
            svgNode("feComposite", { in: compensation, in2: input, operator: "out", result: "wallOutside" }),
            svgNode("feComposite", { in: input, in2: "wall", operator: "in", result: "mediaInside" }),
            svgNode("feComposite", { in: "wallOutside", in2: "mediaInside", operator: "arithmetic",
              k2: 1, k3: 1, result: next }),
          );
          compensation = next;
        }
        // Premultiplied subtraction also restores alpha. Unpremultiplication
        // recovers the original backdrop without a divide or a backing layer.
        primitives.push(
          svgNode("feComposite", { in: "SourceGraphic", in2: compensation, operator: "arithmetic",
            k2: 1, k3: -opacity, result: "restored" }),
          svgNode("feColorMatrix", { in: "restored", type: "matrix",
            values: "1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 0 1" }),
        );
        record.backdrop.setAttribute("x", String(-padding));
        record.backdrop.setAttribute("y", String(-padding));
        record.backdrop.setAttribute("width", String(box.width / sx + padding * 2));
        record.backdrop.setAttribute("height", String(box.height / sy + padding * 2));
        record.backdrop.replaceChildren(...primitives);
      }
      return `backdrop-filter:url("#${id}") ${record.nativeBackdrop}!important;`;
    };
    const activateTint = (record, node) => {
      mark(node, controlAttribute, String(record.id));
      if (node.matches(topControlSelector) || node.closest(topControlSelector) || node.querySelector(topControlSelector))
        mark(node, topControlAttribute, "");
      else unmark(node, topControlAttribute);
    };
    const createTint = (record, node, box, nativeFilter, mediaBoxes) => {
      const sx = box.width / (node.offsetWidth || box.width);
      const sy = box.height / (node.offsetHeight || box.height);
      const backdropRule = restoreBackdrop(record, box, sx, sy, mediaBoxes);
      const key = [box.x, box.y, box.width, box.height, innerWidth, innerHeight, sx, sy,
        nativeFilter, record.nativePosition, record.nativeShadow, record.nativeBackdrop, record.backdropKey].join("|");
      if (record.key === key) {
        if (!record.rule && record.cachedRule) {
          record.rule = record.cachedRule;
          activateTint(record, node);
          stats.layerWrites++;
        }
        return;
      }
      record.key = key;
      const id = `${codexPlusImageOverlayId}-control-${record.id}`;
      if (!record.filter) {
        record.filter = svgNode("filter", { id, filterUnits: "userSpaceOnUse", "color-interpolation-filters": "sRGB" });
        defs.appendChild(record.filter);
        record.filter.append(
          svgNode("feImage", wallpaperInput(box, sx, sy)),
          svgNode("feColorMatrix", { in: "wall", type: "matrix",
            values: `1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 ${opacity} 0`, result: "tint" }),
          svgNode("feComposite", { in: "SourceGraphic", in2: "tint", operator: "out", result: "ordinary" }),
          svgNode("feComposite", { in: "tint", in2: "SourceAlpha", operator: "in", result: "wallInk" }),
          svgNode("feComposite", { in: "ordinary", in2: "wallInk", operator: "arithmetic", k2: 1, k3: 1 }),
        );
      } else {
        const input = record.filter.firstElementChild;
        for (const [name, value] of Object.entries(wallpaperInput(box, sx, sy)))
          if (input.getAttribute(name) !== String(value)) input.setAttribute(name, String(value));
      }
      let padding = 0;
      for (const shadow of record.nativeShadow.replace(/\([^)]*\)/g, "").split(",")) {
        if (shadow.includes("inset")) continue;
        const [dx = 0, dy = 0, blur = 0, spread = 0] = (shadow.match(/-?[\d.]+px/g) || []).map(parseFloat);
        padding = Math.max(padding, Math.max(Math.abs(dx), Math.abs(dy)) + blur * 2 + Math.abs(spread));
      }
      padding += (nativeFilter.match(/-?[\d.]+px/g) || []).reduce((sum, value) => sum + Math.abs(parseFloat(value)) * 3, 0);
      for (const [name, value] of Object.entries({ x: -padding, y: -padding,
        width: box.width / sx + padding * 2, height: box.height / sy + padding * 2 }))
        if (record.filter.getAttribute(name) !== String(value)) record.filter.setAttribute(name, String(value));
      // Tint the control's own ink, preserving alpha. This composes once over
      // an already-tinted backdrop; it does not create a media backing region.
      record.rule = `[${controlAttribute}="${record.id}"]{${record.nativePosition === "static" ? "position:relative!important;" : ""}${backdropRule}filter:${nativeFilter === "none" ? "" : `${nativeFilter} `}url("#${id}")!important;}`;
      record.cachedRule = record.rule;
      activateTint(record, node);
      stats.layerWrites++;
    };
    let lastRules = null;
    let rulesQueued = false;
    const writeRules = () => {
      if (rulesQueued || disposed) return;
      rulesQueued = true;
      // Commit once per mutation batch, before the browser paints.
      queueMicrotask(() => { rulesQueued = false; if (!disposed) flushRules(); });
    };
    const flushRules = () => {
      const rules = nativeRules + [...footers.values()].map(record => record.rule).join("\n") +
        [...paintedNodes].map(node => records.get(node))
        .map(record => record.wallRule).join("\n") +
        [...activeControls].map(node => controls.get(node).rule).concat([...popups.values()].map(record => record.rule))
          .filter(Boolean).join("\n");
      if (rules !== lastRules) { style.textContent = rules; lastRules = rules; }
    };
    const render = () => {
      raf = 0;
      if (disposed || document.hidden) return;
      stats.frames++;
      const previewWasVisible = previewVisible;
      previewVisible = isPreviewOpen();
      if (previewVisible) {
        // 保留预览内容的原生层级，只更新控件背景；不恢复整个会话的媒体/控件扫描。
        if (paintsDirty.size || geometryDirty && geometryPaints.size) refreshPaints(true);
        geometryDirty = false;
        return;
      }
      // 预览期间跳过的祖先变化，在预览关闭后补查一次。
      if (previewWasVisible) for (const node of paintedNodes) {
        const paint = records.get(node)?.paint;
        if (paint) { paint.chainDirty = true; paintsDirty.add(node); }
      }
      if (paintsDirty.size || geometryDirty && geometryPaints.size) refreshPaints();
      const media = [...visibleMedia].filter(node => node.isConnected && !records.get(node)?.nativeBoundary &&
        !inactivePanel(node) &&
        !node.matches(composerSelector) && !node.matches(browserSelector) && !node.matches(reviewHeaderSelector) &&
        node.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true }));
      const mediaBoxes = new Map();
      const mediaGroups = new Map();
      const measureMedia = composer => {
        if (mediaGroups.has(composer)) return mediaGroups.get(composer);
        const group = media.filter(target => !composer || composer.contains(target)).map(target => {
        if (!mediaBoxes.has(target)) {
          const item = records.get(target);
          mediaBoxes.set(target, { node: target, box: !geometryDirty && item.box ? item.box : (item.box = bounds(target)) });
        }
        return mediaBoxes.get(target);
        }).filter(item => inViewport(item.box));
        mediaGroups.set(composer, group);
        return group;
      };
      const candidates = new Set([...visibleControls, ...activeControls]);
      for (const node of candidates) {
        const record = controls.get(node);
        if (!record) continue;
        const composer = node.closest(composerSelector);
        let active = !inactivePanel(node) && visibleControls.has(node) && !node.closest(viewerSelector) &&
          !node.parentElement?.closest(`[${controlAttribute}]`) &&
          !records.has(node) && !node.querySelector(mediaSelector) &&
          node.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true }) &&
          (composer
            ? media.some(target => composer.contains(target) && !node.contains(target))
            : media.length);
        const current = record.rule;
        if (active) {
          if (record.candidate === undefined) {
            const surface = controlSurface(node);
            record.surface = surface;
            const paint = computed(surface);
            record.candidate = paint.position === "absolute" || paint.position === "fixed" ||
              paint.zIndex !== "auto";
          }
          active = record.candidate;
        }
        if (active) {
          const box = !geometryDirty && record.box ? record.box : (record.box = bounds(node));
          active = inViewport(box);
          if (active) {
            active = measureMedia(composer).some(item =>
              !node.contains(item.node) && overlap(box, item.box));
            if (active) {
              if (record.surface && !record.surface.contains(node)) record.surface = null;
              const surface = record.surface ||= controlSurface(node);
              if (surface !== node) {
                if (addControl(surface, node)) {
                  visibleControls.add(surface);
                  candidates.add(surface);
                }
                active = false;
              }
            }
            if (active) {
              let originalStyle = null;
              if (!current || record.paintDirty) {
                if (current) unmark(node, controlAttribute);
                originalStyle = computed(node);
                if (current) mark(node, controlAttribute, String(record.id));
                record.paintDirty = false;
              }
              const baseFilter = originalStyle?.filter || record.nativeFilter;
              if (originalStyle) {
                record.nativePosition = originalStyle.position;
                record.nativeShadow = originalStyle.boxShadow;
                record.nativeBackdrop = originalStyle.backdropFilter;
              }
              record.nativeFilter = baseFilter;
              createTint(record, node, box, baseFilter,
                record.nativeBackdrop === "none" ? [] : measureMedia(composer));
            }
          }
        }
        if (!active && current) {
          record.rule = "";
          unmark(node, controlAttribute);
          unmark(node, topControlAttribute);
          stats.layerWrites++;
        }
        if (record.rule) activeControls.add(node);
        else activeControls.delete(node);
      }
      writeRules();
      geometryDirty = false;
    };
    function schedule(geometry = false) {
      geometryDirty ||= geometry === true;
      if (!disposed && !document.hidden && !raf) raf = requestAnimationFrame(render);
    }
    const onControlMotionEnd = event => {
      const node = event.target?.closest?.(`[${controlAttribute}]`);
      const control = controls.get(node);
      if (!control?.rule) return;
      control.paintDirty = true;
      control.box = null;
      schedule();
    };
    // 侧栏开合等带过渡的 translate 在结束时才落定；不逐帧跟随，只在结束后重算一次。
    const onPaintMotionEnd = event => {
      const target = event.target;
      if (target?.nodeType !== 1 || owned(target)) return;
      if (invalidatePaintChains(target, previewVisible)) schedule();
    };
    const onScroll = event => {
      if (event.target?.closest?.(sidebarSelector)) sidebarScrollUntil = performance.now() + 180;
      if (previewVisible) {
        for (const node of geometryPaints) if (isViewerSurface(node) && event.target?.contains?.(node)) {
          paintsDirty.add(node); schedule();
        }
        return;
      }
      const target = event.target;
      // 只有按视口换算的表面需要跟随滚动重算；沿用 fixed 的表面不受影响，无额外开销。
      if (geometryPaints.size) {
        let moved = false;
        for (const node of geometryPaints) {
          const surface = records.get(node)?.paint?.surface;
          if (!surface || !(target === document || target?.contains?.(surface))) continue;
          paintsDirty.add(node);
          moved = true;
        }
        if (moved) schedule();
      }
      if (!target || target === document || target === document.documentElement ||
        target === document.body || target === document.scrollingElement) {
        if (activeControls.size) schedule(true);
        else invalidateRegion(document.body, false);
        return;
      }
      if (target.closest?.(browserSelector)) return;
      invalidateRegion(target, activeControls.size > 0);
    };
    const invalidateRegion = (target, redraw = true) => {
      let changed = false;
      for (const node of new Set([...visibleMedia, ...visibleControls, ...activeControls])) {
        if (!target.contains?.(node)) continue;
        const record = records.get(node);
        const control = controls.get(node);
        if (record) record.box = null;
        if (control) control.box = null;
        changed = true;
      }
      if (changed && redraw) schedule();
      return changed;
    };
    const onSidebarWheel = event => {
      if (event.target?.closest?.(sidebarSelector)) sidebarScrollUntil = performance.now() + 180;
    };
    const onSidebarHover = event => {
      if (performance.now() >= sidebarScrollUntil || event.buttons || event.pointerType === "touch") return;
      if (event.target?.closest?.(sidebarRowSelector)?.closest(sidebarSelector))
        event.stopImmediatePropagation();
    };
    const events = [
      [document, "visibilitychange", () => {
        if (document.hidden) {
          if (raf) cancelAnimationFrame(raf);
          raf = 0;
        } else schedule(true);
      }, false],
      [document, "scroll", onScroll, true],
      [document, "wheel", onSidebarWheel, true],
      [document, "pointerover", onSidebarHover, true],
      [document, "pointermove", onSidebarHover, true],
      [document, "mouseover", onSidebarHover, true],
      [document, "mousemove", onSidebarHover, true],
      [document, "pointerdown", () => { sidebarScrollUntil = 0; }, true],
      [window, "resize", () => schedule(true), false],
      [document, "load", event => {
        const record = records.get(event.target);
        if (!record || record.nativeBoundary || event.target.matches(browserSelector)) return;
        const region = event.target.closest(`${timelineSelector}, ${composerSelector}, ${sidebarSelector}`);
        if (region) invalidateRegion(region);
        else schedule(true);
      }, true],
      [document, "toggle", schedule, true],
      [document, "transitionend", onControlMotionEnd, true],
      [document, "animationend", onControlMotionEnd, true],
      [document, "transitionend", onPaintMotionEnd, true],
      [document, "transitioncancel", onPaintMotionEnd, true],
      [document, "animationend", onPaintMotionEnd, true],
    ];
    window.__codexPlusImageOverlayCleanup = (keepResource = false) => {
      if (disposed) return;
      disposed = true;
      if (raf) cancelAnimationFrame(raf);
      mutationObserver.disconnect();
      resizeObserver.disconnect();
      intersectionObserver.disconnect();
      for (const [target, event, handler, capture] of events) target.removeEventListener(event, handler, capture);
      for (const [node, attributes] of [...originalAttributes])
        for (const name of [...attributes.keys()]) unmark(node, name);
      overlay.remove();
      style.remove();
      svg.remove();
      image.onload = null;
      image.onerror = null;
      for (const collection of [records, scopes, browserPortals, controls, popups, visibleMedia,
        visibleControls, activeControls, paintedNodes, viewers, scopeKeys, footers, themeInputs,
        paintsDirty, geometryPaints, paintObservers]) collection.clear();
      finishReady(false);
      if (!keepResource && resource) {
        if (resource.blobUrl) URL.revokeObjectURL(resource.blobUrl);
        if (resource.tileInputUrl) URL.revokeObjectURL(resource.tileInputUrl);
        if (window.__codexPlusImageOverlayResource === resource) window.__codexPlusImageOverlayResource = null;
        if (window.__codexPlusImageOverlayBlobUrl === resource.blobUrl) window.__codexPlusImageOverlayBlobUrl = "";
      }
    };
    const install = () => {
      if (disposed) return;
      resource.ready = true;
      resource.opacity = opacity;
      resource.fitMode = fitMode;
      resource.width ||= image.naturalWidth;
      resource.height ||= image.naturalHeight;
      overlay.style.backgroundImage = `url(${JSON.stringify(resource.imageSource)})`;
      style.textContent = nativeRules;
      root.append(style, svg);
      root.insertBefore(overlay, document.body || root.firstChild);
      visit(root);
      themeInputs.set(root, themeKey(root));
      if (document.body) themeInputs.set(document.body, themeKey(document.body));
      mutationObserver.observe(root, { childList: true, subtree: true, attributes: true,
        attributeFilter: ["class", "style", "hidden", "open", "inert", "aria-hidden", "src", "srcset", "sizes", "role", "data-testid", "data-browser-sidebar-webview", "data-browser-sidebar-toolbar", "data-thread-scroll-footer", "data-app-shell-focus-area", "data-composer-overlay-floating-ui", "data-app-shell-header-layout", "data-sticky", "data-composer-placement", "data-composer-surface-variant", "data-composer-radius-variant", "data-composer-utility-bar-variant", "data-menu-overlay-surface", "data-codex-composer-request-navigation", "data-codex-approval-surface", "data-composer-rail-item", "data-composer-attachment-pill", "data-sonner-toast", "data-in-progress-fixed-content", "data-image-preview-composer", "data-image-preview-toolbar"] });
      for (const [target, event, handler, capture] of events)
        target.addEventListener(event, handler, { capture, passive: true });
      // 首次扫描登记的壁纸表面已排了一帧；这里同步渲染，撤掉重复的那一帧。
      if (raf) cancelAnimationFrame(raf);
      render();
      finishReady(true);
      report?.("image_overlay_installed", { opacity, fitMode, sourceKind: source.startsWith("data:") ? "data-uri" : "unknown" });
    };
    if (resource) install();
    else {
      let blobUrl = "";
      const data = source.match(/^data:([^;,]+);base64,([\s\S]*)$/);
      if (data) {
        const decoded = atob(data[2]);
        const bytes = new Uint8Array(decoded.length);
        for (let index = 0; index < decoded.length; index++) bytes[index] = decoded.charCodeAt(index);
        blobUrl = URL.createObjectURL(new Blob([bytes], { type: data[1] }));
      }
      resource = { version: 37, source, blobUrl, imageSource: blobUrl || source };
      window.__codexPlusImageOverlayResource = resource;
      window.__codexPlusImageOverlayBlobUrl = blobUrl;
      image.onload = install;
      image.onerror = () => { if (!disposed) window.__codexPlusImageOverlayCleanup?.(); };
      image.src = resource.imageSource;
    }
  }
