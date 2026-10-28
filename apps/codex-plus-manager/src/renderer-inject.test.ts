import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { readFile } from "node:fs/promises";

describe("session more menu lifecycle", () => {
  async function runtime() {
    const source = await readFile(
      new URL("../../../assets/inject/renderer-inject/90-action-groups.js", import.meta.url), "utf8",
    );
    const menus: ReturnType<typeof fixture>[] = [];
    const timers = new Map<number, () => void>();
    let timerId = 0;
    let menuQueries = 0;
    const events = () => {
      const listeners = new Map<string, () => void>();
      return {
        listeners,
        addEventListener: (name: string, callback: () => void) => listeners.set(name, callback),
        removeEventListener: (name: string) => listeners.delete(name),
      };
    };
    function fixture() {
      const classes = new Set(["codex-session-more-open"]);
      const button = { expanded: "true", setAttribute: (_: string, value: string) => { button.expanded = value; } };
      const deleteButton = { dataset: { codexDeleteVersion: "7" } };
      const group = {
        ...events(), isConnected: true, hasDelete: true, dataset: { codexActionGroupVersion: "7", codexMoreMenuKey: "export" },
        __codexSessionMoreMenu: null as unknown,
        querySelector: (selector: string) => selector === ".more-button" ? button
          : selector === ".delete-button" && group.hasDelete ? deleteButton : null,
        matches: () => false, remove: () => { group.isConnected = false; },
      };
      const row = {
        isConnected: true, dataset: {}, querySelectorAll: () => [group],
        contains: (node: unknown) => node === group, classList: { remove: (name: string) => classes.delete(name) },
      };
      const menu = {
        ...events(), hidden: false, removed: false, isConnected: true, classes, row, group, button,
        __codexSessionMoreRow: row as typeof row | null,
        __codexSessionMoreGroup: group as typeof group | null,
        __codexSessionMoreCleanup: null as (() => void) | null,
        remove: () => { menu.removed = true; menu.isConnected = false; },
        matches: () => false, contains: () => false,
      };
      group.__codexSessionMoreMenu = menu;
      return menu;
    }
    const api = new Function("document", "window", "codexPlusSettings", "codexPlusBackendSettings",
      "moreMenuClass", "moreButtonClass", "buttonClass", "actionGroupClass", "exportButtonClass",
      "codexDeleteVersion", "codexActionGroupVersion", "actionGroupFromRow", "sessionRefFromRow",
      "codexPlusRegistry", "codexPlusExtensionItems",
      `${source.split("  function tryAttachButton(row)")[0]}
       return { pruneSessionMoreMenus, pruneRemovedSessionActions, attachButton, closeSessionMoreMenus, installSessionMoreMenuAutoClose };`,
    )(
      { querySelectorAll: () => { menuQueries++; return menus.filter((menu) => !menu.removed); }, activeElement: null },
      {
        setTimeout: (callback: () => void) => {
          const id = ++timerId;
          timers.set(id, () => { timers.delete(id); callback(); });
          return id;
        },
        clearTimeout: (id: number) => timers.delete(id),
      },
      () => ({ sessionDelete: true, markdownExport: true }), { enhancementsEnabled: true },
      "more-menu", "more-button", "delete-button", "actions", "export-button", "7", "7",
      (row: { querySelectorAll: () => unknown[] }) => row.querySelectorAll()[0], () => ({}),
      { rowActions: new Map() }, (registry: Map<string, unknown>) => [...registry.values()],
    );
    return {
      ...api, timers, menuQueries: () => menuQueries,
      fixture: () => {
        const menu = fixture();
        menus.push(menu);
        return menu;
      },
    };
  }

  it("disposes detached owners and groups without removing connected or synchronously moved rows", async () => {
    const api = await runtime();
    const live = api.fixture();
    const detachedRow = api.fixture();
    const detachedGroup = api.fixture();
    const unrelatedGroup = api.fixture();
    const moved = api.fixture();
    detachedRow.row.isConnected = false;
    detachedGroup.group.isConnected = false;
    unrelatedGroup.row.contains = () => false;
    moved.row.isConnected = false;
    moved.row.isConnected = true;
    assert.equal(api.pruneSessionMoreMenus(), true);
    assert.equal(live.removed, false);
    assert.equal(moved.removed, false);
    for (const menu of [detachedRow, detachedGroup, unrelatedGroup]) {
      assert.equal(menu.removed, true);
      assert.equal(menu.hidden, true);
      assert.equal(menu.button.expanded, "false");
      assert.equal(menu.classes.has("codex-session-more-open"), false);
      assert.equal(menu.__codexSessionMoreRow, null);
      assert.equal(menu.__codexSessionMoreGroup, null);
      assert.equal(menu.group.__codexSessionMoreMenu, null);
    }
    assert.equal(api.pruneSessionMoreMenus(), false);
    assert.equal(live.removed, false);
  });

  it("only reuses action groups with a connected menu owned by the same row and group", async () => {
    const api = await runtime();
    const live = api.fixture();
    api.attachButton(live.row);
    assert.equal(live.group.isConnected, true);
    for (const damage of ["disposed", "menu-removed", "different-owner", "different-group"]) {
      const menu = api.fixture();
      if (damage === "disposed") {
        menu.row.isConnected = false;
        api.pruneSessionMoreMenus();
        menu.row.isConnected = true;
      } else if (damage === "menu-removed") menu.isConnected = false;
      else if (damage === "different-owner") menu.__codexSessionMoreRow = live.row;
      else menu.__codexSessionMoreGroup = live.group;
      api.attachButton(menu.row);
      assert.equal(menu.group.isConnected, false, `${damage} must rebuild the retained group`);
    }
  });

  it("requests repair for a missing delete button but leaves complete action groups alone", async () => {
    const api = await runtime();
    const menu = api.fixture();
    assert.equal(api.pruneSessionMoreMenus(), false);
    menu.group.hasDelete = false;
    assert.equal(api.pruneSessionMoreMenus(), true);
    assert.equal(menu.removed, true);
    assert.equal(api.pruneSessionMoreMenus(), false);
  });

  it("ignores unrelated removals and checks action ancestors and removed body menus", async () => {
    const api = await runtime();
    const menu = api.fixture();
    const node = (matches: boolean, descendant: boolean) => ({
      nodeType: 1, matches: () => matches, querySelector: () => descendant ? {} : null,
      querySelectorAll: () => [],
    });
    for (let i = 0; i < 200; i++) api.pruneRemovedSessionActions([{ removedNodes: [node(false, false)] }]);
    assert.equal(api.menuQueries(), 0, "streamed prose must not sweep body menus");
    assert.equal(api.pruneRemovedSessionActions([{ removedNodes: [node(false, true)] }]), false);
    assert.equal(api.menuQueries(), 1, "a synchronous move still checks final owner state");
    menu.row.isConnected = false;
    api.pruneRemovedSessionActions([{ removedNodes: [node(false, true)] }]);
    assert.equal(menu.removed, true, "sidebar ancestor removal must release owner references");
    const removed = api.fixture();
    removed.remove();
    Object.assign(removed, node(true, false));
    assert.equal(api.pruneRemovedSessionActions([{ removedNodes: [removed] }]), true);
    assert.equal(removed.__codexSessionMoreRow, null, "body menu removal must also be disposed");
  });

  it("resets expansion state when another menu opens or all menus close", async () => {
    const api = await runtime();
    const first = api.fixture();
    const second = api.fixture();
    api.closeSessionMoreMenus(second);
    assert.equal(first.hidden, true);
    assert.equal(first.button.expanded, "false");
    assert.equal(second.hidden, false);
    api.closeSessionMoreMenus();
    assert.equal(second.hidden, true);
    assert.equal(second.button.expanded, "false");
  });

  it("coalesces auto-close timers and cancels listeners and timers on disposal", async () => {
    const api = await runtime();
    const menu = api.fixture();
    api.installSessionMoreMenuAutoClose(menu.row, menu);
    menu.group.listeners.get("pointerleave")!();
    menu.listeners.get("focusout")!();
    assert.equal(api.timers.size, 1);
    for (const callback of api.timers.values()) callback();
    assert.equal(menu.hidden, true);
    assert.equal(menu.button.expanded, "false");
    menu.listeners.get("pointerleave")!();
    menu.row.isConnected = false;
    api.pruneSessionMoreMenus();
    assert.equal(api.timers.size, 0);
    assert.equal(menu.group.listeners.size, 0);
    assert.equal(menu.listeners.size, 0);
    assert.equal(menu.__codexSessionMoreCleanup, null);
  });

  it("prunes removed elements before pending, hidden and relevance scan guards", async () => {
    const schedule = await readFile(
      new URL("../../../assets/inject/renderer-inject/98-scan-schedule.js", import.meta.url), "utf8",
    );
    const source = schedule.slice(schedule.indexOf("  function scheduleScan("), schedule.indexOf("  const onScanVisibilityChange"));
    let pruned = 0;
    let relevanceChecks = 0;
    const window = { __codexSessionDeleteScanPending: true };
    const scheduleScan = new Function("window", "document", "pruneRemovedSessionActions", "shouldScheduleScan",
      `${source}\nreturn scheduleScan;`,
    )(window, { hidden: true }, (mutations: Array<{ removedNodes: Array<{ nodeType: number }> }>) => {
      if (mutations.some((mutation) => mutation.removedNodes.some((node) => node.nodeType === 1))) pruned++;
    }, () => { relevanceChecks++; return false; });
    scheduleScan([{ removedNodes: [{ nodeType: 1 }] }]);
    assert.equal(pruned, 1);
    assert.equal(relevanceChecks, 0);
    window.__codexSessionDeleteScanPending = false;
    scheduleScan([{ removedNodes: [{ nodeType: 1 }] }]);
    assert.equal(pruned, 2);
    assert.equal(relevanceChecks, 1);
    scheduleScan([{ removedNodes: [{ nodeType: 3 }] }]);
    assert.equal(pruned, 2);
    const view = await readFile(
      new URL("../../../assets/inject/renderer-inject/95-conversation-view.js", import.meta.url), "utf8",
    );
    assert.match(view, /function scan\(\) \{\s*pruneSessionMoreMenus\(\);\s*if \(document.hidden\)/);
  });

  it("schedules one repair for a removed owned group even when normal relevance filtering skips it", async () => {
    const schedule = await readFile(
      new URL("../../../assets/inject/renderer-inject/98-scan-schedule.js", import.meta.url), "utf8",
    );
    const source = schedule.slice(schedule.indexOf("  function scheduleScan("), schedule.indexOf("  const onScanVisibilityChange"));
    const window = { __codexSessionDeleteScanPending: false };
    const timers: Array<() => void> = [];
    let needsScan = true;
    const scheduleScan = new Function("window", "document", "pruneRemovedSessionActions", "shouldScheduleScan",
      "setTimeout", "runScheduledScan", `${source}\nreturn scheduleScan;`,
    )(window, { hidden: false }, () => needsScan, () => false,
      (callback: () => void) => { timers.push(callback); return timers.length; },
      () => { window.__codexSessionDeleteScanPending = false; });
    const removed = [{ removedNodes: [{ nodeType: 1 }] }];
    scheduleScan(removed);
    scheduleScan(removed);
    assert.equal(timers.length, 1);
    timers[0]();
    needsScan = false;
    scheduleScan(removed);
    assert.equal(timers.length, 1, "normal removal of owned UI must not feed a scan loop");
  });
});

describe("session row action buttons", () => {
  // 只够 attachButton 用的极简 DOM：支持复合简单选择器（tag / .class / [attr] / [attr="v"]）
  // 和逗号并列，不支持后代组合符。innerHTML 只识别本模块会写入的两种结构。
  class FakeElement {
    tagName: string;
    className = "";
    attributes = new Map<string, string>();
    children: FakeElement[] = [];
    parentElement: FakeElement | null = null;
    dataset: Record<string, string> = {};
    listeners = new Map<string, unknown>();
    hidden = false;
    type = "";
    text = "";
    style = { setProperty: () => {} };
    [key: string]: unknown;
    constructor(tag: string) { this.tagName = tag.toUpperCase(); }
    get isConnected(): boolean { return this.parentElement ? this.parentElement.isConnected : this.tagName === "BODY"; }
    get textContent(): string { return this.text + this.children.map((child) => child.textContent).join(""); }
    set textContent(value: string) { this.text = value; this.children = []; }
    set innerHTML(html: string) {
      this.children = [];
      this.text = "";
      const inner = html.match(/^<span class="codex-session-action-inner">(<svg[^>]*>)/);
      if (inner) {
        const span = this.appendChild(new FakeElement("span"));
        span.className = "codex-session-action-inner";
        const svg = span.appendChild(new FakeElement("svg"));
        for (const [, name, value] of inner[1].matchAll(/([\w-]+)="([^"]*)"/g)) svg.setAttribute(name, value);
        return;
      }
      this.text = html.replace(/<[^>]*>/g, "");
    }
    setAttribute(name: string, value: string) { this.attributes.set(name, String(value)); }
    getAttribute(name: string) { return this.attributes.get(name) ?? null; }
    hasAttribute(name: string) { return this.attributes.has(name); }
    removeAttribute(name: string) { this.attributes.delete(name); }
    appendChild(child: FakeElement) { child.parentElement = this; this.children.push(child); return child; }
    remove() {
      if (!this.parentElement) return;
      this.parentElement.children = this.parentElement.children.filter((child) => child !== this);
      this.parentElement = null;
    }
    replaceWith(next: FakeElement) {
      const parent = this.parentElement;
      if (!parent) return;
      parent.children[parent.children.indexOf(this)] = next;
      next.parentElement = parent;
      this.parentElement = null;
    }
    cloneNode(): FakeElement { return this; }
    addEventListener(name: string, callback: unknown) { this.listeners.set(name, callback); }
    removeEventListener(name: string) { this.listeners.delete(name); }
    matches(selector: string): boolean {
      return selector.split(",").some((part) => {
        const compound = part.trim();
        const tag = compound.match(/^[a-z]+/i)?.[0];
        if (tag && tag.toUpperCase() !== this.tagName) return false;
        const classes = [...compound.matchAll(/\.([\w-]+)/g)].map((match) => match[1]);
        const ownClasses = this.className.split(/\s+/);
        if (!classes.every((name) => ownClasses.includes(name))) return false;
        return [...compound.matchAll(/\[([\w-]+)(?:="([^"]*)")?\]/g)].every(([, name, value]) =>
          value === undefined ? this.hasAttribute(name) : this.getAttribute(name) === value);
      });
    }
    closest(selector: string): FakeElement | null {
      for (let node: FakeElement | null = this; node; node = node.parentElement) if (node.matches(selector)) return node;
      return null;
    }
    querySelectorAll(selector: string): FakeElement[] {
      return this.children.flatMap((child) => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)]);
    }
    querySelector(selector: string) { return this.querySelectorAll(selector)[0] ?? null; }
  }

  async function runtime(settings: { sessionDelete: boolean; markdownExport: boolean }) {
    const source = await readFile(
      new URL("../../../assets/inject/renderer-inject/90-action-groups.js", import.meta.url), "utf8",
    );
    const body = new FakeElement("body");
    const document = {
      body,
      createElement: (tag: string) => new FakeElement(tag),
      querySelectorAll: (selector: string) => body.querySelectorAll(selector),
    };
    const registry = { rowActions: new Map<string, { id: string; label: string; order: number }>() };
    const api = new Function("document", "window", "HTMLElement", "codexPlusSettings", "codexPlusBackendSettings",
      "moreMenuClass", "moreButtonClass", "buttonClass", "actionGroupClass", "exportButtonClass", "actionButtonClass",
      "codexDeleteVersion", "codexActionGroupVersion", "actionGroupFromRow", "sessionRefFromRow",
      "codexPlusRegistry", "codexPlusExtensionItems", "appendCodexPlusExtensionRowActions", "syncActionGroupLayout",
      `${source.split("  function tryAttachButton(row)")[0]}
       return { attachButton };`,
    )(
      document, { setTimeout: () => 0, clearTimeout: () => {}, requestAnimationFrame: () => 0 }, FakeElement,
      () => settings, { enhancementsEnabled: true },
      "more-menu", "more-button", "delete-button", "actions", "export-button", "action-button", "8", "8",
      (row: FakeElement) => row.querySelector(".actions"), () => ({ session_id: "thread-1" }),
      registry, (map: Map<string, { order: number }>) => [...map.values()].sort((a, b) => a.order - b.order),
      (menu: FakeElement) => registry.rowActions.forEach((item) => {
        const node = menu.appendChild(new FakeElement("button"));
        node.text = item.label;
      }),
      () => {},
    );
    const row = body.appendChild(new FakeElement("div"));
    return { ...api, body, row, registry };
  }

  it("only renders the more button when a menu item exists", async () => {
    const deleteOnly = await runtime({ sessionDelete: true, markdownExport: false });
    deleteOnly.attachButton(deleteOnly.row);
    const group = deleteOnly.row.querySelector(".actions");
    assert.ok(group, "the delete button still gets its group");
    assert.ok(group.querySelector(".delete-button"));
    assert.equal(group.querySelector(".more-button"), null, "no export and no extension action means no … button");
    assert.equal(deleteOnly.body.querySelector(".more-menu"), null);

    const nothing = await runtime({ sessionDelete: false, markdownExport: false });
    nothing.attachButton(nothing.row);
    assert.equal(nothing.row.querySelector(".actions"), null, "no delete and no menu item renders nothing");

    const exportOnly = await runtime({ sessionDelete: false, markdownExport: true });
    exportOnly.attachButton(exportOnly.row);
    assert.ok(exportOnly.row.querySelector(".more-button"));
    const menu = exportOnly.body.querySelector(".more-menu");
    assert.deepEqual(menu?.children.map((item: { textContent: string }) => item.textContent), ["⇩导出"]);
  });

  it("adds and removes the more button as extension row actions register and dispose", async () => {
    const api = await runtime({ sessionDelete: true, markdownExport: false });
    api.attachButton(api.row);
    assert.equal(api.row.querySelector(".more-button"), null);
    api.registry.rowActions.set("script:rowAction:a", { id: "script:rowAction:a", label: "拓展操作", order: 1000 });
    api.attachButton(api.row);
    assert.ok(api.row.querySelector(".more-button"), "a registered row action brings the … button back");
    assert.deepEqual(api.body.querySelector(".more-menu")?.children.map((item: { textContent: string }) => item.textContent), ["拓展操作"]);
    api.registry.rowActions.clear();
    api.attachButton(api.row);
    assert.equal(api.row.querySelector(".more-button"), null, "disposing the last row action removes it again");
    assert.equal(api.body.querySelector(".more-menu"), null, "its detached menu is disposed as well");
    assert.ok(api.row.querySelector(".delete-button"));
  });

  it("dresses the delete button in the native pin/archive button classes and data attributes", async () => {
    const api = await runtime({ sessionDelete: true, markdownExport: false });
    const native = api.row.appendChild(new FakeElement("button"));
    native.className = "_Button_abc pointer-events-auto";
    native.setAttribute("aria-label", "归档聊天");
    native.setAttribute("data-color", "secondary");
    native.setAttribute("data-variant", "transparent");
    native.setAttribute("data-size", "3xs");
    native.setAttribute("data-uniform", "");
    const inner = native.appendChild(new FakeElement("span"));
    inner.className = "_ButtonInner_abc";
    inner.appendChild(new FakeElement("svg")).setAttribute("viewBox", "0 0 16 16");
    api.attachButton(api.row);

    const button = api.row.querySelector(".delete-button")!;
    assert.equal(button.className, "_Button_abc pointer-events-auto action-button delete-button");
    assert.equal(button.getAttribute("data-codex-native-style"), "true");
    for (const [name, value] of [["data-color", "secondary"], ["data-variant", "transparent"], ["data-size", "3xs"], ["data-uniform", ""]]) {
      assert.equal(button.getAttribute(name), value, name);
    }
    assert.equal(button.querySelector(".codex-session-action-inner")?.className, "codex-session-action-inner _ButtonInner_abc");
    const svg = button.querySelector("svg")!;
    assert.equal(svg.getAttribute("data-codex-session-action-icon"), "stroke", "trash stays a line icon");
    assert.equal(svg.getAttribute("stroke-width"), "1.05", "fill-based native icons keep the matched light weight");
    assert.equal(button.closest("[data-codex-plus-ext]")?.getAttribute("data-codex-plus-ext"), "session-actions");
  });

  it("ignores the danger confirm button and converts a stroked native icon to the 16 grid", async () => {
    const api = await runtime({ sessionDelete: true, markdownExport: false });
    const confirm = api.row.appendChild(new FakeElement("button"));
    confirm.className = "_Button_danger";
    confirm.setAttribute("aria-label", "归档聊天");
    confirm.setAttribute("data-color", "danger");
    confirm.appendChild(new FakeElement("svg"));
    const pin = api.row.appendChild(new FakeElement("button"));
    pin.className = "_Button_pin";
    pin.setAttribute("aria-label", "Pin chat");
    const svg = pin.appendChild(new FakeElement("svg"));
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("stroke-width", "1.5");
    api.attachButton(api.row);

    const button = api.row.querySelector(".delete-button")!;
    assert.equal(button.className, "_Button_pin action-button delete-button", "never borrows the danger confirm style");
    assert.equal(button.querySelector("svg")?.getAttribute("stroke-width"), "1", "24-grid 1.5 maps to 16-grid 1");
  });

  it("keeps the fallback look aligned with native icon buttons instead of a red or filled hover", async () => {
    const style = await readFile(
      new URL("../../../assets/inject/renderer-inject/10-style.js", import.meta.url), "utf8",
    );
    const fallback = style.slice(style.indexOf(".${actionButtonClass}:not([data-codex-native-style]) {"));
    assert.match(fallback, /width: var\(--control-size-4xs, 20px\);/);
    assert.match(fallback, /border-radius: var\(--control-radius-sm, var\(--radius-sm, 6px\)\);/);
    assert.match(fallback, /color: var\(--color-text-secondary-ghost,/);
    assert.match(fallback, /width: var\(--control-icon-size-xs, 14px\);/);
    // 悬停只提亮文字色、不铺底色，与原生 transparent 变体一致。
    assert.match(style, /:not\(\[data-codex-native-style\]\)\[aria-expanded="true"\] \{\s*background: transparent;\s*color: var\(--color-text-secondary-ghost-hover,/);
    assert.doesNotMatch(style, /--codex-session-action-hover-background|--codex-session-action-color/);
    assert.doesNotMatch(style, /\.\$\{buttonClass\}:hover/);
  });
});

it("keeps background-resume scan scheduling bounded and independent of retired features", async () => {
  const schedule = await readFile(
    new URL("../../../assets/inject/renderer-inject/98-scan-schedule.js", import.meta.url),
    "utf8",
  );
  assert.match(schedule, /if \(document\.hidden\) \{\s*clearTimeout/);
  assert.match(schedule, /window\.__codexSessionDeleteScanPending/);
  assert.match(schedule, /window\.__codexSessionDeleteDeferredPending/);
  assert.match(schedule, /requestAnimationFrame/);
  assert.match(schedule, /document\.addEventListener\("visibilitychange", onScanVisibilityChange\)/);
  assert.match(schedule, /window\.__codexPlusScanScheduleCleanup/);
  assert.doesNotMatch(schedule, /scheduleZedRemoteMenuRefresh|__codexZedRemoteMenuRefresh/);
  const view = await readFile(
    new URL("../../../assets/inject/renderer-inject/95-conversation-view.js", import.meta.url),
    "utf8",
  );
  const scan = view.match(/^  function scan\(\) \{[^]*?^  \}/m)?.[0];
  assert.ok(scan);
  assert.match(scan, /if \(document\.hidden\)/);
  assert.match(scan, /scheduleDeferredScan\(\)/);
  assert.doesNotMatch(scan, /requestAnimationFrame/);
});

it("groups aggregate models by exact member identity instead of bare model name", async () => {
  const picker = await readFile(
    new URL("../../../assets/inject/renderer-inject/71-model-picker-groups.js", import.meta.url),
    "utf8",
  );
  // 同名上游模型（多个成员都发布 deepseek-v4-pro 之类）必须按「模型名 + 供应商」
  // 或完整展示名精确匹配目录，不能只按裸模型名命中目录里第一个同名成员。
  assert.match(picker, /function codexPlusModelPickerCatalogExactMap\(\)/);
  assert.match(picker, /codexPlusModelPickerNormalizeLoose\(split\.name\)/);
  assert.match(picker, /provider:\$\{split\.provider\}/);
  assert.doesNotMatch(
    picker,
    /const known = providers\.get\(codexPlusModelPickerNormalizeStrict\(split\.name\)\)/,
  );
});

const STEPWISE_FRAGMENT_PATHS = [
  "floating-panel/runtime/state.js",
  "floating-panel/core/appearance-runtime.js",
  "floating-panel/runtime/dom.js",
  "floating-panel/runtime/bridge-client.js",
  "floating-panel/runtime/answer-context.js",
  "floating-panel/stepwise/suggestions.js",
  "floating-panel/stepwise/generation.js",
  "floating-panel/runtime/lifecycle.js",
  "floating-panel/runtime/settings.js",
  "floating-panel/core/appearance.js",
  "floating-panel/core/host.js",
  "floating-panel/core/geometry.js",
  "floating-panel/core/interaction.js",
  "floating-panel/core/views.js",
  "floating-panel/outline/parser.js",
  "floating-panel/outline/navigation.js",
  "floating-panel/outline/feature.js",
  "floating-panel/outline/view.js",
  "floating-panel/core/scroll-state.js",
  "floating-panel-inject.js",
].map((name) => new URL(`../../../assets/inject/${name}`, import.meta.url));

async function readStepwiseSource() {
  const fragments = await Promise.all(STEPWISE_FRAGMENT_PATHS.map((url) => readFile(url, "utf8")));
  return `(() => {\n${fragments.join("\n")}\n})();\n`;
}

it("only reveals floating-panel content after the shell has settled open", async () => {
  const source = await readFile(
    new URL("../../../assets/inject/floating-panel/core/appearance.js", import.meta.url),
    "utf8",
  );
  const panelRules = Array.from(
    source.replace(/\$\{[^}]+\}/g, "0").matchAll(/(?:^|\n)\s*([^{}\n]*\.csw-panel)\s*\{([^}]+)\}/g),
    ([, selector, declarations]) => ({ selector: selector.trim(), declarations }),
  );
  const hidden = panelRules.find((rule) => rule.selector === ".csw-panel");
  assert.ok(hidden, "panel needs a hidden default for collapsed and interrupted states");
  assert.match(hidden.declarations, /opacity:\s*0\s*;/);
  assert.match(hidden.declarations, /visibility:\s*hidden\s*;/);
  assert.match(hidden.declarations, /transition:\s*none\s*!important\s*;/);

  const visible = panelRules.filter((rule) =>
    /opacity:\s*1\s*;|visibility:\s*visible\s*;/.test(rule.declarations),
  );
  assert.ok(visible.length > 0, "settled content must remain visible");
  for (const rule of visible) {
    assert.ok(rule.selector.includes('[data-open="true"]'), rule.selector);
    assert.ok(rule.selector.includes('[data-morphing="false"]'), rule.selector);
  }
});

function installRendererStyle(renderer: string) {
  const start = renderer.indexOf("  function installStyle()");
  const end = renderer.indexOf("\n  function defaultCodexPlusSettings", start);
  assert.ok(start >= 0 && end > start);
  const source = renderer.slice(start, end);
  const requiredNames = new Set([
    "styleId",
    "codexDeleteStyleVersion",
    ...Array.from(source.matchAll(/\$\{([A-Za-z_$][A-Za-z0-9_$]*)/g), (match) => match[1]),
  ]);
  const declarations = Array.from(requiredNames, (name) => {
    const declaration = renderer.match(new RegExp(`^  const ${name} = .+;$`, "m"))
      ?? renderer.match(new RegExp(`^  const ${name} = [\\s\\S]*?^  };$`, "m"));
    assert.ok(declaration, `missing renderer declaration for ${name}`);
    return declaration[0];
  }).join("\n");
  const appended: Array<{ dataset: Record<string, string>; id?: string; textContent?: string }> = [];
  const document = {
    getElementById() {
      return null;
    },
    createElement() {
      return { dataset: {} };
    },
    documentElement: {
      appendChild(node: (typeof appended)[number]) {
        appended.push(node);
      },
    },
  };
  const install = new Function("document", `${declarations}\n${source}\ninstallStyle();`) as (documentValue: typeof document) => void;

  install(document);
  return appended;
}

const settingsCatalogUrl = new URL("../../../crates/codex-plus-core/assets/enhancement-settings.json", import.meta.url);

/**
 * 把注入页里按清单生成设置页的那段代码切出来执行：/settings/catalog 由 postJson 桩返回，
 * 其余依赖（键映射、转义、行内输入框的取值）都取产物里的真实实现或固定值。
 */
async function settingsCatalogRuntime(
  postJson: (path: string) => Promise<unknown>,
  platform: { windows: boolean; mac: boolean } = { windows: false, mac: true },
  backendSettings: Record<string, unknown> = { enhancementsEnabled: true },
) {
  const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
  const slice = (startMarker: string, endMarker: string) => {
    const start = renderer.indexOf(startMarker);
    const end = renderer.indexOf(endMarker, start);
    assert.ok(start >= 0 && end > start, `missing ${startMarker}`);
    return renderer.slice(start, end);
  };
  const diagnostics: Array<{ name: string; detail: unknown }> = [];
  const runtime = new Function(
    "postJson", "sendCodexPlusDiagnostic", "codexPlusIsWindowsPlatform", "codexPlusIsMacPlatform", "codexPlusBackendSettings",
    `${slice("  const codexPlusBackendSettingMap = {", "  const codexPlusBackendMappedSettings")}
${slice("  function escapeHtml(value) {", "\n  }\n")}
  }
  const capacityAutoRetryIntervalCeilingSeconds = 600;
  const capacityAutoRetryMaxAttemptsCeiling = 200;
  const conversationViewMinWidth = 320;
  const conversationViewMaxAllowedWidth = 4000;
  const capacityAutoRetryIntervalSeconds = () => 0;
  const capacityAutoRetryMaxAttempts = () => 15;
  const conversationViewWidth = () => 900;
  let refreshes = 0;
  function renderBackendStatus() {}
  function refreshCodexPlusBackendToggles() {}
${slice("  let codexPlusSettingsCatalog = null;", "  function refreshCodexPlusSettingsCatalogView()")}
  function refreshCodexPlusSettingsCatalogView() { refreshes += 1; }
  return {
    load: loadCodexPlusSettingsCatalog,
    sections: renderCodexPlusSettingsCatalogSections,
    status: () => codexPlusSettingsCatalogStatus,
    refreshes: () => refreshes,
  };`,
  )(postJson, (name: string, detail: unknown) => { diagnostics.push({ name, detail }); }, platform.windows, platform.mac, backendSettings) as {
    load: () => Promise<unknown>;
    sections: () => string;
    status: () => { status: string; message: string };
    refreshes: () => number;
  };
  return { ...runtime, diagnostics };
}

/** 从生成的 HTML 里读出分组与每行的标题、说明、绑定的设置 key。 */
function parseSettingsSections(html: string) {
  return Array.from(html.matchAll(/<section class="codex-plus-settings-section" aria-label="([^"]+)"[^>]*>([\s\S]*?)<\/section>/g), ([, title, body]) => ({
    title,
    rows: Array.from(body.matchAll(/<div class="codex-plus-row"(?: data-codex-plus-setting-row="([^"]+)")?[^>]*>\s*<div><div class="codex-plus-row-title">([^<]*)<\/div><div class="codex-plus-row-description"[^>]*>([^<]*)<\/div><\/div>([\s\S]*?)(?=<div class="codex-plus-row"|$)/g), ([, key, rowTitle, description, control]) => ({
      key: key ?? "",
      title: rowTitle,
      description,
      control,
    })),
  }));
}

describe("renderer injection header compatibility", () => {
  it("does not supplement native plugin lists from retired snapshots and still refreshes after installation", async () => {
    const [navigation, plugin] = await Promise.all([
      readFile(new URL("../../../assets/inject/renderer-inject/50-navigation.js", import.meta.url), "utf8"),
      readFile(new URL("../../../assets/inject/renderer-inject/60-plugin-marketplace.js", import.meta.url), "utf8"),
    ]);
    const displayStart = navigation.indexOf("  function displayNameForPluginMarketplaceName(");
    const displayEnd = navigation.indexOf("  function restorePluginMarketplaceName(", displayStart);
    const responseStart = plugin.indexOf("  function patchPluginMarketplaceResult(");
    const responseEnd = plugin.indexOf("  if (window.__CODEX_PLUS_TEST_PLUGIN_MARKETPLACE__)", responseStart);
    const refreshStart = plugin.indexOf("  function clearPluginMarketplaceQueryCache(");
    const refreshEnd = plugin.indexOf("  function installPluginMarketplaceBridgePatch(", refreshStart);
    assert.ok(displayStart >= 0 && displayEnd > displayStart && responseStart >= 0 && responseEnd > responseStart
      && refreshStart >= 0 && refreshEnd > refreshStart);
    const invalidations: unknown[] = [];
    const window = {
      __CODEX_PLUS_PLUGIN_MARKETPLACES__: [
        { name: "native", plugins: [{ name: "cached-supplement" }] },
        { name: "cached-marketplace", plugins: [{ name: "cached-plugin" }] },
      ],
      __REACT_QUERY_CLIENT__: { invalidateQueries: (filter: unknown) => invalidations.push(filter) },
    };
    const api = new Function("window", "sendCodexPlusDiagnostic", "appServerModelRequestMethod",
      navigation.slice(displayStart, displayEnd) + plugin.slice(responseStart, responseEnd)
        + plugin.slice(refreshStart, refreshEnd) + "\nreturn { patchResponseData: patchPluginMarketplaceResponseData };",
    )(window, () => {}, (method: string) => method) as { patchResponseData: (data: unknown) => boolean };
    const plugins = [{ name: "native-plugin" }];
    const native = { marketplaces: [{ name: "native", displayName: "Native", plugins }] };
    assert.equal(api.patchResponseData({ type: "mcp-response", message: { method: "list-plugins", result: native } }), true);
    assert.equal(native.marketplaces.length, 1);
    assert.equal(native.marketplaces[0].plugins, plugins);
    assert.deepEqual(plugins.map(plugin => plugin.name), ["native-plugin"]);
    const failed: { type: string; message: { error?: unknown; result?: { marketplaces: unknown[] } } } = {
      type: "mcp-response",
      message: { error: { message: "ChatGPT authentication required for remote plugin catalog; API key auth is not supported" } },
    };
    assert.equal(api.patchResponseData(failed), true);
    assert.deepEqual(failed.message.result?.marketplaces, [], "authentication fallback must not recover cached catalogs");
    const timedOut: { type: string; message: { error?: unknown; result?: { marketplaces: unknown[] } } } = {
      type: "mcp-response",
      message: {
        error: {
          message: "failed to warm featured plugin ids cache: error sending request for url https://chatgpt.com/backend-api/plugins/featured?platform=codex: operation timed out",
        },
      },
    };
    assert.equal(api.patchResponseData(timedOut), true);
    assert.deepEqual(timedOut.message.result?.marketplaces, [], "remote plugin timeout must become a local fallback");
    api.patchResponseData({ type: "mcp-response", message: { method: "install-plugin", result: { installed: true } } });
    assert.deepEqual(invalidations, [{ queryKey: ["plugins"] }]);
    assert.doesNotMatch(navigation + plugin, /__CODEX_PLUS_PLUGIN_MARKETPLACES__|mergeLocalPluginMarketplaces|plugin_marketplace_local_merged/);
    assert.match(navigation, /codexPluginFilterSourceCache = new WeakMap/);
  });

  it("uses the same plugin enhancement settings regardless of a legacy launch mode", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const settingsStart = renderer.indexOf("  function defaultCodexPlusSettings()");
    const settingsEnd = renderer.indexOf("  // Dream skin runtime", settingsStart);
    const enabledStart = renderer.indexOf("  function codexPluginMarketplacePatchEnabled()");
    const enabledEnd = renderer.indexOf("  let cachedSessionRows", enabledStart);
    assert.ok(settingsStart >= 0 && settingsEnd > settingsStart && enabledStart >= 0 && enabledEnd > enabledStart);
    const runtime = new Function(
      "window", "localStorage", "codexPlusBackendSettings", "codexPlusBackendSettingsLoaded",
      "conversationViewDefaultWidth", "codexPlusSettingsKey",
      renderer.slice(settingsStart, settingsEnd) + renderer.slice(enabledStart, enabledEnd)
        + "\nreturn { settings: codexPlusSettings(), patchEnabled: codexPluginMarketplacePatchEnabled() };",
    );
    for (const launchMode of [undefined, "relay", "patch"]) {
      for (const enhancementsEnabled of [false, true]) {
        for (const codexAppPluginMarketplaceUnlock of [false, true]) {
          const settings = { launchMode, enhancementsEnabled, codexAppPluginMarketplaceUnlock };
          const result = runtime({}, { getItem: () => "{}" }, settings, true, 900, "fixture");
          const expected = enhancementsEnabled && codexAppPluginMarketplaceUnlock;
          assert.equal(result.settings.pluginMarketplaceUnlock, expected);
          assert.equal(result.patchEnabled, expected);
          assert.equal(runtime({}, { getItem: () => "{}" }, settings, false, 900, "fixture").patchEnabled, false);
        }
      }
    }
    assert.doesNotMatch(renderer, /codexPlusBackendSettings\.launchMode|data-relay-unneeded|兼容增强|完整增强/);
    assert.doesNotMatch(renderer, /codexMenuLocalizationScopeSelector\(\)/);
    const catalog = JSON.parse(await readFile(settingsCatalogUrl, "utf8"));
    const page = await settingsCatalogRuntime(async () => catalog);
    await page.load();
    const html = page.sections();
    assert.match(html, /data-codex-backend-setting="enhancementsEnabled"/);
    assert.match(html, /data-codex-plus-setting="pluginMarketplaceUnlock"/);
  });

  it("shows the next-step suggestion feature with its Chinese name", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const generation = await readFile(new URL("../../../assets/inject/floating-panel/stepwise/generation.js", import.meta.url), "utf8");
    // 标题来自设置项清单（与管理器同一份），注入脚本里不再写死。
    const catalog = JSON.parse(await readFile(settingsCatalogUrl, "utf8"));
    const stepwise = catalog.items.find((item: { key: string }) => item.key === "codexAppStepwiseEnabled");
    assert.equal(stepwise.title, "下一步建议");
    assert.ok(stepwise.surfaces.includes("codex"));
    assert.doesNotMatch(renderer, /悬浮球 · Stepwise/);
    for (const item of catalog.items) assert.doesNotMatch(`${item.title}${item.description}`, /Stepwise/);
    assert.match(generation, /window\.prompt\("复制下一步建议提示词", prompt\)/);
    assert.doesNotMatch(generation, /Copy Stepwise prompt/);
    assert.match(renderer, /stepwise: "codexAppStepwiseEnabled"/);
  });

  it("纯 API 会话使用当前真实 provider，不强行改成 custom", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.doesNotMatch(
      renderer,
      /if \(String\(profile\?\.relayMode \|\| ""\) === "pureApi"\) return "custom";/,
    );
    assert.match(renderer, /codexPlusBackendSettings\.activeRelayCodexProvider/);
    assert.match(renderer, /codexModelCatalog\?\.codex_model_provider/);
  });

  it("keeps the retired in-place session copy and auto-rename actions out of every menu", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    // 会话行「…」菜单与 Codex 原生「聊天操作」菜单里都不能再出现这两项。
    assert.doesNotMatch(renderer, /原地复制会话|自动重命名当前会话|已自动重命名/);
    assert.doesNotMatch(renderer, /function (?:activateSessionCopyMenuItem|createSessionCopyMenuItem|refreshSessionCopyMenuItems|sessionCopyMenuRow|sessionCopyMenuScopes|activateSessionAutoRenameMenuItem|nativeSessionRenameCapability|isNativeSessionRenameMenuItem|visibleSessionRenameDialog|rememberSessionActionTrigger)\(/);
    assert.doesNotMatch(renderer, /sessionCopyMenuItemClass|sessionCopyMenuItemVersion|sessionCopyMenuActivationTimeoutMs|sessionAutoRenameTimeoutMs|lastSessionActionTrigger/);
    assert.doesNotMatch(renderer, /__codexSessionCopyRow|__codexSessionAutoRenameRow|codexSessionCopyMenu|codexSessionAutoRenameMenu/);
    assert.doesNotMatch(renderer, /从这里创建聊天分支|Fork from here/);
    // 旧版本同页注入时挂上的触发器监听要被摘掉，不能留下悬空回调。
    assert.match(renderer, /document\.removeEventListener\("pointerdown", window\.__codexSessionActionTriggerHandler, true\);/);
    assert.doesNotMatch(renderer, /document\.addEventListener\("(?:pointerdown|click)", window\.__codexSessionActionTriggerHandler/);
  });

  it("does not install a session sharing button after sharing is retired", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    assert.doesNotMatch(renderer, /\n\s*installSessionShareButton\(\);/);
    assert.doesNotMatch(renderer, /\n\s*installSessionShareImportListener\(\);/);
  });

  it("removes the legacy Codex++ top-bar entry", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.doesNotMatch(renderer, /function installCodexPlusMenu\(\)/);
    assert.doesNotMatch(renderer, /function findNativeMenuInsertionPoint\(\)/);
    assert.doesNotMatch(renderer, /codex-plus-trigger/);
  });

  it("places Codex++ in the native sidebar and opens a main-content page", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /codexPlusSidebarNavId\s*=\s*"codex-plus-sidebar-nav"/);
    assert.match(renderer, /function installCodexPlusSidebarNavigation\(\)/);
    assert.match(renderer, /aside\.app-shell-left-panel nav\[role="navigation"\]/);
    // 新版把 role="navigation" 挪去了缩略图面板/演示目录，兜底要限定在 aside 内；
    // 而图标栏本身也是 aside 里的 <nav> 且文档顺序在前，必须显式排除，
    // 否则 "点导航就关页面" 的监听会挂到图标栏上，点自己的入口就把页面关掉。
    assert.doesNotMatch(renderer, /aside\.app-shell-left-panel nav\[role="navigation"\], nav\[role="navigation"\]/);
    assert.match(renderer, /aside\.app-shell-left-panel nav\[role="navigation"\]'\)/);
    assert.match(renderer, /!nav\.hasAttribute\("data-app-navigation-rail"\)/);
    assert.match(renderer, /const insertionButton = pluginButton \|\| navButtons\.find/);
    assert.match(renderer, /selectors\.pluginNavButton/);
    assert.match(renderer, /button\.querySelector\(selectors\.pluginSvgPath\)/);
    assert.match(renderer, /\^\(插件\|Plugins\)\$/);
    assert.match(renderer, /openCodexPlusPage\(\)/);
    assert.match(renderer, /codex-plus-page-overlay/);
    assert.match(renderer, /positionCodexPlusPage/);
    assert.match(renderer, /overlay\.remove\(\);\s*if \(pageMode\) setCodexPlusSidebarNavActive\(false\);/);
    assert.match(renderer, /function closeCodexPlusPage\(\)/);
    assert.match(renderer, /function installCodexPlusPageNavigationCloseHandler\(\)/);
    assert.match(renderer, /target\?\.closest\(selectors\.sidebarThread\)/);
    assert.match(renderer, /closeCodexPlusPageAfterNativeNavigation\(\)/);
    assert.match(renderer, /const timer = setTimeout\(\(\) => \{\s*if \(window\.__codexPlusPageNavigationCloseTimer !== timer\) return;/);
    assert.match(renderer, /if \(document\.querySelector\(`\.\$\{codexPlusPageClass\}`\) === page\) closeCodexPlusPage\(\);/);
    assert.match(renderer, /installCodexPlusNavigationEntries\(\);/);
    assert.match(renderer, /document\.querySelectorAll\(`#\$\{codexPlusMenuId\}/);
  });

  it("mounts Codex++ and 拓展 entries into the new navigation rail", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /codexPlusRailNavId\s*=\s*"codex-plus-rail-nav"/);
    assert.match(renderer, /codexPlusRailExtensionsId\s*=\s*"codex-plus-rail-extensions"/);
    assert.match(renderer, /codexPlusRailSelector\s*=\s*"nav\[data-app-navigation-rail\]"/);
    assert.match(renderer, /function installCodexPlusRailNavigation\(\)/);
    // 模板按钮取自原生 destination，clone 后必须清掉这几个属性，
    // 否则会被 Codex 的自定义/排序逻辑当成真 destination。
    assert.match(renderer, /codexPlusRailDestinationSelector\s*=\s*"\[data-sidebar-destination\]"/);
    assert.match(renderer, /button\.removeAttribute\("data-sidebar-destination"\)/);
    assert.match(renderer, /button\.removeAttribute\("aria-current"\)/);
    // 图标栏是纯图标，不塞文字标签。
    assert.match(renderer, /class="codex-plus-rail-icon"/);
    assert.doesNotMatch(renderer, /codex-plus-rail-icon"[^]*?<span class="truncate">/);
  });

  it("falls back to the legacy sidebar entry when the rail is absent", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /function installCodexPlusNavigationEntries\(\)/);
    // 两条路径互斥：有 rail 就移除旧入口，没有就移除 rail 入口。
    assert.match(renderer, /if \(installCodexPlusRailNavigation\(\)\) \{\s*detachCodexPlusSidebarNavigation\(\);\s*return;\s*\}/);
    assert.match(renderer, /removeCodexPlusRailNavigation\(\);\s*installCodexPlusSidebarNavigation\(\);/);
    assert.match(renderer, /function detachCodexPlusSidebarNavigation\(\)/);
    // 启动补扫要认两条路径任意一条已装上。
    assert.match(renderer, /const installed = document\.getElementById\(codexPlusSidebarNavId\)\s*\|\| document\.getElementById\(codexPlusRailNavId\);/);
  });

  it("opens 拓展 as a standalone page instead of a Codex++ tab", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /codexPlusExtensionsTab\s*=\s*"extensions"/);
    assert.match(renderer, /function openCodexPlusExtensions\(\)/);
    assert.match(renderer, /openCodexPlusModal\(\{ page: true, tab: codexPlusExtensionsTab \}\)/);
    // 旧名 userScripts 仍要能映射过去，避免存量调用点失效。
    assert.match(renderer, /if \(tab === "extensions" \|\| tab === "userScripts"\) return codexPlusExtensionsTab;/);
    // 弹窗必须尊重传入的初始 tab，而不是硬编码 home。
    assert.match(renderer, /selectCodexPlusTab\(initialTab\);/);
    // 激活态要靠 data-codex-plus-active-tab 区分页面，必须排在 selectCodexPlusTab 之后。
    assert.match(renderer, /selectCodexPlusTab\(initialTab\);\s*\/\/[^]*?if \(pageMode\) setCodexPlusSidebarNavActive\(true, codexPlusActiveEntry\(\) \|\| "home"\);/);
  });

  it("gives the page mode a two-column layout with its own left panel", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /function installCodexPlusPageLayout\(overlay, tab\)/);
    assert.match(renderer, /function refreshCodexPlusPageNav\(tab\)/);
    assert.match(renderer, /function renderCodexPlusPageNavItems\(tab\)/);
    // 三栏容器是 createElement + className 赋值的，不是 markup 字面量。
    assert.match(renderer, /layout\.className = "codex-plus-page-layout"/);
    assert.match(renderer, /main\.className = "codex-plus-page-main"/);
    // 只搬动已有的 modal-body，不重建里面的 data-codex-* 挂载点，
    // 否则 renderUserScripts / 各 toggle 的 querySelector 会找不到目标。
    assert.match(renderer, /main\.appendChild\(body\)/);
    // 左面板只有「拓展」需要（它是脚本列表）；主页与推荐内容是单栏内容页，
    // 页面切换交给图标栏那三个入口，所以导航容器要在条件分支里创建。
    assert.match(renderer, /if \(tab === codexPlusExtensionsTab\) \{[\s\S]{0,400}nav\.className = "codex-plus-page-nav"/);
    assert.match(renderer, /data-codex-plus-page-nav-body="true"/);
    // 布局必须在 selectCodexPlusTab 之前建好，否则刷新左面板时找不到容器。
    assert.match(renderer, /installCodexPlusPageLayout\(overlay, initialTab\);[\s\S]{0,900}selectCodexPlusTab\(initialTab\);/);
    // 左面板分组导航走同一个选中函数。
    assert.match(renderer, /const pageNav = target\?\.closest\("\[data-codex-plus-page-nav\]"\)/);
    assert.match(renderer, /selectCodexPlusTab\(pageNav\.getAttribute\("data-codex-plus-page-nav"\)\)/);
  });

  it("把三个 rail 入口都接成独立页面，并跟随 Codex 的界面缩放", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    // 推荐内容从弹窗二级 tab 提成图标栏上的一级入口。
    assert.match(renderer, /const codexPlusRailSponsorId = "codex-plus-rail-sponsor"/);
    assert.match(renderer, /const codexPlusSponsorTab = "sponsor"/);
    assert.match(renderer, /function openCodexPlusSponsor\(\)/);
    assert.match(renderer, /id: codexPlusRailSponsorId, label: "推荐内容"/);
    // 三个入口都要参与选中态映射，否则「推荐内容」亮不起来。
    assert.match(renderer, /\[codexPlusRailSponsorId, "sponsor"\],/);
    assert.match(renderer, /if \(tab === codexPlusSponsorTab\) return "sponsor";/);

    // 弹窗里的 tab 条已随入口外移删除，不该再有残留。
    assert.doesNotMatch(renderer, /codex-plus-tab-button/);

    // 「拓展」入口用 VSCode 的扩展字形，和列表默认图标同一份 path。
    assert.match(renderer, /extensions: `<svg viewBox="0 0 16 16" fill="currentColor"><path d="\$\{codexPlusDefaultExtensionIconPath\}"/);

    // 界面缩放：读 Codex 的 zoom 变量，套到 overlay 上，尺寸用 calc 反向抵消。
    assert.match(renderer, /const codexPlusWindowZoomVar = "--codex-window-zoom"/);
    assert.match(renderer, /function codexPlusWindowZoom\(\)/);
    assert.match(renderer, /function applyCodexPlusZoom\(overlay\)/);
    assert.match(renderer, /overlay\.style\.setProperty\("zoom", String\(zoom\)\)/);
    assert.match(renderer, /width: calc\(100vw \/ var\(--codex-plus-zoom, 1\)\)/);
    // 页面模式的 left 偏移要按 zoom 折算回布局坐标，CSS 里的 width 用的是同一个空间，
    // 两边量纲不一致会把右边缘算短（曾出现 62 + 1652 = 1714，视口 1727）。
    assert.match(renderer, /const layoutLeft = zoom === 1 \? left : left \/ zoom;/);
    assert.match(renderer, /overlay\.style\.setProperty\("--codex-plus-page-left", `\$\{layoutLeft\}px`\)/);
  });

  it("颜色走 Codex 的语义令牌，不写死调色板", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    // applyCodexPlusTheme 读宿主算好的令牌，取不到才回落。
    assert.match(renderer, /function applyCodexPlusTheme\(overlay\)/);
    assert.match(renderer, /--color-token-text-primary/);
    assert.match(renderer, /--color-token-text-secondary/);
    assert.match(renderer, /--color-token-bg-primary/);
    // 早先写死的 zinc 调色板不能再出现在主题函数里。
    const themeBody = renderer.slice(
      renderer.indexOf("function applyCodexPlusTheme(overlay)"),
      renderer.indexOf("function codexPlusModalTab(tab)"),
    );
    assert.ok(themeBody.length > 0, "找不到 applyCodexPlusTheme 的函数体");
    assert.ok(!themeBody.includes('text: "#f3f4f6"'), "主题函数里残留硬编码的正文色");
    assert.ok(!themeBody.includes('textSecondary: "#a1a1aa"'), "主题函数里残留硬编码的次要色");
  });

  it("lists user scripts in the 拓展 page left panel", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /function renderCodexPlusExtensionsNav\(\)/);
    // 点左面板的行 = 选中（右侧看详情），不直接切开关——
    // 开关移到详情页，避免点行就误触发启停。
    assert.match(renderer, /data-codex-extensions-select="installed:\$\{escapeHtml\(entry\.key\)\}"/);
    assert.match(renderer, /function renderCodexPlusExtensionsOverview\(\)/);
    assert.match(renderer, /data-codex-extensions-browse="market"/);
    assert.match(renderer, /codex-plus-page-nav-item-state/);
    // 每个条目都有图标，没有自带图标的用默认字形。
    assert.match(renderer, /codex-plus-page-nav-item-icon/);
    assert.match(renderer, /codexPlusDefaultExtensionIconPath/);
    // 脚本启停/状态变化要同步到左面板与详情。
    assert.match(renderer, /if \(codexPlusActiveEntry\(\) === "extensions"\) refreshCodexPlusExtensionsView\(\);/);
  });

  it("shows a native-style detail pane for the selected 拓展", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /function renderCodexPlusExtensionsDetail\(\)/);
    assert.match(renderer, /data-codex-plus-extensions-detail="true"/);
    assert.match(renderer, /function codexPlusExtensionsSelectionDetail\(\)/);
    // 详情里给已安装的开关（复用 user-script-key 委托）与卸载入口。
    assert.match(renderer, /data-codex-extensions-uninstall=/);
    assert.match(renderer, /function uninstallUserScript\(key\)/);
    assert.match(renderer, /postJson\("\/user-scripts\/delete", \{ key \}\)/);
    // 内置脚本在只读目录里，只给用户脚本提供卸载。
    assert.match(renderer, /if \(local\?\.source === "user"\)/);
    // 市场条目在详情里也能直接安装。
    assert.match(renderer, /codex-plus-extensions-detail-primary/);
    // 选中项要高亮。
    assert.match(renderer, /data-active="\$\{String\(selected\)\}"/);
  });

  it("gives 拓展 searchable 已安装 / 市场 views while retaining the search input", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /data-codex-extensions-search="true"/);
    assert.match(renderer, /function refreshCodexPlusExtensionsView\(\)/);
    assert.match(renderer, /function codexPlusExtensionsEntries\(\)/);
    assert.match(renderer, /function loadScriptMarket\(/);
    // 市场走 bridge 的两个新路由。
    assert.match(renderer, /postJson\("\/script-market\/list", \{\}\)/);
    assert.match(renderer, /postJson\("\/script-market\/install", \{ id \}\)/);
    assert.match(renderer, /data-codex-extensions-browse="installed"/);
    const start = renderer.indexOf("  function refreshCodexPlusExtensionsView()");
    const end = renderer.indexOf("  function codexPlusExtensionsSelectionDetail()", start);
    assert.ok(start >= 0 && end > start);
    // 搜索框留在主区标题栏，刷新只更新侧栏和结果，不重建输入框或抢焦点。
    assert.doesNotMatch(renderer.slice(start, end), /data-codex-extensions-search|setSelectionRange/);
  });

  it("does not install Codex++ UI in embedded browser documents", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /window\.top\s*!==\s*window/);
    assert.match(renderer, /!window\.electronBridge/);
    assert.ok(renderer.includes("/^app:\\\/\\\/\\-\\//i.test(window.location.href)"));
    assert.match(renderer, /codexPlusIsNodeTestHarness/);
  });

  it("initializes renderer styles without unresolved template identifiers", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    const appended = installRendererStyle(renderer);

    assert.equal(appended.length, 1);
    assert.match(appended[0].textContent ?? "", /#codex-plus-sidebar-nav/);
  });

  it("does not override the host document root typography or foreground", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const appended = installRendererStyle(renderer);
    const css = appended[0].textContent ?? "";
    const rootRule = css.match(/:root\s*\{([^}]*)\}/)?.[1] ?? "";

    assert.doesNotMatch(rootRule, /(?:^|;)\s*font(?:-family)?\s*:/);
    assert.doesNotMatch(rootRule, /(?:^|;)\s*color\s*:/);
    assert.match(css, /:where\([^)]*codex-plus-modal-overlay[^)]*\)\s*\{[^}]*font-family:\s*inherit;/s);
  });

  it("labels settings switches and keeps their checked state in sync while backend controls wait", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const createButton = (key: string, title: string, relayUnneeded = false) => {
      const attributes: Record<string, string> = { "data-codex-plus-setting": key };
      return {
        attributes,
        dataset: { relayUnneeded: String(relayUnneeded) } as Record<string, string>,
        disabled: false,
        getAttribute: (name: string) => attributes[name] ?? null,
        setAttribute: (name: string, value: string) => { attributes[name] = value; },
        closest: () => ({ querySelector: () => ({ textContent: title }) }),
      };
    };
    const mapped = createButton("pluginMarketplaceUnlock", "插件市场解锁");
    const local = createButton("threadScrollRestore", "切换对话保留位置");
    const unavailable = createButton("modelWhitelistUnlock", "模型白名单解锁", true);
    const buttons = [mapped, local, unavailable];
    const labelStart = renderer.indexOf('    overlay.querySelectorAll(".codex-plus-toggle[data-codex-plus-setting],');
    const labelEnd = renderer.indexOf("    const closeButton", labelStart);
    assert.ok(labelStart >= 0 && labelEnd > labelStart);
    new Function("overlay", renderer.slice(labelStart, labelEnd))({ querySelectorAll: () => buttons });
    for (const [button, label] of [[mapped, "插件市场解锁"], [local, "切换对话保留位置"], [unavailable, "模型白名单解锁"]] as const) {
      assert.equal(button.attributes.role, "switch");
      assert.equal(button.attributes["aria-label"], label);
      assert.equal(button.attributes["aria-checked"], "false");
    }

    const renderStart = renderer.indexOf("  function renderCodexPlusMenu()");
    const renderEnd = renderer.indexOf("\n  let codexPlusBackendSettings =", renderStart);
    assert.ok(renderStart >= 0 && renderEnd > renderStart);
    const settings: Record<string, boolean> = { pluginMarketplaceUnlock: true, threadScrollRestore: true, modelWhitelistUnlock: true };
    const render = new Function("document", "codexPlusSettings", "codexPlusBackendMappedSettings", "refreshConversationViewControls",
      `let codexPlusBackendSettingsLoaded = false; const codexPlusBackendSettings = {};
${renderer.slice(renderStart, renderEnd)}
return (loaded) => { codexPlusBackendSettingsLoaded = loaded; renderCodexPlusMenu(); };`,
    )(
      { querySelectorAll: (selector: string) => selector === ".codex-plus-toggle[data-codex-plus-setting]" ? buttons : [] },
      () => settings,
      new Set(["pluginMarketplaceUnlock", "modelWhitelistUnlock"]),
      () => {},
      () => {},
    ) as (loaded: boolean) => void;
    render(false);
    assert.equal(mapped.dataset.pending, "true");
    assert.equal(mapped.disabled, true);
    assert.equal(local.dataset.pending, "false");
    assert.equal(local.disabled, false);
    for (const button of buttons) assert.equal(button.attributes["aria-checked"], button.dataset.enabled);
    assert.equal(mapped.attributes["aria-checked"], "true");

    for (const key of Object.keys(settings)) settings[key] = false;
    render(true);
    for (const button of buttons) {
      assert.equal(button.dataset.pending, "false");
      assert.equal(button.dataset.enabled, "false");
      assert.equal(button.attributes["aria-checked"], "false");
    }
    assert.equal(mapped.disabled, false);
    assert.equal(local.disabled, false);
    assert.equal(unavailable.disabled, true, "a relay-unavailable switch stays disabled after settings load");
  });

  it("refreshes backend switches from enabled to disabled without losing their accessible state", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const start = renderer.indexOf("  function syncCodexPlusBackendToggleButtons()");
    const end = renderer.indexOf("\n  let codexPlusUserScripts =", start);
    assert.ok(start >= 0 && end > start);
    const settings = { enhancementsEnabled: true };
    const attributes: Record<string, string> = {};
    let attributeWrites = 0;
    const button = {
      dataset: {} as Record<string, string>,
      disabled: true,
      getAttribute: () => "enhancementsEnabled",
      setAttribute: (name: string, value: string) => { attributes[name] = value; attributeWrites += 1; },
    };
    let menuRefreshes = 0;
    const refresh = new Function("document", "codexPlusBackendSettings", "syncStepwisePanel", "runScanStep", "syncCodexPlusTypingEffects", "renderCodexPlusMenu", "scan", "syncCodexPlusSettingsCatalogDescriptions",
      `${renderer.slice(start, end)}\nreturn refreshCodexPlusBackendToggles;`,
    )(
      { querySelectorAll: () => [button] }, settings, () => {}, () => {}, () => {},
      () => { menuRefreshes += 1; }, () => {}, () => {},
    ) as () => void;
    refresh();
    assert.equal(button.dataset.enabled, "true");
    assert.equal(attributes["aria-checked"], "true");
    settings.enhancementsEnabled = false;
    refresh();
    assert.equal(button.dataset.enabled, "false");
    assert.equal(attributes["aria-checked"], "false");
    assert.equal(button.disabled, true, "backend refresh preserves the control's existing disabled state");
    assert.equal(menuRefreshes, 2, "each backend refresh also synchronizes the other settings controls");
    // 心跳每 5 秒同步一次：值没变就不写属性，避免无谓的 DOM 变更触发扫描。
    const writes = attributeWrites;
    refresh();
    assert.equal(attributeWrites, writes);
  });

  it("groups the settings homepage into labelled cards and keeps hidden service controls hidden", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const homeStart = renderer.indexOf('data-codex-plus-panel="home"');
    const homeEnd = renderer.indexOf("${renderCodexPlusExtensionMenuRows()}", homeStart);
    assert.ok(homeStart >= 0 && homeEnd > homeStart);
    // 模板里只剩清单挂载点和固定的「工具与关于」；设置分组全部按清单生成。
    const template = renderer.slice(homeStart, homeEnd);
    assert.match(template, /\$\{renderCodexPlusSettingsCatalogSections\(\)\}\s*<section class="codex-plus-settings-section" aria-label="工具与关于" data-codex-plus-settings-tools="true">/);
    const catalog = JSON.parse(await readFile(settingsCatalogUrl, "utf8"));
    const page = await settingsCatalogRuntime(async () => catalog);
    await page.load();
    const html = page.sections() + template.slice(template.indexOf('<section class="codex-plus-settings-section" aria-label="工具与关于"'));
    const sections = Array.from(html.matchAll(/<section class="codex-plus-settings-section" aria-label="([^"]+)"[^>]*>([\s\S]*?)<\/section>/g));
    const expectedGroups = catalog.groups
      .filter((group: { id: string }) => catalog.items.some((item: { group: string; surfaces: string[] }) => item.group === group.id && item.surfaces.includes("codex")))
      .map((group: { title: string }) => group.title);
    assert.deepEqual(sections.map((match) => match[1]), [...expectedGroups, "工具与关于"]);
    for (const [, title, contents] of sections) {
      assert.ok(contents.includes(`<h2 class="codex-plus-settings-section-title">${title}</h2>`));
      assert.ok(contents.includes('class="codex-plus-settings-card"'));
      assert.ok(contents.includes('class="codex-plus-row"'));
    }
    const layoutStart = renderer.indexOf("  function installCodexPlusPageLayout(");
    const layoutEnd = renderer.indexOf("  function codexPlusPageTitle(", layoutStart);
    assert.ok(layoutStart >= 0 && layoutEnd > layoutStart);
    assert.match(renderer.slice(layoutStart, layoutEnd), /if \(tab === "home"\)\s*\{[\s\S]*?body\.querySelector\("\.codex-plus-settings"\)\?\.prepend\(header\)/);
    const css = installRendererStyle(renderer)[0].textContent ?? "";
    assert.match(css, /\.codex-plus-settings\s+\.codex-plus-row\[hidden\]\s*\{\s*display:\s*none\s*;/);
  });

  it("synchronizes the typing-effect dropdown label, selection and disabled state", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const start = renderer.indexOf("  function syncCodexPlusTypingEffectDropdown(");
    const end = renderer.indexOf("  function openCodexPlusTypingEffectDropdown(", start);
    assert.ok(start >= 0 && end > start);
    const label = { textContent: "" };
    const triggerAttributes: Record<string, string> = { "aria-expanded": "true" };
    const trigger = {
      disabled: false,
      querySelector: () => label,
      setAttribute: (name: string, value: string) => { triggerAttributes[name] = value; },
    };
    const options = ["off", "rainbow", "fireworks", "stars"].map((value) => {
      const attributes: Record<string, string> = {};
      return {
        attributes,
        dataset: { codexPlusTypingEffectOption: value },
        tabIndex: -1,
        setAttribute: (name: string, next: string) => { attributes[name] = next; },
      };
    });
    const overlay = { querySelector: () => trigger, querySelectorAll: () => options };
    const menu = { hidden: false, parentElement: overlay };
    const select = { value: "rainbow", disabled: false, selectedOptions: [{ textContent: "彩虹粒子" }], closest: () => overlay };
    let cleanups = 0;
    const sync = new Function("document", "cleanup",
      `let codexPlusTypingEffectDropdownCleanup = cleanup;\n${renderer.slice(start, end)}\nreturn syncCodexPlusTypingEffectDropdown;`,
    )({ querySelector: () => menu }, () => { cleanups += 1; }) as (control: typeof select) => void;
    sync(select);
    assert.equal(label.textContent, "彩虹粒子");
    assert.equal(trigger.disabled, false);
    assert.equal(menu.hidden, false);
    for (const option of options) {
      const selected = option.dataset.codexPlusTypingEffectOption === "rainbow";
      assert.equal(option.attributes["aria-selected"], String(selected));
      assert.equal(option.tabIndex, selected ? 0 : -1);
    }
    select.value = "off";
    select.selectedOptions = [];
    sync(select);
    assert.equal(label.textContent, "关闭");
    for (const option of options) {
      const selected = option.dataset.codexPlusTypingEffectOption === "off";
      assert.equal(option.attributes["aria-selected"], String(selected));
      assert.equal(option.tabIndex, selected ? 0 : -1);
    }
    select.disabled = true;
    sync(select);
    assert.equal(trigger.disabled, true);
    assert.equal(menu.hidden, true);
    assert.equal(triggerAttributes["aria-expanded"], "false");
    assert.equal(cleanups, 1, "disabling an open menu removes its document listeners");
    sync(select);
    assert.equal(cleanups, 1, "menu cleanup is consumed once");
  });

  it("saves dropdown selections through the existing hidden select change contract", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const catalog = JSON.parse(await readFile(settingsCatalogUrl, "utf8"));
    const page = await settingsCatalogRuntime(async () => catalog);
    await page.load();
    const selectMarkup = page.sections().match(/<select[^>]*data-codex-plus-typing-effect="true"[^>]*>[\s\S]*?<\/select>/)?.[0];
    assert.ok(selectMarkup, "the select data attribute remains available to existing change handling");
    assert.match(selectMarkup, /\bhidden\b/);
    assert.match(selectMarkup, /aria-hidden="true"/);
    assert.deepEqual(Array.from(selectMarkup.matchAll(/<option value="([^"]+)"/g), (match) => match[1]), ["off", "rainbow", "fireworks", "stars"]);
    const changeStart = renderer.indexOf('      const typingEffectSelect = target?.closest("[data-codex-plus-typing-effect]");');
    const changeEnd = renderer.indexOf("      const widthInput", changeStart);
    const optionStart = renderer.indexOf('      const typingEffectOption = target?.closest("[data-codex-plus-typing-effect-option]");');
    const optionEnd = renderer.indexOf("      // 左面板", optionStart);
    assert.ok(changeStart >= 0 && changeEnd > changeStart && optionStart >= 0 && optionEnd > optionStart);
    const saves: Array<{ key: string; value: string }> = [];
    const events: Array<{ type: string; bubbles: boolean }> = [];
    const onChange = new Function("target", "setCodexPlusSetting", renderer.slice(changeStart, changeEnd));
    const select: { value: string; disabled: boolean; dispatchEvent: (event: { type: string; bubbles: boolean }) => void } = {
      value: "off", disabled: false,
      dispatchEvent(event) {
        events.push(event);
        onChange({ closest: () => select }, (key: string, value: string) => { saves.push({ key, value }); });
      },
    };
    const option = { dataset: { codexPlusTypingEffectOption: "stars" } };
    let focusRestores = 0;
    const choose = new Function("target", "overlay", "Event", "closeCodexPlusTypingEffectDropdown", renderer.slice(optionStart, optionEnd));
    const click = () => choose(
      { closest: () => option }, { querySelector: () => select },
      class {
        type: string;
        bubbles: boolean;
        constructor(type: string, options: { bubbles: boolean }) {
          this.type = type;
          this.bubbles = options.bubbles;
        }
      },
      (restore: boolean) => { if (restore) focusRestores += 1; },
    );
    click();
    assert.equal(select.value, "stars");
    assert.equal(events.length, 1);
    assert.equal(events[0].type, "change");
    assert.equal(events[0].bubbles, true);
    assert.deepEqual(saves, [{ key: "typingEffect", value: "stars" }]);
    click();
    assert.equal(events.length, 1, "selecting the current value does not save again");
    select.disabled = true;
    option.dataset.codexPlusTypingEffectOption = "rainbow";
    click();
    assert.equal(select.value, "stars");
    assert.equal(events.length, 1, "disabled selections cannot change the saved setting");
    assert.equal(focusRestores, 3);
  });

  it("rewrites official usage status at the cache boundary instead of scanning alerts", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const scanStart = renderer.indexOf("  function scanLightweight()");
    const scanEnd = renderer.indexOf("  function officialUsagePolicy()", scanStart);
    assert.ok(scanStart >= 0 && scanEnd > scanStart);
    const scan = renderer.slice(scanStart, scanEnd);

    assert.doesNotMatch(scan, /syncOfficialUsagePolicy|refreshOfficialUsageAlert/);
    assert.match(renderer, /typeof nextStatus\.hideOfficialUsageAlert === "boolean"/);
    assert.match(renderer, /window\.__CODEX_PLUS_HIDE_OFFICIAL_USAGE_ALERT__ = nextStatus\.hideOfficialUsageAlert/);
    assert.match(renderer, /function syncOfficialUsagePolicy\(\)/);
    assert.match(renderer, /queryKey\[1\] !== "image-generation"/);
    assert.match(renderer, /image_generation_limit_reached/);
    assert.match(renderer, /if \(loaded\) syncOfficialUsagePolicy\(\);/);
    assert.doesNotMatch(renderer, /officialUsageAlertCards|refreshOfficialUsageAlertVisibility|codex-plus-hide-usage-alert/);
    assert.doesNotMatch(renderer, /mutationTouchesUsageAlert/);
    assert.match(renderer, /function isOfficialLowQuotaSidebarCard/);
    assert.match(renderer, /function isOfficialLowQuotaComposerBanner/);
    assert.match(renderer, /upsell-banner-title-/);
    assert.match(renderer, /function isOfficialLowQuotaComposerAside/);
    assert.match(renderer, /tagName !== "ASIDE"/);
    assert.match(renderer, /rounded-3xl/);
    assert.match(renderer, /function syncOfficialUsageWindowMode/);
    assert.match(renderer, /key === "official-hide"/);
    assert.match(renderer, /officialUsageWindowObserver\?\.disconnect\(\)/);
    assert.match(renderer, /officialUsageWindowMarker\}="hidden"/);
    assert.doesNotMatch(scan, /startOfficialUsageWindowBlock|hideOfficialUsageWindowsWithin/);
    const policyStart = renderer.indexOf("  function syncOfficialUsagePolicy()");
    const policyEnd = renderer.indexOf("  if (window.__CODEX_PLUS_TEST_RATE_LIMIT_UNLOCK__)", policyStart);
    assert.ok(policyStart >= 0 && policyEnd > policyStart);
    const policy = renderer.slice(policyStart, policyEnd);
    const sameKey = policy.slice(policy.indexOf("if (key === officialUsagePolicyApplied)"), policy.indexOf("const previous"));
    assert.match(sameKey, /rewriteCachedOfficialUsage/);
    assert.doesNotMatch(sameKey, /hideOfficialUsageWindowsWithin|querySelectorAll/);
    assert.match(renderer, /function codexPlusPublishUsageData/);
    assert.match(renderer, /unlockSend: mixed/);
    assert.match(renderer, /limit_reached: false/);
    assert.doesNotMatch(renderer, /__codexPlusApiQuotaGate/);
    assert.doesNotMatch(renderer, /installExternalApiQuotaGate|__codexPlusApiQuotaBreakpoint|refreshComposers/);
  });

  // issue #2169：HTTP 回落成功不得掩盖桥接通道故障。桥接失败计数独立于后端状态，
  // 连续失败时状态灯降级呈现；回落路径绝不能清零计数或刷新 bridge 健康时间戳，
  // 否则启动器侧健康检查失去修复动力，重注入风暴对用户完全静默。
  it("surfaces persistent bridge degradation while the http fallback keeps the backend reachable", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /const CODEX_PLUS_BRIDGE_FAILURE_THRESHOLD = 3;/);
    assert.match(renderer, /function recordCodexPlusBridgeFailure\(\) \{\s*codexPlusBridgeFailureCount \+= 1;/);
    // 失败次数必须落到 window 上的 health 快照，启动器看门狗才能判定桥接挂死。
    assert.match(renderer, /health\.failures = codexPlusBridgeFailureCount;/);
    // 降级渲染：桥接连续失败 + 后端 ok → degraded
    assert.match(renderer, /bridgeDegraded && rawStatus === "ok" \? "degraded" : rawStatus/);
    assert.match(renderer, /桥接降级，自动修复中/);
    // 回落分支只记失败，不得触碰成功路径
    const fallbackBlock = renderer.match(
      /const fallback = await fetchBackendStatusFromHelper\(path, payload\);[\s\S]*?return fallback;\s*\}/,
    );
    assert.ok(fallbackBlock, "http fallback block not found in postJson");
    assert.doesNotMatch(fallbackBlock[0], /recordCodexPlusBridgeSuccess\(\)/);
    // 降级状态有专属样式（指示灯与文本）
    assert.match(renderer, /\.codex-plus-backend-indicator\[data-status="degraded"\]/);
    assert.match(renderer, /\.codex-plus-backend-label\[data-status="degraded"\]/);
  });

  it("keeps Windows Dream Skin compatible with the modern Codex main surface", async () => {
    const dreamSkinRenderer = await readFile(
      new URL("../../../assets/inject/upstream/dream-skin/windows/renderer-inject.js", import.meta.url),
      "utf8",
    );
    const cidalaRenderer = await readFile(
      new URL("../../../assets/inject/upstream/cidala-tiger/windows/renderer-inject.js", import.meta.url),
      "utf8",
    );

    assert.match(dreamSkinRenderer, /codex-dream-skin-selectors\/1/);
    assert.match(dreamSkinRenderer, /MainContentSurface/);
    assert.match(dreamSkinRenderer, /data-ds-part/);
    assert.match(cidalaRenderer, /MainContentSurface/);
    assert.match(cidalaRenderer, /data-codex-plus-dream-surface/);
    assert.match(cidalaRenderer, /ensureShellMain/);
  });
});

/** 从注入脚本里取出 `shouldScheduleScan`，配上可控的依赖来跑。 */
function shouldScheduleScanRuntime(renderer: string) {
  const start = renderer.indexOf("  function shouldScheduleScan(");
  const end = renderer.indexOf("\n  function runScheduledScan(", start);
  assert.ok(start >= 0 && end > start, "shouldScheduleScan not found in renderer-inject.js");
  const source = renderer.slice(start, end);
  const factory = new Function(
    "isChatContentMutation",
    "isExtensionUiNode",
    "nodeSelfOrAncestorMatchesScanRelevance",
    "isScanRelevantNode",
    `${source}\nreturn shouldScheduleScan;`,
  );
  return factory(
    () => false,
    (node: { extension?: boolean }) => Boolean(node?.extension),
    // Codex 的容器（header / 侧栏 nav）本身就是 scan-relevant，这是自喂循环的关键前提。
    (node: { relevant?: boolean }) => Boolean(node?.relevant),
    (node: { relevant?: boolean; extension?: boolean }) =>
      Boolean(node?.relevant) && !node?.extension,
  ) as (mutations: unknown[]) => boolean;
}

const codexContainer = { nodeType: 1, relevant: true };

function mutation(addedNodes: unknown[] = [], removedNodes: unknown[] = []) {
  return { target: codexContainer, addedNodes, removedNodes };
}

describe("native-style extension and sponsor discovery", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);
  function segment(source: string, from: string, to: string) {
    const start = source.indexOf(from);
    const end = source.indexOf(to, start);
    assert.ok(start >= 0 && end > start, `missing renderer segment: ${from}`);
    return source.slice(start, end);
  }
  function escapeSource(renderer: string) {
    return segment(renderer, "  function escapeHtml(value)", "  function confirmDelete(");
  }
  function extensionRuntime(renderer: string) {
    const icon = renderer.match(/^  const codexPlusDefaultExtensionIconPath = .+;$/m)?.[0];
    assert.ok(icon);
    return new Function(`${icon}\n${escapeSource(renderer)}
let codexPlusUserScripts = { scripts: [] }, codexPlusUserScriptsLoaded = false;
let codexPlusScriptMarket = { scripts: [], loaded: false, loading: false, message: "" };
let codexPlusExtensionsQuery = "", codexPlusExtensionsFilter = "market", codexPlusExtensionsSelected = null;
${segment(renderer, "  function userScriptStatusLabel(", "  async function uninstallUserScript(")}
${segment(renderer, "  function codexPlusExtensionsEntries()", "  const codexPlusAdsUrl =")}
return {
  nav: renderCodexPlusExtensionsNav, overview: renderCodexPlusExtensionsOverview, detail: renderCodexPlusExtensionsDetail,
  configure(scripts, market, options = {}) {
    codexPlusUserScripts = { scripts }; codexPlusUserScriptsLoaded = options.userLoaded ?? true;
    codexPlusScriptMarket = { scripts: market, loaded: options.marketLoaded ?? true, loading: options.loading ?? false, message: options.message || "" };
    codexPlusExtensionsQuery = options.query || ""; codexPlusExtensionsFilter = options.filter || "market";
    codexPlusExtensionsSelected = options.selected || null;
  }
};`)() as {
      nav: () => string; overview: () => string; detail: () => string;
      configure: (scripts: Record<string, unknown>[], market: Record<string, unknown>[], options?: Record<string, unknown>) => void;
    };
  }

  it("filters installed and market entries, escapes their data and keeps actions outside selection buttons", async () => {
    const runtime = extensionRuntime(await readFile(rendererPath, "utf8"));
    const scripts = [
      { key: 'user"&', name: "Owned <user>", source: "user", market_id: "owned", enabled: true, status: "loaded" },
      { key: "builtin.js", name: "Builtin", source: "builtin", enabled: false, status: "disabled" },
    ];
    const market = [
      { id: "owned", name: "Owned market", description: "Owned-document", tags: ["owned-tag"], version: "2" },
      { id: 'available"&', name: "Available <script>", description: 'Catalog <docs> & "more"', tags: ["catalog"], author: "Author", version: "1", icon: "https://example.test/logo.png?x=1&y=2" },
    ];
    runtime.configure(scripts, market);
    const nav = runtime.nav();
    assert.ok(nav.includes('data-codex-extensions-select="installed:user&quot;&amp;"'));
    assert.ok(nav.includes('data-codex-extensions-select="installed:builtin.js"'));
    assert.ok(nav.includes('data-codex-extensions-browse="market"'));
    assert.doesNotMatch(nav, /data-codex-market-install|data-codex-extensions-search/);
    const overview = runtime.overview();
    assert.ok(overview.includes('data-codex-extensions-select="market:available&quot;&amp;"'));
    assert.ok(overview.includes('data-codex-market-install="available&quot;&amp;"'));
    assert.ok(overview.includes("Available &lt;script&gt;"));
    assert.ok(overview.includes("Catalog &lt;docs&gt; &amp; &quot;more&quot;"));
    assert.ok(overview.includes('src="https://example.test/logo.png?x=1&amp;y=2"'));
    assert.doesNotMatch(overview, /<script>|data-codex-market-install="owned"|Owned market|Builtin/);
    const cards = Array.from(overview.matchAll(/<article class="codex-plus-extension-card">([\s\S]*?)<\/article>/g));
    assert.equal(cards.length, 1);
    assert.match(cards[0][1], /<\/button>\s*<div class="codex-plus-extension-card-actions"><button/);
    assert.equal(Array.from(cards[0][1].matchAll(/<button\b/g)).length, 2);
    assert.doesNotMatch(cards[0][1], /<button\b(?:(?!<\/button>)[\s\S])*<button\b/);
    assert.equal(runtime.detail(), overview, "opening extensions initially displays the browse results");

    runtime.configure(scripts, market, { filter: "installed" });
    const installed = runtime.overview();
    assert.ok(installed.includes("Owned market"), "installed market scripts use their catalog display name");
    assert.ok(installed.includes('data-codex-user-script-key="user&quot;&amp;"'));
    assert.ok(installed.includes('role="switch" aria-checked="true"'));
    assert.ok(installed.includes('role="switch" aria-checked="false"'));
    assert.doesNotMatch(installed, /data-codex-market-install|Available/);
    runtime.configure(scripts, market, { filter: "installed", query: "OWNED-DOCUMENT" });
    assert.ok(runtime.overview().includes('data-codex-user-script-key="user&quot;&amp;"'));
    assert.doesNotMatch(runtime.overview(), /Builtin/);
    runtime.configure(scripts, market, { filter: "installed", query: "Owned <user>" });
    assert.ok(runtime.overview().includes('data-codex-user-script-key="user&quot;&amp;"'), "the original local name remains searchable");
    runtime.configure(scripts, market, { filter: "installed", query: 'user"&' });
    assert.ok(runtime.overview().includes('data-codex-user-script-key="user&quot;&amp;"'), "the local script key remains searchable");
    runtime.configure(scripts, market, { query: "CATALOG" });
    assert.ok(runtime.overview().includes("Available &lt;script&gt;"));
    runtime.configure(scripts, market, { selected: { kind: "installed", key: 'user"&' } });
    assert.ok(runtime.detail().includes('data-codex-extensions-uninstall="user&quot;&amp;"'));
    runtime.configure(scripts, market, { selected: { kind: "installed", key: "builtin.js" } });
    assert.doesNotMatch(runtime.detail(), /data-codex-extensions-uninstall/);
  });

  it("retains loading, empty and escaped search/error states in extension browse results", async () => {
    const runtime = extensionRuntime(await readFile(rendererPath, "utf8"));
    runtime.configure([], [], { marketLoaded: false, loading: true, userLoaded: false });
    assert.match(runtime.overview(), /role="status">正在读取拓展…/);
    assert.match(runtime.nav(), /正在读取用户拓展…/);
    runtime.configure([], [], { filter: "installed", userLoaded: false });
    assert.match(runtime.overview(), /正在读取用户拓展…/);
    runtime.configure([], [], { filter: "installed" });
    assert.match(runtime.overview(), /未发现已安装的拓展。/);
    runtime.configure([], []);
    assert.match(runtime.overview(), /市场里没有可安装的拓展。/);
    runtime.configure([], [], { message: "<backend> & unavailable" });
    assert.ok(runtime.overview().includes("&lt;backend&gt; &amp; unavailable"));
    runtime.configure([], [], { query: '<missing> "query"' });
    assert.ok(runtime.overview().includes("没有匹配「&lt;missing&gt; &quot;query&quot;」的拓展。"));
    runtime.configure([], [], { selected: { kind: "market", key: "removed" } });
    assert.match(runtime.detail(), /市场里没有可安装的拓展。/);
  });

  it("updates extension results without replacing or refocusing the search input", async () => {
    const renderer = await readFile(rendererPath, "utf8");
    const input = { value: "query", selectionStart: 2, selectionEnd: 2 };
    const nav = { innerHTML: "old nav" }, detail = { innerHTML: "old detail" };
    const nodes: Record<string, object> = {
      "[data-codex-plus-page-nav-body]": nav,
      "[data-codex-plus-extensions-detail]": detail,
      "[data-codex-extensions-search]": input,
    };
    const requested: string[] = [];
    const renderedSelections: unknown[] = [];
    const runtime = new Function("document", "codexPlusActiveEntry", "renderNav", "renderDetail", "codexPlusExtensionsSelectionDetail",
      `let codexPlusExtensionsSelected = { kind: "market", key: "removed" };
function renderCodexPlusExtensionsNav() { return renderNav(codexPlusExtensionsSelected); }
function renderCodexPlusExtensionsDetail() { return renderDetail(codexPlusExtensionsSelected); }
${segment(renderer, "  function refreshCodexPlusExtensionsView()", "  function codexPlusExtensionsSelectionDetail()")}
return { refresh: refreshCodexPlusExtensionsView, selection: () => codexPlusExtensionsSelected };`,
    )(
      { querySelector(selector: string) { requested.push(selector); return nodes[selector]; } }, () => "extensions",
      (selection: unknown) => { renderedSelections.push(selection); return "new nav"; },
      (selection: unknown) => { renderedSelections.push(selection); return "new results"; },
      () => null,
    ) as { refresh: () => void; selection: () => unknown };
    runtime.refresh();
    assert.equal(runtime.selection(), null, "removed selections return to the browse list");
    assert.deepEqual(renderedSelections, [null, null], "the sidebar and results render from the same normalized selection");
    assert.equal(nav.innerHTML, "new nav");
    assert.equal(detail.innerHTML, "new results");
    assert.equal(nodes["[data-codex-extensions-search]"], input);
    assert.deepEqual(input, { value: "query", selectionStart: 2, selectionEnd: 2 });
    assert.deepEqual(requested, ["[data-codex-plus-page-nav-body]", "[data-codex-plus-extensions-detail]"]);
  });

  it("preserves sponsor logos, destinations and highlights in the native recommendation list", async () => {
    const renderer = await readFile(rendererPath, "utf8");
    const logo = await readFile(new URL("../../../docs/images/sponsor-runapi.png", import.meta.url));
    const image = `data:image/png;base64,${logo.toString("base64")}`;
    const runtime = new Function(`${escapeSource(renderer)}\n${segment(renderer, "  const codexPlusAdsUrl =", "  function cacheBustCodexPlusAdUrl(")}
return {
  render: renderCodexPlusAds,
  set(payload, loaded = true) { codexPlusAds = normalizeCodexPlusAds(payload); codexPlusAdsLoaded = loaded; return codexPlusAds; }
};`)() as { render: () => string; set: (payload: unknown, loaded?: boolean) => Array<Record<string, unknown>> };
    const ad = { id: "runapi", type: "sponsor", title: "RunAPI｜套餐", description: "原始 <描述>", url: "https://example.test/partner?plan=pro&source=codex", image, highlights: ['原始 "优惠" & 说明', "保留第二条亮点"] };
    const normalized = runtime.set({ ads: [ad, { ...ad, id: "normal", type: "normal" }, { ...ad, id: "expired", expires_at: "2000-01-01" }] });
    assert.equal(normalized.length, 1);
    assert.equal(normalized[0].image, image);
    assert.equal(normalized[0].url, ad.url);
    assert.deepEqual(normalized[0].highlights, ad.highlights);
    const html = runtime.render();
    assert.ok(html.includes(`src="${image}"`));
    assert.ok(html.includes('data-codex-plus-ad-url="https://example.test/partner?plan=pro&amp;source=codex"'));
    assert.ok(html.includes('href="https://example.test/partner?plan=pro&amp;source=codex"'));
    assert.ok(html.includes("原始 &lt;描述&gt;"));
    assert.ok(html.includes("原始 &quot;优惠&quot; &amp; 说明"));
    assert.doesNotMatch(html, /codex-plus-ad-icon-fallback|<描述>/);
    runtime.set({ ads: [ad] }, false);
    assert.match(runtime.render(), /推荐内容加载中…/);
    runtime.set({ ads: [] });
    assert.match(runtime.render(), /暂无推荐内容。/);
  });

  it("drops sponsor destinations that are not http(s)", async () => {
    const renderer = await readFile(rendererPath, "utf8");
    const runtime = new Function(`${escapeSource(renderer)}\n${segment(renderer, "  const codexPlusAdsUrl =", "  function cacheBustCodexPlusAdUrl(")}
return {
  render: renderCodexPlusAds,
  set(payload, loaded = true) { codexPlusAds = normalizeCodexPlusAds(payload); codexPlusAdsLoaded = loaded; return codexPlusAds; }
};`)() as { render: () => string; set: (payload: unknown, loaded?: boolean) => Array<Record<string, unknown>> };
    const base = { type: "sponsor", title: "赞助商", description: "推荐内容" };
    const normalized = runtime.set({
      ads: [
        { ...base, id: "ok", url: "https://example.test/partner" },
        { ...base, id: "javascript", url: " javascript:alert(document.domain)" },
        { ...base, id: "data", url: "data:text/html,<script>alert(1)</script>" },
        { ...base, id: "file", url: "file:///etc/passwd" },
        { ...base, id: "scheme-relative", url: "//example.test/partner" },
      ],
    });
    assert.deepEqual(normalized.map((ad) => ad.id), ["ok"]);
    const html = runtime.render();
    assert.ok(html.includes('data-codex-plus-ad-url="https://example.test/partner"'));
    assert.doesNotMatch(html, /javascript:|data:text\/html|file:\/\/|scheme-relative/);
  });
});

describe("renderer injection scan scheduling", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：我们把自己的节点挂进 Codex 的容器，容器是 scan-relevant，
  // 于是每次写入都会再排一次 scan，scan 又重新写入，空闲时 CPU 被吃满。
  it("ignores mutations that only move the extension's own nodes", async () => {
    const shouldScheduleScan = shouldScheduleScanRuntime(await readFile(rendererPath, "utf8"));
    const ownNode = { nodeType: 1, extension: true };

    assert.equal(shouldScheduleScan([mutation([ownNode])]), false);
    // appendChild 一个已经在位的子节点会同时报 removed + added。
    assert.equal(shouldScheduleScan([mutation([ownNode], [ownNode])]), false);
  });

  it("still scans when Codex itself changes the same container", async () => {
    const shouldScheduleScan = shouldScheduleScanRuntime(await readFile(rendererPath, "utf8"));
    const codexNode = { nodeType: 1, relevant: true };
    const ownNode = { nodeType: 1, extension: true };

    assert.equal(shouldScheduleScan([mutation([codexNode])]), true);
    // 混合变更里只要有一个不是我们的，就不能跳过。
    assert.equal(shouldScheduleScan([mutation([ownNode, codexNode])]), true);
    // 属性变更没有 added/removed 节点，仍按容器相关性判定。
    assert.equal(shouldScheduleScan([mutation()]), true);
  });
});

interface MarketplacePatchHarness {
  install: () => void;
  sweeps: () => number;
  diagnostics: () => string[];
  settle: () => Promise<void>;
}

function marketplacePatchRuntime(renderer: string, patchSucceeds: boolean): MarketplacePatchHarness {
  const start = renderer.indexOf("  const pluginMarketplaceRequestPatchMaxMisses = ");
  const end = renderer.indexOf("\n  function codexPluginMarketplacePatchEnabled(", start);
  assert.ok(start >= 0 && end > start, "marketplace patch block not found in renderer-inject.js");
  const source = renderer.slice(start, end);

  let sweeps = 0;
  let pending: Array<() => void> = [];
  const diagnostics: string[] = [];
  const fakeWindow: Record<string, unknown> = {};

  const factory = new Function(
    "window",
    "codexPluginMarketplaceUnlockVersion",
    "codexPluginMarketplacePatchEnabled",
    "codexPlusSettings",
    "loadAppServerRequestCandidates",
    "patchPluginMarketplaceRequestClient",
    "sendCodexPlusDiagnostic",
    "__note",
    `${source}\nreturn installPluginMarketplaceRequestPatch;`,
  );

  const install = factory(
    fakeWindow,
    1,
    () => true,
    () => ({ pluginMarketplaceUnlock: true }),
    // 每轮 sweep 在真实实现里会 fetch 全部 app asset，这里只计数并挂起，
    // 好让测试能在「上一轮尚未结束」的时刻再次调用 install。
    () =>
      new Promise((resolve) => {
        sweeps += 1;
        pending.push(() => resolve({ modules: [{}], candidates: [{}], sources: [], discovery: "fallback" }));
      }),
    () => patchSucceeds,
    (event: string) => diagnostics.push(event),
  ) as () => void;

  const settle = async () => {
    // 放行所有挂起的 sweep，并把微任务队列排空。
    while (pending.length) {
      const flush = pending;
      pending = [];
      flush.forEach((resolve) => resolve());
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    }
  };

  return { install, sweeps: () => sweeps, diagnostics: () => diagnostics, settle };
}

/** 取出共用的 module loader，用假时钟驱动它的失败冷却。 */
function moduleLoaderRuntime(renderer: string) {
  const start = renderer.indexOf("  // issue #1960：失败必须被记住。");
  const end = renderer.indexOf("\n  async function loadOptionalCodexAppModule(", start);
  assert.ok(start >= 0 && end > start, "loadCodexAppModule not found in renderer-inject.js");
  const source = renderer.slice(start, end);

  let sweeps = 0;
  let clock = 1_000_000;
  const factory = new Function(
    "codexServiceTierModulePromises",
    "codexAppModuleFailures",
    "codexAppModuleRetryCooldownMs",
    "codexAppModuleMaxAttempts",
    "codexAppAssetUrl",
    "codexAppAssetUrlFromScriptText",
    "Date",
    `${source}\nreturn loadCodexAppModule;`,
  );
  const load = factory(
    new Map(),
    new Map(),
    30000,
    8,
    () => "",
    // 真实实现在这里会把全部 app asset 拉一遍；这里只计数并同样返回“没找到”。
    async () => {
      sweeps += 1;
      return "";
    },
    { now: () => clock },
  ) as (namePart: string) => Promise<unknown>;

  const attempt = async (namePart = "vscode-api-") => {
    try {
      await load(namePart);
    } catch {
      /* 预期失败 */
    }
  };
  return { attempt, sweeps: () => sweeps, advance: (ms: number) => { clock += ms; } };
}

describe("renderer injection codex app module loader", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：失败以前只是把 promise 删掉，等于没有负缓存，
  // 调用方一重试就重新 fetch 全部 app asset（实测 301 次请求/秒）。
  it("does not re-sweep every asset while the failure is still in cooldown", async () => {
    const loader = moduleLoaderRuntime(await readFile(rendererPath, "utf8"));

    for (let i = 0; i < 20; i += 1) await loader.attempt();

    assert.equal(loader.sweeps(), 1);
  });

  it("retries once per cooldown window, then gives up for good", async () => {
    const loader = moduleLoaderRuntime(await readFile(rendererPath, "utf8"));

    // 冷却期满就允许再试一次，避免 Codex 更新后 asset 回来了却永远发现不了。
    for (let i = 0; i < 30; i += 1) {
      await loader.attempt();
      loader.advance(30001);
    }

    // 连续失败达到上限(8)后彻底停手，而不是每个冷却窗口都再扫一遍。
    assert.equal(loader.sweeps(), 8);
  });

  it("keeps failures separate per asset prefix", async () => {
    const loader = moduleLoaderRuntime(await readFile(rendererPath, "utf8"));

    await loader.attempt("vscode-api-");
    await loader.attempt("app-initial-");
    await loader.attempt("vscode-api-");

    // 两个前缀各自试了一次；第三次命中 vscode-api- 自己的冷却。
    assert.equal(loader.sweeps(), 2);
  });
});

interface DispatcherPatchHarness {
  install: () => void;
  attempts: () => number;
  diagnostics: () => string[];
  settle: () => Promise<void>;
}

function dispatcherPatchRuntime(renderer: string, dispatcherFound: boolean): DispatcherPatchHarness {
  const start = renderer.indexOf("  const serviceTierDispatcherPatchMaxMisses = ");
  const end = renderer.indexOf("\n  async function loadBackendSettingsState(", start);
  assert.ok(start >= 0 && end > start, "service tier dispatcher patch block not found");
  const source = renderer.slice(start, end);

  let attempts = 0;
  let pending: Array<() => void> = [];
  const diagnostics: string[] = [];
  const fakeWindow: Record<string, unknown> = {};

  const factory = new Function(
    "window",
    "codexServiceTierRequestOverrideVersion",
    "loadCodexAppModule",
    "codexServiceTierDispatcherFromModule",
    "dispatchCodexPlusMessage",
    "installCodexRemoteSessionDispatcherSubscription",
    "sendCodexPlusDiagnostic",
    `${source}\nreturn installCodexServiceTierDispatcherPatch;`,
  );

  const install = factory(
    fakeWindow,
    1,
    // 真实实现每轮会依次试三个前缀，每个 miss 都触发一轮全量 asset 扫描。
    () =>
      new Promise((resolve, reject) => {
        attempts += 1;
        pending.push(() => (dispatcherFound ? resolve({}) : reject(new Error("未找到 Codex App asset"))));
      }),
    () => (dispatcherFound ? { dispatchMessage() {}, subscribe() {} } : null),
    () => undefined,
    () => undefined,
    (event: string) => diagnostics.push(event),
  ) as () => void;

  const settle = async () => {
    while (pending.length) {
      const flush = pending;
      pending = [];
      flush.forEach((resolve) => resolve());
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    }
  };

  return { install, attempts: () => attempts, diagnostics: () => diagnostics, settle };
}

describe("renderer injection service tier dispatcher patch", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：这个补丁挂在 scanLightweight() 里每轮都跑，是 #1324 同一缺陷的第三个实例。
  it("does not start a new sweep while the previous one is still running", async () => {
    const harness = dispatcherPatchRuntime(await readFile(rendererPath, "utf8"), false);

    for (let i = 0; i < 20; i += 1) harness.install();

    assert.equal(harness.attempts(), 1);
    await harness.settle();
  });

  it("stops retrying and stops re-reporting once the dispatcher is clearly gone", async () => {
    const harness = dispatcherPatchRuntime(await readFile(rendererPath, "utf8"), false);

    for (let i = 0; i < 40; i += 1) {
      harness.install();
      await harness.settle();
    }

    // 三个前缀里第一个就抛，loadDispatcher 会继续试下一个，所以每轮不止一次尝试；
    // 关键是达到 maxMisses(8) 之后彻底停手。
    assert.equal(harness.diagnostics().filter((e) => e === "service_tier_dispatcher_patch_failed").length, 1);
    assert.deepEqual(harness.diagnostics().at(-1), "service_tier_dispatcher_patch_skipped");
    const settled = harness.attempts();
    harness.install();
    await harness.settle();
    assert.equal(harness.attempts(), settled);
  });

  // #2657：26.1016 的 dispatcher 只从 app-shared-* 导出（带 getInstance 的类），
  // app-initial-* 能加载但不导出它，setting-storage-* / vscode-api-* 已不存在。
  it("finds the dispatcher class exported from app-shared on Codex 26.1016", async () => {
    const renderer = await readFile(rendererPath, "utf8");
    const start = renderer.indexOf("  const serviceTierDispatcherPatchMaxMisses = ");
    const end = renderer.indexOf("\n  async function loadBackendSettingsState(", start);
    const fromStart = renderer.indexOf("  function codexServiceTierDispatcherFromModule(");
    const fromEnd = renderer.indexOf("\n  }\n", fromStart) + 4;
    assert.ok(start >= 0 && end > start && fromStart >= 0 && fromEnd > fromStart);

    class Dispatcher {
      static instance: Dispatcher | null = null;
      static getInstance() { return (this.instance ??= new Dispatcher()); }
      dispatchMessage() {}
      subscribe() {}
    }
    const modules: Record<string, unknown> = {
      "app-initial-": { idt: "not-a-dispatcher", unrelated: () => undefined },
      "app-shared-": { idt: "OwlRemoteSearchSuggestions", AFt: Dispatcher },
    };
    const details: Array<[string, unknown]> = [];
    const install = new Function(
      "window",
      "codexServiceTierRequestOverrideVersion",
      "loadCodexAppModule",
      "dispatchCodexPlusMessage",
      "installCodexRemoteSessionDispatcherSubscription",
      "sendCodexPlusDiagnostic",
      `${renderer.slice(fromStart, fromEnd)}\n${renderer.slice(start, end)}\nreturn installCodexServiceTierDispatcherPatch;`,
    )(
      {},
      1,
      async (prefix: string) => {
        if (prefix in modules) return modules[prefix];
        throw new Error(`未找到 Codex App asset: ${prefix}`);
      },
      () => undefined,
      () => undefined,
      (event: string, detail: unknown) => details.push([event, detail]),
    ) as () => void;

    install();
    for (let i = 0; i < 10; i += 1) await Promise.resolve();

    assert.deepEqual(details, [["service_tier_dispatcher_patch_installed", { assetPrefix: "app-shared-" }]]);
  });

  it("keeps working normally when the dispatcher is found", async () => {
    const harness = dispatcherPatchRuntime(await readFile(rendererPath, "utf8"), true);

    harness.install();
    await harness.settle();
    for (let i = 0; i < 10; i += 1) harness.install();

    assert.equal(harness.attempts(), 1);
    assert.deepEqual(harness.diagnostics(), ["service_tier_dispatcher_patch_installed"]);
  });
});

describe("renderer injection plugin marketplace patch", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：scanDeferred() 每轮都调用这个补丁，而早退守卫只在打上补丁后才写入。
  // Codex 侧 asset 改名后这层永远成功不了，过去既不去重也不放弃，
  // 于是每轮 scan 都把全部 app asset 重新 fetch 一遍（实测 530 次 fetch/秒）。
  it("does not start a new sweep while the previous one is still running", async () => {
    const harness = marketplacePatchRuntime(await readFile(rendererPath, "utf8"), false);

    // 模拟连续多轮 scan：上一轮还挂着，后续调用必须被 in-flight 守卫挡掉。
    for (let i = 0; i < 20; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    await harness.settle();
  });

  it("stops retrying once the asset is clearly unavailable", async () => {
    const harness = marketplacePatchRuntime(await readFile(rendererPath, "utf8"), false);

    // 每次都跑完再发起下一轮，模拟长时间运行中的反复 scan。
    for (let i = 0; i < 40; i += 1) {
      harness.install();
      await harness.settle();
    }

    // 达到 maxMisses(8) 之后必须彻底停掉，而不是无限重试。
    assert.equal(harness.sweeps(), 8);
    // 首次 miss 上报一次，停用时再报一次，中间保持噤声。
    assert.deepEqual(harness.diagnostics(), [
      "plugin_marketplace_request_patch_not_found",
      "plugin_marketplace_request_patch_skipped",
    ]);
  });

  it("keeps working normally when the patch actually lands", async () => {
    const harness = marketplacePatchRuntime(await readFile(rendererPath, "utf8"), true);

    harness.install();
    await harness.settle();
    // 打上补丁后守卫生效，后续 scan 不再重复扫描。
    for (let i = 0; i < 10; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    assert.deepEqual(harness.diagnostics(), ["plugin_marketplace_request_patch_installed"]);
  });

  // 26.928.31416 起 Codex 又把过滤器里的标识符换了一轮名（!Mj(e.marketplaceName)||e.marketplaceName===n）。
  // 过去按压缩字面量匹配，发版即失效：补丁照装，但 plugin_build_flavor_filter_bypassed 从不触发，
  // 插件解锁静默失灵。现在改用结构正则，下面把历史形态与新形态一起钉住。
  function extractFilterPattern(renderer: string, name: string): RegExp {
    const match = renderer.match(new RegExp(`const ${name}\\s*=\\s*\\/([\\s\\S]*?)\\/;`));
    assert.ok(match, `${name} 未在产物中找到`);
    return new RegExp(match![1]);
  }

  it("matches build-flavor filter shapes across Codex builds", async () => {
    const pattern = extractFilterPattern(await readFile(rendererPath, "utf8"), "codexPluginBuildFlavorFilterSourcePattern");
    const shapes = [
      "function EHr({buildFlavor:e,plugins:t}){let n=Gpt(e);return t.filter(e=>!Mj(e.marketplaceName)||e.marketplaceName===n)}", // 26.928.31416
      "e.filter(e=>!u(e.marketplaceName)||e.marketplaceName===r)",
      "e.filter(e=>!ne(e.marketplaceName)||e.marketplaceName===n)",
      "e.filter(e=>!Eu(e.marketplaceName)||e.marketplaceName===n)",
    ];
    for (const source of shapes) assert.ok(pattern.test(source), `应命中: ${source}`);
    assert.ok(!pattern.test("e.filter(x=>x.enabled&&x.name.length>3)"), "不应命中无关过滤器");
  });

  it("detects the featured plugin id filter separately", async () => {
    const pattern = extractFilterPattern(await readFile(rendererPath, "utf8"), "codexPluginFeaturedFilterSourcePattern");
    // featuredPluginIds 那条入参是字符串 id，形态与 build-flavor 不同，必须单独认。
    assert.ok(pattern.test("function THr({buildFlavor:e,featuredPluginIds:t}){let n=Gpt(e);return t.filter(e=>{let t=Gj(e);return t==null||!Mj(t)||t===n})}"));
    assert.ok(!pattern.test("e.filter(e=>!Mj(e.marketplaceName)||e.marketplaceName===n)"), "不应与 build-flavor 过滤器混淆");
    // 真实 bundle 里存在这个形状相近的无关函数；右值 `SFe(t)` 是调用，
    // 早期写法 `(?!\s*\()` 会被贪婪回溯绕过（SFe 退成 SF），实测踩过。
    assert.ok(
      !pattern.test("let r=bSe(e.path)?.pluginMarketplaceName??null;return t==null||r==null||!ds(r)||r===SFe(t)"),
      "不应命中 bundle 里的无关 null-guard",
    );
  });

  it("matches hidden-marketplace filter shapes structurally", async () => {
    const pattern = extractFilterPattern(await readFile(rendererPath, "utf8"), "codexPluginHiddenFilterSourcePattern");
    assert.ok(pattern.test("function hHr(e,t){return t.length===0?e:e.filter(e=>!t.includes(e.name))}"));
    assert.ok(pattern.test("marketplaces:a.filter(e=>!n.includes(e.name)&&(!r.Po(e.name)||e.name===c))"));
    assert.ok(!pattern.test("e.filter(e=>e.plugins.some(p=>p.name))"), "不应命中无关过滤器");
    // 真实 bundle 里的无关守卫：不是 filter 箭头形态，必须排掉。
    assert.ok(!pattern.test("if(n[t.name]=t,!w4.includes(t.name)&&typeof t.setupOnce==`function`"), "不应命中非 filter 守卫");
  });
});

describe("relay pureApi provider resolution", () => {
  function providerRuntime(
    renderer: string,
    backendSettings: Record<string, unknown>,
    catalog: Record<string, unknown>,
    profile: Record<string, unknown>,
  ) {
    const start = renderer.indexOf("function codexRelayConfigModelProvider(");
    const codeStart = renderer.indexOf("function codexRemoteSessionTargetProvider(");
    const end = renderer.indexOf("\n  function codexRemoteSessionProviderRequestMethod", codeStart);
    assert.ok(start >= 0 && codeStart >= 0 && end > codeStart);
    const source = renderer.slice(start, end);
    const create = new Function(
      "codexPlusBackendSettings",
      "codexModelCatalog",
      "codexRemoteSessionActiveProfile",
      `${source}\nreturn { codexRelayConfigModelProvider, codexRemoteSessionTargetProvider };`,
    ) as (
      backend: Record<string, unknown>,
      cat: Record<string, unknown>,
      activeProfile: () => Record<string, unknown>,
    ) => {
      codexRelayConfigModelProvider: (configContents: string) => string;
      codexRemoteSessionTargetProvider: () => string;
    };
    return create(backendSettings, catalog, () => profile);
  }

  it("resolves the real model_provider from a pureApi relay profile instead of hardcoding custom", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      {},
      { codex_model_provider: "deepseek" },
      { relayMode: "pureApi", configContents: 'model = "deepseek-v4-flash-vision-exp"\nmodel_provider = "deepseek"' },
    );

    assert.equal(runtime.codexRelayConfigModelProvider('model_provider = "deepseek"'), "deepseek");
    assert.equal(runtime.codexRemoteSessionTargetProvider(), "deepseek");
  });

  it("still returns custom for pureApi relays that genuinely declare the custom provider", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      {},
      { codex_model_provider: "custom" },
      { relayMode: "pureApi", configContents: 'model_provider = "custom"\n[model_providers.custom]' },
    );

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "custom");
  });

  it("falls back to custom when a pureApi relay declares no provider", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(renderer, {}, { codex_model_provider: "" }, { relayMode: "pureApi", configContents: "" });

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "custom");
  });

  // activeRelayCodexProvider 是全局缓存，切换供应商后可能还留着上一个的值。
  // pureApi 时优先信 profile 自己的 configContents；profile 没声明就回到
  // "custom"，不采信这个缓存——cdp_bridge.rs 的 refreshedPureApiResumeProvider
  // 正是钉这个：陈旧缓存是 stale_custom_provider 时必须仍解析成 custom。
  it("ignores a possibly stale activeRelayCodexProvider for pureApi relays", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      { activeRelayCodexProvider: "stale_custom_provider" },
      { codex_model_provider: "" },
      { relayMode: "pureApi", configContents: "" },
    );

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "custom");
  });

  // 非 pureApi 才拿 activeRelayCodexProvider 兜底。
  it("still falls back to activeRelayCodexProvider outside pureApi", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      { activeRelayCodexProvider: "deepseek" },
      { codex_model_provider: "" },
      { relayMode: "mixedApi", configContents: "" },
    );

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "deepseek");
  });

  // profile 自己声明了供应方时，优先级高于全局缓存。
  it("prefers the profile's own configContents over the cached provider", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      { activeRelayCodexProvider: "stale_custom_provider" },
      { codex_model_provider: "" },
      { relayMode: "pureApi", configContents: 'model_provider = "deepseek"' },
    );

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "deepseek");
  });
});

it("opens Stepwise configuration in the manager enhancement page", async () => {
  const source = await readFile(new URL("../../../assets/inject/floating-panel/core/views.js", import.meta.url), "utf8");
  const begin = source.indexOf("  async function openManager()");
  assert.ok(begin >= 0);
  const state = { runtimeGeneration: 3, settingsStatus: "" };
  const requests: Array<{ route: string; payload: unknown }> = [];
  let renders = 0;
  const openManager = new Function("isCurrentRuntime", "state", "renderFloat", "bridgeCall",
    source.slice(begin) + "\nreturn openManager;",
  )(
    (generation?: number) => generation === undefined || generation === state.runtimeGeneration,
    state,
    () => { renders += 1; },
    async (route: string, payload: unknown) => { requests.push({ route, payload }); return { status: "ok" }; },
  );
  await openManager();
  assert.deepEqual(requests, [{ route: "/manager/open-transient", payload: { page: "enhance", section: "stepwise" } }]);
  assert.equal(state.settingsStatus, "已打开 Codex++");
  assert.equal(renders, 2);
});

describe("Stepwise generation mode contracts", () => {
  it("exposes automatic and manual generation in manager enhancements", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const renderer = await readFile(
      new URL("../../../assets/inject/renderer-inject.js", import.meta.url),
      "utf8",
    );

    assert.match(app, /type StepwiseGenerationMode = "auto" \| "manual";/);
    assert.match(app, /type StepwiseProtocol = "auto" \| "chat_completions" \| "responses" \| "anthropic_messages";/);
    assert.match(app, /codexAppStepwiseProtocol: "chat_completions",/);
    assert.match(app, /codexAppStepwiseGenerationMode: "auto",/);
    assert.match(app, /codexAppAnswerOutlineEnabled: false,/);
    assert.match(renderer, /answerOutline: false,/);
    assert.match(app, /<Field label=\{t\("模式"\)\}>/);
    assert.match(app, /\{ value: "auto", label: t\("自动生成"\) \}/);
    assert.match(app, /\{ value: "manual", label: t\("手动刷新"\) \}/);
    assert.match(app, /\{ value: "auto", label: t\("自动兼容"\) \}/);
    assert.match(app, /\{ value: "anthropic_messages", label: "Anthropic Messages" \}/);
    assert.match(app, /function normalizeStepwiseProtocol\(/);
    assert.match(app, /return value === "manual" \? "manual" : "auto";/);
  });

  it("defers manual generation until refresh and rejects stale mode results", async () => {
    const stepwise = await readStepwiseSource();

    assert.match(stepwise, /const manualRequestPending = generationMode === "manual"/);
    assert.match(stepwise, /if \(generationMode === "manual" && !manualResultVisible && !manualRequestPending\)/);
    assert.match(stepwise, /\} else if \(manualRequestPending\) \{/);
    assert.match(stepwise, /state\.bridgeStatus = "manual-ready";/);
    assert.match(
      stepwise,
      /requestBridgeStepwise\(bridgeKey, userText, assistantText, generationMode, \{ userInitiated: true \}\)/,
    );
    assert.match(stepwise, /requestBridgeStepwise\(bridgeKey, userText, assistantText, "auto"\)/);
    assert.match(stepwise, /normalizedMode === "manual" && options\.userInitiated !== true/);
    assert.match(stepwise, /stepwiseGenerationMode\(\) === normalizedMode/);
    assert.match(stepwise, /state\.bridgePendingMode === normalizedMode/);
    assert.match(stepwise, /Object\.prototype\.hasOwnProperty\.call\(normalizedPatch, "generationMode"\)/);
    assert.match(stepwise, /if \(!Object\.prototype\.hasOwnProperty\.call\(nextSettings, "generationMode"\)\)/);
    assert.match(stepwise, /nextSettings\.generationMode = stepwiseGenerationMode\(\);/);
    const appearanceStart = stepwise.indexOf("function appearanceSettingsHtml()");
    const settingsStart = stepwise.indexOf("function settingsHtml()", appearanceStart);
    const appearanceMarkup = stepwise.slice(appearanceStart, settingsStart);
    assert.doesNotMatch(appearanceMarkup, /data-action="generation-mode"/);
    const footerStart = stepwise.indexOf('<div class="csw-runtime-grid"', settingsStart);
    const generationModeControl = stepwise.indexOf('data-action="generation-mode"', footerStart);
    const promptClickControl = stepwise.indexOf('data-action="prompt-click-mode"', footerStart);
    assert.ok(footerStart >= 0 && generationModeControl > footerStart && promptClickControl > generationModeControl);
    assert.match(stepwise, /<span class="csw-metric-label">模式<\/span>/);
    assert.match(stepwise, /return normalizeGenerationMode\(value\) === "manual" \? "手动刷新" : "自动生成";/);
    assert.match(stepwise, /return setGenerationMode\(nextGenerationMode\(\)\);/);
    assert.match(stepwise, /return writePromptClickMode\(nextPromptClickMode\(\)\);/);
    assert.match(stepwise, /button\.csw-metric-action\s*\{[^}]*padding:\s*0;/s);
    assert.match(
      stepwise,
      /\.csw-click-mode,[\s\S]*?\.csw-generation-mode\s*\{[^}]*min-width:\s*0;/,
    );
    assert.match(stepwise, /\.csw-generation-mode\s*\{[^}]*white-space:\s*nowrap;/s);
    assert.match(
      stepwise,
      /\.csw-metric-value,[\s\S]*?\.csw-metric-action\s*\{[^}]*overflow:\s*visible;[^}]*text-overflow:\s*clip;/,
    );
    assert.match(
      stepwise,
      /\.csw-metric-value,[\s\S]*?\.csw-generation-mode \.csw-metric-action\s*\{[^}]*white-space:\s*nowrap;/,
    );
    assert.match(
      stepwise,
      /\.csw-click-mode \.csw-metric-action\s*\{[^}]*overflow-wrap:\s*anywhere;[^}]*white-space:\s*normal;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 440px\)[\s\S]*?\.csw-settings-footer\s*\{[^}]*display:\s*grid;[^}]*grid-template-columns:\s*minmax\(max-content, 1fr\) auto;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 440px\)[\s\S]*?\.csw-runtime-grid\s*\{[^}]*grid-template-columns:\s*minmax\(0, max-content\) minmax\(0, 1fr\);[^}]*width:\s*100%;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 440px\)[\s\S]*?\.csw-command-button\s*\{[^}]*flex:\s*0 0 30px;[^}]*height:\s*30px;[^}]*padding:\s*0;[^}]*width:\s*30px;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 440px\)[\s\S]*?\.csw-command-label\s*\{[^}]*display:\s*none;/,
    );
    assert.match(
      stepwise,
      /class="csw-command-button"[^>]*title="\$\{escapeAttr\(title\)\}"[^>]*aria-label="\$\{escapeAttr\(title\)\}"/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 360px\)[\s\S]*?\.csw-runtime-grid\s*\{[^}]*grid-template-columns:\s*minmax\(0, 1fr\);/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 360px\)[\s\S]*?\.csw-generation-mode,[\s\S]*?\.csw-click-mode\s*\{[^}]*width:\s*100%;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 320px\)[\s\S]*?\.csw-metric\s*\{[^}]*white-space:\s*nowrap;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 320px\)[\s\S]*?\.csw-command-button\s*\{[^}]*flex:\s*0 0 28px;[^}]*height:\s*28px;[^}]*width:\s*28px;/,
    );
    const toggleStart = stepwise.indexOf("async function setGenerationMode(value)");
    const immediateCancel = stepwise.indexOf(
      "applyRuntimeSettings({ ...(state.settings || {}), generationMode: nextMode });",
      toggleStart,
    );
    const settingsSave = stepwise.indexOf('bridgeCall("/settings/set", {', toggleStart);
    assert.ok(toggleStart >= 0 && immediateCancel > toggleStart && settingsSave > immediateCancel);

    const progressStart = stepwise.indexOf("function nextProgressState()");
    const manualProgressGuard = stepwise.indexOf('if (stepwiseGenerationMode() === "manual") return null;', progressStart);
    const localScanProgress = stepwise.indexOf('state.scanStatus === "assistant-changed"', progressStart);
    assert.ok(progressStart >= 0 && manualProgressGuard > progressStart && localScanProgress > manualProgressGuard);
    assert.match(stepwise, /title: "当前为手动模式"/);
    assert.doesNotMatch(stepwise, /title: "待生成"/);

    const outlineExpressionStart = stepwise.indexOf("function usesOutlineExpression(");
    const outlineExpressionEnd = stepwise.indexOf("function resolveFabExpression(", outlineExpressionStart);
    const outlineExpression = stepwise.slice(outlineExpressionStart, outlineExpressionEnd);
    assert.match(outlineExpression, /stepwiseWaitingForManualRefresh\(\)/);

    const runtimePresentationStart = stepwise.indexOf("function settingsRuntimePresentation(");
    const runtimePresentationEnd = stepwise.indexOf("function settingsCommandHtml(", runtimePresentationStart);
    const runtimePresentation = stepwise.slice(runtimePresentationStart, runtimePresentationEnd);
    assert.match(runtimePresentation, /!outlineExpression && stepwiseWaitingForManualRefresh\(settings\)/);

    const scanStart = stepwise.indexOf("function scan(");
    const outlineRefresh = stepwise.indexOf("void refreshOutline({ message, assistantHash: hash });", scanStart);
    const manualScanBranch = stepwise.indexOf('if (generationMode === "manual" && !manualResultVisible && !manualRequestPending)', scanStart);
    const cachedScanBranch = stepwise.indexOf('else if (hasSuccessfulCache)', scanStart);
    const automaticGenerate = stepwise.indexOf('requestBridgeStepwise(bridgeKey, userText, assistantText, "auto")', scanStart);
    assert.ok(scanStart >= 0 && outlineRefresh > scanStart && manualScanBranch > outlineRefresh);
    assert.ok(cachedScanBranch > manualScanBranch);
    assert.ok(automaticGenerate > cachedScanBranch);
  });

  it("keeps feature tab order draggable and persistent", async () => {
    const stepwise = await readStepwiseSource();

    assert.match(stepwise, /const VIEW_ORDER_KEY = "codex-stepwise-view-order-v1";/);
    assert.match(stepwise, /function normalizeViewOrder\(value\)/);
    assert.match(stepwise, /function persistViewOrder\(order\)/);
    assert.match(stepwise, /function installViewTabReorder\(\)/);
    assert.match(stepwise, /data-reorderable="true"/);
    assert.match(stepwise, /state\.viewReorderCleanup\?\.\(\)/);
    assert.match(stepwise, /syncViewTabSelection\(state\.activeTab, true\);/);
    assert.match(stepwise, /dataset\.codexStepwiseStyleVersion === SCRIPT_VERSION/);
    assert.match(stepwise, /style\.dataset\.codexStepwiseStyleVersion = SCRIPT_VERSION/);
    assert.match(stepwise, /VIEW_SLIDE_MS = 240/);
    assert.match(stepwise, /VIEW_INDICATOR_MS = 220/);
  });

  it("keeps Stepwise Manager controls at one height", async () => {
    const styles = await readFile(new URL("./styles.css", import.meta.url), "utf8");

    assert.match(styles, /\.stepwise-settings-block\s*\{[\s\S]*?--stepwise-control-height:\s*40px;/);
    assert.match(
      styles,
      /\.stepwise-settings-block input[^,]*,[\s\S]*?\.stepwise-settings-block \.app-select-trigger,[\s\S]*?\.stepwise-settings-block \.field-select,[\s\S]*?\.stepwise-settings-block \.select-input\s*\{[\s\S]*?height:\s*var\(--stepwise-control-height\);[\s\S]*?min-height:\s*var\(--stepwise-control-height\);/,
    );
  });
});

// issue #2256/#2255：app-server model request patch 的 miss 熔断以前被 provider
// 重试路径提前 return 绕过，失败变成 250ms 无限重试（每轮全量 fetch 全部 app asset）。
describe("renderer injection app-server model request patch", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  interface AppServerPatchHarness {
    install: () => void;
    sweeps: () => number;
    diagnostics: () => string[];
    settle: () => Promise<void>;
  }

  function appServerPatchRuntime(renderer: string, patchSucceeds: boolean): AppServerPatchHarness {
    const start = renderer.indexOf("  const appServerModelRequestPatchMaxMisses = ");
    const end = renderer.indexOf("\n  function ensureCodexModelWhitelistInstalls(", start);
    assert.ok(start >= 0 && end > start, "app-server model request patch block not found");
    const source = renderer.slice(start, end);

    let sweeps = 0;
    let pending: Array<() => void> = [];
    const diagnostics: string[] = [];
    const timers: Array<number> = [];
    const fakeWindow: Record<string, unknown> = {
      setTimeout: ((fn: () => void) => {
        timers.push(0);
        pending.push(fn);
        return 0;
      }) as unknown,
      clearTimeout: () => {},
    };

    const factory = new Function(
      "window",
      "codexAppServerModelRequestPatchVersion",
      "codexRemoteSessionProviderPatchEnabled",
      "loadAppServerRequestCandidates",
      "patchAppServerModelRequestClient",
      "sendCodexPlusDiagnostic",
      "Date",
      `${source}\nreturn installAppServerModelRequestPatch;`,
    );

    const install = factory(
      fakeWindow,
      1,
      // provider patch 开关两态都要测：以前 enabled 时走提前 return 绕过熔断。
      () => true,
      () =>
        new Promise((resolve) => {
          sweeps += 1;
          pending.push(() => resolve({ modules: [{}], candidates: [{}], sources: [], discovery: "fallback" }));
        }),
      () => patchSucceeds,
      (event: string) => diagnostics.push(event),
      Date,
    ) as () => void;

    const settle = async () => {
      // 重试定时器是挂起的回调：排空 sweep 再触发到期的 retry，直到没有新定时器。
      for (let round = 0; round < 32; round += 1) {
        if (!pending.length) break;
        const flushSweeps = pending;
        pending = [];
        flushSweeps.forEach((resolve) => resolve());
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
      }
    };

    return { install, sweeps: () => sweeps, diagnostics: () => diagnostics, settle };
  }

  it("does not start a new sweep while the previous one is still running", async () => {
    const harness = appServerPatchRuntime(await readFile(rendererPath, "utf8"), false);

    for (let i = 0; i < 20; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    await harness.settle();
  });

  it("stops retrying via the provider path once maxMisses is reached", async () => {
    const harness = appServerPatchRuntime(await readFile(rendererPath, "utf8"), false);

    // 反复 install + settle，让每轮 miss 走完 provider 重试调度。
    for (let i = 0; i < 40; i += 1) {
      harness.install();
      await harness.settle();
    }

    // 关键回归断言：以前 provider 路径无限重试（40 轮 = 40 次 sweep），
    // 现在到 maxMisses(8) 就熔断停手。
    assert.equal(harness.sweeps(), 8);
    assert.equal(harness.diagnostics().filter((e) => e === "model_app_server_request_patch_not_found").length, 1);
    assert.deepEqual(harness.diagnostics().at(-1), "model_app_server_request_patch_skipped");
    const settled = harness.sweeps();
    harness.install();
    await harness.settle();
    assert.equal(harness.sweeps(), settled);
  });

  it("keeps working normally when the patch actually lands", async () => {
    const harness = appServerPatchRuntime(await readFile(rendererPath, "utf8"), true);

    harness.install();
    await harness.settle();
    for (let i = 0; i < 10; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    assert.deepEqual(harness.diagnostics(), ["model_app_server_request_patch_installed"]);
  });
});

// renderer-inject.js 现在是「分片源码 + 生成产物」：真正的源码在
// assets/inject/renderer-inject/*.js，产物是拼回去的单体文件，供 assets.rs 的
// include_str! 和本文件的大量按路径断言消费。
// 这组用例防止有人只改产物、不回改分片，让两边悄悄分叉。
describe("renderer inject 分片与产物一致", () => {
  const fragmentDir = new URL("../../../assets/inject/renderer-inject/", import.meta.url);
  const artifactPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  async function assembleFromFragments(): Promise<{
    source: string;
    names: string[];
    fragments: Array<{ name: string; description?: string; lines: number }>;
  }> {
    const manifest = JSON.parse(
      await readFile(new URL("manifest.json", fragmentDir), "utf8"),
    ) as { fragments: Array<{ name: string; description?: string; lines: number }> };
    assert.ok(
      Array.isArray(manifest.fragments) && manifest.fragments.length > 0,
      "manifest.json 必须有 fragments",
    );
    const bodies = await Promise.all(
      manifest.fragments.map(async (fragment) => {
        assert.ok(fragment.name, "分片必须带 name");
        const body = await readFile(new URL(fragment.name, fragmentDir), "utf8");
        // 每片以换行收尾，直接相接才能还原原始行结构；漏了会让两处声明粘连。
        assert.ok(body.endsWith("\n"), `分片 ${fragment.name} 未以换行结尾`);
        return body;
      }),
    );
    return {
      source: bodies.join(""),
      names: manifest.fragments.map((f) => f.name),
      fragments: manifest.fragments,
    };
  }

  it("分片按 manifest 顺序拼接后与产物逐字节一致", async () => {
    const { source, names } = await assembleFromFragments();
    const artifact = await readFile(artifactPath, "utf8");
    assert.equal(
      source,
      artifact,
      `产物与分片不一致（分片 ${names.length} 个）。` +
        "改分片后请跑 node scripts/assemble-renderer-inject.mjs 重新组装。",
    );
  });

  it("拼接结果是完整可解析的单个 IIFE", async () => {
    const { source } = await assembleFromFragments();
    assert.ok(source.startsWith("(() => {\n"), "必须以 IIFE 开头");
    assert.ok(source.includes("\n})();\n"), "必须包含主 IIFE 的收尾");
    // 语法解析（不执行）能抓出分片切坏造成的括号不匹配。
    assert.doesNotThrow(() => new Function(source));
  });

  it("manifest 的行数与分片内容保持一致且没有重复文件", async () => {
    const { source, fragments } = await assembleFromFragments();
    const names = fragments.map((fragment) => fragment.name);
    assert.equal(new Set(names).size, names.length, "manifest 不得重复引用同一分片");
    assert.ok(fragments.every((fragment) => Number.isInteger(fragment.lines) && fragment.lines > 0));
    const manifestLines = fragments.reduce((total, fragment) => total + fragment.lines, 0);
    const artifactLines = source.endsWith("\n") ? source.slice(0, -1).split("\n").length : source.split("\n").length;
    assert.equal(manifestLines, artifactLines, "manifest 总行数必须覆盖完整产物");
    for (const fragment of fragments) {
      const body = await readFile(new URL(fragment.name, fragmentDir), "utf8");
      const lineCount = body.endsWith("\n") ? body.slice(0, -1).split("\n").length : body.split("\n").length;
      assert.equal(lineCount, fragment.lines, fragment.name);
    }
  });

  it("测试辅助依赖的锚点仍落在同一个分片里且顺序不变", async () => {
    // installRendererStyle() 用 indexOf 划区间，靠这两个标记定位。
    // 它们一旦被拆到不同分片、或前后顺序反转，helper 会静默取到空区间。
    const { source } = await assembleFromFragments();
    const start = source.indexOf("  function installStyle()");
    const end = source.indexOf("\n  function defaultCodexPlusSettings", start);
    assert.ok(start >= 0, "找不到 installStyle 锚点");
    assert.ok(end > start, "defaultCodexPlusSettings 必须排在 installStyle 之后");
  });

  it("拓展列表渲染不裸取 entry.item", async () => {
    // 已安装条目的形状是 { kind, key, name, meta, enabled, script }，没有 item；
    // 只有市场条目才有。裸取 entry.item.description 会在列表里只要有任意一个
    // 已安装脚本时就抛 TypeError，整块渲染中断，左面板停在「正在读取…」占位。
    // 已安装条目要走 codexPlusExtensionMarketItem(entry) 按 market_id 回查。
    const { source } = await assembleFromFragments();
    const openingTag = source.indexOf("  function renderCodexPlusExtensionsNav()");
    const closingTag = source.indexOf("  function refreshCodexPlusExtensionsView()", openingTag);
    assert.ok(openingTag >= 0, "找不到 renderCodexPlusExtensionsNav");
    assert.ok(closingTag > openingTag, "找不到函数结束位置");
    const body = source.slice(openingTag, closingTag);
    assert.ok(
      !/entry\.item\s*\./.test(body),
      "itemHtml 里出现了不带可选链的 entry.item.*，会在已安装条目上抛 TypeError",
    );
    assert.ok(
      body.includes("codexPlusExtensionMarketItem(entry)"),
      "itemHtml 应该用 codexPlusExtensionMarketItem(entry) 解析图标与简介来源",
    );
  });
});

/**
 * 拓展接口层的契约测试。
 *
 * 这些测试把 01-registry.js 与 91-extension-api.js 的定义抽出来单独执行，
 * 不依赖真实 DOM——注册中心本身刻意不读 DOM，正是为了让它可以这样被验证。
 */
describe("拓展注册中心", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  interface RegistryHarness {
    registry: { rowActions: Map<string, unknown>; navEntries: Map<string, unknown>; pages: Map<string, unknown>; menuItems: Map<string, unknown> };
    register: (kind: string, reg: Map<string, unknown>, id: string, def: unknown, key: string) => () => void;
    registerSelector: (selector: string) => boolean;
    extensionSelector: () => string;
    isExtensionNode: (node: unknown) => boolean;
    items: (reg: Map<string, unknown>) => Array<Record<string, unknown>>;
    runCallback: (key: string, label: string, cb: () => unknown) => { ok: boolean; value?: unknown; error?: string };
    failures: () => Array<{ script_key: string; message: string }>;
    registryLog: () => unknown[];
  }

  async function registryRuntime(): Promise<RegistryHarness> {
    // 直接读注册中心分片本身：它刻意不读 DOM、不依赖 prelude 常量，所以能独立执行。
    // 不要从产物里 indexOf 切片——注册中心与接口层之间隔着十来个分片，区间会失控。
    const source = await readFile(
      new URL("../../../assets/inject/renderer-inject/01-registry.js", import.meta.url),
      "utf8",
    );

    const windowValue: Record<string, unknown> = {};
    const documentValue = {
      // registerCodexPlusExtensionSelector 用空 fragment 验证选择器语法，
      // 非法选择器会抛错，模拟这个方法就足以覆盖合法性判定。
      createDocumentFragment: () => ({
        querySelector: (selector: string) => {
          if (/[<>]/.test(selector)) throw new Error("invalid selector");
          return null;
        },
      }),
    };
    const factory = new Function(
      "window",
      "document",
      `${source}
return { codexPlusRegistry, registerCodexPlusExtension, registerCodexPlusExtensionSelector,
  codexPlusExtensionSelector, isCodexPlusExtensionNode, codexPlusExtensionItems,
  runCodexPlusExtensionCallback };`,
    );
    const api = factory(windowValue, documentValue) as Record<string, unknown>;
    return {
      registry: api.codexPlusRegistry as RegistryHarness["registry"],
      register: api.registerCodexPlusExtension as RegistryHarness["register"],
      registerSelector: api.registerCodexPlusExtensionSelector as RegistryHarness["registerSelector"],
      extensionSelector: api.codexPlusExtensionSelector as RegistryHarness["extensionSelector"],
      isExtensionNode: api.isCodexPlusExtensionNode as RegistryHarness["isExtensionNode"],
      items: api.codexPlusExtensionItems as RegistryHarness["items"],
      runCallback: api.runCodexPlusExtensionCallback as RegistryHarness["runCallback"],
      failures: () => (windowValue.__codexPlusExtensionFailures || []) as Array<{ script_key: string; message: string }>,
      registryLog: () => (windowValue.__codexPlusRegistryLog || []) as unknown[],
    };
  }

  it("第三方项从 1000 起排，不会插到内置项前面", async () => {
    const runtime = await registryRuntime();
    runtime.register("rowAction", runtime.registry.rowActions, "builtin:a", { order: 10, onActivate() {} }, "builtin");
    runtime.register("rowAction", runtime.registry.rowActions, "ext:b", { order: 5, onActivate() {} }, "user:x.js");
    const items = runtime.items(runtime.registry.rowActions);
    // 传入的 5 被抬到 1000：内置项永远排在拓展项之前。
    assert.deepEqual(items.map((item) => item.id), ["builtin:a", "ext:b"]);
    assert.equal(items[1].order, 1000);
  });

  it("按脚本配额限制注册数量，超出时抛错", async () => {
    const runtime = await registryRuntime();
    for (let index = 0; index < 16; index += 1) {
      runtime.register("navEntry", runtime.registry.navEntries, `ext:${index}`, { onActivate() {} }, "user:noisy.js");
    }
    assert.throws(
      () => runtime.register("navEntry", runtime.registry.navEntries, "ext:overflow", { onActivate() {} }, "user:noisy.js"),
      /最多注册 16 项/,
    );
    // 另一个脚本不受影响。
    assert.doesNotThrow(
      () => runtime.register("navEntry", runtime.registry.navEntries, "ext:other", { onActivate() {} }, "user:other.js"),
    );
  });

  it("拒绝重复 id 与缺少回调的定义", async () => {
    const runtime = await registryRuntime();
    runtime.register("page", runtime.registry.pages, "ext:p", { render() {} }, "user:x.js");
    assert.throws(
      () => runtime.register("page", runtime.registry.pages, "ext:p", { render() {} }, "user:x.js"),
      /已被占用/,
    );
    assert.throws(
      () => runtime.register("page", runtime.registry.pages, "ext:empty", {}, "user:x.js"),
      /必须提供 render \/ onActivate/,
    );
  });

  it("菜单项的开关形态（只给 onChange）不被校验误拒", async () => {
    const runtime = await registryRuntime();
    // 开关形态没有 render / onActivate，早期实现会把它当成非法定义拒掉。
    assert.doesNotThrow(
      () => runtime.register("menuItem", runtime.registry.menuItems, "ext:toggle", { onChange() {} }, "user:x.js"),
    );
    assert.doesNotThrow(
      () => runtime.register("page", runtime.registry.pages, "ext:cleanup-only", { onCleanup() {} }, "user:x.js"),
    );
  });

  it("dispose 只移除自己注册的那一项", async () => {
    const runtime = await registryRuntime();
    const dispose = runtime.register("page", runtime.registry.pages, "ext:first", { render() {} }, "user:x.js");
    runtime.register("page", runtime.registry.pages, "ext:second", { render() {} }, "user:y.js");
    dispose();
    assert.deepEqual([...runtime.registry.pages.keys()], ["ext:second"]);
    // 重复 dispose 不应误删后来者。
    dispose();
    assert.deepEqual([...runtime.registry.pages.keys()], ["ext:second"]);
  });

  it("选择器登记拒绝非法语法，避免 closest() 在每次 mutation 上抛错", async () => {
    const runtime = await registryRuntime();
    assert.equal(runtime.registerSelector('[data-codex-plus-ext="a"]'), true);
    assert.match(runtime.extensionSelector(), /data-codex-plus-ext/);
    assert.equal(runtime.registerSelector("div << p"), false);
    assert.equal(runtime.registerSelector(""), false);
  });

  it("回调抛错时记进该脚本的失败通道，不向调用方抛出", async () => {
    const runtime = await registryRuntime();
    const outcome = runtime.runCallback("user:broken.js", "rowAction.onActivate", () => {
      throw new Error("boom");
    });
    assert.equal(outcome.ok, false);
    assert.match(String(outcome.error), /boom/);
    assert.equal(runtime.failures().length, 1);
    assert.equal(runtime.failures()[0].script_key, "user:broken.js");
  });
});

/**
 * 对外接口层的护栏测试。
 *
 * 接口层挂在 window.codexPlus 上、面向第三方脚本，所以「哪些能力被开放」必须
 * 有测试守着——新增路由时默认不开放，漏网才是 bug。
 */
describe("拓展接口层", () => {
  const apiFragmentPath = new URL(
    "../../../assets/inject/renderer-inject/91-extension-api.js",
    import.meta.url,
  );
  const artifactPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  async function readRouteWhitelist(): Promise<string[]> {
    const source = await readFile(apiFragmentPath, "utf8");
    const start = source.indexOf("const codexPlusExtensionRoutes = new Set([");
    const end = source.indexOf("]);", start);
    assert.ok(start >= 0 && end > start, "找不到路由白名单");
    const body = source.slice(start, end);
    return Array.from(body.matchAll(/"([^"]+)"/g), ([, route]) => route);
  }

  it("路由白名单不含设置写入与远端控制，CDP 注入需显式开放", async () => {
    const routes = await readRouteWhitelist();
    assert.ok(routes.length > 0, "白名单不能为空");
    // 这些路由一旦开放，第三方脚本就能改用户配置或操作远端，必须显式决策后才能加。
    for (const forbidden of ["/settings/set", "/settings/get", "/delete", "/undo", "/share/create"]) {
      assert.ok(!routes.includes(forbidden), `${forbidden} 不应默认开放给拓展`);
    }
    assert.ok(routes.includes("/diagnostics/log"), "诊断上报应保持开放");
    assert.ok(routes.includes("/cdp/targets"), "CDP 页面列表应开放给拓展");
    assert.ok(routes.includes("/cdp/evaluate"), "CDP evaluate 应开放给拓展");
  });

  it("接口对象在 IIFE 收尾前挂载", async () => {
    const renderer = await readFile(artifactPath, "utf8");
    const mount = renderer.indexOf("window.codexPlus = buildCodexPlusExtensionApi();");
    const tail = renderer.indexOf("\n})();\n");
    assert.ok(mount >= 0, "找不到 window.codexPlus 的挂载点");
    // 挂载必须在主 IIFE 结束之前，否则闭包里的函数已经不可达。
    assert.ok(mount < tail, "挂载点必须位于 IIFE 收尾之前");
  });
});

/**
 * 菜单项的接入方式护栏。
 *
 * 菜单的接入点是「在 home 面板模板末尾追加一块」，不是把内置的一百多行模板
 * 拆成数据结构——后者会动到内置 UI 主干。这些断言守住这个决定，以及点击
 * 委托的分支顺序。
 */
describe("拓展菜单项", () => {
  const settingsPath = new URL(
    "../../../assets/inject/renderer-inject/40-backend-settings.js",
    import.meta.url,
  );
  const hostPath = new URL(
    "../../../assets/inject/renderer-inject/92-extension-host.js",
    import.meta.url,
  );

  it("内置菜单模板保持原样，只追加一个拓展挂载点", async () => {
    const source = await readFile(settingsPath, "utf8");
    // 挂载点在 home 面板里，且位于「提出问题」之后（即内置项末尾）。
    const openIdx = source.indexOf("overlay.innerHTML = `");
    const mountIdx = source.indexOf("${renderCodexPlusExtensionMenuRows()}");
    const issueIdx = source.indexOf("提出问题");
    assert.ok(mountIdx > 0, "找不到拓展菜单挂载点");
    assert.ok(mountIdx > issueIdx, "挂载点应位于内置项之后");
    // 关键：内置的行仍然是内联模板，没有被拆成数组。
    assert.ok(
      source.slice(openIdx, mountIdx).includes('class="codex-plus-row"'),
      "内置菜单行应当仍是内联模板",
    );
    assert.ok(
      !/const codexPlusBuiltinMenuRows\s*=/.test(source),
      "不应把内置菜单行抽成数组",
    );
  });

  it("点击委托里拓展分支排在内置分支之前", async () => {
    const source = await readFile(settingsPath, "utf8");
    const handler = source.indexOf('overlay.addEventListener("click"');
    assert.ok(handler > 0, "找不到点击委托");
    const end = source.indexOf('overlay.addEventListener("keydown"', handler);
    assert.ok(end > handler, "找不到点击委托之后的键盘委托");
    const body = source.slice(handler, end);
    const extIdx = body.indexOf("handleCodexPlusExtensionMenuClick(target)");
    const devtoolsIdx = body.indexOf("data-codex-open-devtools");
    assert.ok(extIdx > 0, "点击委托应当调用拓展菜单处理器");
    assert.ok(devtoolsIdx > 0, "点击委托应当包含内置 DevTools 分支");
    assert.ok(extIdx < devtoolsIdx, "拓展分支应排在内置分支之前");
  });

  it("开关与按钮分别渲染成不同的控件", async () => {
    const source = await readFile(hostPath, "utf8");
    const start = source.indexOf("function renderCodexPlusExtensionMenuRows()");
    const end = source.indexOf("function handleCodexPlusExtensionMenuClick(", start);
    assert.ok(start > 0 && end > start, "找不到菜单渲染函数");
    const body = source.slice(start, end);
    assert.match(body, /data-codex-plus-ext-setting/, "开关应有自己的 data 属性");
    assert.match(body, /data-codex-plus-ext-action/, "按钮应有自己的 data 属性");
    // 是追加到 home 面板，不是替换它。
    assert.ok(!/panel\.innerHTML\s*=/.test(body), "不应整体替换面板内容");
  });

  it("菜单项失败经由脚本状态上报，不向调用方抛出", async () => {
    const source = await readFile(hostPath, "utf8");
    const start = source.indexOf("function handleCodexPlusExtensionMenuClick(");
    const body = source.slice(start, start + 1400);
    assert.match(body, /runCodexPlusExtensionCallback/, "回调必须经失败隔离包装");
    assert.ok(!/try\s*{[\s\S]*item\.onActivate\(/.test(body), "不应裸调 onActivate");
  });
});

/**
 * 会话视图对齐（issue #2258 / #2085）的夹具运行时。
 *
 * 这里跑的是产物里真实的目标查找与宽度计算函数，配一个极简 DOM 桩——
 * 只实现查找链路真正用到的方法（querySelector/matches/closest/getBoundingClientRect）。
 * 类名夹具直接取自 Codex 打包产物里的字符串，不手写臆造。
 */
function conversationViewRuntime(renderer: string) {
  const start = renderer.indexOf("  // 旧版（26.9xx 之前）内容容器类名清单");
  const end = renderer.indexOf("  function removeLegacyCodexServiceTierBadges(", start);
  assert.ok(start >= 0 && end > start, "conversation view target finder block not found");
  const finderSource = renderer.slice(start, end);
  // 宽度计算与目标查找在文件里不相邻，单独切一段。
  const widthStart = renderer.indexOf("  function conversationViewEffectiveWidth(");
  const widthEnd = renderer.indexOf("  function conversationViewHtmlCenter(", widthStart);
  assert.ok(widthStart >= 0 && widthEnd > widthStart, "conversation view width block not found");
  const widthSource = renderer.slice(widthStart, widthEnd);
  const source = `${finderSource}\n${widthSource}`;

  const selectorStart = renderer.indexOf("  const selectors = {");
  const selectorEnd = renderer.indexOf("  };", selectorStart) + 4;
  assert.ok(selectorStart >= 0, "selectors table not found");
  const selectorsTable = renderer.slice(selectorStart, selectorEnd);

  type FakeEl = {
    tagName: string;
    className: string;
    attrs: Record<string, string>;
    children: FakeEl[];
    parentElement: FakeEl | null;
    style: Record<string, string>;
    dataset: Record<string, string>;
    rect: { left: number; width: number };
    vars: Record<string, string>;
    computedMaxWidth: string;
    padding: { left: string; right: string };
    querySelectorAll(selector: string): FakeEl[];
    querySelector(selector: string): FakeEl | null;
    matches(selector: string): boolean;
    closest(selector: string): FakeEl | null;
    contains(el: FakeEl): boolean;
    getBoundingClientRect(): { left: number; width: number; right: number; top: number; bottom: number; height: number };
  };

  function matches(el: FakeEl, selector: string): boolean {
    if (selector.includes(",")) return selector.split(",").some((part) => matches(el, part.trim()));
    const taggedAttribute = selector.match(/^([a-z]+)(\[.*\])$/);
    if (taggedAttribute) return el.tagName.toLowerCase() === taggedAttribute[1] && matches(el, taggedAttribute[2]);
    if (selector === "div") return el.tagName === "DIV";
    if (/^[a-z]+$/.test(selector)) return el.tagName.toLowerCase() === selector;
    if (selector.startsWith("#")) return el.attrs.id === selector.slice(1);
    if (selector.startsWith("[") && selector.endsWith("]")) {
      const body = selector.slice(1, -1);
      const eq = body.indexOf("=");
      const name = eq === -1 ? body : body.slice(0, eq);
      const value = eq === -1 ? null : body.slice(eq + 1).replace(/^["']|["']$/g, "");
      const actual = el.attrs[name];
      if (actual === undefined) return false;
      return value === null ? true : actual === value;
    }
    if (selector.startsWith(".")) return el.className.split(/\s+/).includes(selector.slice(1));
    throw new Error(`夹具未实现的选择器: ${selector}`);
  }

  function descendants(root: FakeEl, out: FakeEl[] = []): FakeEl[] {
    for (const child of root.children) {
      out.push(child);
      descendants(child, out);
    }
    return out;
  }

  function query(root: FakeEl, selector: string): FakeEl[] {
    const parts = selector.split(",").map((part) => part.trim());
    return descendants(root).filter((el) => parts.some((part) => matches(el, part)));
  }

  function el(tag: string, className = "", attrs: Record<string, string> = {}): FakeEl {
    const node: FakeEl = {
      tagName: tag.toUpperCase(),
      className,
      attrs,
      children: [],
      parentElement: null,
      style: {},
      dataset: {},
      rect: { left: 0, width: 0 },
      vars: {},
      computedMaxWidth: "none",
      padding: { left: "", right: "" },
      querySelectorAll: (selector: string) => query(node, selector),
      querySelector: (selector: string) => query(node, selector)[0] ?? null,
      matches: (selector) => matches(node, selector),
      closest: (selector) => {
        let current: FakeEl | null = node;
        while (current) {
          if (matches(current, selector)) return current;
          current = current.parentElement;
        }
        return null;
      },
      contains: (el) => node === el || descendants(node).includes(el),
      getBoundingClientRect: () => ({
        left: node.rect.left,
        width: node.rect.width,
        right: node.rect.left + node.rect.width,
        top: 0,
        bottom: 0,
        height: 0,
      }),
    };
    return node;
  }

  function append(parent: FakeEl, child: FakeEl): FakeEl {
    parent.children.push(child);
    child.parentElement = parent;
    return child;
  }

  const root = el("body");
  const documentValue = {
    querySelector: (selector: string) => query(root, selector)[0] ?? null,
    querySelectorAll: (selector: string) => query(root, selector),
    documentElement: { getBoundingClientRect: () => ({ left: 0, width: 1440, right: 1440, top: 0, bottom: 0, height: 0 }) },
  };
  const factory = new Function(
    "document",
    "getComputedStyle",
    "sendCodexPlusDiagnostic",
    "codexPlusSettings",
    "localStorage",
    `${selectorsTable}
const conversationViewMinWidth = 320;
const conversationViewMaxAllowedWidth = 4000;
const conversationViewDefaultWidth = 900;
const conversationViewLegacyWidthKey = "codexPlus.threadCenter.maxWidth";
// 夹具只需要观察 style 写入结果，原始值记录这一步用空实现顶掉。
function conversationViewRememberOriginals() {}
function normalizeConversationViewWidth(value) {
  if (value === null || value === undefined || String(value).trim() === "") return null;
  const number = Number(value);
  if (!Number.isFinite(number)) return null;
  return Math.max(320, Math.min(4000, Math.round(number)));
}
function conversationViewWidth() {
  const settingsWidth = normalizeConversationViewWidth(codexPlusSettings().conversationViewMaxWidth);
  if (settingsWidth) return settingsWidth;
  const legacyWidth = normalizeConversationViewWidth(localStorage.getItem(conversationViewLegacyWidthKey));
  return legacyWidth || 900;
}
${source}
return {
  findContent: conversationViewFindContentEl,
  findComposer: conversationViewFindComposerEl,
  effective: conversationViewEffectiveWidth,
  available: conversationViewAvailableWidth,
  apply: conversationViewApplyNativeWidth,
};`,
  );

  const api = factory(
    documentValue,
    (node: FakeEl) => {
      // 同时充当 padding 来源与 CSSStyleDeclaration：兜底的变量反查会读索引属性。
      const style: Record<string, unknown> = {
        paddingLeft: node.padding.left,
        paddingRight: node.padding.right,
        maxWidth: node.computedMaxWidth,
        length: Object.keys(node.vars).length,
        getPropertyValue: (name: string) => node.vars[name] ?? "",
      };
      Object.keys(node.vars).forEach((name, index) => { style[index] = name; });
      return style;
    },
    () => undefined,
    () => ({ conversationView: true, conversationViewMaxWidth: 900 }),
    { getItem: () => null },
  ) as {
    findContent(): FakeEl | null;
    findComposer(): FakeEl | null;
    effective(containerWidth: number): number;
    available(el: FakeEl): number;
    apply(el: FakeEl, effectiveWidth: number): void;
  };

  return { root, el, append, api };
}

describe("renderer injection conversation view alignment", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // 夹具类名取自真实产物：
  //   旧版内容容器 "… flex shrink-0 flex-col pb-8"
  //   新版内容容器 "relative flex flex-1 shrink-0 flex-col"
  //   页脚包裹层   "relative z-10 flex flex-col mx-auto w-full max-w-(--thread-content-max-width) px-toolbar"
  const legacyContentClass = "mx-auto w-full max-w-(--thread-content-max-width) px-toolbar relative flex shrink-0 flex-col pb-8";
  const newContentClass = "relative flex flex-1 shrink-0 flex-col";

  async function buildFixture(mode: "legacy" | "modern") {
    const runtime = conversationViewRuntime(await readFile(rendererPath, "utf8"));
    const scroller = runtime.append(runtime.root, runtime.el("div", "thread-scroll-container"));
    const host = runtime.append(scroller, runtime.el("div", "h-full flex"));
    if (mode === "legacy") {
      runtime.append(host, runtime.el("div", legacyContentClass));
    } else {
      // 新版：结构上只剩居中 + 满宽 + thread 宽度工具类，pb-8 已并入条件组合。
      runtime.append(host, runtime.el(
        "div",
        "relative flex flex-1 shrink-0 flex-col",
        { "data-thread-user-message-navigation-content": "" },
      ));
    }
    // 页脚包裹层带同样的宽度工具类，是最容易被误认成内容容器的节点。
    const footer = runtime.append(host, runtime.el(
      "div",
      "relative z-10 flex flex-col mx-auto w-full max-w-(--thread-content-max-width) px-toolbar",
      { "data-thread-scroll-footer": "true" },
    ));
    runtime.append(footer, runtime.el(
      "div",
      "mx-auto flex w-full max-w-(--thread-body-max-width) flex-col gap-8",
    ));
    return { runtime, scroller, host, footer };
  }

  it("旧版类名容器仍然命中", async () => {
    const { runtime } = await buildFixture("legacy");
    const found = runtime.api.findContent();
    assert.ok(found, "旧版类名应命中");
    assert.match(found.className, /pb-8/);
  });

  // 回归点：新版 Codex 把 max-w-(--thread-content-max-width) 换成
  // max-w-(--thread-body-max-width)、pb-8 消失后，全等匹配归零、居中宽度规则整体失效（#2258）。
  it("新版容器结构变化后仍能命中内容容器", async () => {
    const { runtime, footer } = await buildFixture("modern");
    const found = runtime.api.findContent();
    assert.ok(found, "新版类名变化后必须仍有降级路径，不能返回 null");
    assert.notEqual(found, footer, "不能把页脚包裹层当成内容容器");
    assert.equal(found.attrs["data-thread-user-message-navigation-content"], "");
  });

  it("作曲器优先落在页脚结构内，而不是内容容器", async () => {
    const { runtime, footer } = await buildFixture("modern");
    const found = runtime.api.findComposer();
    assert.ok(found, "作曲器应命中");
    assert.ok(found.closest("[data-thread-scroll-footer]"), "作曲器应在页脚包裹层内");
    assert.notEqual(found, footer);
  });

  // 用途词换过好几轮（content → body），所以按形状而不是字面量识别；
  // 与 50-navigation.js 的 ...FilterSourcePattern 同一套做法。
  it("宽度工具类按形状识别，换用途词也认", async () => {
    const renderer = await readFile(rendererPath, "utf8");
    const match = renderer.match(/const conversationViewThreadWidthTokenPattern\s*=\s*\/([\s\S]*?)\/;/);
    assert.ok(match, "未找到 conversationViewThreadWidthTokenPattern");
    const pattern = new RegExp(match![1]);
    for (const token of [
      "max-w-(--thread-content-max-width)", // 旧版
      "max-w-(--thread-body-max-width)", // 26.9xx 起
      "max-w-(--thread-content-responsive-max-width)",
      "md:max-w-(--thread-content-max-width)", // 带断点前缀
    ]) {
      assert.ok(pattern.test(token), `应命中: ${token}`);
    }
    // 哈希类名与无关工具类必须排掉；尤其不能把 max-w-2xl 这类固定宽度也算进来。
    for (const token of ["max-w-2xl", "_shell_151xi_3", "max-w-full", "w-full"]) {
      assert.ok(!pattern.test(token), `不应命中: ${token}`);
    }
  });

  it("两处类名都不对时按 CSS 变量反查宿主节点", async () => {
    const runtime = conversationViewRuntime(await readFile(rendererPath, "utf8"));
    const scroller = runtime.append(runtime.root, runtime.el("div", "thread-scroll-container"));
    const host = runtime.append(scroller, runtime.el("div", "h-full flex"));
    // 既没有新版类名工具类，也没有 data-* 锚点，只剩 Codex 注入的 CSS 变量。
    const bare = runtime.append(host, runtime.el("div", "some-renamed-class"));
    bare.vars["--thread-body-max-width"] = "calc(900px + 0px)";
    bare.computedMaxWidth = "900px";
    const found = runtime.api.findContent();
    assert.ok(found, "兜底候选应能反查到宿主节点");
    assert.equal(found, bare);
  });

  it("目标全部缺失时上报诊断，不再静默 return", async () => {
    const renderer = await readFile(rendererPath, "utf8");
    const start = renderer.indexOf("  function conversationViewAlignNow()");
    const end = renderer.indexOf("  function conversationViewHasRoomForHtmlCenterAt(", start);
    assert.ok(start >= 0 && end > start);
    const body = renderer.slice(start, end);
    assert.match(body, /conversationViewReportMissingTargets\(\)/, "归零路径必须上报诊断");
    assert.match(renderer, /"conversation_view_target_not_found"/);
  });

  // #2085：设置值是上限，实际宽度按容器可用宽度收敛。
  it("容器窄于设置上限时按容器宽度收敛，宽于上限时用上限", async () => {
    const { runtime } = await buildFixture("modern");
    // 设置 900（夹具里 conversationViewMaxWidth = 900）。
    assert.equal(runtime.api.effective(1400), 900, "宽容器应按设置上限");
    assert.equal(runtime.api.effective(600), 600, "窄容器应按容器可用宽度");
    assert.equal(runtime.api.effective(0), 900, "拿不到几何时回落设置上限");
    assert.equal(runtime.api.effective(Number.NaN), 900);
    // 硬下限仍是既有边界，不因自适应而改变。
    assert.equal(runtime.api.effective(100), 320);
  });

  it("可用宽度取宿主内容盒（扣掉左右内边距）", async () => {
    const { runtime, host } = await buildFixture("modern");
    host.rect.width = 1000;
    host.padding.left = "16px";
    host.padding.right = "16px";
    const target = runtime.api.findContent()!;
    assert.equal(runtime.api.available(target), 968);
    // 写进 style 的是收敛后的值，不是写死设置值。
    runtime.api.apply(target, runtime.api.effective(runtime.api.available(target)));
    assert.equal(target.style.maxWidth, "900px");
    host.rect.width = 500;
    runtime.api.apply(target, runtime.api.effective(runtime.api.available(target)));
    assert.equal(target.style.maxWidth, "468px");
  });

  // 自适应计算必须留在既有的「先统一写 style、再统一读几何」两阶段里，
  // 否则退回 commit 82fb0924 修掉的读-写交替强制重排。
  it("自适应计算不引入逐元素读-写交替", async () => {
    const renderer = await readFile(rendererPath, "utf8");
    const start = renderer.indexOf("  function conversationViewAlignNow()");
    const end = renderer.indexOf("  function conversationViewReportMissingTargets(", start);
    const body = renderer.slice(start, end);
    // 宽度读取必须集中在写之前，且是一次 map 批量读取。
    const readIndex = body.indexOf("conversationViewAvailableWidth(el)");
    const writeIndex = body.indexOf("conversationViewApplyNativeWidth(el");
    assert.ok(readIndex >= 0 && writeIndex > readIndex, "读取必须在写入之前");
    assert.match(body, /targets\.map\(\(el\) => conversationViewAvailableWidth\(el\)\)/);
  });
});

describe("renderer model reasoning efforts patch", () => {
  // 补全逻辑只依赖 codexPlusModelMetadata，抽出来单独跑，避免再复制一份实现。
  function reasoningRuntime(renderer: string, metadata: Record<string, unknown> | null) {
    const start = renderer.indexOf("function modelReasoningEfforts(");
    const end = renderer.indexOf("\n  function applyCodexPlusModelMetadata(", start);
    assert.ok(start >= 0 && end > start, "产物里找不到 modelReasoningEfforts");
    const source = renderer.slice(start, end);
    const create = new Function(
      "codexPlusModelMetadata",
      `${source}\nreturn modelReasoningEfforts;`,
    ) as (
      readMetadata: () => Record<string, unknown> | null,
    ) => (modelName: string) => Array<{ reasoningEffort: string }>;
    return create(() => metadata);
  }

  const threeLevelDeclaration = {
    supportedReasoningEfforts: [
      { reasoningEffort: "low", description: "Fast responses with lighter reasoning" },
      { reasoningEffort: "high", description: "Greater reasoning depth for complex problems" },
      { reasoningEffort: "max", description: "Maximum reasoning depth for the hardest problems" },
    ],
  };

  it("不会给只声明三档的 deepseek 模型补出 ultra", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const efforts = reasoningRuntime(renderer, threeLevelDeclaration)("cpx-relay-mv5bil9a/deepseek-flash");

    assert.deepEqual(
      efforts.map((entry) => entry.reasoningEffort),
      ["low", "high", "max"],
    );
  });

  it("仍为名单内确认支持 ultra 的模型补 max 与 ultra", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const efforts = reasoningRuntime(renderer, {
      supportedReasoningEfforts: ["low", "medium", "high", "xhigh"].map((reasoningEffort) => ({ reasoningEffort })),
    })("gpt-5.6-sol");

    assert.deepEqual(
      efforts.map((entry) => entry.reasoningEffort),
      ["low", "medium", "high", "xhigh", "max", "ultra"],
    );
  });

  it("没有档位声明时仍回落到完整兜底列表", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const efforts = reasoningRuntime(renderer, null)("deepseek-flash");

    assert.deepEqual(
      efforts.map((entry) => entry.reasoningEffort),
      ["low", "medium", "high", "xhigh", "max", "ultra"],
    );
  });
});

describe("Codex++ 设置页按设置项清单生成", () => {
  type CatalogItem = {
    key: string; group: string; control: string; title: string; description: string; descriptionWhenEnhancementsOn?: string;
    surfaces: string[]; platforms?: string[]; codexInputs?: Array<{ id: string; label: string; hint?: string }>;
  };
  type Catalog = { groups: Array<{ id: string; title: string; description?: string }>; items: CatalogItem[] };
  const fragmentDir = new URL("../../../assets/inject/renderer-inject/", import.meta.url);
  const readCatalog = async (): Promise<Catalog> => JSON.parse(await readFile(settingsCatalogUrl, "utf8"));
  const visibleOn = (platform: string) => (item: CatalogItem) => item.surfaces.includes("codex")
    && (!item.platforms || item.platforms.includes(platform));

  it("注入分片里不再写死任何设置项标题、说明或分组名", async () => {
    const catalog = await readCatalog();
    const settingsPage = await readFile(new URL("40-backend-settings.js", fragmentDir), "utf8");
    for (const group of catalog.groups) {
      assert.ok(!settingsPage.includes(`aria-label="${group.title}"`), `分组「${group.title}」不应写死在设置页模板里`);
      assert.ok(!settingsPage.includes(`>${group.title}</h2>`), `分组「${group.title}」不应写死在设置页模板里`);
    }
    for (const item of catalog.items) {
      assert.ok(!settingsPage.includes(item.title), `「${item.title}」的标题应来自清单`);
      assert.ok(!settingsPage.includes(item.description), `「${item.title}」的说明应来自清单`);
    }
    // 说明文案足够长、足够独特：任何分片里出现都意味着又多了一份副本。
    const manifest = JSON.parse(await readFile(new URL("manifest.json", fragmentDir), "utf8"));
    for (const fragment of manifest.fragments as Array<{ name: string }>) {
      const source = await readFile(new URL(fragment.name, fragmentDir), "utf8");
      for (const item of catalog.items) assert.ok(!source.includes(item.description), `${fragment.name} 里有「${item.title}」说明的副本`);
    }
  });

  it("每个在 Codex 显示的设置项生成一行，文案与绑定都取自清单", async () => {
    const catalog = await readCatalog();
    const page = await settingsCatalogRuntime(async (path) => {
      assert.equal(path, "/settings/catalog");
      return catalog;
    });
    await page.load();
    assert.equal(page.status().status, "ok");
    const html = page.sections();
    const sections = parseSettingsSections(html);
    const rows = sections.flatMap((section) => section.rows.filter((row) => row.key));
    const expected = catalog.groups.flatMap((group) => catalog.items.filter((item) => item.group === group.id && visibleOn("macos")(item)));
    assert.deepEqual(rows.map((row) => row.key), expected.map((item) => item.key));
    for (const item of expected) {
      const row = rows.find((candidate) => candidate.key === item.key)!;
      const section = sections.find((candidate) => candidate.rows.includes(row))!;
      assert.equal(section.title, catalog.groups.find((group) => group.id === item.group)!.title);
      assert.equal(row.title, item.title);
      assert.equal(row.description, item.descriptionWhenEnhancementsOn ?? item.description);
      if (item.control === "bool") {
        assert.ok(row.control.includes(`role="switch" aria-checked="false" aria-label="${item.title}"`), `${item.key} 开关缺少读屏标签`);
        assert.match(row.control, /data-codex-plus-setting="[^"]+"|data-codex-backend-setting="[^"]+"/);
      }
      for (const input of item.codexInputs ?? []) {
        assert.ok(row.control.includes(`aria-label="${input.label}"`), `${item.key} 缺少行内输入框 ${input.id}`);
      }
    }
    // 第一组最前面固定是后端连接状态，它不是设置项。
    assert.match(sections[0].rows[0].control, /data-codex-backend-status="true"/);
    // 能映射到本页 key 的走 setCodexPlusSetting，其余直接写后端字段。
    assert.match(html, /data-codex-plus-setting="stepwise"/);
    assert.match(html, /data-codex-plus-setting="sessionShare"/);
    assert.match(html, /data-codex-backend-setting="providerSyncEnabled"/);
    assert.match(html, /data-codex-plus-capacity-retry-interval="true" min="0" max="600"/);
    assert.match(html, /data-codex-plus-conversation-view-width="true" min="320" max="4000" step="10" type="number" value="900"/);
    // 管理器专属项不出现在 Codex 设置页。
    for (const item of catalog.items.filter((candidate) => !candidate.surfaces.includes("codex"))) {
      assert.ok(!html.includes(item.title), `${item.key} 只在管理器显示`);
    }
  });

  it("独立模式补丁的说明随增强总开关切换，与管理器一致", async () => {
    const catalog = await readCatalog();
    const switching = catalog.items.filter((item) => item.descriptionWhenEnhancementsOn && item.surfaces.includes("codex"));
    assert.ok(switching.length > 0);
    for (const enhancementsEnabled of [true, false]) {
      const page = await settingsCatalogRuntime(async () => catalog, undefined, { enhancementsEnabled });
      await page.load();
      const rows = parseSettingsSections(page.sections()).flatMap((section) => section.rows);
      for (const item of switching) {
        const row = rows.find((candidate) => candidate.key === item.key)!;
        assert.equal(row.description, enhancementsEnabled ? item.descriptionWhenEnhancementsOn : item.description, `${item.key} enhancements=${enhancementsEnabled}`);
        assert.match(row.control + page.sections(), new RegExp(`data-codex-plus-setting-description="${item.key}"`));
      }
    }
  });

  it("按清单的平台限制过滤设置项", async () => {
    const catalog = await readCatalog();
    const limited = catalog.items.filter((item) => item.platforms && item.surfaces.includes("codex"));
    assert.ok(limited.length > 0);
    for (const [platform, flags] of [["windows", { windows: true, mac: false }], ["macos", { windows: false, mac: true }], ["linux", { windows: false, mac: false }]] as const) {
      const page = await settingsCatalogRuntime(async () => catalog, flags);
      await page.load();
      const html = page.sections();
      for (const item of limited) {
        assert.equal(html.includes(`data-codex-plus-setting-row="${item.key}"`), item.platforms!.includes(platform), `${item.key} on ${platform}`);
      }
    }
  });

  it("读不到清单时给出原因和重试，不退回写死的副本", async () => {
    const catalog = await readCatalog();
    let response: unknown = { status: "failed", message: "Unknown bridge path" };
    const page = await settingsCatalogRuntime(async () => response);
    const loading = page.sections();
    assert.match(loading, /data-codex-plus-settings-catalog-state="loading"/);
    assert.match(loading, /data-codex-backend-status="true"/);
    await page.load();
    assert.equal(page.status().status, "failed");
    const failed = page.sections();
    assert.match(failed, /data-codex-plus-settings-catalog-state="failed" role="alert"/);
    assert.match(failed, /Unknown bridge path/);
    assert.match(failed, /data-codex-plus-settings-catalog-retry="true"/);
    assert.match(failed, /data-codex-backend-status="true"/, "后端连接状态在失败时也要能看到");
    for (const item of catalog.items) assert.ok(!failed.includes(item.title), "失败时不能显示任何兜底的设置副本");
    assert.deepEqual(page.diagnostics.map((entry) => entry.name), ["settings_catalog_load_failed"]);
    assert.ok(page.refreshes() >= 1);

    response = { groups: "broken" };
    await page.load();
    assert.equal(page.status().status, "failed");
    assert.match(page.sections(), /格式不正确/);

    response = catalog;
    await page.load();
    assert.equal(page.status().status, "ok");
    assert.match(page.sections(), /data-codex-plus-setting-row="enhancementsEnabled"/);

    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    assert.match(renderer, /if \(target\?\.closest\("\[data-codex-plus-settings-catalog-retry\]"\)\) \{\s*void loadCodexPlusSettingsCatalog\(\);/);
  });

  it("清单路由只读且不开放给拓展", async () => {
    const api = await readFile(new URL("91-extension-api.js", fragmentDir), "utf8");
    const start = api.indexOf("const codexPlusExtensionRoutes = new Set([");
    const whitelist = api.slice(start, api.indexOf("]);", start));
    assert.ok(whitelist.length > 0);
    assert.doesNotMatch(whitelist, /\/settings\/catalog/);
    const settingsPage = await readFile(new URL("40-backend-settings.js", fragmentDir), "utf8");
    assert.match(settingsPage, /postJson\("\/settings\/catalog", \{\}\)/);
  });
});
