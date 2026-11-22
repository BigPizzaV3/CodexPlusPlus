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
    // 悬浮左栏的鼠标逻辑不在布局生命周期里。
    function installCodexPlusNavPeek() {}
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
    // 收起后原生侧栏仍按原宽排着，只是被裁掉；悬浮卡片比它窄左边那 4px 缝。
    add("[data-sidebar-navigation-root]", { left: 62, top: 40, width: 258, height: 760, right: 320 });
    h.position(h.overlay);
    assert.equal(h.overlay.style.values.get("--codex-plus-page-radius"), "0px", "sidebar collapsed: continuous with the panel");
    assert.equal(h.overlay.dataset.codexPlusSidebar, "floating", "sidebar collapsed: left column floats like the native peek");
    assert.equal(h.overlay.style.values.get("--codex-plus-native-sidebar-width"), undefined);
    assert.equal(h.overlay.style.values.get("--codex-plus-native-peek-width"), `${258 / zoom - 4}px`);
    (h.nodes["[data-app-shell-surface-backing]"] as unknown as { background: string }).background = "rgba(0, 0, 0, 0)";
    h.position(h.overlay);
    assert.equal(h.overlay.dataset.codexPlusSurface, undefined, "a skin-cleared panel never makes the page see-through");
    assert.equal(h.overlay.style.values.get("--codex-plus-shell-surface"), undefined);
    delete h.nodes["[data-app-shell-surface-backing]"];
    h.position(h.overlay);
    assert.equal(h.overlay.style.top, `${40 / zoom}px`, "older shells keep starting below the header");
    assert.equal(h.overlay.dataset.codexPlusTitlebar, undefined);
    assert.equal(h.overlay.style.values.get("--codex-plus-page-radius-end"), undefined);
    assert.equal(h.overlay.dataset.codexPlusSidebar, undefined, "older shells have no floating sidebar");
    assert.equal(h.overlay.style.values.get("--codex-plus-native-peek-width"), undefined);
  }
});

it("floats each rail entry's own sidebar card like the native sidebar peek while the sidebar is collapsed", async () => {
  const listeners = new Map<string, (event: Record<string, unknown>) => void>();
  class Element {}
  const node = <T extends object>(value: T) => Object.setPrototypeOf(value, Element.prototype) as T;
  const cardItem = node({ closest: () => null });
  const cardTransitions: string[] = [];
  const card = {
    contains: (target: unknown) => target === cardItem,
    offsetWidth: 284,
    style: {
      set transition(value: string) { cardTransitions.push(`${value}:${overlay.dataset.codexPlusNavPeek ?? "closed"}`); },
      removeProperty: (name: string) => cardTransitions.push(`-${name}:${overlay.dataset.codexPlusNavPeek ?? "closed"}`),
    },
  };
  const railButton = (id: string, current: boolean) => {
    const button: Record<string, unknown> = node({
      parentElement: { id },
      getAttribute: (name: string) => (name === "aria-current" && current ? "page" : null),
      closest: (selector: string) => (selector === "[data-codex-plus-rail] > button" ? button : null),
    });
    return button;
  };
  const currentEntry = railButton("codex-plus-rail-remote", true);
  const otherEntry = railButton("codex-plus-rail-weixin", false);
  const unsupportedEntry = railButton("codex-plus-rail-provider", false);
  const nativeEntry: Record<string, unknown> = node({ closest: (selector: string) => (selector === "[data-sidebar-destination]" ? nativeEntry : null) });
  const elsewhere = node({ closest: () => null });
  const peekItem = node({ closest: () => null });
  const peek = { dataset: {} as Record<string, string>, contains: (target: unknown) => target === peekItem };
  const shown: string[] = [];
  const snaps: boolean[] = [];
  let nativeOpen = false;
  const overlay = { dataset: {} as Record<string, string>, querySelector: () => card };
  const document = {
    querySelector: (selector: string) => {
      if (selector === '.codex-plus-page-overlay[data-codex-plus-sidebar="floating"]') return overlay;
      if (selector === '[data-sidebar-navigation-root][class*="sidebar-popover-background"]' && nativeOpen) return {};
      return null;
    },
    getElementById: (id: string) => (id === "codex-plus-rail-peek" && peek.dataset.open ? peek : null),
    addEventListener: (type: string, listener: (event: Record<string, unknown>) => void) => listeners.set(type, listener),
    removeEventListener: (type: string) => listeners.delete(type),
  };
  let floating = true;
  const window: Record<string, unknown> = {};
  new Function("document", "window", "Element", "codexPlusSidebarFloating", "showCodexPlusRailPeek", "hideCodexPlusRailPeek", `
    const codexPlusPageClass = "codex-plus-page-overlay";
    const codexPlusRailPeekId = "codex-plus-rail-peek";
    const codexPlusRailDestinationSelector = "[data-sidebar-destination]";
    const codexPlusBackendGeneration = 1;
    ${functionSource("installCodexPlusNavPeek")}
    installCodexPlusNavPeek();
    installCodexPlusNavPeek();
  `)(
    document,
    window,
    Element,
    () => floating,
    (id: string, _load: boolean, snap: boolean) => {
      if (id === "codex-plus-rail-provider") return false;
      shown.push(id);
      snaps.push(snap);
      peek.dataset.open = "true";
      return true;
    },
    () => { if (peek.dataset.open) peek.dataset.open = "false"; },
  );
  const move = (target: unknown, clientX = 300) => listeners.get("pointermove")!({ type: "pointermove", target, clientX });
  const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
  assert.equal(listeners.size, 2, "installing again in the same generation keeps the one handler");
  move(currentEntry, 26);
  assert.equal(overlay.dataset.codexPlusNavPeek, undefined, "a short delay before it slides out, like the native peek");
  await wait(160);
  assert.equal(overlay.dataset.codexPlusNavPeek, "true", "hovering the current page's rail entry shows the page's own left column");
  assert.deepEqual(cardTransitions, [], "showing from nothing keeps the slide-and-fade transition");
  move(cardItem, 150);
  await wait(300);
  assert.equal(overlay.dataset.codexPlusNavPeek, "true", "the card stays while the pointer is on it");
  move(otherEntry, 26);
  await wait(20);
  assert.deepEqual(shown, ["codex-plus-rail-weixin"], "another entry swaps in its own card at once, like the native rail");
  assert.equal(peek.dataset.open, "true");
  assert.equal(overlay.dataset.codexPlusNavPeek, undefined, "the page's column steps aside");
  assert.deepEqual(snaps, [true], "swapping from one floating card to another is instant, like the native rail");
  assert.deepEqual(cardTransitions, ["none:true", "-transition:closed"], "the page's column leaves without its fade when another card takes its place");
  move(peekItem, 150);
  await wait(300);
  assert.equal(peek.dataset.open, "true", "the entry's card stays while the pointer is on it");
  move(nativeEntry, 26);
  await wait(20);
  assert.equal(peek.dataset.open, "false", "a native rail entry gets the floor at once (its own card comes up)");
  move(unsupportedEntry, 26);
  await wait(160);
  assert.equal(peek.dataset.open, "false", "entries without a card show nothing");
  move(elsewhere, 2);
  await wait(160);
  assert.equal(overlay.dataset.codexPlusNavPeek, "true", "the window's left edge shows the page's column");
  listeners.get("pointerout")!({ type: "pointerout", target: elsewhere, relatedTarget: null, clientX: 2 });
  await wait(300);
  assert.equal(overlay.dataset.codexPlusNavPeek, undefined, "leaving the window hides it");
  move(otherEntry, 26);
  await wait(160);
  assert.deepEqual(snaps.slice(1), [false], "from nothing the entry's card fades and slides in");
  move(elsewhere, 300);
  await wait(300);
  nativeOpen = true;
  move(otherEntry, 26);
  await wait(20);
  assert.deepEqual(snaps.slice(2), [true], "coming from the native card it swaps in at once, as native entries do");
  nativeOpen = false;
  move(elsewhere, 300);
  await wait(300);
  floating = false;
  move(otherEntry, 26);
  await wait(160);
  assert.equal(shown.length, 3, "with the sidebar open, hovering the rail floats nothing, like native");
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
  // Chromium 把写出来的 none 算成 no-drag（会盖掉标题栏拖动区），必须用 initial。
  assert.match(existing!.textContent!, /\[data-codex-plus-titlebar="inset"\] > \.codex-plus-modal-content \{\s*-webkit-app-region: initial;/);
  assert.doesNotMatch(existing!.textContent!, /-webkit-app-region: none/);
  // 侧栏展开：有左栏的页面整块透明、左栏对齐原生侧栏，原生会话列表与页面内容藏起来，标题栏照常显示。
  assert.match(existing!.textContent!, /\[data-codex-plus-sidebar="open"\]:has\(:is\(\.codex-plus-page-nav, \.codex-plus-provider-sidebar, \.codex-plus-task-sidebar\)\) \{\s*border-radius: 0 !important;\s*background: transparent !important;/);
  assert.match(existing!.textContent!, /\[data-codex-plus-sidebar="open"\] :is\(\.codex-plus-page-nav, \.codex-plus-provider-sidebar, \.codex-plus-task-sidebar\) \{[^}]*width: var\(--codex-plus-native-sidebar-width\) !important;/);
  assert.match(existing!.textContent!, /html:has\([^)]*\[data-codex-plus-sidebar="open"\]\) :is\(\[data-sidebar-navigation-root\], \[data-app-shell-active-page\]\) \{\s*visibility: hidden !important;/);
  assert.match(existing!.textContent!, /html:has\([^)]*\[data-codex-plus-sidebar="open"\]\) \[data-app-shell-titlebar\] \{\s*visibility: visible !important;/);
  // 侧栏收起：左栏是悬浮卡片（位置、圆角、底色照原生悬浮侧栏），平时藏着，data-codex-plus-nav-peek 时浮出。
  const floating = existing!.textContent!.match(/\[data-codex-plus-sidebar="floating"\] \.codex-plus-modal-content > \.codex-plus-page-layout > \.codex-plus-page-nav \{([^}]+)\}/)![1];
  assert.match(floating, /position: absolute;/);
  assert.match(floating, /top: calc\(var\(--codex-plus-page-inset-top, 0px\) \+ 4px\);\s*bottom: 4px;\s*left: 4px;/);
  assert.match(floating, /width: var\(--codex-plus-native-peek-width, 284px\) !important;/);
  assert.match(floating, /border-radius: var\(--app-shell-sidebar-popover-radius, 12px\);/);
  assert.match(floating, /background: var\(--app-shell-sidebar-popover-background, var\(--codex-plus-nav-bg\)\) !important;/);
  assert.match(floating, /visibility: hidden;\s*pointer-events: none;/);
  assert.match(existing!.textContent!, /\[data-codex-plus-sidebar="floating"\]\[data-codex-plus-nav-peek="true"\] \.codex-plus-modal-content > \.codex-plus-page-layout > \.codex-plus-page-nav \{\s*opacity: 1;\s*transform: none;\s*visibility: visible;\s*pointer-events: auto;/);
  // 原生自己的悬浮侧栏浮在页面之上。
  assert.match(existing!.textContent!, /html\[data-codex-plus-page-open\]:has\(\[data-sidebar-navigation-root\]\[class\*="sidebar-popover-background"\]\) aside\.app-shell-left-panel \{\s*z-index: 2147482002 !important;/);
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
