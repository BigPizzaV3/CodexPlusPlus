import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/96-custom-layout.js", import.meta.url), "utf8");
const physicsSource = readFileSync(new URL("../../../assets/inject/renderer-inject/96-layout-physics.js", import.meta.url), "utf8");
const conversationSource = readFileSync(new URL("../../../assets/inject/renderer-inject/90-action-groups.js", import.meta.url), "utf8");
const conversationOriginalsStart = conversationSource.indexOf("  function conversationViewRememberOriginals(");
const conversationOriginalsEnd = conversationSource.indexOf("  function conversationViewResetOwnOffset(", conversationOriginalsStart);
const conversationCleanupStart = conversationSource.indexOf("  function cleanupConversationView(");
const conversationCleanupEnd = conversationSource.indexOf("  window.__codexPlusConversationViewCleanup", conversationCleanupStart);
assert.ok(conversationOriginalsStart >= 0 && conversationOriginalsEnd > conversationOriginalsStart && conversationCleanupStart >= 0 && conversationCleanupEnd > conversationCleanupStart);
const conversationFunctions = `${conversationSource.slice(conversationOriginalsStart, conversationOriginalsEnd)}\n${conversationSource.slice(conversationCleanupStart, conversationCleanupEnd)}`;
const storageKey = "codexPlus.customLayout.v1";
type Rect = { left: number; top: number; width: number; height: number };
type Listener = (event: Record<string, unknown>) => void;
type RecordPosition = { x: number; y: number; width?: number; height?: number; snapX?: string | null; snapY?: string | null; grouped?: boolean };
type Geometry = {
  normalizeLayout(value: unknown): { version: number; panels: Record<string, RecordPosition> };
  clampRect(rect: Rect, bounds: Rect, minWidth?: number, minHeight?: number): Rect;
  snapRect(rect: Rect, bounds: Rect, others?: Rect[], threshold?: number): { rect: Rect; snapX: string | null; snapY: string | null };
  rectToRecord(rect: Rect, bounds: Rect, snaps?: { snapX?: string | null; snapY?: string | null }): RecordPosition;
  recordToRect(record: RecordPosition, nativeRect: Rect, bounds: Rect): Rect;
};

function styleDeclaration() {
  const properties = new Map<string, { value: string; priority: string }>();
  const name = (key: string) => key.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
  // CSSStyleDeclaration 会把 38.400px 读回成 38.4px；样式接管必须比较浏览器读回值。
  const canonical = (value: string) => /^[-+]?(?:\d+\.?\d*|\.\d+)px$/.test(value) ? `${Number.parseFloat(value)}px` : value;
  const methods = {
    setProperty(key: string, value: string, priority = "") { properties.set(key, { value: canonical(String(value)), priority }); },
    getPropertyValue(key: string) { return properties.get(key)?.value || ""; },
    getPropertyPriority(key: string) { return properties.get(key)?.priority || ""; },
    removeProperty(key: string) { const old = properties.get(key)?.value || ""; properties.delete(key); return old; },
  };
  return new Proxy(methods, {
    get(target, key) {
      if (key in target) return target[key as keyof typeof target];
      if (key === "cssText") return [...properties].map(([key, entry]) => `${key}: ${entry.value}${entry.priority ? " !important" : ""};`).join(" ");
      return typeof key === "string" ? properties.get(name(key))?.value || "" : undefined;
    },
    set(_target, key, value) {
      if (key === "cssText") {
        properties.clear();
        for (const entry of String(value).split(";")) {
          const colon = entry.indexOf(":");
          if (colon <= 0) continue;
          const raw = entry.slice(colon + 1).trim();
          properties.set(entry.slice(0, colon).trim(), { value: canonical(raw.replace(/\s*!important$/, "")), priority: /!important$/.test(raw) ? "important" : "" });
        }
      } else if (typeof key === "string") properties.set(name(key), { value: canonical(String(value)), priority: "" });
      return true;
    },
  }) as unknown as CSSStyleDeclaration;
}

function fixture(enabled = false, initialStorage?: string, backendLoaded = true, options: { splitShells?: boolean; zoom?: number } = {}) {
  const documentListeners = new Map<string, Set<Listener>>();
  const windowListeners = new Map<string, Set<Listener>>();
  const frames = new Map<number, (time: number) => void>();
  const timers = new Map<number, () => void>();
  const mediaListeners = new Set<Listener>();
  const media = {
    matches: false,
    addEventListener: (_name: string, listener: Listener) => mediaListeners.add(listener),
    removeEventListener: (_name: string, listener: Listener) => mediaListeners.delete(listener),
  };
  const observers: FakeObserver[] = [];
  const created: FakeElement[] = [];
  const stored = new Map<string, string>();
  const writes: Array<[string, string]> = [];
  const settings = { customLayout: enabled, conversationView: false };
  const backend = { loaded: backendLoaded };
  if (initialStorage !== undefined) stored.set(storageKey, initialStorage);
  let nextId = 0;
  let now = 0;
  const add = (map: Map<string, Set<Listener>>, event: string, listener: Listener) => {
    if (!map.has(event)) map.set(event, new Set());
    map.get(event)!.add(listener);
  };
  const remove = (map: Map<string, Set<Listener>>, event: string, listener: Listener) => map.get(event)?.delete(listener);

  class FakeElement {
    tagName: string;
    nodeType = 1;
    attrs: Record<string, string> = {};
    style = styleDeclaration();
    children: FakeElement[] = [];
    parentElement: FakeElement | null = null;
    listeners = new Map<string, Set<Listener>>();
    nativeRect: Rect;
    textContent = "";
    disabled = false;
    title = "";
    type = "";
    tabIndex = 0;
    constructor(tag: string, rect: Rect = { left: 0, top: 0, width: 0, height: 0 }) {
      this.tagName = tag.toUpperCase(); this.nativeRect = rect;
    }
    get isConnected(): boolean { return this === documentElement || !!this.parentElement?.isConnected; }
    get parentNode() { return this.parentElement; }
    get childNodes() { return this.children; }
    get id() { return this.attrs.id || ""; }
    set id(value: string) { this.attrs.id = value; }
    get className() { return this.attrs.class || ""; }
    set className(value: string) { this.attrs.class = value; }
    get dataset() {
      const attribute = (key: string) => `data-${key.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}`;
      return new Proxy({}, {
        get: (_target, key) => typeof key === "string" ? this.attrs[attribute(key)] : undefined,
        set: (_target, key, value) => { this.setAttribute(attribute(String(key)), String(value)); return true; },
        has: (_target, key) => typeof key === "string" && attribute(key) in this.attrs,
        deleteProperty: (_target, key) => { delete this.attrs[attribute(String(key))]; return true; },
      });
    }
    get classList() {
      const set = () => new Set(this.className.split(/\s+/).filter(Boolean));
      return {
        contains: (token: string) => set().has(token),
        add: (...tokens: string[]) => { const values = set(); tokens.forEach((token) => values.add(token)); this.className = [...values].join(" "); },
        remove: (...tokens: string[]) => { const values = set(); tokens.forEach((token) => values.delete(token)); this.className = [...values].join(" "); },
        toggle: (token: string, force?: boolean) => { const values = set(); const on = force ?? !values.has(token); if (on) values.add(token); else values.delete(token); this.className = [...values].join(" "); return on; },
      };
    }
    get offsetWidth() { return this.getBoundingClientRect().width; }
    get offsetHeight() { return this.getBoundingClientRect().height; }
    get clientWidth() { return this.offsetWidth; }
    get clientHeight() { return this.offsetHeight; }
    setAttribute(key: string, value: string) { if (key === "style") this.style.cssText = value; else this.attrs[key] = String(value); }
    getAttribute(key: string) { return key === "style" ? this.style.cssText || null : this.attrs[key] ?? null; }
    hasAttribute(key: string) { return this.getAttribute(key) !== null; }
    removeAttribute(key: string) { if (key === "style") this.style.cssText = ""; else delete this.attrs[key]; }
    appendChild(child: FakeElement) { child.remove(); child.parentElement = this; this.children.push(child); return child; }
    append(...children: FakeElement[]) { children.forEach((child) => this.appendChild(child)); }
    replaceChildren(...children: FakeElement[]) { for (const child of [...this.children]) child.remove(); this.append(...children); }
    remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter((child) => child !== this); this.parentElement = null; }
    contains(node: unknown): boolean { for (let next = node as FakeElement | null; next; next = next.parentElement) if (next === this) return true; return false; }
    matches(selector: string): boolean {
      return selector.split(",").some((part) => {
        let item = part.trim();
        if (!item || item.includes(" ") || item.includes(">")) return false;
        if (item === ":disabled") return this.disabled;
        const attrs = [...item.matchAll(/\[([^=\]~]+)(?:=["']?([^\]"']+)["']?)?\]/g)];
        if (!attrs.every((match) => match[1] in this.attrs && (match[2] === undefined || this.attrs[match[1]] === match[2]))) return false;
        item = item.replace(/\[[^\]]+\]/g, "");
        const id = item.match(/#([\w-]+)/);
        if (id && this.id !== id[1]) return false;
        if (![...item.matchAll(/\.([\w-]+)/g)].every((match) => this.classList.contains(match[1]))) return false;
        const tag = item.match(/^[\w-]+/);
        return !tag || this.tagName === tag[0].toUpperCase();
      });
    }
    closest(selector: string): FakeElement | null { for (let node: FakeElement | null = this; node; node = node.parentElement) if (node.matches(selector)) return node; return null; }
    querySelectorAll(selector: string): FakeElement[] {
      const results: FakeElement[] = [];
      const walk = (node: FakeElement) => {
        for (const child of node.children) {
          const matches = selector.split(",").some((part) => {
            const tokens = part.trim().split(/\s+/);
            const last = tokens.pop()!;
            if (!child.matches(last)) return false;
            let ancestor = child.parentElement;
            while (tokens.length) {
              const next = tokens.pop()!;
              if (next === ">") continue;
              while (ancestor && !ancestor.matches(next)) ancestor = ancestor.parentElement;
              if (!ancestor) return false;
              ancestor = ancestor.parentElement;
            }
            return true;
          });
          if (matches) results.push(child);
          walk(child);
        }
      };
      walk(this); return results;
    }
    querySelector(selector: string) { return this.querySelectorAll(selector)[0] || null; }
    getBoundingClientRect() {
      let zoom = 1;
      for (let element: FakeElement | null = this; element; element = element.parentElement) {
        const raw = element.style.zoom;
        const value = Number.parseFloat(raw);
        if (Number.isFinite(value) && value > 0) zoom *= raw.endsWith("%") ? value / 100 : value;
      }
      const pixel = (value: string, fallback: number) => /px$/.test(value) ? parseFloat(value) * zoom : fallback;
      const rect = { ...this.nativeRect };
      rect.width = pixel(this.style.width, pixel(this.style.flexBasis, rect.width)); rect.height = pixel(this.style.height, rect.height);
      if (this.style.position === "fixed") {
        rect.left = pixel(this.style.left, rect.left); rect.top = pixel(this.style.top, rect.top);
      } else if (this.parentElement) {
        const parent = this.parentElement;
        const current = parent.getBoundingClientRect();
        rect.left += current.left - parent.nativeRect.left;
        rect.top += current.top - parent.nativeRect.top;
        // 模拟原生 flex 的尾边填充：rail 保持宽度，sidebar 内容跟随外壳剩余宽度。
        if (!this.style.width && !/px$/.test(this.style.flexBasis) && Math.abs(this.nativeRect.left + this.nativeRect.width - parent.nativeRect.left - parent.nativeRect.width) < .001) {
          rect.width += current.width - parent.nativeRect.width;
        }
        if (!this.style.height && Math.abs(this.nativeRect.top + this.nativeRect.height - parent.nativeRect.top - parent.nativeRect.height) < .001) {
          rect.height += current.height - parent.nativeRect.height;
        }
      }
      rect.width = Math.min(rect.width, pixel(this.style.maxWidth, Infinity));
      rect.height = Math.min(rect.height, pixel(this.style.maxHeight, Infinity));
      return { ...rect, right: rect.left + rect.width, bottom: rect.top + rect.height, x: rect.left, y: rect.top };
    }
    addEventListener(name: string, listener: Listener) { add(this.listeners, name, listener); }
    removeEventListener(name: string, listener: Listener) { remove(this.listeners, name, listener); }
    setPointerCapture() {}
    releasePointerCapture() {}
    focus() { document.activeElement = this; }
  }
  class FakeObserver {
    connected = false;
    callback: () => void;
    constructor(callback: () => void) { this.callback = callback; observers.push(this); }
    observe() { this.connected = true; }
    disconnect() { this.connected = false; }
  }
  const documentElement = new FakeElement("html", { left: 0, top: 0, width: 1200, height: 800 });
  const body = documentElement.appendChild(new FakeElement("body", documentElement.nativeRect));
  const appRoot = body.appendChild(new FakeElement("main", body.nativeRect));
  appRoot.style.backgroundColor = "rgb(24, 24, 24)";
  if (options.zoom) appRoot.style.zoom = String(options.zoom);
  const nativeSidebarSurface = appRoot.appendChild(new FakeElement("aside", {
    left: options.splitShells ? 54 : 0, top: 48, width: options.splitShells ? 216 : 270, height: 752,
  }));
  nativeSidebarSurface.className = "app-shell-left-panel";
  nativeSidebarSurface.style.backgroundColor = "rgb(32, 32, 32)";
  nativeSidebarSurface.style.borderRightWidth = "1px";
  nativeSidebarSurface.style.borderRightStyle = "solid";
  nativeSidebarSurface.style.overflow = "hidden";
  const nativeRailSurface = options.splitShells
    ? appRoot.appendChild(new FakeElement("aside", { left: 0, top: 48, width: 54, height: 752 }))
    : nativeSidebarSurface;
  if (options.splitShells) {
    nativeRailSurface.className = "app-shell-navigation-rail";
    nativeRailSurface.style.backgroundColor = "rgb(40, 40, 40)";
    nativeRailSurface.style.borderRightWidth = "1px";
    nativeRailSurface.style.borderRightStyle = "solid";
  }
  const rail = nativeRailSurface.appendChild(new FakeElement("nav", { left: 0, top: 48, width: 54, height: 752 }));
  rail.setAttribute("data-app-navigation-rail", "");
  const sidebarShell = nativeSidebarSurface.appendChild(new FakeElement("div", { left: 54, top: 48, width: 216, height: 752 }));
  sidebarShell.id = "app-shell-sidebar";
  const sidebar = sidebarShell.appendChild(new FakeElement("div", sidebarShell.nativeRect));
  sidebar.className = "sidebar-navigation";
  const summary = appRoot.appendChild(new FakeElement("aside", { left: 980, top: 65, width: 210, height: 260 }));
  summary.setAttribute("data-summary-panel-variant", "floating");
  const composer = appRoot.appendChild(new FakeElement("div", { left: 340, top: 650, width: 620, height: 128 }));
  composer.setAttribute("data-codex-composer-root", "");
  const editor = composer.appendChild(new FakeElement("textarea"));
  const native = { rail, sidebar, summary, composer };
  const conversationState = {
    elements: new Set<FakeElement>(), rafId: 0, pollId: 0, mo: null, ro: null,
    moObserved: false, runtimeStarted: false, observed: new WeakSet<FakeElement>(), contentEl: null, composerEl: null,
  };
  const document = {
    documentElement, body, head: body, activeElement: editor,
    createElement(tag: string) { const element = new FakeElement(tag); created.push(element); return element; },
    querySelector: (selector: string) => documentElement.querySelector(selector),
    querySelectorAll: (selector: string) => documentElement.querySelectorAll(selector),
    getElementById: (id: string) => documentElement.querySelector(`#${id}`),
    addEventListener: (name: string, listener: Listener) => add(documentListeners, name, listener),
    removeEventListener: (name: string, listener: Listener) => remove(documentListeners, name, listener),
  };
  const window = {
    innerWidth: 1200, innerHeight: 800,
    matchMedia: () => media,
    __CODEX_PLUS_TEST_CUSTOM_LAYOUT__: {} as Geometry,
    testInstall: undefined as (() => void) | undefined,
    testLayout: undefined as (() => { version: number; panels: Record<string, RecordPosition> }) | undefined,
    testCenter: undefined as undefined | { remember(element: FakeElement): void; cleanup(): void },
    __codexPlusCustomLayoutRuntime: undefined as undefined | {
      setEditing(value: boolean): void; toggleEditing(): void; reset(): void; refresh(): void; cleanup(): void;
    },
    addEventListener: (name: string, listener: Listener) => add(windowListeners, name, listener),
    removeEventListener: (name: string, listener: Listener) => remove(windowListeners, name, listener),
  };
  const storage = {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => { stored.set(key, value); writes.push([key, value]); },
    removeItem: (key: string) => stored.delete(key),
  };
  const context = createContext({
    window, document, localStorage: storage, Element: FakeElement, HTMLElement: FakeElement,
    performance: { now: () => now },
    MutationObserver: FakeObserver, ResizeObserver: FakeObserver,
    codexPlusSettings: () => settings,
    get codexPlusBackendSettingsLoaded() { return backend.loaded; },
    conversationViewState: conversationState,
    refreshConversationView: () => {},
    registerCodexPlusExtensionSelector: (selector: string) => { assert.ok(selector.includes("data-codex-plus-ext")); return true; },
    isExtensionUiNode: (element: FakeElement) => !!element.closest("[data-codex-plus-ext]"),
    visibleElement: (element: FakeElement) => element.isConnected && element.getBoundingClientRect().width > 0 && element.getBoundingClientRect().height > 0,
    conversationViewFindComposerEl: () => composer.isConnected ? composer : null,
    getComputedStyle: (element: FakeElement) => ({
      position: element.style.position || "static", display: "block", visibility: "visible", transform: "none", zoom: element.style.zoom || "1",
      overflow: element.style.overflow || "visible", overflowX: element.style.overflowX || element.style.overflow || "visible", overflowY: element.style.overflowY || element.style.overflow || "visible",
      backgroundColor: element.style.backgroundColor || "rgba(0, 0, 0, 0)", backgroundImage: "none",
      borderLeftWidth: element.style.borderLeftWidth || "0px", borderRightWidth: element.style.borderRightWidth || "0px",
      borderTopWidth: element.style.borderTopWidth || "0px", borderBottomWidth: element.style.borderBottomWidth || "0px",
      getPropertyValue: (key: string) => element.style.getPropertyValue(key),
    }),
    requestAnimationFrame: (callback: (time: number) => void) => { frames.set(++nextId, callback); return nextId; },
    cancelAnimationFrame: (id: number) => frames.delete(id),
    setTimeout: (callback: () => void) => { timers.set(++nextId, callback); return nextId; },
    clearTimeout: (id: number) => timers.delete(id),
    setInterval: (callback: () => void) => { timers.set(++nextId, callback); return nextId; },
    clearInterval: (id: number) => timers.delete(id),
    queueMicrotask: (callback: () => void) => callback(),
  });
  const load = () => runInContext(`(function() { ${conversationFunctions}\n${physicsSource}\n${source}\nwindow.testCenter = { remember: conversationViewRememberOriginals, cleanup: cleanupConversationView }; window.testLayout = () => JSON.parse(JSON.stringify(codexPlusCustomLayoutState.layout)); window.testInstall = installCodexPlusCustomLayout; installCodexPlusCustomLayout(); })();`, context);
  load();
  const runtime = () => { assert.ok(window.__codexPlusCustomLayoutRuntime); return window.__codexPlusCustomLayoutRuntime!; };
  const dispatch = (name: string, target: FakeElement, extra: Record<string, unknown> = {}) => {
    const event: Record<string, unknown> = { type: name, target, currentTarget: target, button: 0, pointerId: 1, isPrimary: true, clientX: 0, clientY: 0, preventDefault() {}, stopPropagation() {}, ...extra };
    for (let node: FakeElement | null = target; node; node = node.parentElement) {
      event.currentTarget = node;
      for (const listener of node.listeners.get(name) || []) listener(event);
    }
    event.currentTarget = document;
    for (const listener of documentListeners.get(name) || []) listener(event);
    event.currentTarget = window;
    for (const listener of windowListeners.get(name) || []) listener(event);
  };
  const frame = (milliseconds = 1000 / 60) => {
    now += milliseconds;
    const pendingFrames = [...frames.values()]; frames.clear(); pendingFrames.forEach((callback) => callback(now));
  };
  const flush = () => {
    for (let pass = 0; pass < 240 && (frames.size || timers.size); pass++) {
      frame();
      const pendingTimers = [...timers.values()]; timers.clear(); pendingTimers.forEach((callback) => callback());
    }
  };
  const reduced = (value: boolean) => { media.matches = value; for (const listener of mediaListeners) listener({ matches: value }); };
  return { geometry: window.__CODEX_PLUS_TEST_CUSTOM_LAYOUT__, native, nativeSidebarSurface, nativeRailSurface, sidebarShell, appRoot, body, document, window, settings, backend, conversationState, stored, writes, created, documentListeners, windowListeners, observers, frames, timers, runtime, dispatch, frame, flush, reduced, mediaListeners, load, install: () => window.testInstall!() };
}

const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const bounds = { left: 8, top: 52, width: 1184, height: 740 };

test("layout persistence accepts only finite records for known panels and the supported schema", () => {
  const { geometry } = fixture();
  for (const input of [null, "wrong", [], { version: 2, panels: { rail: { x: .5, y: .5 } } }]) {
    assert.deepEqual(plain(geometry.normalizeLayout(input)), { version: 1, panels: {} });
  }
  const layout = geometry.normalizeLayout({
    version: 1, panels: {
      rail: { x: -100, y: 100, width: 54, height: 700, snapX: "left", snapY: "bottom" },
      sidebar: { x: Infinity, y: .5 }, summary: { x: .5, y: NaN },
      composer: { x: .6, y: .7, width: -20, height: "128", snapX: "arbitrary", snapY: "left" },
      unknown: { x: 0, y: 0 },
    },
  });
  assert.deepEqual(Object.keys(layout.panels).sort(), ["composer", "rail"]);
  assert.equal(layout.panels.rail.x, 0); assert.equal(layout.panels.rail.y, 1);
  assert.equal(layout.panels.rail.snapX, "left"); assert.equal(layout.panels.rail.snapY, "bottom");
  assert.equal(layout.panels.composer.width, undefined); assert.equal(layout.panels.composer.height, undefined);
  assert.equal(layout.panels.composer.snapX, null); assert.equal(layout.panels.composer.snapY, null);
});

test("saved relative coordinates restore proportionally and edge docking wins after a window resize", () => {
  const { geometry } = fixture();
  const original = { left: 450, top: 282, width: 300, height: 100 };
  const record = geometry.rectToRecord(original, bounds);
  assert.deepEqual(plain(geometry.recordToRect(record, original, bounds)), original);
  const smaller = { left: 8, top: 52, width: 784, height: 540 };
  const restored = geometry.recordToRect(record, original, smaller);
  assert.ok(Math.abs(restored.left - (8 + record.x * (784 - 300))) < .001);
  assert.ok(Math.abs(restored.top - (52 + record.y * (540 - 100))) < .001);
  const docked = geometry.recordToRect({ ...record, snapX: "right", snapY: "bottom" }, original, smaller);
  assert.equal(docked.left + docked.width, smaller.left + smaller.width);
  assert.equal(docked.top + docked.height, smaller.top + smaller.height);
});

test("bounds correction keeps all panel edges reachable even when the window becomes smaller than the panel", () => {
  const { geometry } = fixture();
  const smaller = { left: 8, top: 52, width: 180, height: 120 };
  const corrected = geometry.clampRect({ left: 3000, top: -900, width: 650, height: 700 }, smaller);
  assert.ok(corrected.width > 0 && corrected.width <= smaller.width);
  assert.ok(corrected.height > 0 && corrected.height <= smaller.height);
  assert.ok(corrected.left >= smaller.left && corrected.left + corrected.width <= smaller.left + smaller.width);
  assert.ok(corrected.top >= smaller.top && corrected.top + corrected.height <= smaller.top + smaller.height);
  const docked = geometry.recordToRect({ x: 1, y: 1, width: 900, height: 800, snapX: "right", snapY: "bottom" }, corrected, smaller);
  assert.equal(docked.left, smaller.left); assert.equal(docked.top, smaller.top);
});

test("magnetic placement supports every window edge and aligns with another panel", () => {
  const { geometry } = fixture();
  for (const [rect, axis, expected] of [
    [{ left: 13, top: 200, width: 200, height: 100 }, "snapX", "left"],
    [{ left: 987, top: 200, width: 200, height: 100 }, "snapX", "right"],
    [{ left: 300, top: 59, width: 200, height: 100 }, "snapY", "top"],
    [{ left: 300, top: 685, width: 200, height: 100 }, "snapY", "bottom"],
  ] as const) {
    const snapped = geometry.snapRect(rect, bounds);
    assert.equal(snapped[axis], expected);
    if (expected === "left") assert.equal(snapped.rect.left, bounds.left);
    if (expected === "right") assert.equal(snapped.rect.left + snapped.rect.width, bounds.left + bounds.width);
    if (expected === "top") assert.equal(snapped.rect.top, bounds.top);
    if (expected === "bottom") assert.equal(snapped.rect.top + snapped.rect.height, bounds.top + bounds.height);
  }
  const beside = geometry.snapRect({ left: 400, top: 300, width: 200, height: 100 }, bounds, [{ left: 610, top: 300, width: 220, height: 260 }]);
  assert.equal(beside.rect.left + beside.rect.width, 610);
  assert.equal(beside.snapX, null, "alignment with a panel must not become window-edge docking");
  const far = { left: 400, top: 300, width: 200, height: 100 };
  assert.deepEqual(plain(geometry.snapRect(far, bounds, [], 16).rect), far);
});

test("custom layout is opt-in and keeps the native React nodes in their original parents", () => {
  const f = fixture(false);
  const parents = Object.values(f.native).map((element) => element.parentElement);
  for (const element of Object.values(f.native)) {
    assert.equal(element.getAttribute("style"), null);
    assert.equal(element.getAttribute("data-codex-plus-layout-panel"), null);
  }
  assert.equal(f.created.filter((element) => element.isConnected).length, 0);
  assert.equal(f.frames.size, 0); assert.equal(f.timers.size, 0);
  f.settings.customLayout = true;
  f.install(); f.runtime().setEditing(true); f.flush();
  assert.deepEqual(Object.values(f.native).map((element) => element.parentElement), parents);
  assert.equal(f.document.activeElement, f.native.composer.children[0], "layout editing preserves the editor and its focus");
  const decorations = f.created.filter((element) => element.isConnected);
  assert.ok(decorations.length > 0);
  for (const decoration of decorations) assert.ok(decoration.closest("[data-codex-plus-ext]"), "inserted nodes are excluded from extension mutation scans");
});

type Fixture = ReturnType<typeof fixture>;
function moveGrip(f: Fixture, label = "输入框") {
  const grip = f.document.querySelectorAll(".codex-plus-custom-layout-grip")
    .find((element) => element.getAttribute("aria-label")?.startsWith(`拖动${label}`));
  assert.ok(grip); return grip!;
}
function dragComposer(f: Fixture, target: { left: number; top: number }, commit = true) {
  const grip = moveGrip(f);
  const origin = f.native.composer.getBoundingClientRect();
  f.dispatch("pointerdown", grip, { clientX: 100, clientY: 100 });
  f.dispatch("pointermove", grip, { clientX: 100 + target.left - origin.left, clientY: 100 + target.top - origin.top });
  f.dispatch(commit ? "pointerup" : "pointercancel", grip);
  f.flush();
}

function dragSidebar(f: Fixture, target: { left: number; top: number }, commit = true) {
  const grip = moveGrip(f, "侧边栏");
  const origin = f.nativeRailSurface.getBoundingClientRect();
  f.dispatch("pointerdown", grip, { clientX: 100, clientY: 100 });
  f.dispatch("pointermove", grip, { clientX: 100 + target.left - origin.left, clientY: 100 + target.top - origin.top, altKey: true });
  f.dispatch(commit ? "pointerup" : "pointercancel", grip);
  f.flush();
}

function sidebarNodes(f: Fixture) {
  return [...new Set([f.nativeSidebarSurface, f.nativeRailSurface, f.sidebarShell, f.native.rail, f.native.sidebar])];
}

for (const splitShells of [false, true]) {
  const variant = splitShells ? "split painted shells" : "a common painted aside";
  test(`sidebar dragging moves the rail, content and background together for ${variant}`, () => {
    const f = fixture(true, undefined, true, { splitShells });
    f.reduced(true); f.runtime().setEditing(true); f.flush();
    const nodes = sidebarNodes(f);
    const parents = nodes.map((node) => node.parentElement);
    const before = nodes.map((node) => node.getBoundingClientRect());
    const grips = f.document.querySelectorAll(".codex-plus-custom-layout-grip");
    assert.equal(grips.filter((grip) => grip.getAttribute("aria-label")?.startsWith("拖动侧边栏")).length, 1);
    assert.equal(grips.filter((grip) => grip.getAttribute("aria-label")?.startsWith("拖动图标栏")).length, 0);
    dragSidebar(f, { left: 400, top: 20 });
    const after = nodes.map((node) => node.getBoundingClientRect());
    const delta = { x: after[0].left - before[0].left, y: after[0].top - before[0].top };
    for (let index = 0; index < nodes.length; index++) {
      assert.ok(Math.abs(after[index].left - before[index].left - delta.x) < .01, "all painted shells and their contents share one horizontal movement");
      assert.ok(Math.abs(after[index].top - before[index].top - delta.y) < .01, "all painted shells and their contents share one vertical movement");
      assert.equal(nodes[index].parentElement, parents[index], "React's parent relationships remain intact");
    }
    assert.equal(f.nativeRailSurface.getBoundingClientRect().left, 400);
    assert.equal(f.nativeRailSurface.getBoundingClientRect().top, 20);
    assert.equal(f.nativeSidebarSurface.style.position, "fixed");
    assert.equal(f.nativeRailSurface.style.position, "fixed");
    assert.equal(f.nativeSidebarSurface.style.backgroundColor, "rgb(32, 32, 32)");
    assert.equal(f.nativeSidebarSurface.style.borderRightWidth, "1px");
    assert.equal(f.native.rail.style.position, "", "the rail content stays in its painted shell");
    assert.equal(f.native.sidebar.style.position, "", "the chat content stays in its painted shell");
    assert.equal(f.sidebarShell.style.position, "", "no empty original shell is left behind a separately floated child");
    assert.equal(f.appRoot.style.position, "", "the application root is not mistaken for the shared sidebar surface");
    const saved = JSON.parse(f.stored.get(storageKey)!);
    assert.equal(saved.panels.sidebar.grouped, true);
    assert.equal(saved.panels.sidebar.width, 270);
    assert.equal(saved.panels.rail, undefined, "rail and sidebar participate as one solver and persistence entry");
  });

  test(`cancel, reset and disable restore every sidebar part for ${variant}`, () => {
    const f = fixture(false, undefined, true, { splitShells });
    const nodes = sidebarNodes(f);
    const beforeStyles = nodes.map((node) => node.style.cssText);
    const beforeRects = nodes.map((node) => node.getBoundingClientRect());
    f.settings.customLayout = true; f.install(); f.reduced(true); f.runtime().setEditing(true); f.flush();
    dragSidebar(f, { left: 400, top: 20 }, false);
    assert.deepEqual(nodes.map((node) => node.getBoundingClientRect()), beforeRects);
    assert.deepEqual(nodes.map((node) => node.style.cssText), beforeStyles);
    assert.equal(f.writes.length, 0);
    dragSidebar(f, { left: 400, top: 20 });
    f.runtime().reset(); f.flush();
    assert.deepEqual(nodes.map((node) => node.getBoundingClientRect()), beforeRects);
    assert.deepEqual(nodes.map((node) => node.style.cssText), beforeStyles);
    assert.deepEqual(JSON.parse(f.stored.get(storageKey)!), { version: 1, panels: {} });
    dragSidebar(f, { left: 400, top: 20 });
    f.nativeSidebarSurface.style.backgroundColor = "rgb(45, 45, 45)";
    f.settings.customLayout = false; f.install();
    assert.deepEqual(nodes.map((node) => node.getBoundingClientRect()), beforeRects);
    assert.equal(f.nativeSidebarSurface.style.backgroundColor, "rgb(45, 45, 45)", "unrelated native paint updates survive cleanup");
    for (const node of nodes) assert.equal(node.style.position, "");
    assert.equal(f.nativeSidebarSurface.style.borderRightWidth, "1px");
    assert.equal(f.nativeSidebarSurface.style.overflow, "hidden");
    assert.equal(f.frames.size, 0); assert.equal(f.timers.size, 0);
  });
}

test("sidebar drag coordinates remain viewport-relative when the application is zoomed", () => {
  for (const splitShells of [false, true]) {
    const f = fixture(true, undefined, true, { splitShells, zoom: 1.5 });
    f.reduced(true); f.runtime().setEditing(true); f.flush();
    dragSidebar(f, { left: 400, top: 20 });
    const rail = f.nativeRailSurface.getBoundingClientRect();
    const sidebar = f.native.sidebar.getBoundingClientRect();
    assert.ok(Math.abs(rail.left - 400) < .01);
    assert.ok(Math.abs(rail.top - 20) < .01);
    assert.ok(Math.abs(sidebar.left - rail.left - 54) < .01);
    assert.ok(Math.abs(parseFloat(f.nativeRailSurface.style.left) - 400 / 1.5) < .01, "CSS offsets compensate for ancestor zoom");
    assert.equal(JSON.parse(f.stored.get(storageKey)!).panels.sidebar.width, 270, "saved geometry remains in viewport pixels");
  }
});

test("consecutive common-shell resizing keeps the rail fixed-width and restores native child sizing", () => {
  for (const finish of ["reset", "disable"]) {
    const f = fixture(false);
    f.sidebarShell.style.setProperty("flex-basis", "216px", "important");
    f.sidebarShell.style.flexGrow = "0"; f.sidebarShell.style.flexShrink = "0";
    f.native.rail.style.flexBasis = "54px";
    const nodes = sidebarNodes(f);
    const originalStyles = nodes.map((node) => node.style.cssText);
    const parents = nodes.map((node) => node.parentElement);
    f.settings.customLayout = true; f.install(); f.reduced(true); f.runtime().setEditing(true); f.flush();
    dragSidebar(f, { left: 400, top: 20 });
    for (const expectedWidth of [246, 234]) {
      const resize = f.document.querySelectorAll(".codex-plus-custom-layout-grip")
        .find((grip) => grip.getAttribute("aria-label") === "调整侧边栏尺寸");
      assert.ok(resize);
      f.dispatch("keydown", resize, { key: "ArrowLeft", shiftKey: true });
      f.flush(); f.runtime().refresh();
      const surface = f.nativeSidebarSurface.getBoundingClientRect();
      const rail = f.native.rail.getBoundingClientRect();
      const chat = f.sidebarShell.getBoundingClientRect();
      assert.equal(surface.width, expectedWidth);
      assert.equal(f.nativeSidebarSurface.style.position, "fixed");
      assert.equal(f.native.rail.style.position, ""); assert.equal(f.sidebarShell.style.position, "");
      assert.equal(rail.width, 54);
      assert.equal(chat.width, expectedWidth - 54);
      assert.ok(Math.abs(chat.left - rail.right) < .01, "rail and chat remain adjacent after every resize");
      assert.ok(Math.abs(chat.right - surface.right) < .01, "child content does not overflow the painted parent");
      assert.equal(f.native.sidebar.getBoundingClientRect().width, expectedWidth - 54);
      assert.equal(f.window.testLayout!().panels.sidebar.grouped, true);
      assert.equal(f.document.querySelectorAll(".codex-plus-custom-layout-grip").filter((grip) => grip.getAttribute("aria-label")?.startsWith("拖动侧边栏")).length, 1);
      assert.deepEqual(nodes.map((node) => node.parentElement), parents);
    }
    if (finish === "reset") { f.runtime().reset(); f.flush(); }
    else { f.settings.customLayout = false; f.install(); }
    assert.deepEqual(nodes.map((node) => node.style.cssText), originalStyles, `${finish} restores each original width and flex property`);
    assert.equal(f.sidebarShell.style.getPropertyPriority("flex-basis"), "important");
    assert.equal(f.nativeSidebarSurface.getBoundingClientRect().width, 270);
    assert.equal(f.sidebarShell.getBoundingClientRect().width, 216);
    assert.equal(f.native.rail.getBoundingClientRect().width, 54);
  }
});

test("replacing a common-shell sidebar child preserves the grouped owner during its temporary width mismatch", () => {
  const f = fixture(true);
  f.reduced(true); f.runtime().setEditing(true); f.flush();
  dragSidebar(f, { left: 400, top: 20 });
  const resize = f.document.querySelectorAll(".codex-plus-custom-layout-grip")
    .find((grip) => grip.getAttribute("aria-label") === "调整侧边栏尺寸");
  assert.ok(resize);
  for (let count = 0; count < 2; count++) { f.dispatch("keydown", resize, { key: "ArrowLeft", shiftKey: true }); f.flush(); }
  const saved = f.stored.get(storageKey);
  const previous = f.sidebarShell;
  previous.remove();
  const shell = f.document.createElement("div");
  shell.id = "app-shell-sidebar"; shell.nativeRect = { ...previous.nativeRect };
  shell.style.flexBasis = "216px";
  const sidebar = f.document.createElement("div");
  sidebar.className = "sidebar-navigation"; sidebar.nativeRect = { ...f.native.sidebar.nativeRect };
  shell.append(sidebar); f.nativeSidebarSurface.append(shell);
  assert.equal(shell.getBoundingClientRect().width, 216, "the rebuilt native child temporarily exceeds its resized parent");
  f.runtime().refresh(); f.flush();
  assert.equal(f.nativeSidebarSurface.style.position, "fixed");
  assert.equal(f.nativeSidebarSurface.getBoundingClientRect().width, 234);
  assert.equal(shell.style.position, ""); assert.equal(sidebar.style.position, "");
  assert.equal(shell.getBoundingClientRect().width, 180);
  assert.equal(f.native.rail.getBoundingClientRect().width, 54);
  assert.ok(Math.abs(shell.getBoundingClientRect().left - f.native.rail.getBoundingClientRect().right) < .01);
  assert.equal(shell.parentElement, f.nativeSidebarSurface);
  assert.equal(sidebar.parentElement, shell);
  assert.equal(previous.style.width, "", "the stale child is released when React replaces it");
  assert.equal(f.stored.get(storageKey), saved);
  assert.equal(f.document.querySelectorAll(".codex-plus-custom-layout-grip").filter((grip) => grip.getAttribute("aria-label")?.startsWith("拖动侧边栏")).length, 1);
  f.runtime().reset(); f.flush();
  assert.equal(shell.style.width, "");
  assert.equal(shell.style.flexBasis, "216px");
  assert.equal(shell.getBoundingClientRect().width, 216);
});

test("legacy sidebar and rail records migrate once into the grouped sidebar without losing the preferred chat position", () => {
  const f = fixture();
  const runtimeBounds = { left: 8, top: 8, width: 1184, height: 784 };
  const sidebarRecord = f.geometry.rectToRecord({ left: 454, top: 20, width: 216, height: 752 }, runtimeBounds);
  const railRecord = f.geometry.rectToRecord({ left: 80, top: 20, width: 54, height: 752 }, runtimeBounds);
  const legacy = JSON.stringify({ version: 1, panels: { sidebar: sidebarRecord, rail: railRecord } });
  const migrated = fixture(true, legacy);
  migrated.flush();
  const record = migrated.window.testLayout!().panels.sidebar;
  assert.equal(record.grouped, true);
  assert.equal(record.width, 270);
  assert.equal(migrated.window.testLayout!().panels.rail, undefined);
  assert.ok(Math.abs(migrated.native.sidebar.getBoundingClientRect().left - 454) < .01, "legacy sidebar coordinates take precedence over a conflicting rail record");
  const migratedWidth = record.width;
  for (let count = 0; count < 3; count++) migrated.runtime().refresh();
  assert.equal(migrated.window.testLayout!().panels.sidebar.width, migratedWidth, "refresh must not repeatedly add the rail width");
  const grouped = JSON.stringify(migrated.window.testLayout!());
  const reopened = fixture(true, grouped);
  assert.equal(reopened.window.testLayout!().panels.sidebar.width, 270);
  assert.ok(Math.abs(reopened.native.sidebar.getBoundingClientRect().left - 454) < .01);
  const railOnly = fixture(true, JSON.stringify({ version: 1, panels: { rail: { ...railRecord, x: .5 } } }));
  const railOnlyRecord = railOnly.window.testLayout!().panels.sidebar;
  assert.equal(railOnlyRecord.grouped, true);
  assert.equal(railOnlyRecord.width, 270);
  assert.equal(railOnly.window.testLayout!().panels.rail, undefined);
  assert.equal(railOnly.nativeSidebarSurface.style.position, "fixed");
});

test("a rebuilt sidebar painted parent receives the grouped layout while its stale React subtree is released", () => {
  const f = fixture(true);
  f.reduced(true); f.runtime().setEditing(true); f.flush();
  dragSidebar(f, { left: 400, top: 20 });
  const saved = f.stored.get(storageKey);
  const previous = f.nativeSidebarSurface;
  const previousRect = previous.getBoundingClientRect();
  previous.remove(); f.runtime().refresh();
  assert.equal(previous.style.position, "");
  const replacement = f.document.createElement("aside");
  replacement.nativeRect = { ...previous.nativeRect };
  replacement.className = "app-shell-left-panel";
  replacement.style.backgroundColor = "rgb(32, 32, 32)";
  const rail = f.document.createElement("nav");
  rail.nativeRect = { ...f.native.rail.nativeRect }; rail.setAttribute("data-app-navigation-rail", "");
  const shell = f.document.createElement("div");
  shell.nativeRect = { ...f.sidebarShell.nativeRect }; shell.id = "app-shell-sidebar";
  const sidebar = f.document.createElement("div");
  sidebar.nativeRect = { ...f.native.sidebar.nativeRect }; sidebar.className = "sidebar-navigation";
  replacement.append(rail, shell); shell.append(sidebar); f.appRoot.append(replacement);
  f.runtime().refresh(); f.flush();
  assert.equal(replacement.style.position, "fixed");
  assert.ok(Math.abs(replacement.getBoundingClientRect().left - previousRect.left) < .01);
  assert.ok(Math.abs(replacement.getBoundingClientRect().top - previousRect.top) < .01);
  assert.equal(rail.parentElement, replacement);
  assert.equal(sidebar.parentElement, shell);
  assert.equal(rail.style.position, ""); assert.equal(sidebar.style.position, "");
  assert.equal(f.stored.get(storageKey), saved, "React parent replacement does not rewrite the saved layout");
});

test("dragging saves once on completion, restores next launch, and stays inside a resized window", () => {
  const f = fixture(true);
  f.runtime().setEditing(true); f.flush();
  const grip = moveGrip(f);
  f.dispatch("pointerdown", grip, { clientX: 100, clientY: 100 });
  f.dispatch("pointermove", grip, { clientX: 210, clientY: -250 });
  assert.equal(f.native.composer.style.position, "fixed");
  assert.equal(f.native.composer.getBoundingClientRect().left, 450);
  assert.equal(f.native.composer.getBoundingClientRect().top, 300);
  assert.equal(f.writes.length, 0, "pointer moves must not repeatedly write storage");
  f.dispatch("pointerup", grip); f.flush();
  assert.equal(f.writes.length, 1);
  assert.equal(f.writes[0][0], storageKey);
  const layout = JSON.parse(f.writes[0][1]);
  assert.equal(layout.version, 1);
  assert.ok(layout.panels.composer.x > 0 && layout.panels.composer.x < 1);
  assert.ok(layout.panels.composer.y > 0 && layout.panels.composer.y < 1);
  assert.equal(layout.panels.composer.height, undefined, "composer height remains controlled by its content");
  assert.equal(f.windowListeners.get("pointermove")?.size, 0);
  assert.equal(f.windowListeners.get("pointerup")?.size, 0);
  assert.equal(f.windowListeners.get("pointercancel")?.size, 0);

  const reopened = fixture(true, f.writes[0][1]);
  assert.equal(reopened.native.composer.getBoundingClientRect().left, 450);
  assert.equal(reopened.native.composer.getBoundingClientRect().top, 300);
  assert.equal(reopened.created.filter((element) => element.isConnected).length, 0, "saved positioning does not expose editing controls until requested");
  reopened.window.innerWidth = 380; reopened.window.innerHeight = 240;
  reopened.dispatch("resize", reopened.body); reopened.flush();
  const rect = reopened.native.composer.getBoundingClientRect();
  assert.ok(rect.left >= 0 && rect.left + rect.width <= 380);
  assert.ok(rect.top >= 0 && rect.top + rect.height <= 240);
});

test("a canceled pointer drag and Escape restore the preceding position without saving", () => {
  const f = fixture(true);
  f.runtime().setEditing(true); f.flush();
  dragComposer(f, { left: 450, top: 300 });
  const saved = f.stored.get(storageKey);
  const writes = f.writes.length;
  dragComposer(f, { left: 560, top: 190 }, false);
  assert.equal(f.native.composer.getBoundingClientRect().left, 450);
  assert.equal(f.native.composer.getBoundingClientRect().top, 300);
  assert.equal(f.writes.length, writes);
  assert.equal(f.stored.get(storageKey), saved);
  const grip = moveGrip(f);
  f.dispatch("pointerdown", grip, { clientX: 100, clientY: 100 });
  f.dispatch("pointermove", grip, { clientX: 250, clientY: 80 });
  f.dispatch("keydown", grip, { key: "Escape" }); f.flush();
  assert.equal(f.native.composer.getBoundingClientRect().left, 450);
  assert.equal(f.native.composer.getBoundingClientRect().top, 300);
  assert.equal(f.writes.length, writes);
});

test("disabling restores inline styles, removes the editing UI, and detaches runtime observers and listeners", () => {
  const f = fixture(false);
  f.native.composer.style.setProperty("position", "relative");
  f.native.composer.style.setProperty("left", "12px", "important");
  f.native.composer.style.setProperty("color", "red");
  const before = f.native.composer.style.cssText;
  f.settings.customLayout = true; f.install(); f.runtime().setEditing(true); f.flush();
  dragComposer(f, { left: 450, top: 300 });
  f.native.composer.style.setProperty("color", "blue");
  f.settings.customLayout = false; f.install();
  assert.equal(f.native.composer.style.position, "relative");
  assert.equal(f.native.composer.style.left, "12px");
  assert.equal(f.native.composer.style.getPropertyPriority("left"), "important");
  assert.equal(f.native.composer.style.color, "blue", "unrelated native style updates survive cleanup");
  assert.ok(before.includes("position: relative"));
  assert.equal(f.created.filter((element) => element.isConnected).length, 0);
  assert.equal([...f.documentListeners.values()].reduce((count, listeners) => count + listeners.size, 0), 0);
  assert.equal([...f.windowListeners.values()].reduce((count, listeners) => count + listeners.size, 0), 0);
  assert.ok(f.observers.every((observer) => !observer.connected));
  assert.equal(f.frames.size, 0); assert.equal(f.timers.size, 0);
  assert.ok(f.stored.get(storageKey), "disabling retains the user's saved arrangement");
});

test("reset clears saved positions and repeat injection cleans up the old editing instance", () => {
  const f = fixture(true);
  f.native.composer.style.left = "12.000px";
  f.native.composer.style.width = "610.500px";
  f.runtime().setEditing(true); f.flush();
  dragComposer(f, { left: 450, top: 300 });
  for (let refresh = 0; refresh < 3; refresh++) f.runtime().refresh();
  f.runtime().reset(); f.flush();
  assert.deepEqual(JSON.parse(f.stored.get(storageKey)!), { version: 1, panels: {} });
  assert.equal(f.native.composer.style.position, "");
  assert.equal(f.native.composer.style.left, "12px", "browser normalization of our applied coordinates must not become the restore baseline");
  assert.equal(f.native.composer.style.width, "610.5px", "reset restores the native width rather than the normalized custom width");
  const oldDecorations = f.created.filter((element) => element.isConnected);
  const oldObservers = [...f.observers];
  f.load(); f.flush();
  assert.ok(oldDecorations.every((element) => !element.isConnected));
  assert.ok(oldObservers.every((observer) => !observer.connected));
  assert.equal(f.windowListeners.get("resize")?.size, 1);
  assert.equal(f.documentListeners.get("keydown")?.size, 1);
});

test("a rebuilt native panel receives the saved layout while the stale node is released", () => {
  const saved = JSON.stringify({ version: 1, panels: { summary: { x: .5, y: .4, width: 250 } } });
  const f = fixture(true, saved);
  const previous = f.native.summary;
  const previousRect = previous.getBoundingClientRect();
  previous.remove(); f.runtime().refresh();
  assert.equal(previous.style.position, "");
  const replacement = f.document.createElement("aside");
  replacement.nativeRect = { ...previous.nativeRect };
  replacement.setAttribute("data-summary-panel-variant", "floating");
  f.body.appendChild(replacement); f.runtime().refresh();
  assert.equal(replacement.style.position, "fixed");
  assert.equal(replacement.getBoundingClientRect().left, previousRect.left);
  assert.equal(replacement.getBoundingClientRect().top, previousRect.top);
  assert.equal(replacement.getBoundingClientRect().width, 250);
  assert.equal(f.stored.get(storageKey), saved, "DOM replacement must not overwrite the persisted layout");
});

test("temporary ancestor overrides are restored and native style rewrites become the cleanup baseline", () => {
  const f = fixture(false, JSON.stringify({ version: 1, panels: { composer: { x: .5, y: .4 } } }));
  f.body.style.setProperty("transform", "translateX(8px)");
  f.body.style.setProperty("overflow-x", "hidden");
  f.native.composer.style.setProperty("position", "relative");
  f.settings.customLayout = true; f.install();
  assert.equal(f.body.style.transform, "none");
  assert.equal(f.body.style.getPropertyValue("overflow-x"), "visible");
  f.native.composer.style.setProperty("position", "absolute");
  f.runtime().refresh();
  assert.equal(f.native.composer.style.position, "fixed");
  f.settings.customLayout = false; f.install();
  assert.equal(f.native.composer.style.position, "absolute", "cleanup preserves React's latest style baseline");
  assert.equal(f.body.style.transform, "translateX(8px)");
  assert.equal(f.body.style.getPropertyValue("overflow-x"), "hidden");
});

test("layout waits for backend readiness and retains an existing layout while settings reload", () => {
  const saved = JSON.stringify({ version: 1, panels: { composer: { x: .5, y: .4, width: 620 } } });
  const f = fixture(true, saved, false);
  assert.equal(f.native.composer.style.position, "");
  assert.equal(f.created.filter((element) => element.isConnected).length, 0);
  assert.equal(f.observers.length, 0);
  assert.equal(f.windowListeners.size, 0);
  f.backend.loaded = true; f.install();
  assert.equal(f.native.composer.style.position, "fixed");
  const applied = f.native.composer.style.cssText;
  f.backend.loaded = false; f.settings.customLayout = false; f.install();
  assert.equal(f.native.composer.style.cssText, applied, "a loading-time default must not briefly dismantle the existing layout");
  f.backend.loaded = true; f.install();
  assert.equal(f.native.composer.style.position, "");
});

test("custom layout releases the composer centering snapshot before taking ownership", () => {
  const f = fixture(false);
  const composer = f.native.composer;
  composer.style.width = "620px";
  composer.style.maxWidth = "none";
  composer.style.left = "";
  composer.style.transform = "none";
  f.settings.conversationView = true;
  f.window.testCenter!.remember(composer);
  composer.style.width = "100%";
  composer.style.maxWidth = "900px";
  composer.style.left = "120px";
  composer.style.transform = "translateX(-50%)";
  assert.ok(f.conversationState.elements.has(composer));
  f.settings.customLayout = true; f.install(); f.runtime().setEditing(true); f.flush();
  dragComposer(f, { left: 450, top: 300 });
  assert.equal(f.conversationState.elements.has(composer), false);
  assert.equal("codexPlusConversationViewOriginalWidth" in composer.dataset, false);
  const positioned = composer.style.cssText;
  f.settings.conversationView = false; f.window.testCenter!.cleanup();
  assert.equal(composer.style.cssText, positioned, "turning off centering must not restore its stale snapshot over the custom layout");
  f.settings.customLayout = false; f.install();
  assert.equal(composer.style.width, "620px");
  assert.equal(composer.style.maxWidth, "none");
  assert.equal(composer.style.left, "");
  assert.equal(composer.style.transform, "none");
});

function collisionFixture(reducedMotion = false) {
  const f = fixture(false);
  f.nativeSidebarSurface.nativeRect = { left: 8, top: 8, width: 292, height: 400 };
  f.native.rail.nativeRect = { left: 8, top: 8, width: 54, height: 400 };
  f.native.sidebar.nativeRect = { left: 80, top: 8, width: 220, height: 400 };
  f.sidebarShell.nativeRect = { ...f.native.sidebar.nativeRect };
  f.native.summary.nativeRect = { left: 620, top: 180, width: 260, height: 200 };
  f.native.composer.nativeRect = { left: 330, top: 600, width: 320, height: 120 };
  f.reduced(reducedMotion);
  f.settings.customLayout = true; f.install(); f.runtime().setEditing(true); f.flush();
  return f;
}

function beginCollision(f: Fixture) {
  const grip = moveGrip(f);
  f.dispatch("pointerdown", grip, { clientX: 100, clientY: 100 });
  f.dispatch("pointermove", grip, { clientX: 390, clientY: -320, altKey: true });
  return grip;
}

function rects(f: Fixture) {
  return Object.fromEntries(Object.entries(f.native).map(([id, element]) => [id, element.getBoundingClientRect()]));
}

function assertNoOverlap(f: Fixture) {
  const entries = Object.entries(rects(f));
  for (let first = 0; first < entries.length; first++) {
    for (let second = first + 1; second < entries.length; second++) {
      const [aId, a] = entries[first]; const [bId, b] = entries[second];
      const overlapWidth = Math.min(a.right, b.right) - Math.max(a.left, b.left);
      const overlapHeight = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
      assert.ok(overlapWidth < .01 || overlapHeight < .01, `${aId} should give way to ${bId}`);
    }
  }
}

test("the active panel follows the pointer immediately while displaced panels spring to a saved nonoverlapping layout", () => {
  const f = collisionFixture();
  const summaryBefore = f.native.summary.getBoundingClientRect();
  const grip = beginCollision(f);
  assert.equal(f.native.composer.getBoundingClientRect().left, 620);
  assert.equal(f.native.composer.getBoundingClientRect().top, 180);
  assert.ok(f.frames.size > 0, "displaced panels need animation frames");
  const target = f.window.testLayout!().panels.summary;
  assert.ok(target, "the displaced panel must join the same persisted layout transaction");
  const final = f.geometry.recordToRect(target, summaryBefore, { left: 8, top: 8, width: 1184, height: 784 });
  f.frame();
  const intermediate = f.native.summary.getBoundingClientRect();
  assert.ok(Math.hypot(intermediate.left - summaryBefore.left, intermediate.top - summaryBefore.top) > 0);
  assert.ok(Math.hypot(intermediate.left - final.left, intermediate.top - final.top) > .01, "the first frame should interpolate rather than jump straight to the final target");
  assert.equal(f.writes.length, 0);
  f.dispatch("pointerup", grip); f.flush();
  assertNoOverlap(f);
  assert.equal(f.frames.size, 0, "a settled spring must not leave an animation loop running");
  assert.equal(f.writes.length, 1);
  const saved = JSON.parse(f.stored.get(storageKey)!);
  assert.ok(saved.panels.composer && saved.panels.summary);
  const reopened = fixture(false, f.stored.get(storageKey));
  reopened.nativeSidebarSurface.nativeRect = { ...f.nativeSidebarSurface.nativeRect };
  reopened.sidebarShell.nativeRect = { ...f.sidebarShell.nativeRect };
  for (const id of Object.keys(f.native) as Array<keyof typeof f.native>) reopened.native[id].nativeRect = { ...f.native[id].nativeRect };
  reopened.settings.customLayout = true; reopened.install(); reopened.flush();
  assertNoOverlap(reopened);
  for (const id of ["composer", "summary"] as const) {
    assert.equal(reopened.native[id].getBoundingClientRect().left, f.native[id].getBoundingClientRect().left);
    assert.equal(reopened.native[id].getBoundingClientRect().top, f.native[id].getBoundingClientRect().top);
  }
});

test("cancel restores the entire collision transaction and repeated pointer targets do not push neighbors cumulatively", () => {
  const f = collisionFixture();
  const before = rects(f);
  const beforeLayout = f.window.testLayout!();
  const grip = beginCollision(f);
  const firstTarget = f.window.testLayout!();
  f.dispatch("pointermove", grip, { clientX: 420, clientY: -320, altKey: true });
  f.dispatch("pointermove", grip, { clientX: 390, clientY: -320, altKey: true });
  assert.deepEqual(f.window.testLayout!(), firstTarget, "each move should solve from the fixed pointer-down baseline");
  f.dispatch("pointercancel", grip); f.flush();
  assert.deepEqual(f.window.testLayout!(), beforeLayout);
  assert.deepEqual(rects(f), before);
  assert.equal(f.writes.length, 0, "a canceled interaction must not persist any displaced panel");
  assert.equal(f.frames.size, 0);
});

test("reduced motion applies displaced targets immediately and disabling stops an in-flight spring", () => {
  const instant = collisionFixture(true);
  const instantGrip = beginCollision(instant);
  assertNoOverlap(instant);
  instant.dispatch("pointerup", instantGrip); instant.flush();
  assert.equal(instant.frames.size, 0);

  const animated = collisionFixture();
  const baseline = rects(animated);
  const animatedGrip = beginCollision(animated);
  animated.frame();
  assert.ok(animated.frames.size > 0);
  animated.settings.customLayout = false; animated.install();
  assert.equal(animated.frames.size, 0, "cleanup must cancel the spring RAF as well as scanning frames");
  assert.equal(animated.mediaListeners.size, 0);
  assert.deepEqual(rects(animated), baseline);
  const cleaned = Object.values(animated.native).map((element) => element.style.cssText);
  animated.dispatch("pointerup", animatedGrip); animated.frame(300);
  assert.deepEqual(Object.values(animated.native).map((element) => element.style.cssText), cleaned);
  assert.equal(animated.writes.length, 0);
});
