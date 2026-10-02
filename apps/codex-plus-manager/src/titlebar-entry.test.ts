import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";

const source = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

// Minimal DOM fixture for placement, native menu ownership and reinjection behavior.
class EntryNode {
  id = "";
  className = "native-menu-button";
  title = "";
  dataset: Record<string, string> = {};
  attributes: Record<string, string> = {};
  parentElement: EntryNode | null = null;
  children: EntryNode[] = [];
  handlers: Record<string, () => void> = {};
  inserts = 0;
  indicator: EntryNode | null = null;
  tag: string;
  constructor(tag = "div") { this.tag = tag; }
  set innerHTML(_value: string) { this.indicator = new EntryNode("span"); }
  get nextElementSibling(): EntryNode | null {
    const siblings = this.parentElement?.children ?? [];
    return siblings[siblings.indexOf(this) + 1] ?? null;
  }
  get nextSibling() { return this.nextElementSibling; }
  setAttribute(name: string, value: string) { this.attributes[name] = value; }
  removeAttribute(name: string) { delete this.attributes[name]; }
  addEventListener(name: string, callback: (e: unknown) => void) {
    this.handlers[name] = () => callback({ preventDefault() {}, stopPropagation() {} });
  }
  remove() {
    if (this.parentElement) this.parentElement.children = this.parentElement.children.filter(n => n !== this);
    this.parentElement = null;
  }
  appendChild(child: EntryNode) { this.insertBefore(child, null); }
  insertBefore(child: EntryNode, next: EntryNode | null) {
    child.remove();
    this.children.splice(next ? this.children.indexOf(next) : this.children.length, 0, child);
    child.parentElement = this;
    this.inserts++;
  }
  closest(selector: string): EntryNode | null {
    if (selector === '[role="menubar"]' && this.attributes.role === "menubar") return this;
    return this.parentElement?.closest(selector) ?? null;
  }
  querySelector(selector: string): EntryNode | null {
    if (selector === ".codex-plus-titlebar-status") return this.indicator;
    return this.children.find(n => n.tag === selector) ?? null;
  }
  querySelectorAll(selector: string): EntryNode[] {
    return this.children.filter(n => selector === '[role="menuitem"]' && n.attributes.role === "menuitem");
  }
}

function harness() {
  const root = new EntryNode();
  const bar = new EntryNode();
  const menu = new EntryNode();
  menu.attributes.role = "menubar";
  const help = new EntryNode("button");
  help.id = "application-menu-trigger-help-menu";
  help.attributes = { role: "menuitem", tabindex: "-1", "aria-haspopup": "menu" };
  menu.appendChild(help);
  bar.appendChild(menu);
  root.appendChild(bar);
  const sidebar = new EntryNode();
  sidebar.id = "codex-plus-sidebar-nav";
  root.appendChild(sidebar);
  const find = (node: EntryNode, id: string): EntryNode | null => node.id === id ? node : node.children.map(n => find(n, id)).find(Boolean) ?? null;
  let open = false;
  let activeEntry = "home";
  let opened = 0;
  const document = {
    getElementById: (id: string) => find(root, id),
    createElement: (tag: string) => new EntryNode(tag),
    querySelector: (selector: string) => selector === '[role="menubar"]'
      ? (menu.parentElement?.parentElement === root ? menu : null)
      : selector === ".codex-plus-page-overlay" && open ? new EntryNode() : null,
    querySelectorAll: () => [],
  };
  const start = source.indexOf("  function updateCodexPlusTitlebarStatus(");
  const end = source.indexOf("  function installCodexPlusNavigationEntries(", start);
  const activeStart = source.indexOf("  function setCodexPlusSidebarNavActive(");
  const activeEnd = source.indexOf("  function positionCodexPlusPage(", activeStart);
  const navEnd = source.indexOf("\n  const codexPluginRemoteOnlyMarketplaceKinds", end);
  const sidebarStart = source.indexOf("  function installCodexPlusSidebarNavigation(");
  const sidebarEnd = source.indexOf("  function removeCodexPlusRailNavigation(", sidebarStart);
  assert.ok(start >= 0 && end > start && activeEnd > activeStart);
  const runtime = new Function("document", "openCodexPlusPage", "closeCodexPlusPage", "codexPlusActiveEntry", `
    const codexPlusTitlebarEntryId = 'codex-plus-titlebar-entry';
    const codexPlusSidebarNavId = 'codex-plus-sidebar-nav';
    const codexPlusRailNavId = 'codex-plus-rail-nav';
    const codexPlusRailExtensionsId = 'codex-plus-rail-extensions';
    const codexPlusRailSponsorId = 'codex-plus-rail-sponsor';
    const codexPlusPageClass = 'codex-plus-page-overlay';
    const codexPlusBackendStatus = {status:'ok'};
    const codexPlusMenuId = "codex-plus-menu";
    function positionCodexPlusPage() {}
    function installCodexPlusRailNavigation() { return false; }
    function removeCodexPlusRailNavigation() {}
    function detachCodexPlusSidebarNavigation() { document.getElementById(codexPlusSidebarNavId)?.remove(); }
    ${source.slice(activeStart, activeEnd)}
    ${source.slice(start, end)}
    ${source.slice(sidebarStart, sidebarEnd)}
    ${source.slice(end, navEnd)}
    return {install:installCodexPlusTitlebarEntry, status:updateCodexPlusTitlebarStatus,
      active:setCodexPlusSidebarNavActive, navigate:installCodexPlusNavigationEntries};
  `)(document, () => { open = true; activeEntry = "home"; opened++; }, () => { open = false; },
    () => open ? activeEntry : null) as {
    install(): boolean; status(value: string): void; active(value: boolean): void; navigate(): void;
  };
  return { ...runtime, root, bar, menu, help, sidebar, document, opened: () => opened, isOpen: () => open,
    openOther: (entry: string) => { open = true; activeEntry = entry; },
    entry: () => document.getElementById("codex-plus-titlebar-entry")!,
  };
}

it("mounts once after the menu, removes the old entry and leaves native keyboard semantics intact", () => {
  const h = harness();
  assert.equal(h.install(), true);
  const entry = h.entry();
  const button = entry.querySelector("button")!;
  assert.equal(h.menu.nextElementSibling, entry);
  assert.equal(entry.parentElement, h.bar);
  assert.equal(h.sidebar.parentElement, h.root);
  assert.equal(button.attributes.role, undefined);
  assert.equal(button.attributes.tabindex, undefined);
  assert.equal(button.attributes["aria-haspopup"], undefined);
  assert.deepEqual(h.help.attributes, {role:"menuitem", tabindex:"-1", "aria-haspopup":"menu"});
  const inserts = h.bar.inserts;
  for (let i = 0; i < 50; i++) h.install();
  assert.equal(h.bar.inserts, inserts);
  button.handlers.click();
  assert.equal(h.opened(), 1);
  assert.equal(h.isOpen(), true);
  button.handlers.click();
  assert.equal(h.isOpen(), false);
});

it("recovers after the host menu is reparented or replaced", () => {
  const h = harness();
  h.install();
  const entry = h.entry();
  const replacement = new EntryNode();
  h.root.appendChild(replacement);
  replacement.appendChild(h.menu);
  h.install();
  assert.equal(h.entry(), entry);
  assert.equal(entry.parentElement, replacement);
  replacement.remove();
  assert.equal(h.install(), false);
  h.root.appendChild(replacement);
  h.install();
  assert.equal(h.document.getElementById("codex-plus-titlebar-entry"), entry);
});

it("reports actual backend states without losing active state", () => {
  const h = harness();
  h.install();
  const button = h.entry().querySelector("button")!;
  for (const status of ["ok", "checking", "failed"]) {
    h.status(status);
    assert.equal(button.indicator!.dataset.status, status);
    assert.equal(button.attributes["aria-label"], button.title);
  }
  h.active(true);
  assert.equal(button.attributes["aria-expanded"], "true");
  h.status("ok");
  assert.equal(button.dataset.active, "true");
});

it("keeps a mounted and synchronized titlebar entry while the host rail is absent", () => {
  const h = harness();
  h.openOther("extensions");
  for (let index = 0; index < 50; index++) {
    h.navigate();
    assert.equal(h.entry()?.parentElement, h.bar);
    assert.equal(h.entry().querySelector("button")?.attributes["aria-current"], undefined);
  }
  assert.equal(h.sidebar.parentElement, null);
  h.entry().querySelector("button")!.handlers.click();
  h.navigate();
  assert.equal(h.entry().querySelector("button")!.attributes["aria-current"], "page");
});

it("uses the native menubar structure when the help trigger identifier changes", () => {
  const h = harness();
  h.help.id = "renamed-native-menu";
  assert.equal(h.install(), true);
  assert.equal(h.menu.nextElementSibling, h.entry());
  assert.equal(h.entry().querySelector("button")?.attributes.role, undefined);
});

it("allows the existing sidebar fallback when the host has no application menubar", () => {
  const h = harness();
  h.menu.remove();
  assert.equal(h.install(), false);
  assert.equal(h.entry(), null);
  assert.equal(h.sidebar.parentElement, h.root);
});

it("switches from another injected page to home without closing that page first", () => {
  const h = harness();
  h.install();
  const button = h.entry().querySelector("button")!;
  for (const entry of ["extensions", "sponsor"]) {
    h.openOther(entry);
    h.install();
    assert.equal(button.dataset.active, "false");
    assert.equal(button.attributes["aria-expanded"], "false");
    button.handlers.click();
    assert.equal(h.isOpen(), true);
    h.install();
    assert.equal(button.dataset.active, "true");
  }
  assert.equal(h.opened(), 2);
});

it("defines titlebar entry functions only in the navigation fragment", async () => {
  const fragments = new URL("../../../assets/inject/renderer-inject/", import.meta.url);
  const navigation = await readFile(new URL("50-navigation.js", fragments), "utf8");
  const settings = await readFile(new URL("40-backend-settings.js", fragments), "utf8");
  for (const name of ["installCodexPlusTitlebarEntry", "updateCodexPlusTitlebarStatus"]) {
    assert.ok(navigation.includes(`function ${name}(`));
    assert.ok(!settings.includes(`function ${name}(`));
  }
});

it("keeps the upstream rail pages while selecting exactly one home-entry location", () => {
  const start = source.indexOf("  function installCodexPlusNavigationEntries()");
  const end = source.indexOf("\n  const codexPluginRemoteOnlyMarketplaceKinds", start);
  assert.ok(start >= 0 && end > start);
  for (const titlebar of [false, true]) {
    for (const rail of [false, true]) {
      const calls: unknown[] = [];
      new Function(
        "document", "installCodexPlusTitlebarEntry", "installCodexPlusRailNavigation",
        "detachCodexPlusSidebarNavigation", "removeCodexPlusRailNavigation", "installCodexPlusSidebarNavigation",
        "codexPlusActiveEntry", "setCodexPlusSidebarNavActive", "positionCodexPlusPage",
        `const codexPlusTitlebarEntryId = 'titlebar'; const codexPlusPageClass = 'page';
        ${source.slice(start, end)}\ninstallCodexPlusNavigationEntries();`,
      )(
        { getElementById: () => ({ remove: () => calls.push("remove-titlebar") }), querySelector: () => ({}) },
        () => titlebar,
        (includeHome: boolean) => { calls.push(["rail", includeHome]); return rail; },
        () => calls.push("detach-sidebar"), () => calls.push("remove-rail"),
        (installed: boolean) => calls.push(["sidebar", installed]), () => "extensions",
        (active: boolean, entry: string) => calls.push(["active", active, entry]), () => calls.push("position"),
      );
      assert.deepEqual(calls.filter(Array.isArray).find(call => call[0] === "rail"), ["rail", !titlebar]);
      assert.equal(calls.includes("remove-titlebar"), !titlebar);
      if (rail) {
        assert.ok(calls.includes("detach-sidebar"));
        assert.deepEqual(calls.filter(Array.isArray).find(call => call[0] === "active"), ["active", true, "extensions"]);
        assert.ok(calls.includes("position"));
        assert.ok(!calls.some(call => Array.isArray(call) && call[0] === "sidebar"));
      } else {
        assert.ok(calls.includes("remove-rail"));
        assert.deepEqual(calls.filter(Array.isArray).find(call => call[0] === "sidebar"), ["sidebar", titlebar]);
      }
    }
  }
});
