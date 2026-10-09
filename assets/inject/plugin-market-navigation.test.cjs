const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");

const fragment = name => readFileSync(`${__dirname}/renderer-inject/${name}`, "utf8");
const adapter = fragment("62-plugin-market-adapter.js");
const navigation = fragment("40-backend-settings.js");
const entryNavigation = fragment("50-navigation.js");
const scheduler = fragment("98-scan-schedule.js");
const extract = (source, name) => source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0] || "";

class Element {
  constructor(tag, attrs = {}) {
    this.tagName = tag.toUpperCase(); this.nodeType = 1; this.attrs = { ...attrs };
    this.children = []; this.parentElement = null; this.className = ""; this.dataset = {};
    this.textContent = ""; this.listeners = {}; this.visible = true; this.disabled = false; this.clicks = 0;
  }
  get id() { return this.attrs.id || ""; }
  set id(value) { this.attrs.id = value; }
  get isConnected() { return this.tagName === "DOCUMENT" || !!this.parentElement?.isConnected; }
  get nextSibling() { return this.parentElement?.children[this.parentElement.children.indexOf(this) + 1] || null; }
  get firstElementChild() { return this.children[0] || null; }
  setAttribute(key, value) { this.attrs[key] = value; }
  getAttribute(key) { return this.attrs[key] ?? null; }
  hasAttribute(key) { return Object.hasOwn(this.attrs, key); }
  removeAttribute(key) { delete this.attrs[key]; }
  appendChild(node) { node.remove(); this.children.push(node); node.parentElement = this; return node; }
  insertBefore(node, before) {
    if (node === before) return node;
    node.remove(); const index = before ? this.children.indexOf(before) : -1;
    if (index < 0) this.children.push(node); else this.children.splice(index, 0, node);
    node.parentElement = this; return node;
  }
  remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter(child => child !== this); this.parentElement = null; }
  contains(node) { return node === this || this.children.some(child => child.contains(node)); }
  cloneNode(deep) {
    const next = new Element(this.tagName, this.attrs); next.className = this.className; next.textContent = this.textContent;
    next.dataset = { ...this.dataset }; if (deep) this.children.forEach(child => next.appendChild(child.cloneNode(true))); return next;
  }
  matches(selector) {
    return selector.split(",").some(part => {
      const tokens = part.trim().split(/\s+(?![^[]*\])/); const atom = tokens.pop();
      const tag = atom.match(/^[a-z]+/i)?.[0]; if (tag && this.tagName !== tag.toUpperCase()) return false;
      const id = atom.match(/#([\w-]+)/)?.[1]; if (id && this.id !== id) return false;
      for (const match of atom.matchAll(/\.([\w-]+)/g)) if (!this.className.split(/\s+/).includes(match[1])) return false;
      for (const match of atom.matchAll(/\[([\w-]+)(?:([*]?=)["']([^"']*)["'])?\]/g)) {
        const value = match[1] === "class" ? this.className : this.getAttribute(match[1]);
        if (value === null || (match[2] === "=" && value !== match[3]) || (match[2] === "*=" && !value.includes(match[3]))) return false;
      }
      return !tokens.length || !!this.parentElement?.closest(tokens.join(" "));
    });
  }
  closest(selector) { return this.matches(selector) ? this : this.parentElement?.closest(selector) || null; }
  querySelectorAll(selector) { return this.children.flatMap(child => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)]); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  addEventListener(type, listener) { this.listeners[type] = listener; }
  click() { this.clicks += 1; if (this.clicks > 3) throw new Error("recursive fallback navigation"); this.listeners.click?.({ preventDefault() {}, stopPropagation() {} }); }
}

function fixture(layout = "sidebar") {
  const document = new Element("document");
  document.body = document.appendChild(new Element("body"));
  document.getElementById = id => document.querySelector(`#${id}`);
  document.createElement = tag => new Element(tag);
  const aside = document.body.appendChild(new Element("aside")); aside.className = "app-shell-left-panel";
  const nav = aside.appendChild(new Element("nav", layout === "rail" ? { "data-app-navigation-rail": "" } : { role: "navigation" }));
  const anchor = nav.appendChild(new Element("button", { "data-sidebar-destination": "builtin:home", "aria-label": "Home" }));
  const sidebarAnchor = nav.appendChild(new Element("div", { id: "codex-plus-sidebar-nav" }));
  const calls = [];
  const extension = node => !!node?.closest('[data-codex-plus-ext], [data-codex-plus-rail], .registered-extension');
  const context = vm.createContext({ document, window: {}, Element, HTMLElement: Element,
    codexPlusSidebarPluginMarketId: "codex-plus-sidebar-plugin-market", codexPlusRailPluginMarketId: "codex-plus-rail-plugin-market",
    codexPlusSidebarNavId: "codex-plus-sidebar-nav", codexPlusMenuId: "codex-plus-menu", codexPlusPageClass: "codex-plus-page",
    codexPlusRailNavId: "codex-plus-rail-nav", codexPlusRailExtensionsId: "codex-plus-rail-extensions", codexPlusRailSponsorId: "codex-plus-rail-sponsor",
    codexPlusRailSelector: 'nav[data-app-navigation-rail]', codexPlusRailDestinationSelector: "[data-sidebar-destination]",
    codexPlusDefaultExtensionIconPath: "", codexPlusExtensionConstants: { extensionAttribute: "data-codex-plus-ext" },
    codexPlusBackendStatus: { status: "ok" }, codexPlusActiveEntry: () => "", setCodexPlusSidebarNavActive() {},
    closeCodexPlusPage() {}, closeCodexPlusPageAfterNativeNavigation() {}, clearPluginMarketplaceQueryCache() {},
    openCodexPlusPage() {}, openCodexPlusExtensions() {}, openCodexPlusSponsor() {},
    markCodexPlusExtensionNode: (node, owner) => node.setAttribute("data-codex-plus-ext", owner),
    isExtensionUiNode: extension,
    visibleElement: node => node.isConnected && node.visible,
    postJson: async (path, payload) => { calls.push({ path, payload }); return { status: "ok" }; },
    selectors: { sidebarThread: "[data-app-action-sidebar-thread-id]", threadTitle: ".thread-title", appHeader: "header",
      archiveNav: "[data-archive-nav]", pluginNavButton: 'nav[role="navigation"] button.h-token-nav-row.w-full', pluginSvgPath: "path[data-plugin-icon]", disabledInstallButton: "[data-disabled-install]" },
    codexPluginMarketplacePatchEnabled: () => false, nodeOrAncestorLooksLikeCodexUserBubble: () => false, nodeLooksLikeCodexUserBubble: () => false,
  });
  const functions = [
    extract(adapter, "codexPlusNativePluginNavigationEntry"), extract(adapter, "openCodexPlusNativePluginMarket"),
    ...["codexPlusRailTemplateButton", "codexPlusRailPrimaryAnchor", "createCodexPlusRailButton", "installCodexPlusRailNavigation", "installCodexPlusSidebarNavigation", "detachCodexPlusSidebarNavigation", "removeCodexPlusRailNavigation"].map(name => extract(navigation, name)),
    extract(entryNavigation, "installCodexPlusNavigationEntries"),
    ...["scanRelevantSelector", "nodeSelfOrAncestorMatchesScanRelevance", "isScanRelevantNode", "isChatContentMutation", "shouldScheduleScan"].map(name => extract(scheduler, name)),
  ].join("\n");
  vm.runInContext(functions, context);
  const install = () => context.installCodexPlusNavigationEntries();
  const fallback = () => document.getElementById(layout === "rail" ? "codex-plus-rail-plugin-market" : "codex-plus-sidebar-plugin-market");
  const native = (attrs = {}) => nav.appendChild(new Element("button", { "data-sidebar-destination": "builtin:plugins", "aria-label": "Plugins", ...attrs }));
  const legacy = (kind = layout, parent = nav) => {
    const wrapper = parent.appendChild(new Element("div", {
      id: kind === "rail" ? "codex-plus-rail-plugin-market" : "codex-plus-sidebar-plugin-market",
      "data-codex-plus-ext": "builtin-plugin-market",
    }));
    wrapper.appendChild(new Element("button", { "aria-label": "CodeX 插件市场" }));
    return wrapper;
  };
  return { document, nav, context, calls, install, fallback, native, legacy };
}

for (const layout of ["sidebar", "rail"]) {
  test(`${layout}: retired injected stores stay absent through late native arrival, removal and repeated scans`, () => {
    const f = fixture(layout);
    const oldRail = f.legacy("rail"), oldSidebar = f.legacy("sidebar");
    f.install();
    assert.ok(!oldRail.isConnected && !oldSidebar.isConnected, "both exact legacy IDs are removed even before native navigation appears");
    assert.ok(!f.fallback());
    for (let index = 0; index < 3; index++) f.install();
    assert.ok(!f.fallback(), "scanning must never recreate a duplicate store button");
    const native = f.native(); f.install(); assert.ok(!f.fallback());
    assert.ok(native.isConnected, "native plugin navigation is preserved");
    native.remove(); f.install(); assert.ok(!f.fallback());
    const rebuilt = f.native(); f.install(); assert.ok(!f.fallback());
    assert.equal(rebuilt.clicks, 0, "scanning must not navigate");
  });

  test(`${layout}: rebuilt navigation removes stale stores and preserves the replacement native entry`, () => {
    const f = fixture(layout); f.install();
    const aside = f.nav.parentElement; f.nav.remove();
    const next = aside.appendChild(new Element("nav", layout === "rail" ? { "data-app-navigation-rail": "" } : { role: "navigation" }));
    const anchor = next.appendChild(new Element("button", { "data-sidebar-destination": "builtin:home", "aria-label": "Home" }));
    const entry = next.appendChild(new Element("div", { id: "codex-plus-sidebar-nav" }));
    const stale = f.legacy(layout, next);
    const install = f.install;
    install(); assert.ok(!f.fallback()); assert.equal(stale.isConnected, false);
    const native = next.appendChild(new Element("button", { "data-sidebar-destination": "plugins" }));
    install(); assert.ok(!f.fallback());
    f.context.openCodexPlusNativePluginMarket(); assert.equal(native.clicks, 1);
  });
}

test("extension/self entries cannot impersonate native plugins or cause recursive clicks", () => {
  const f = fixture();
  const owned = f.native({ "data-codex-plus-ext": "example-script" });
  const registered = f.native(); registered.className = "registered-extension";
  f.install(); assert.ok(!f.fallback());
  f.context.openCodexPlusNativePluginMarket();
  assert.equal(owned.clicks, 0); assert.equal(registered.clicks, 0);
  assert.equal(f.calls.length, 1); assert.equal(f.calls[0].path, "/manager/open");
  assert.equal(f.calls[0].payload.page, "pluginMarket");
});

test("legacy injected IDs and non-navigation Plugins buttons cannot masquerade as native", () => {
  const f = fixture();
  const fallback = f.legacy(); fallback.removeAttribute("data-codex-plus-ext");
  const button = fallback.querySelector("button"); button.setAttribute("data-sidebar-destination", "plugins"); button.setAttribute("aria-label", "Plugins");
  const outside = f.document.body.appendChild(new Element("button", { "data-sidebar-destination": "plugins", "aria-label": "Plugins" }));
  f.context.openCodexPlusNativePluginMarket();
  assert.equal(button.clicks, 0); assert.equal(outside.clicks, 0); assert.equal(f.calls.length, 1);
});

test("disabled native navigation does not create a duplicate and activation opens the manager", () => {
  const f = fixture(); const native = f.native(); native.disabled = true;
  f.install(); assert.ok(!f.fallback());
  f.context.openCodexPlusNativePluginMarket(); assert.equal(native.clicks, 0); assert.equal(f.calls.length, 1);
});

test("native activation rechecks rebuilt DOM and never clicks a stale or invisible entry", () => {
  const f = fixture("rail"); const first = f.native();
  f.context.openCodexPlusNativePluginMarket(); assert.equal(first.clicks, 1);
  first.remove(); const next = f.native();
  f.context.openCodexPlusNativePluginMarket(); assert.equal(next.clicks, 1); assert.equal(first.clicks, 1);
  next.visible = false;
  f.context.openCodexPlusNativePluginMarket(); assert.equal(next.clicks, 1); assert.equal(f.calls.length, 1);
});

test("classic navigation without destination attributes recognizes Chinese and English native labels", () => {
  for (const label of ["插件", "Plugins"]) {
    const f = fixture(); const native = f.native(); native.removeAttribute("data-sidebar-destination"); native.setAttribute("aria-label", label);
    f.install(); assert.ok(!f.fallback());
    f.context.openCodexPlusNativePluginMarket(); assert.equal(native.clicks, 1);
  }
});

test("native rail mutations schedule reconciliation while owned fallback writes stay ignored", () => {
  const f = fixture("rail"); f.install();
  const stale = f.legacy();
  assert.equal(f.context.shouldScheduleScan([{ target: f.nav, addedNodes: [stale], removedNodes: [] }]), false);
  f.install(); assert.equal(stale.isConnected, false);
  assert.equal(f.context.shouldScheduleScan([{ target: f.nav, addedNodes: [], removedNodes: [stale] }]), false);
  const native = f.native();
  assert.equal(f.context.shouldScheduleScan([{ target: f.nav, addedNodes: [native], removedNodes: [] }]), true);
  native.remove();
  assert.equal(f.context.shouldScheduleScan([{ target: f.nav, addedNodes: [], removedNodes: [native] }]), true);
});
