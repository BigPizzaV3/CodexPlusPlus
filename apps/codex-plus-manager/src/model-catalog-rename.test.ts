import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const bundle = readFileSync(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const manifest = JSON.parse(readFileSync(new URL("../../../assets/inject/renderer-inject/manifest.json", import.meta.url), "utf8"));
const pickerIndex = manifest.fragments.findIndex((entry: { name: string }) => entry.name === "71-model-picker-groups.js");
const prefixLines = manifest.fragments.slice(0, pickerIndex + 1).reduce((sum: number, entry: { lines: number }) => sum + entry.lines, 0);
// 执行最终产物的真实安装代码；只把现有 service-tier 测试出口移到 71 安装之后。
const prefix = bundle.split("\n").slice(0, prefixLines).join("\n").replace(
  "    return;\n  }\n\n  function codexPlusModelUnlockEnabled()",
  "  }\n\n  function codexPlusModelUnlockEnabled()",
);
const startupIndex = manifest.fragments.findIndex((entry: { name: string }) => entry.name === "99-startup.js");
const installedLines = manifest.fragments.slice(0, startupIndex).reduce((sum: number, entry: { lines: number }) => sum + entry.lines, 0);
const installed = bundle.split("\n").slice(0, installedLines).join("\n").replace(
  "    return;\n  }\n\n  function codexPlusModelUnlockEnabled()",
  "  }\n\n  function codexPlusModelUnlockEnabled()",
);
const recentsKey = "codexPlus.modelPicker.recents.v1";

class Element {
  tagName: string;
  children: Element[] = [];
  parentElement: Element | null = null;
  attributes = new Map<string, string>();
  listeners = new Map<string, Array<(event: any) => void>>();
  dataset = {};
  hidden = false;
  isConnected = true;
  innerHTML = "";
  title = "";
  private text = "";
  private styles = new Map<string, string>();
  style = {
    setProperty: (name: string, value: string) => this.styles.set(name, value),
    getPropertyValue: (name: string) => this.styles.get(name) || "",
    getPropertyPriority: () => "",
    removeProperty: (name: string) => this.styles.delete(name),
  };
  classList = {
    contains: (name: string) => this.className.split(/\s+/).includes(name),
    add: (...names: string[]) => { this.className = [...new Set([...this.className.split(/\s+/).filter(Boolean), ...names])].join(" "); },
    remove: (...names: string[]) => { this.className = this.className.split(/\s+/).filter((name) => !names.includes(name)).join(" "); },
    toggle: (name: string, force: boolean) => force ? this.classList.add(name) : this.classList.remove(name),
  };
  constructor(tag = "div") { this.tagName = tag.toUpperCase(); }
  get className() { return this.getAttribute("class") || ""; }
  set className(value: string) { this.setAttribute("class", value); }
  get textContent(): string { return this.text + this.children.map((child) => child.textContent).join(""); }
  set textContent(value: string) { this.text = value; this.children = []; }
  get firstChild() { return this.children[0] || null; }
  get firstElementChild() { return this.firstChild; }
  get nextSibling(): Element | null { return this.parentElement?.children[(this.parentElement?.children.indexOf(this) ?? -1) + 1] || null; }
  setAttribute(name: string, value: string) { this.attributes.set(name, String(value)); }
  getAttribute(name: string) { return this.attributes.get(name) ?? null; }
  hasAttribute(name: string) { return this.attributes.has(name); }
  removeAttribute(name: string) { this.attributes.delete(name); }
  toggleAttribute(name: string, force: boolean) { if (force) this.setAttribute(name, ""); else this.removeAttribute(name); }
  appendChild(node: Element) { return this.insertBefore(node, null); }
  append(...nodes: Element[]) { nodes.forEach((node) => this.appendChild(node)); }
  prepend(node: Element) { this.insertBefore(node, this.firstChild); }
  insertBefore(node: Element, anchor: Element | null) {
    node.remove();
    const index = anchor ? this.children.indexOf(anchor) : this.children.length;
    this.children.splice(index, 0, node);
    node.parentElement = this;
    return node;
  }
  remove() {
    if (this.parentElement) this.parentElement.children.splice(this.parentElement.children.indexOf(this), 1);
    this.parentElement = null;
  }
  contains(node: Element | null): boolean { return node === this || this.children.some((child) => child.contains(node)); }
  matches(selector: string): boolean {
    return selector.split(",").some((part) => {
      const simple = part.trim();
      const tag = simple.match(/^[A-Za-z]+/)?.[0];
      if (tag && this.tagName !== tag.toUpperCase()) return false;
      const classes = [...simple.replace(/\[[^\]]*\]/g, "").matchAll(/\.([\w-]+)/g)];
      if (classes.some(([, name]) => !this.classList.contains(name))) return false;
      return [...simple.matchAll(/\[([\w-]+)(?:(\*=|\^=|=)["']?([^"'\]]*)["']?)?\]/g)].every(([, name, operator, value]) => {
        const actual = this.getAttribute(name);
        return operator === "=" ? actual === value : operator === "*=" ? actual?.includes(value)
          : operator === "^=" ? actual?.startsWith(value) : actual !== null;
      });
    });
  }
  querySelectorAll(selector: string): Element[] {
    if (selector.startsWith(":scope > ")) return this.children.filter((child) => child.matches(selector.slice(9)));
    return this.children.flatMap((child) => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  querySelector(selector: string) { return this.querySelectorAll(selector)[0] || null; }
  closest(selector: string): Element | null { return this.matches(selector) ? this : this.parentElement?.closest(selector) || null; }
  addEventListener(name: string, callback: (event: any) => void) {
    this.listeners.set(name, [...(this.listeners.get(name) || []), callback]);
  }
  removeEventListener(name: string, callback: (event: any) => void) {
    this.listeners.set(name, (this.listeners.get(name) || []).filter((item) => item !== callback));
  }
  click() {
    let stopped = false;
    const event = { target: this, currentTarget: this as Element, preventDefault() {}, stopPropagation() { stopped = true; } };
    for (let node: Element | null = this; node && !stopped; node = node.parentElement) {
      event.currentTarget = node;
      node.listeners.get("click")?.forEach((callback) => callback(event));
    }
  }
  focus() {}
  getClientRects() { return [{}]; }
}

function row(label: string, slug?: string) {
  const node = new Element("button");
  node.setAttribute("role", "menuitemradio");
  const content = new Element("div");
  content.setAttribute("data-menu-row-content", "");
  const outer = new Element("span");
  outer.className = "flex-1";
  const inner = new Element("span");
  inner.className = "truncate";
  const native = new Element("span");
  native.textContent = label;
  inner.appendChild(native); outer.appendChild(inner); content.appendChild(outer); node.appendChild(content);
  if (slug) (node as any).__reactFiber$fixture = { key: slug, memoizedProps: {}, return: null };
  return node;
}

function catalog(names: Record<string, string>) {
  return {
    status: "ok", model_provider: "codex-plus-gateway", model: Object.keys(names)[0],
    default_model: Object.keys(names)[0], effective_config_model: Object.keys(names)[0],
    models: Object.keys(names), sources: [{ id: "relay-profile:codex-plus-gateway" }],
    modelMetadata: Object.fromEntries(Object.entries(names).map(([slug, displayName]) => [slug, {
      displayName, description: `${displayName} · 模型组`, codex_plus_group: "模型组",
      codex_plus_group_id: "__codex_groups__", codex_plus_price: { input: 1, output: 2 },
    }])),
  };
}

function runtime(initial = catalog({ "group/stable": "Before rename" }), includeBuiltins = false, loadEarly = false) {
  const store = new Map<string, string>();
  // 已记住默认供应商、且已有价格的网关目录，原先不会再由菜单兜底请求刷新。
  store.set("codexPlus.modelPicker.collapsedGroups.v1", JSON.stringify({ defaultGroupId: "remembered-provider", providers: {} }));
  const root = new Element();
  const head = new Element("head");
  const paths: string[] = [];
  const timers = new Map<number, () => void>();
  let timerId = 0;
  let current = initial;
  let fingerprint = "before";
  let catalogRead = async () => current;
  const context: any = {
    __CODEX_PLUS_TEST_SERVICE_TIER__: true, process, console,
    setTimeout: includeBuiltins ? (callback: () => void) => { timers.set(++timerId, callback); return timerId; } : setTimeout,
    clearTimeout: includeBuiltins ? (id: number) => timers.delete(id) : clearTimeout,
    setInterval: includeBuiltins ? () => ++timerId : setInterval,
    clearInterval: includeBuiltins ? () => {} : clearInterval,
    requestAnimationFrame: () => 0, cancelAnimationFrame() {},
    queueMicrotask, Promise, URL, JSON,
    addEventListener() {}, removeEventListener() {},
    document: {
      scripts: [], documentElement: root, body: new Element("body"), head, activeElement: null,
      createElement: (tag: string) => new Element(tag), createElementNS: (_namespace: string, tag: string) => new Element(tag),
      getElementById: () => null, querySelector: () => null, querySelectorAll: () => [],
      addEventListener() {}, removeEventListener() {},
    },
    localStorage: { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => store.set(key, String(value)) },
    location: { href: "https://codex.test/", pathname: "/", search: "", hash: "" },
    navigator: { userAgent: "node-test", language: "en" }, performance: { getEntriesByType: () => [] },
    MutationObserver: class { observe() {} disconnect() {} },
    __codexSessionDeleteBridge: async (path: string) => {
      paths.push(path);
      if (path === "/settings/get") return { enhancementsEnabled: true, modelCatalogFingerprint: fingerprint };
      if (path === "/codex-model-catalog") return catalogRead();
      throw new Error(`unexpected bridge request: ${path}`);
    },
  };
  context.window = context; context.globalThis = context;
  const source = includeBuiltins ? installed : prefix;
  const pickerInstall = `  registerCodexPlusExtensionSelector('[data-codex-plus-ext="builtin-model-picker-groups"]');`;
  vm.runInNewContext(`${loadEarly ? source.replace(pickerInstall, `window.earlyCatalogLoad = loadCodexModelCatalog(true);\n${pickerInstall}`) : source}
    ${includeBuiltins ? "" : "function findCodexQueryClient() { return null; }"}
    function syncOfficialUsagePolicy() {}
    function refreshConversationViewControls() {}
    window.renameHarness = {
      heartbeat: syncBackendSettingsFromHeartbeat,
      patch: codexPlusModelPickerPatch,
      refresh: runCodexModelWhitelistRefreshPass,
      rows: codexPlusModelPickerRows,
      syncRecents: codexPlusModelPickerSyncRecents,
      recents: codexPlusModelPickerRecents,
      ${includeBuiltins ? `installPicker() {
        window.__codexPlusBuiltinExtensionState = {
          disabled: codexPlusBuiltinExtensionCatalog().filter((entry) => entry.id !== "model-picker-groups").map((entry) => entry.id),
          replacedBy: {},
        };
        syncCodexPlusBuiltinExtensions();
        return codexPlusBuiltinExtensionHandles.has("model-picker-groups");
      },` : ""}
    };
  })();`, context);
  const api = context.__codexPlusServiceTierTest;
  api.setBackendSettings({
    relayProfilesEnabled: true, codexAppModelWhitelistUnlock: false, activeRelayId: "codex-plus-gateway",
    modelCatalogFingerprint: fingerprint, relayProfiles: [{ id: "codex-plus-gateway", relayMode: "pureApi" }],
  });
  if (!loadEarly) api.setModelCatalog(initial);
  return {
    window: context, api, store, paths, harness: context.renameHarness,
    change(next: typeof initial) { current = next; fingerprint = "after"; },
    setCatalogRead(read: typeof catalogRead) { catalogRead = read; },
    runPickerTimer() {
      const task = [...timers].find(([, callback]) => callback.toString().includes("codexPlusModelPickerScan()"));
      assert.ok(task, "builtin activation must schedule the installed picker");
      timers.delete(task[0]); task[1]();
    },
  };
}

test("installed picker and native model/list use the renamed group after a settings heartbeat", async () => {
  const app = runtime();
  assert.ok(app.window.__codexPlusModelPickerGroupsRuntime, "71 must install from the generated artifact");
  const list = new Element();
  const native = row("Before rename");
  list.appendChild(native);
  let selections = 0;
  native.addEventListener("click", () => { selections += 1; });
  app.harness.patch(list);
  native.click();
  assert.equal(app.harness.recents()[0].slug, "group/stable");

  app.change(catalog({ "group/stable": "After rename" }));
  await app.harness.heartbeat();
  assert.deepEqual(app.paths, ["/settings/get", "/codex-model-catalog"]);
  app.harness.patch(list);
  assert.equal(native.querySelector(".codex-plus-model-group-own-label")?.textContent, "After rename");
  assert.equal(app.harness.recents()[0].label, "After rename");
  const recent = list.querySelector(".codex-plus-model-recent-row");
  assert.equal(recent?.querySelector(".codex-plus-model-recent-name")?.textContent, "After rename");
  recent?.click();
  assert.equal(selections, 2, "recent click still chooses the original stable group row");

  class Client {
    hostId = "local";
    async sendRequest(_method: string, _params: unknown) {
      return { data: [{ model: "group/stable", id: "group/stable", displayName: "Before rename", description: "Before rename · 模型组" }], nextCursor: null };
    }
  }
  app.api.setBackendSettings({ codexAppModelWhitelistUnlock: true });
  app.window.__codexPlusAppServerClientClass = Client;
  assert.equal(app.api.installAppServerClientPrototypePatch(), true);
  const response = await new Client().sendRequest("model/list", { cursor: null, limit: 100 });
  assert.equal(response.data[0].displayName, "After rename");
  assert.equal(response.data[0].description, "After rename · 模型组");
  assert.equal(response.data[0].model, "group/stable");
  const reopened = new Element();
  const refreshedRow = row(response.data[0].displayName);
  reopened.appendChild(refreshedRow);
  app.harness.patch(reopened);
  assert.equal(refreshedRow.querySelector(".codex-plus-model-group-own-label")?.textContent, "After rename");
});

for (const loadEarly of [false, true]) {
  test(`generated catalog ${loadEarly ? "first load before 95" : "heartbeat"} refetches active native composer models`, async () => {
    const next = catalog({ "group/stable": "After rename" });
    const app = runtime(loadEarly ? next : catalog({ "group/stable": "Before rename" }), true, loadEarly);
    const composer = new Element("button");
    composer.textContent = "Before rename";
    let requests = 0;
    class Client {
      hostId = "local";
      async sendRequest(_method: string, _params: unknown) {
        requests += 1;
        return { data: [{ model: "group/stable", displayName: "Before rename" }], nextCursor: null };
      }
    }
    app.api.setBackendSettings({ codexAppModelWhitelistUnlock: true });
    app.window.__codexPlusAppServerClientClass = Client;
    assert.equal(app.api.installAppServerClientPrototypePatch(), true);
    const native = new Client();
    const queries = [
      { queryKey: ["models", "list", "local", "apikey", 100, "provider", "catalog.json"], active: true },
      { queryKey: ["models", "list", "remote", "apikey", 100], active: true },
      { queryKey: ["models", "list", "local", "team", { accountId: "team" }], active: true },
      { queryKey: ["models", "list", "local", "no-auth", 100], active: false },
    ].map((query) => ({ ...query, invalidated: false, state: { data: { data: [{ model: "group/stable", displayName: "Before rename" }] } } }));
    let invalidations = 0;
    let refetched = Promise.resolve();
    const queryClient = {
      getQueryCache: () => ({ findAll: () => queries }),
      setQueryData() {},
      invalidateQueries(filters: { queryKey: string[]; predicate(query: typeof queries[number]): boolean }) {
        invalidations += 1;
        assert.deepEqual(Array.from(filters.queryKey), ["models", "list", "local"]);
        refetched = Promise.all(queries.map(async (query) => {
          if (!filters.queryKey.every((part, index) => query.queryKey[index] === part) || !filters.predicate(query)) return;
          query.invalidated = true;
          if (!query.active) return;
          query.state.data = await native.sendRequest("model/list", { includeHidden: true, cursor: null, limit: 100 });
          composer.textContent = query.state.data.data.find((model) => model.model === "group/stable")!.displayName;
        })).then(() => {});
        return refetched;
      },
    };
    // 通过最终产物的 95 React 根查找器发现客户端，而非替换发现函数。
    app.window.document.documentElement.__reactContainer$native = { memoizedProps: { client: queryClient } };
    try {
      if (loadEarly) await app.window.earlyCatalogLoad;
      else {
        app.change(next);
        await app.harness.heartbeat();
      }
      await refetched;
      assert.equal(invalidations, 1);
      assert.equal(requests, 1, "only the active local non-team query refetches through the installed native wrapper");
      assert.equal(composer.textContent, "After rename");
      assert.equal(queries[0].state.data.data[0].model, "group/stable");
      assert.deepEqual(queries.map((query) => query.invalidated), [true, false, false, true]);
      await app.api.loadModelCatalog(true);
      assert.equal(invalidations, 1, "the unchanged catalog does not repeatedly invalidate native queries");
    } finally {
      app.window.__codexPlusModelPickerGroupsRuntime.dispose();
    }
  });
}

test("legacy recents migrate by the previous catalog identity when another group takes the old name", async () => {
  const app = runtime(catalog({ "group/original": "Old name", "group/other": "Other name" }));
  app.store.set(recentsKey, JSON.stringify({ items: [{ k: "oldname", label: "Old name", name: "Old name", provider: "模型组", groupId: "__codex_groups__" }] }));
  app.change(catalog({ "group/original": "New name", "group/other": "Old name" }));
  await app.harness.heartbeat();
  assert.equal(app.harness.recents()[0].slug, "group/original");
  assert.equal(app.harness.recents()[0].label, "New name");
});

test("native hover descriptions recover from late query discovery and stale responses for every renamed group", async () => {
  const names = { "group/first": "6.1 Sol", "group/second": "6.1 Terra", "group/third": "Another" };
  const app = runtime(catalog(names), true);
  app.api.setBackendSettings({ codexAppModelWhitelistUnlock: true });
  const stale = Object.entries(names).map(([model, displayName]) => Object.freeze({
    model, displayName, description: `${model} · 模型组`, hidden: false,
  }));
  const ordinary = Object.freeze({ model: "provider/model", displayName: "Ordinary", description: "Ordinary description" });
  const response = () => Object.freeze({ data: Object.freeze([...stale, ordinary]), nextCursor: "next" });
  const queries = [
    ["models", "list", "local", "apikey", 100],
    ["models", "list", "local", "no-auth", 100],
    ["models", "list", "remote", "apikey", 100],
    ["models", "list", "local", "team", { accountId: "team" }],
    ["other", "list", "local", "apikey", 100],
  ].map((queryKey) => ({ queryKey, state: { data: response(), dataUpdatedAt: 123 } }));
  let writes = 0;
  const queryClient = {
    invalidateQueries: async () => {},
    getQueryCache: () => ({
      findAll: (filters: { queryKey: unknown[]; predicate(query: typeof queries[number]): boolean }) =>
        queries.filter((query) => filters.queryKey.every((part, index) => query.queryKey[index] === part) && filters.predicate(query)),
    }),
    setQueryData(queryKey: unknown[], data: typeof queries[number]["state"]["data"], options: { updatedAt: number }) {
      const query = queries.find((entry) => entry.queryKey === queryKey)!;
      assert.equal(options.updatedAt, query.state.dataUpdatedAt, "display sync preserves network freshness");
      query.state.data = data;
      writes += 1;
    },
  };
  try {
    // 第一次目录发布时 React 查询客户端尚未挂载，显示名也早已正确。
    await app.api.loadModelCatalog(true);
    app.window.document.documentElement.__reactContainer$native = { memoizedProps: { client: queryClient } };
    app.harness.refresh();
    assert.equal(writes, 2, "both existing local non-team caches refresh without a request wrapper");
    for (const query of queries.slice(0, 2)) {
      for (const [index, [model, displayName]] of Object.entries(names).entries()) {
        assert.equal(query.state.data.data[index].model, model);
        assert.equal(query.state.data.data[index].displayName, displayName);
        assert.equal(query.state.data.data[index].description, `${displayName} · 模型组`);
        assert.equal(stale[index].description, `${model} · 模型组`, "old descriptors stay immutable");
      }
      assert.equal(query.state.data.data[3], ordinary);
      assert.equal(query.state.data.nextCursor, "next");
    }
    for (const query of queries.slice(2)) assert.equal(query.state.data.data[0], stale[0]);
    app.harness.refresh();
    assert.equal(writes, 2, "unchanged metadata does not republish");

    // 未包装的旧原生响应晚到；下一轮真实刷新入口必须再次修正描述。
    queries[0].state.data = response();
    app.harness.refresh();
    assert.equal(writes, 3);
    const renamed = { "group/first": "Final Sol", "group/second": "6.1 Sol", "group/third": "6.1 Sol" };
    app.change(catalog(renamed));
    await app.harness.heartbeat();
    for (const query of queries.slice(0, 2)) {
      for (const [index, [model, displayName]] of Object.entries(renamed).entries()) {
        assert.equal(query.state.data.data[index].model, model);
        assert.equal(query.state.data.data[index].description, `${displayName} · 模型组`);
      }
    }
    assert.equal(writes, 5, "all group identities update even when old names are reused or shared");
  } finally {
    app.window.__codexPlusModelPickerGroupsRuntime.dispose();
  }
});

test("restart cannot associate an old recent without a stable identity with a reused name", async () => {
  const app = runtime({ ...catalog({}), modelMetadata: {} });
  app.store.set(recentsKey, JSON.stringify({ items: [{ k: "oldname", label: "Old name", name: "Old name", provider: "模型组", groupId: "__codex_groups__" }] }));
  app.change(catalog({ "group/original": "New name", "group/other": "Old name" }));
  await app.api.loadModelCatalog(true);
  assert.equal(app.harness.recents()[0].label, "Old name", "preserve unresolved stored history");
  const list = new Element();
  list.append(row("New name"), row("Old name"));
  app.harness.syncRecents(list, app.harness.rows(list), []);
  assert.equal(list.querySelector(".codex-plus-model-recent-row"), null);
});

test("first catalog load preserves ordinary legacy recent models", async () => {
  const app = runtime({ ...catalog({}), modelMetadata: {} });
  app.store.set(recentsKey, JSON.stringify({ items: [{ k: "existingmodel", label: "Existing model", name: "Existing model" }] }));
  app.change(catalog({ "provider/model": "Existing model" }));
  await app.api.loadModelCatalog(true);
  const list = new Element();
  list.appendChild(row("Existing model"));
  app.harness.syncRecents(list, app.harness.rows(list), []);
  assert.equal(list.querySelector(".codex-plus-model-recent-name")?.textContent, "Existing model");
});

test("three same-name rows keep legacy recent matching ambiguous", () => {
  const app = runtime(catalog({ "group/a": "Same", "group/b": "Same", "group/c": "Same" }));
  app.store.set(recentsKey, JSON.stringify({ items: [{ k: "same", label: "Same", name: "Same" }] }));
  const list = new Element();
  for (const slug of ["group/a", "group/b", "group/c"]) list.appendChild(row("Same", slug));
  app.harness.syncRecents(list, app.harness.rows(list), []);
  assert.equal(list.querySelector(".codex-plus-model-recent-row"), null);
  assert.equal(app.harness.recents()[0].label, "Same");
});

test("persisted stable group recents receive the current name on the first catalog load", async () => {
  const app = runtime({ ...catalog({}), modelMetadata: {} });
  app.store.set(recentsKey, JSON.stringify({ items: [{ slug: "group/stable", k: "beforerename", label: "Before rename", name: "Before rename" }] }));
  app.change(catalog({ "group/stable": "After rename" }));
  await app.api.loadModelCatalog(true);
  assert.equal(app.harness.recents()[0].slug, "group/stable");
  assert.equal(app.harness.recents()[0].label, "After rename");
});

test("generated builtin registration activates the installed picker and reaches its real scan", async () => {
  const app = runtime(catalog({ "group/stable": "Before rename" }), true);
  const menu = new Element();
  menu.setAttribute("data-model-picker-view", "simple");
  const list = new Element();
  list.appendChild(row("Before rename"));
  menu.appendChild(list);
  app.window.document.querySelectorAll = (selector: string) => selector === "[data-model-picker-view]" ? [menu] : [];
  try {
    assert.equal(app.harness.installPicker(), true);
    app.runPickerTimer();
    assert.equal(app.window.__codexPlusModelPickerGroupsRuntime.state().hasActiveList, true);
    assert.equal(list.getAttribute("data-codex-plus-model-groups"), "true");
    app.change(catalog({ "group/stable": "After rename" }));
    await app.harness.heartbeat();
    app.window.__codexPlusModelPickerGroupsRuntime.scan();
    assert.equal(list.querySelector(".codex-plus-model-group-own-label")?.textContent, "After rename");
  } finally {
    app.window.__codexPlusBuiltinExtensionHandles.forEach((handle: { dispose(): void }) => handle.dispose());
    app.window.__codexPlusModelPickerGroupsRuntime.dispose();
  }
});

for (const staleFails of [false, true]) {
  test(`a ${staleFails ? "failed" : "successful"} older catalog request cannot overwrite the forced rename refresh`, async () => {
    const app = runtime();
    let resolveOld!: (value: ReturnType<typeof catalog>) => void;
    let rejectOld!: (reason: Error) => void;
    const old = new Promise<ReturnType<typeof catalog>>((resolve, reject) => { resolveOld = resolve; rejectOld = reject; });
    let calls = 0;
    app.setCatalogRead(() => ++calls === 1 ? old : Promise.resolve(catalog({ "group/stable": "After rename" })));
    const older = app.api.loadModelCatalog(true);
    await Promise.resolve();
    await app.api.loadModelCatalog(true);
    if (staleFails) rejectOld(new Error("old request failed"));
    else resolveOld(catalog({ "group/stable": "Before rename" }));
    await older;
    assert.equal(app.api.modelDescriptor("group/stable").displayName, "After rename");
  });
}
