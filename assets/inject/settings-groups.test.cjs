// Codex++ 设置页分组导航（40-backend-settings.js「设置页分组导航」一段）的行为测试：
// 左栏列出哪些分组、默认选哪组、切换后右边只显示这一组并记住、记住的分组不在了落回第一组。
const assert = require("node:assert/strict");
const fs = require("node:fs");
const test = require("node:test");
const vm = require("node:vm");

const shard = fs.readFileSync(require.resolve("./renderer-inject/40-backend-settings.js"), "utf8");
const start = shard.indexOf("  // ── 设置页分组导航");
const end = shard.indexOf("  function normalizeCodexPlusSettingsCatalog(", start);
assert.ok(start >= 0 && end > start, "settings group navigation block not found");
const source = shard.slice(start, end);

/** 够这段代码用的最小 DOM：属性、dataset、children、matches / closest / querySelector(All)。 */
class FakeElement {
  constructor(attrs = {}, children = []) {
    this.attrs = new Map(Object.entries(attrs));
    this.children = [];
    this.parent = null;
    this.scrollTop = 0;
    children.forEach((child) => this.append(child));
    this.dataset = new Proxy({}, {
      get: (_, key) => this.getAttribute(`data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`) ?? undefined,
      set: (_, key, value) => { this.setAttribute(`data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`, String(value)); return true; },
    });
  }
  append(child) { child.parent = this; this.children.push(child); return child; }
  getAttribute(name) { return this.attrs.has(name) ? this.attrs.get(name) : null; }
  setAttribute(name, value) { this.attrs.set(name, String(value)); }
  removeAttribute(name) { this.attrs.delete(name); }
  hasAttribute(name) { return this.attrs.has(name); }
  matchesSimple(selector) {
    const cls = selector.match(/^\.([\w-]+)$/);
    if (cls) return (this.getAttribute("class") || "").split(/\s+/).includes(cls[1]);
    const attr = selector.match(/^\[([\w-]+)(?:="([^"]*)")?\]$/);
    if (!attr) throw new Error(`unsupported selector ${selector}`);
    return attr[2] === undefined ? this.hasAttribute(attr[1]) : this.getAttribute(attr[1]) === attr[2];
  }
  matches(selector) {
    const parts = selector.split(/\s*>\s*/);
    if (parts.length === 1) return this.matchesSimple(selector);
    return this.matchesSimple(parts[1]) && !!this.parent?.matchesSimple(parts[0]);
  }
  closest(selector) {
    for (let node = this; node; node = node.parent) if (node.matches(selector)) return node;
    return null;
  }
  descendants() { return this.children.flatMap((child) => [child, ...child.descendants()]); }
  querySelectorAll(selector) { return this.descendants().filter((node) => node.matches(selector)); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
}

const group = (id, label, children = []) => new FakeElement({
  class: "codex-plus-settings-group", "data-codex-plus-settings-group": id, "data-codex-plus-settings-label": label, "data-active": "false",
}, children);

function harness({ stored = "", extMenu = false } = {}) {
  const storage = new Map(stored ? [["codex-plus-settings-group", stored]] : []);
  const panel = new FakeElement({ "data-codex-plus-panel": "home" }, [
    group("core", "常规"),
    group("plugins", "模型与插件"),
    group("tools", "工具与关于"),
    group("ext-menu", "拓展设置", extMenu ? [new FakeElement({ "data-codex-plus-ext-menu": "true" })] : []),
  ]);
  const body = new FakeElement({ class: "codex-plus-modal-body" }, [panel]);
  const nav = new FakeElement({ "data-codex-plus-page-nav-body": "true" });
  const root = new FakeElement({}, [body, nav]);
  body.scrollTop = 480;
  const context = vm.createContext({
    window: { localStorage: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, String(value)) } },
    document: root,
    escapeHtml: (value) => String(value ?? "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;"),
  });
  vm.runInContext(source, context);
  /** 把渲染出来的左栏按钮放进 nav，好让 sync 能找到它们。 */
  const renderNav = () => {
    const html = context.renderCodexPlusSettingsNavItems();
    nav.children = [];
    for (const [, id, active] of html.matchAll(/data-codex-plus-settings-nav="([^"]+)" data-active="(true|false)"/g)) {
      nav.append(new FakeElement({ "data-codex-plus-settings-nav": id, "data-active": active }));
    }
    return html;
  };
  const activeGroups = () => panel.children.filter((node) => node.getAttribute("data-active") === "true").map((node) => node.getAttribute("data-codex-plus-settings-group"));
  return { context, panel, body, nav, storage, renderNav, activeGroups };
}

test("左栏列出有内容的分组，没记住时选第一组", () => {
  const h = harness();
  const html = h.renderNav();
  assert.deepEqual([...html.matchAll(/data-codex-plus-settings-nav="([^"]+)"/g)].map((m) => m[1]), ["core", "plugins", "tools"], "没有拓展菜单项时不列「拓展设置」");
  assert.match(html, /<span class="codex-plus-page-nav-item-icon" aria-hidden="true"><svg [^]*?<\/svg><\/span><span class="codex-plus-page-nav-item-name">常规<\/span><\/button>/, "每项前面有图标");
  assert.match(html, /data-codex-plus-settings-nav="core" data-active="true" aria-current="page"/);
  h.context.syncCodexPlusSettingsGroups();
  assert.deepEqual(h.activeGroups(), ["core"]);
});

test("切换分组：右边只留这一组，左栏选中跟着走，记住选择并回到顶部", () => {
  const h = harness();
  h.renderNav();
  h.context.selectCodexPlusSettingsGroup("plugins");
  assert.deepEqual(h.activeGroups(), ["plugins"]);
  const current = h.nav.children.filter((button) => button.getAttribute("aria-current") === "page").map((button) => button.getAttribute("data-codex-plus-settings-nav"));
  assert.deepEqual(current, ["plugins"]);
  assert.equal(h.nav.children.find((button) => button.getAttribute("data-codex-plus-settings-nav") === "core").getAttribute("data-active"), "false");
  assert.equal(h.storage.get("codex-plus-settings-group"), "plugins");
  assert.equal(h.body.scrollTop, 0);
});

test("记住的分组还在就打开它；不在了（清单变了）落回第一组", () => {
  const remembered = harness({ stored: "tools" });
  remembered.context.syncCodexPlusSettingsGroups();
  assert.deepEqual(remembered.activeGroups(), ["tools"]);
  const gone = harness({ stored: "removed-group" });
  gone.context.syncCodexPlusSettingsGroups();
  assert.deepEqual(gone.activeGroups(), ["core"]);
  assert.match(gone.renderNav(), /data-codex-plus-settings-nav="core" data-active="true"/);
});

test("有拓展菜单项时左栏多出「拓展设置」，可以切过去", () => {
  const h = harness({ extMenu: true, stored: "ext-menu" });
  const html = h.renderNav();
  assert.match(html, /data-codex-plus-settings-nav="ext-menu" data-active="true"[^]*?>拓展设置</);
  h.context.syncCodexPlusSettingsGroups();
  assert.deepEqual(h.activeGroups(), ["ext-menu"]);
});
