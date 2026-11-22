import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";

const source = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

function functionSource(name: string) {
  const matches = Array.from(source.matchAll(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "gm")));
  assert.ok(matches.length, `missing ${name}`);
  return matches.at(-1)![0];
}

function styleNode() {
  const values = new Map<string, string>();
  return {
    left: "", top: "",
    setProperty(name: string, value: string) { values.set(name, value); },
    removeProperty(name: string) { values.delete(name); },
    values,
  };
}

function layoutHarness(zoom = 1) {
  const nodes: Record<string, { rect: Record<string, number>; getBoundingClientRect(): Record<string, number> }> = {};
  const rectangles: Record<string, Record<string, number>> = {
    main: { left: 320, top: 40, width: 960, height: 760, right: 1280 },
    "aside.app-shell-left-panel": { left: 0, top: 40, width: 320, height: 760, right: 320 },
    "nav[data-app-navigation-rail]": { left: 0, top: 40, width: 62, height: 760, right: 62 },
    "[data-app-shell-workspace-row]": { left: 0, top: 40, width: 962, height: 740, right: 962, bottom: 780 },
  };
  for (const [selector, rect] of Object.entries(rectangles)) {
    nodes[selector] = { rect, getBoundingClientRect() { return this.rect; } };
  }
  const style = styleNode();
  const overlay = { style, dataset: {} as Record<string, string>, isConnected: true, classList: { contains: (name: string) => name === "codex-plus-page-overlay" } };
  const listeners = new Set<() => void>();
  const observers: Observer[] = [];
  class Observer {
    targets = new Set<unknown>();
    disconnects = 0;
    callback: () => void;
    constructor(callback: () => void) { this.callback = callback; observers.push(this); }
    observe(node: unknown) { this.targets.add(node); }
    disconnect() { this.disconnects++; this.targets.clear(); }
  }
  const window: Record<string, unknown> = {
    innerWidth: 1000,
    innerHeight: 800,
    addEventListener(_event: string, listener: () => void) { listeners.add(listener); },
    removeEventListener(_event: string, listener: () => void) { listeners.delete(listener); },
  };
  const document = {
    querySelector: (selector: string) => nodes[selector] ?? null,
    querySelectorAll: (selector: string) => selector === "[data-app-shell-titlebar]"
      ? [...titlebars, ...(nodes[selector] ? [nodes[selector]] : [])]
      : overlay.isConnected ? [{ remove() { overlay.isConnected = false; } }] : [],
  };
  // 外壳里排在前面的那个 0×0 标题栏空壳。
  const titlebars = [{ getBoundingClientRect: () => ({ left: 0, top: 0, width: 0, height: 0, right: 0, bottom: 0 }) }];
  const runtime = new Function("document", "window", "ResizeObserver", "codexPlusWindowZoom", "getComputedStyle", `
    const codexPlusPageClass = "codex-plus-page-overlay";
    const codexPlusRailSelector = "nav[data-app-navigation-rail]";
    function setCodexPlusSidebarNavActive() {}
    // 返回按钮行为由 page-back-navigation.test.ts 验证，此处只隔离布局生命周期。
    function syncCodexPlusPageBackButton() {}
    function restoreCodexPlusPageBackButton() {}
    ${functionSource("positionCodexPlusPage")}
    ${functionSource("observeCodexPlusPageLayout")}
    ${functionSource("closeCodexPlusPage")}
    return { position: positionCodexPlusPage, attach: observeCodexPlusPageLayout, close: closeCodexPlusPage };
  `)(document, window, Observer, () => zoom, (node: { radius?: string; background?: string }) => ({ borderTopLeftRadius: node.radius ?? "0px", backgroundColor: node.background ?? "rgba(0, 0, 0, 0)", getPropertyValue: () => "" })) as {
    position(node: typeof overlay): void; attach(node: typeof overlay): void; close(): void;
  };
  return { ...runtime, overlay, nodes, listeners, observers, window, document };
}

it("keeps panel edges in layout coordinates without rewriting zoom during positioning", () => {
  for (const zoom of [0.8, 1, 1.25]) {
    const h = layoutHarness(zoom);
    h.position(h.overlay);
    assert.equal(h.overlay.style.left, `${62 / zoom}px`);
    assert.equal(h.overlay.style.top, `${40 / zoom}px`);
    assert.equal(h.overlay.style.values.get("--codex-plus-page-top"), `${40 / zoom}px`);
    assert.equal(h.overlay.style.values.get("--codex-plus-page-right"), `${38 / zoom}px`);
    assert.equal(h.overlay.style.values.get("--codex-plus-page-bottom"), `${20 / zoom}px`);
    assert.equal(h.overlay.style.values.get("--codex-plus-zoom"), undefined);
    assert.equal(h.overlay.style.values.get("zoom"), undefined);
    delete h.nodes["nav[data-app-navigation-rail]"];
    h.position(h.overlay);
    assert.equal(h.overlay.style.left, `${320 / zoom}px`, "legacy sidebar boundary is unchanged");
    h.nodes.main.rect.height = 0;
    h.nodes["aside.app-shell-left-panel"].rect.top = 24;
    h.position(h.overlay);
    assert.equal(h.overlay.style.top, `${24 / zoom}px`);
    delete h.nodes.main;
    delete h.nodes["aside.app-shell-left-panel"];
    h.position(h.overlay);
    assert.equal(h.overlay.style.top, "0px");
    assert.equal(h.overlay.style.left, "0px");
  }
});

it("starts at the window top under the transparent titlebar when the native panel does", () => {
  for (const zoom of [1, 1.25]) {
    const h = layoutHarness(zoom);
    const add = (selector: string, rect: Record<string, number>, extra: Record<string, unknown> = {}) => {
      h.nodes[selector] = { rect, getBoundingClientRect() { return this.rect; }, ...extra };
    };
    add("[data-app-shell-titlebar]", { left: 0, top: 0, width: 1000, height: 40, right: 1000, bottom: 40 });
    add("[data-app-shell-surface-backing]", { left: 320, top: 0, width: 680, height: 800, right: 1000 }, { radius: "12px", background: "rgb(24, 24, 24)" });
    h.position(h.overlay);
    assert.equal(h.overlay.style.top, "0px", "no strip of native backing is left above the page");
    assert.equal(h.overlay.dataset.codexPlusSurface, "native", "page, nav and bars take the native panel colour");
    assert.equal(h.overlay.style.values.get("--codex-plus-shell-surface"), "rgb(24, 24, 24)");
    assert.equal(h.overlay.style.values.get("--codex-plus-page-inset-top"), `${40 / zoom}px`);
    assert.equal(h.overlay.dataset.codexPlusTitlebar, "inset");
    assert.equal(h.overlay.style.values.get("--codex-plus-page-radius"), `${12 / zoom}px`, "sidebar open: rounded like the native panel");
    assert.equal(h.overlay.style.values.get("--codex-plus-page-radius-end"), "0px");
    assert.equal(h.overlay.dataset.codexPlusSidebar, "open", "sidebar open: pages with a left column draw over the native sidebar");
    assert.equal(h.overlay.style.values.get("--codex-plus-native-sidebar-width"), `${(320 - 62) / zoom}px`, "left column ends where the native panel starts");
    h.nodes["[data-app-shell-surface-backing]"].rect.left = 0;
    h.position(h.overlay);
    assert.equal(h.overlay.style.values.get("--codex-plus-page-radius"), "0px", "sidebar collapsed: continuous with the panel");
    assert.equal(h.overlay.dataset.codexPlusSidebar, undefined);
    assert.equal(h.overlay.style.values.get("--codex-plus-native-sidebar-width"), undefined);
    (h.nodes["[data-app-shell-surface-backing]"] as unknown as { background: string }).background = "rgba(0, 0, 0, 0)";
    h.position(h.overlay);
    assert.equal(h.overlay.dataset.codexPlusSurface, undefined, "a skin-cleared panel never makes the page see-through");
    assert.equal(h.overlay.style.values.get("--codex-plus-shell-surface"), undefined);
    delete h.nodes["[data-app-shell-surface-backing]"];
    h.position(h.overlay);
    assert.equal(h.overlay.style.top, `${40 / zoom}px`, "older shells keep starting below the header");
    assert.equal(h.overlay.dataset.codexPlusTitlebar, undefined);
    assert.equal(h.overlay.style.values.get("--codex-plus-page-radius-end"), undefined);
  }
});

it("retargets the layout observer when the host replaces its main or rail", () => {
  const h = layoutHarness();
  h.attach(h.overlay);
  const observer = h.observers[0];
  assert.equal(observer.targets.size, 3);
  const oldRail = h.nodes["nav[data-app-navigation-rail]"];
  h.nodes["nav[data-app-navigation-rail]"] = {
    rect: { ...oldRail.rect, top: 60 },
    getBoundingClientRect() { return this.rect; },
  };
  h.position(h.overlay);
  assert.equal(observer.targets.has(oldRail), false);
  assert.equal(observer.targets.has(h.nodes["nav[data-app-navigation-rail]"]), true);
  assert.equal(h.overlay.style.top, "60px");
  const disconnects = observer.disconnects;
  h.position(h.overlay);
  assert.equal(observer.disconnects, disconnects, "unchanged hosts do not resubscribe");
});

it("closes and reopens pages without retaining observers, resize handlers or queued layout writes", () => {
  const h = layoutHarness();
  let cleanupCalls = 0;
  for (let i = 0; i < 50; i++) {
    h.overlay.isConnected = true;
    h.attach(h.overlay);
    assert.equal(h.listeners.size, 1);
    h.window.__codexPlusExtensionPageCleanup = () => { cleanupCalls++; };
    const observer = h.observers.at(-1)!;
    h.close();
    h.close();
    assert.equal(cleanupCalls, i + 1, "extension cleanup executes exactly once");
    assert.equal(h.listeners.size, 0);
    assert.equal(observer.targets.size, 0);
    assert.equal(h.window.__codexPlusPageResizeHandler, null);
    assert.equal(h.window.__codexPlusPageLayoutObserver, null);
    assert.equal(h.window.__codexPlusPageLayoutTargets, null);
    const top = h.overlay.style.top;
    h.nodes.main.rect.top += 1;
    observer.callback();
    assert.equal(h.overlay.style.top, top, "a queued callback cannot reposition a removed page");
  }
});

it("runs old extension cleanup before replacing a page with either a built-in page or a modal", () => {
  for (const page of [true, false]) {
    const h = layoutHarness();
    h.attach(h.overlay);
    let cleanups = 0;
    h.window.__codexPlusExtensionPageCleanup = () => { cleanups++; };
    const stop = new Error("stop before building the new DOM");
    const document = { ...h.document, createElement() { throw stop; } };
    const open = new Function("document", "closeCodexPlusPage", "codexPlusModalTab", `
      const codexPlusPageClass = "codex-plus-page-overlay";
      ${functionSource("openCodexPlusModal")}
      return openCodexPlusModal;
    `)(document, h.close, (tab: string) => tab);
    assert.throws(() => open({ page }), (error: unknown) => error === stop);
    assert.equal(cleanups, 1);
    assert.equal(h.listeners.size, 0);
    assert.equal(h.observers[0].targets.size, 0);
  }
});

it("cleans up the previous extension before opening another extension page", () => {
  const h = layoutHarness();
  h.attach(h.overlay);
  let cleanups = 0;
  h.window.__codexPlusExtensionPageCleanup = () => { cleanups++; };
  const stop = new Error("stop before building the next extension DOM");
  const open = new Function("document", "closeCodexPlusPage", `
    const codexPlusPageClass = "codex-plus-page-overlay";
    ${functionSource("openCodexPlusModalForExtension")}
    return openCodexPlusModalForExtension;
  `)({ ...h.document, createElement() { throw stop; } }, h.close);
  assert.throws(() => open("next", {}), (error: unknown) => error === stop);
  assert.equal(cleanups, 1);
  assert.equal(h.listeners.size, 0);
});

it("keeps only the current rail entry selected and removes inactive aria-current attributes", () => {
  const buttons = Array.from({ length: 8 }, () => ({
    dataset: {} as Record<string, string>,
    attributes: new Map<string, string>(),
    setAttribute(name: string, value: string) { this.attributes.set(name, value); },
    removeAttribute(name: string) { this.attributes.delete(name); },
  }));
  const ids = ["home", "extensions", "sponsor", "provider", "remote", "weixin", "tasks"];
  const document = {
    getElementById: (id: string) => {
      assert.equal(id, "sidebar", "retired plugin navigation must not be queried");
      return { querySelector: () => buttons[0] };
    },
    querySelector: (selector: string) => buttons[ids.findIndex(id => selector === `#${id} > button`) + 1],
  };
  const active = new Function("document", `
    const codexPlusSidebarNavId = "sidebar";
    const codexPlusRailNavId = "home";
    const codexPlusRailExtensionsId = "extensions";
    const codexPlusRailSponsorId = "sponsor";
    const codexPlusRailProviderId = "provider";
    const codexPlusRailRemoteId = "remote";
    const codexPlusRailWeixinId = "weixin";
    const codexPlusRailTasksId = "tasks";
    function syncCodexPlusRailNativeSelection() {}
    ${functionSource("setCodexPlusSidebarNavActive")}
    return setCodexPlusSidebarNavActive;
  `)(document);
  for (const entry of ids) {
    active(true, entry);
    for (const [index, button] of buttons.entries()) {
      const selected = (index === 0 ? "home" : ids[index - 1]) === entry;
      assert.equal(button.attributes.get("aria-current"), selected ? "page" : undefined);
      assert.equal(button.dataset.active, String(selected));
    }
  }
  active(false);
  for (const button of buttons) assert.equal(button.attributes.has("aria-current"), false);
});

it("replaces version 31 styles with bounded panel geometry and keeps reinjection idempotent", () => {
  const styleSource = functionSource("installStyle");
  const names = new Set([
    "styleId", "codexDeleteStyleVersion",
    ...Array.from(styleSource.matchAll(/\$\{([A-Za-z_$][A-Za-z0-9_$]*)/g), match => match[1]),
  ]);
  const declarations = [...names].map(name => {
    const match = source.match(new RegExp(`^  const ${name} = .+;$`, "m"))
      ?? source.match(new RegExp(`^  const ${name} = [\\s\\S]*?^  };$`, "m"));
    assert.ok(match, name);
    return match[0];
  }).join("\n");
  let existing: { dataset: Record<string, string>; textContent?: string; remove(): void } | null = {
    dataset: { codexDeleteStyleVersion: "31" },
    remove() { existing = null; },
  };
  let appended = 0;
  const document = {
    getElementById: () => existing,
    createElement: () => ({ dataset: {}, remove() { existing = null; } }),
    documentElement: { appendChild(node: NonNullable<typeof existing>) { existing = node; appended++; } },
  };
  const install = new Function("document", `${declarations}\n${styleSource}\nreturn installStyle;`)(document);
  install();
  // 版本 = 手写前缀 + 样式全文哈希（改 CSS 不必手动 +1）。
  assert.match(existing?.dataset.codexDeleteStyleVersion ?? "", /^\d+-[0-9a-z]+$/);
  assert.ok(Number.parseInt(existing?.dataset.codexDeleteStyleVersion ?? "", 10) >= 32);
  const pageRules = existing!.textContent!.match(/\.codex-plus-page-overlay\s*\{([^}]+)\}/)![1];
  assert.match(pageRules, /right: var\(--codex-plus-page-right, 0px\)/);
  assert.match(pageRules, /bottom: var\(--codex-plus-page-bottom, 0px\)/);
  assert.match(pageRules, /border-radius: var\(--codex-plus-page-radius, 0px\) var\(--codex-plus-page-radius-end, var\(--codex-plus-page-radius, 0px\)\)/);
  assert.match(pageRules, /height: calc\(100vh \/ var\(--codex-plus-zoom, 1\) - var\(--codex-plus-page-top, 0px\) - var\(--codex-plus-page-bottom, 0px\)\)/);
  // 标题栏抬到页面之上（仍低于弹窗）；让出的那条不声明 no-drag，窗口照样能拖。
  assert.match(existing!.textContent!, /\[data-app-shell-titlebar\] \{\s*z-index: 2147482001 !important;/);
  assert.match(existing!.textContent!, /\[data-codex-plus-titlebar="inset"\] > \.codex-plus-modal-content \{\s*-webkit-app-region: none;/);
  // 侧栏展开：有左栏的页面整块透明、左栏对齐原生侧栏，原生会话列表与页面内容藏起来，标题栏照常显示。
  assert.match(existing!.textContent!, /\[data-codex-plus-sidebar="open"\]:has\(:is\(\.codex-plus-page-nav, \.codex-plus-provider-sidebar, \.codex-plus-task-sidebar\)\) \{\s*border-radius: 0 !important;\s*background: transparent !important;/);
  assert.match(existing!.textContent!, /\[data-codex-plus-sidebar="open"\] :is\(\.codex-plus-page-nav, \.codex-plus-provider-sidebar, \.codex-plus-task-sidebar\) \{[^}]*width: var\(--codex-plus-native-sidebar-width\) !important;/);
  assert.match(existing!.textContent!, /html:has\([^)]*\[data-codex-plus-sidebar="open"\]\) :is\(\[data-sidebar-navigation-root\], \[data-app-shell-active-page\]\) \{\s*visibility: hidden !important;/);
  assert.match(existing!.textContent!, /html:has\([^)]*\[data-codex-plus-sidebar="open"\]\) \[data-app-shell-titlebar\] \{\s*visibility: visible !important;/);
  assert.match(existing!.textContent!, /\[data-codex-plus-surface="native"\] \{\s*--codex-plus-bg-primary: var\(--codex-plus-shell-surface\) !important;\s*--codex-plus-settings-page-bg: var\(--codex-plus-shell-surface\) !important;\s*--codex-plus-nav-bg: var\(--codex-plus-shell-surface\) !important;/);
  const modalRules = existing!.textContent!.match(/\.codex-plus-modal-overlay\s*\{([^}]+)\}/)![1];
  assert.match(modalRules, /height: calc\(100vh \/ var\(--codex-plus-zoom, 1\)\)/);
  install();
  assert.equal(appended, 1);
  // 手写前缀没变、只是 CSS 内容变了（旧注入留下的样式表）：照样换新，不能让新 JS 配旧样式。
  const prefix = existing!.dataset.codexDeleteStyleVersion.split("-")[0];
  existing!.dataset.codexDeleteStyleVersion = `${prefix}-stale`;
  install();
  assert.equal(appended, 2);
  install();
  assert.equal(appended, 2);
});
