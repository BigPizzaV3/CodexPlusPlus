import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const fragment = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");
const share = fragment("80-session-share.js");

// 只实现夹具需要的选择器，但节点搜索、祖先与挂载都按 DOM 关系执行。
class Element {
  tagName: string;
  className = "";
  parentElement: Element | null = null;
  children: Element[] = [];
  attributes = new Map<string, string>();
  dataset: Record<string, string> = {};
  style: Record<string, string> = {};
  textContent = "";
  listeners: Record<string, Function> = {};
  visible = true;
  appendCount = 0;
  constructor(tag: string, attributes: Record<string, string> = {}) {
    this.tagName = tag.toUpperCase();
    for (const [key, value] of Object.entries(attributes)) this.setAttribute(key, value);
  }
  setAttribute(key: string, value: string) { this.attributes.set(key, value); }
  getAttribute(key: string) { return this.attributes.get(key) ?? null; }
  get isConnected(): boolean { return this.tagName === "DOCUMENT" || !!this.parentElement?.isConnected; }
  appendChild(node: Element) { node.remove(); this.children.push(node); node.parentElement = this; this.appendCount += 1; return node; }
  remove() {
    if (this.parentElement) this.parentElement.children = this.parentElement.children.filter((child) => child !== this);
    this.parentElement = null;
  }
  contains(node: Element): boolean { return node === this || this.children.some((child): boolean => child.contains(node)); }
  addEventListener(type: string, listener: Function) { this.listeners[type] = listener; }
  matches(selector: string): boolean {
    return selector.split(",").some((part) => {
      const tokens = part.trim().split(/\s+(?![^[]*\])/);
      const atom = tokens.pop()!;
      const tag = atom.match(/^[a-z]+/i)?.[0];
      if (tag && this.tagName !== tag.toUpperCase()) return false;
      for (const name of atom.matchAll(/\.([\w-]+)/g)) if (!this.className.split(/\s+/).includes(name[1])) return false;
      for (const match of atom.matchAll(/\[([\w-]+)(?:([*]?=)["']([^"']*)["'])?\]/g)) {
        const value = match[1] === "class" ? this.className : this.getAttribute(match[1]);
        if (value === null || (match[2] === "=" && value !== match[3]) || (match[2] === "*=" && !value.includes(match[3]))) return false;
      }
      if (!tokens.length) return true;
      return !!this.parentElement?.closest(tokens.join(" "));
    });
  }
  get classList() { return { contains: (name: string) => this.className.split(/\s+/).includes(name) }; }
  closest(selector: string): Element | null { return this.matches(selector) ? this : this.parentElement?.closest(selector) ?? null; }
  querySelectorAll(selector: string): Element[] {
    return this.children.flatMap((child) => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  querySelector(selector: string) { return this.querySelectorAll(selector)[0] ?? null; }
}

function fixture() {
  const document = new Element("document") as Element & { body: Element; createElement: (tag: string) => Element };
  document.body = document.appendChild(new Element("body"));
  document.createElement = (tag) => new Element(tag);
  const review = document.body.appendChild(new Element("header"));
  const reviewActions = review.appendChild(new Element("div")); reviewActions.className = "ms-auto";
  const shell = document.body.appendChild(new Element("main"));
  const header = shell.appendChild(new Element("header"));
  const actions = header.appendChild(new Element("div")); actions.className = "ms-auto";
  const anchor = actions.appendChild(new Element("button", { "data-testid": "app-shell-header-context-menu-surface" }));
  const native = actions.appendChild(new Element("button", { "aria-label": "Share" }));
  const settings = { sessionShare: true };
  const ref = { session_id: "thread-a" };
  const context = vm.createContext({ document, Element, HTMLElement: Element, settings, ref,
    codexPlusSettings: () => settings, currentSessionRef: () => ref,
    sessionShareButtonClass: "codex-session-share-button", sessionShareButtonVersion: "1",
    headerContextButtonClass: "whitespace-nowrap", selectors: { appHeader: "header" },
    visibleElement: (node: Element) => node.visible && node.isConnected,
    createSessionShare: () => {},
  });
  const helpers = ["sessionSharePlacement", "installSessionShareButton"].map((name) =>
    share.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0] ?? "").join("\n");
  vm.runInContext(helpers, context);
  const install = () => context.installSessionShareButton();
  const buttons = () => document.querySelectorAll(".codex-session-share-button");
  return { document, review, reviewActions, header, actions, anchor, native, settings, ref, install, buttons };
}

test("share mounts only in the active conversation toolbar despite earlier review headers", () => {
  const f = fixture(); f.install();
  assert.equal(f.buttons().length, 1);
  assert.ok(f.buttons()[0].parentElement === f.actions);
  assert.equal(f.buttons()[0].getAttribute("data-codex-plus-ext"), "session-share");
  const appends = f.actions.appendCount;
  f.install(); assert.equal(f.buttons().length, 1); assert.equal(f.actions.appendCount, appends);
});

test("missing or ambiguous conversation anchors remove stale buttons without a body/header fallback", () => {
  const f = fixture(); f.install(); f.anchor.remove(); f.install();
  assert.equal(f.buttons().length, 0);
  f.actions.appendChild(f.anchor);
  f.reviewActions.appendChild(new Element("button", { "data-testid": "app-shell-header-context-menu-surface" }));
  f.install(); assert.equal(f.buttons().length, 0);
});

test("share toggle and session navigation remove and restore a single button", () => {
  const f = fixture(); f.install();
  f.settings.sessionShare = false; f.install(); assert.equal(f.buttons().length, 0);
  f.settings.sessionShare = true; f.install(); assert.equal(f.buttons().length, 1);
  f.ref.session_id = ""; f.install(); assert.equal(f.buttons().length, 0);
  f.ref.session_id = "thread-b"; f.install(); assert.equal(f.buttons().length, 1);
});

test("header replacement and absent native Share keep the button in the recognized toolbar", () => {
  const f = fixture(); f.native.remove(); f.install();
  assert.ok(f.buttons()[0].parentElement === f.actions);
  const next = f.document.body.appendChild(new Element("header"));
  const group = next.appendChild(new Element("div")); group.className = "ms-auto";
  group.appendChild(f.anchor); f.header.remove(); f.install();
  assert.equal(f.buttons().length, 1); assert.ok(f.buttons()[0].parentElement === group);
  group.remove(); f.install(); assert.equal(f.buttons().length, 0);
});

test("modern header anchors work but invisible or extension anchors cannot own sharing", () => {
  const f = fixture(); f.header.tagName = "DIV"; f.header.setAttribute("data-app-shell-header-edge-scroll", "");
  f.install(); assert.ok(f.buttons()[0].parentElement === f.actions);
  f.reviewActions.setAttribute("data-codex-plus-ext", "example-script");
  f.reviewActions.appendChild(new Element("button", { "data-testid": "app-shell-header-context-menu-surface" }));
  f.install(); assert.equal(f.buttons().length, 1);
  f.anchor.visible = false; f.install(); assert.equal(f.buttons().length, 0);
});

test("extension Share controls cannot redirect the native session button into their toolbar", () => {
  const f = fixture(); f.native.remove();
  const extension = f.header.appendChild(new Element("div", { "data-codex-plus-ext": "example-script" }));
  extension.className = "ms-auto";
  extension.appendChild(new Element("button", { "aria-label": "Share" }));
  f.install(); assert.equal(f.buttons().length, 1); assert.ok(f.buttons()[0].parentElement === f.actions);
});

test("a recognized header without a unique action group leaves native controls unchanged", () => {
  const f = fixture(); f.native.remove(); f.actions.className = "";
  f.install(); assert.equal(f.buttons().length, 0);
  f.actions.className = "ms-auto";
  const second = f.header.appendChild(new Element("div")); second.className = "ms-auto";
  f.install(); assert.equal(f.buttons().length, 0);
  assert.deepEqual(f.header.style, {}); assert.deepEqual(f.actions.style, {});
});

test("share backend defaults preserve old installations and obey master gating", () => {
  const style = fragment("10-style.js");
  const start = style.indexOf("  function defaultCodexPlusSettings()");
  const end = style.indexOf("  // Dream skin runtime", start);
  const context = vm.createContext({ window: {}, localStorage: { getItem: () => "{}" },
    codexPlusBackendSettings: {}, conversationViewDefaultWidth: 900, codexPlusSettingsKey: "settings" });
  vm.runInContext(style.slice(start, end), context);
  assert.equal(context.codexPlusSettings().sessionShare, true);
  context.codexPlusBackendSettings = { enhancementsEnabled: true, codexAppSessionShare: false };
  assert.equal(context.codexPlusSettings().sessionShare, false);
  context.codexPlusBackendSettings = { enhancementsEnabled: false, codexAppSessionShare: true };
  assert.equal(context.codexPlusSettings().sessionShare, false);
});

test("the in-app share toggle persists a backend update rather than a temporary local preference", async () => {
  const style = fragment("10-style.js");
  const writes: unknown[] = [];
  const context = vm.createContext({ window: {}, codexPlusBackendSettings: {}, conversationViewDefaultWidth: 900,
    codexPlusSettingsKey: "settings", localStorage: { getItem: () => "{}", setItem: () => { throw new Error("must use backend"); } },
    setBackendSetting: async (key: string, value: unknown) => { writes.push([key, value]); },
    loadBackendSettings: async () => {}, syncStepwisePanel: () => {},
  });
  const start = style.indexOf("  function defaultCodexPlusSettings()");
  const end = style.indexOf("  // Dream skin runtime", start);
  const setterStart = style.indexOf("  function setCodexPlusSetting(");
  const setterEnd = style.indexOf("  function syncStepwisePanel(", setterStart);
  vm.runInContext(`${style.slice(start, end)}\n${style.slice(setterStart, setterEnd)}`, context);
  context.setCodexPlusSetting("sessionShare", false); await Promise.resolve();
  assert.deepEqual(writes, [["codexAppSessionShare", false]]);
  assert.ok(fragment("40-backend-settings.js").includes('data-codex-plus-setting="sessionShare"'));
});

test("backend refresh synchronizes share immediately and skips failed heartbeat reads", async () => {
  const source = fragment("40-backend-settings.js");
  let loaded = true, installs = 0;
  const context = vm.createContext({ codexPlusSettings: () => ({ conversationView: false }),
    loadBackendSettingsState: async () => loaded, syncOfficialUsagePolicy: () => {},
    syncCodexPlusTypingEffects: () => {}, renderCodexPlusMenu: () => {}, refreshConversationView: () => {},
    installSessionShareButton: () => { installs += 1; }, runScanStep: (fn: () => void) => fn(),
  });
  const start = source.indexOf("  let syncBackendSettingsInFlight");
  const end = source.indexOf("  async function setBackendSetting(", start);
  vm.runInContext(source.slice(start, end), context);
  await context.syncBackendSettingsFromHeartbeat(); assert.equal(installs, 1);
  loaded = false; await context.syncBackendSettingsFromHeartbeat(); assert.equal(installs, 1);
});

test("optimistic backend writes immediately clean up sharing before their response arrives", async () => {
  const source = fragment("40-backend-settings.js");
  const f = fixture(); f.install();
  let finish: (value: unknown) => void = () => {};
  const context = vm.createContext({ codexPlusBackendSettingsSeq: 0, codexPlusBackendSettings: {},
    postJson: () => new Promise((resolve) => { finish = resolve; }),
    refreshCodexPlusBackendToggles: () => { f.settings.sessionShare = context.codexPlusBackendSettings.codexAppSessionShare; f.install(); },
  });
  const start = source.indexOf("  async function setBackendSetting(");
  const end = source.indexOf("  function refreshCodexPlusBackendToggles(", start);
  vm.runInContext(source.slice(start, end), context);
  const saved = context.setBackendSetting("codexAppSessionShare", false);
  assert.equal(f.buttons().length, 0);
  finish({ codexAppSessionShare: false }); await saved;
  assert.equal(f.buttons().length, 0);
});
