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
  let opened = 0;
  const document = {
    getElementById: (id: string) => find(root, id),
    createElement: (tag: string) => new EntryNode(tag),
    querySelector: () => open ? new EntryNode() : null,
  };
  const start = source.indexOf("  function updateCodexPlusTitlebarStatus(");
  const end = source.indexOf("  function installCodexPlusSidebarNavigation(", start);
  const activeStart = source.indexOf("  function setCodexPlusSidebarNavActive(");
  const activeEnd = source.indexOf("  function positionCodexPlusPage(", activeStart);
  assert.ok(start >= 0 && end > start && activeEnd > activeStart);
  const runtime = new Function("document", "openCodexPlusPage", "closeCodexPlusPage", `
    const codexPlusTitlebarEntryId = 'codex-plus-titlebar-entry';
    const codexPlusSidebarNavId = 'codex-plus-sidebar-nav';
    const codexPlusPageClass = 'codex-plus-page-overlay';
    const codexPlusBackendStatus = {status:'ok'};
    let codexPlusBridgeFailureCount = 0;
    const CODEX_PLUS_BRIDGE_FAILURE_THRESHOLD = 3;
    function positionCodexPlusPage() {}
    ${source.slice(activeStart, activeEnd)}
    ${source.slice(start, end)}
    return {install:installCodexPlusTitlebarEntry, status:updateCodexPlusTitlebarStatus,
      active:setCodexPlusSidebarNavActive, degrade:()=>{codexPlusBridgeFailureCount=3;}};
  `)(document, () => { open = true; opened++; }, () => { open = false; }) as {
    install(): boolean; status(value: string): void; active(value: boolean): void; degrade(): void;
  };
  return { ...runtime, root, bar, menu, help, sidebar, document, opened: () => opened, isOpen: () => open,
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
  assert.equal(h.sidebar.parentElement, null);
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

it("reports normal, checking, failed and degraded states without losing active state", () => {
  const h = harness();
  h.install();
  const button = h.entry().querySelector("button")!;
  for (const status of ["ok", "checking", "failed", "degraded"]) {
    h.status(status);
    assert.equal(button.indicator!.dataset.status, status);
    assert.equal(button.attributes["aria-label"], button.title);
  }
  h.active(true);
  assert.equal(button.attributes["aria-expanded"], "true");
  h.status("ok");
  assert.equal(button.dataset.active, "true");
  h.degrade();
  h.install();
  assert.equal(button.indicator!.dataset.status, "degraded");
});

it("allows the existing sidebar fallback when the host has no application menubar", () => {
  const h = harness();
  h.menu.remove();
  assert.equal(h.install(), false);
  assert.equal(h.entry(), null);
  assert.equal(h.sidebar.parentElement, h.root);
});
