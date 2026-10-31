import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

type ModelMetadata = Record<string, {
  displayName?: string;
  display_name?: string;
  codex_plus_group: string;
  codex_plus_group_id: string;
  codex_plus_favorite?: boolean;
}>;

type Catalog = {
  status: string;
  models: string[];
  modelMetadata: ModelMetadata;
  codex_plus_default_group_id?: string;
  model?: string;
  default_model?: string;
  effective_config_model?: string;
  sources?: Array<{ id: string; type: string }>;
};

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/71-model-picker-groups.js", import.meta.url), "utf8");
const startup = source.indexOf("\n  codexPlusModelPickerInstallStyle();");
assert.ok(startup > 0);

function fixture(catalog: Catalog) {
  const context: any = {
    window: {}, codexModelCatalog: catalog,
    registerCodexPlusExtensionSelector() {}, queueMicrotask() {},
  };
  vm.runInNewContext(`${source.slice(0, startup)}
    globalThis.testModelPicker = {
      rows: codexPlusModelPickerRows,
      groups: (list) => codexPlusModelPickerUniqueGroupIds(
        codexPlusModelPickerGroups(codexPlusModelPickerRows(list), codexPlusModelPickerDefaultProviderName())),
      refresh: (catalog) => { codexModelCatalog = catalog; codexPlusModelPickerCatalogChanged(); },
    };`, context);
  return context.testModelPicker;
}

function row(text: string, options: { model?: string; selectedParent?: string; subtitle?: string } = {}) {
  const label = { textContent: text, classList: { contains: () => false }, getAttribute: () => null };
  const children = [label];
  if (options.subtitle) children.push({ ...label, textContent: options.subtitle });
  const inner = { children, firstElementChild: label, classList: { contains: (name: string) => name === "flex-col" && !!options.subtitle } };
  const outer = options.subtitle ? inner : { firstElementChild: inner, classList: { contains: () => false } };
  const content = { querySelector: (selector: string) => selector === ".flex-1" ? outer : null };
  const element: any = {
    querySelector: (selector: string) => selector === "[data-menu-row-content]" ? content : null,
    classList: { contains: () => false }, getAttribute: () => null,
  };
  if (options.model || options.selectedParent) {
    element.__reactFiber$fixture = { key: options.model, memoizedProps: {}, return: {
      memoizedProps: { model: options.selectedParent || "a/sol", models: [], onSelectModel() {} },
    } };
  }
  return element;
}

function list(rows: unknown[]) {
  return { querySelectorAll: (selector: string) => {
    assert.equal(selector, '[role="menuitemradio"]');
    return rows;
  } };
}

function entries(api: any, rows: unknown[]) {
  return Array.from(api.rows(list(rows)), (entry: any) => ({
    name: entry.name, groupId: entry.groupId, favorite: entry.favorite,
  }));
}

class FixtureElement {
  tagName: string;
  parentElement: FixtureElement | null = null;
  children: FixtureElement[] = [];
  attributes = new Map<string, string>();
  classes = new Set<string>();
  classAdds = 0;
  listeners = new Map<string, Array<(event: any) => void>>();
  text = "";
  titleValue = "";
  titleWrites = 0;
  id = "";
  type = "";
  isConnected = true;
  style = {
    values: new Map<string, string>(),
    setProperty: (name: string, value: string) => this.style.values.set(name, value),
    removeProperty: (name: string) => this.style.values.delete(name),
  };
  classList = {
    add: (...names: string[]) => { this.classAdds++; names.forEach((name) => this.classes.add(name)); },
    remove: (...names: string[]) => names.forEach((name) => this.classes.delete(name)),
    contains: (name: string) => this.classes.has(name),
    toggle: (name: string, force?: boolean) => {
      const enabled = force ?? !this.classes.has(name);
      if (enabled) this.classes.add(name);
      else this.classes.delete(name);
      return enabled;
    },
  };

  constructor(tagName = "div") { this.tagName = tagName.toLowerCase(); }
  get title() { return this.titleValue; }
  set title(value: string) { this.titleWrites++; this.titleValue = value; }
  get className() { return [...this.classes].join(" "); }
  set className(value: string) { this.classes = new Set(value.split(/\s+/).filter(Boolean)); }
  get textContent(): string { return this.text + this.children.map((child) => child.textContent).join(""); }
  set textContent(value: string) { this.text = value; this.children.forEach((child) => { child.parentElement = null; }); this.children = []; }
  get firstElementChild() { return this.children[0] || null; }
  get firstChild() { return this.firstElementChild; }
  get nextElementSibling() {
    const siblings = this.parentElement?.children || [];
    return siblings[siblings.indexOf(this) + 1] || null;
  }
  get nextSibling() { return this.nextElementSibling; }
  setAttribute(name: string, value: string) {
    if (name === "class") this.className = value;
    else this.attributes.set(name, String(value));
  }
  getAttribute(name: string) { return name === "class" ? this.className : this.attributes.get(name) ?? null; }
  hasAttribute(name: string) { return this.attributes.has(name); }
  removeAttribute(name: string) { this.attributes.delete(name); }
  appendChild(child: FixtureElement) { return this.insertBefore(child, null); }
  append(...children: FixtureElement[]) { children.forEach((child) => this.appendChild(child)); }
  insertBefore(child: FixtureElement, before: FixtureElement | null) {
    child.remove();
    const index = before ? this.children.indexOf(before) : this.children.length;
    assert.ok(index >= 0);
    this.children.splice(index, 0, child);
    child.parentElement = this;
    child.isConnected = this.isConnected;
    return child;
  }
  remove() {
    if (!this.parentElement) return;
    const index = this.parentElement.children.indexOf(this);
    if (index >= 0) this.parentElement.children.splice(index, 1);
    this.parentElement = null;
    this.isConnected = false;
  }
  matches(selector: string): boolean {
    if (selector.includes(",")) return selector.split(",").some((part) => this.matches(part.trim()));
    if (selector.startsWith(".")) return this.classes.has(selector.slice(1));
    if (selector === '[class*="_ModelList_"]') return this.className.includes("_ModelList_");
    const attribute = selector.match(/^\[([^=\]]+)(?:="([^"]*)")?\]$/);
    if (attribute) return this.hasAttribute(attribute[1]) && (attribute[2] === undefined || this.getAttribute(attribute[1]) === attribute[2]);
    return this.tagName === selector.toLowerCase();
  }
  closest(selector: string): FixtureElement | null {
    for (let node: FixtureElement | null = this; node; node = node.parentElement) {
      if (node.matches(selector)) return node;
    }
    return null;
  }
  querySelectorAll(selector: string): FixtureElement[] {
    if (selector.startsWith(":scope > ")) return this.children.filter((child) => child.matches(selector.slice(9)));
    return this.children.flatMap((child) => [
      ...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector),
    ]);
  }
  querySelector(selector: string) { return this.querySelectorAll(selector)[0] || null; }
  addEventListener(type: string, handler: (event: any) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) || []), handler]);
  }
  getClientRects() { return [{}]; }
  getBoundingClientRect() { return { top: 0 }; }
  focus() {}
}

class FixtureDocument {
  documentElement = { lang: "en" };
  head = new FixtureElement("head");
  body = new FixtureElement("body");
  listeners = new Map<string, Array<(event: any) => void>>();
  createElement(tagName: string) { const node = new FixtureElement(tagName); node.isConnected = false; return node; }
  createElementNS(_namespace: string, tagName: string) { return this.createElement(tagName); }
  getElementById(id: string) { return [this.head, this.body].flatMap((root) => [root, ...root.querySelectorAll("*")]).find((node) => node.id === id) || null; }
  querySelectorAll(selector: string) { return this.body.querySelectorAll(selector); }
  addEventListener(type: string, handler: (event: any) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) || []), handler]);
  }
  removeEventListener(type: string, handler: (event: any) => void) {
    this.listeners.set(type, (this.listeners.get(type) || []).filter((item) => item !== handler));
  }
  dispatch(type: string, target: FixtureElement, key = "") {
    const event = {
      type, target, key, defaultPrevented: false, stopped: false,
      preventDefault() { this.defaultPrevented = true; },
      stopImmediatePropagation() { this.stopped = true; },
    };
    for (const handler of this.listeners.get(type) || []) {
      handler(event);
      if (event.stopped) break;
    }
    if (!event.stopped) for (const handler of target.listeners.get(type) || []) handler(event);
    return event;
  }
}

function domRow(label: string, subtitle = "") {
  const row = new FixtureElement();
  row.setAttribute("role", "menuitemradio");
  const content = new FixtureElement();
  content.setAttribute("data-menu-row-content", "");
  const inner = new FixtureElement("span");
  inner.className = "flex-1 flex-col";
  const title = new FixtureElement("span");
  title.textContent = label;
  inner.appendChild(title);
  const description = new FixtureElement("span");
  if (subtitle) { description.textContent = subtitle; inner.appendChild(description); }
  const check = new FixtureElement("svg");
  content.append(inner, check);
  row.appendChild(content);
  return { row, title, description, check, inner };
}

function domFixture(catalog: Catalog, defaultModel = "a/sol") {
  const document = new FixtureDocument();
  const nativeCalls: string[] = [];
  const pinnedCalls: string[] = [];
  const legacyCalls: string[] = [];
  const oldClick = () => legacyCalls.push("click");
  const oldKey = () => legacyCalls.push("keydown");
  document.addEventListener("click", oldClick);
  document.addEventListener("keydown", oldKey);
  const defaults = domRow("Default", "Recommended set of models");
  (defaults.title as any).__reactProps$fixture = { children: "Default" };
  (defaults.description as any).__reactProps$fixture = { children: "Recommended set of models" };
  defaults.row.setAttribute("aria-checked", "true");
  (defaults.row as any).__reactFiber$fixture = {
    memoizedProps: { children: { props: { id: "composer.modelPicker.default.label" } } },
    return: { memoizedProps: { model: "a/sol", models: [{ model: "a/sol" }],
      onSelectModel: (model: string) => pinnedCalls.push(model) } },
  };
  defaults.row.addEventListener("click", () => nativeCalls.push("default"));
  defaults.row.addEventListener("keydown", (event) => {
    if (["Enter", " "].includes(event.key)) nativeCalls.push("default");
  });
  const menu = new FixtureElement();
  menu.setAttribute("data-model-picker-view", "simple");
  const modelList = new FixtureElement();
  modelList.className = "_ModelList_test";
  modelList.appendChild(defaults.row);
  menu.appendChild(modelList);
  document.body.appendChild(menu);
  const context: any = {
    document, navigator: { language: "en-US" },
    window: { __codexPlusModelPickerDefaultClickHandler: oldClick, __codexPlusModelPickerDefaultKeyHandler: oldKey },
    codexModelCatalog: catalog, codexPlusBackendSettingsLoaded: true,
    codexModelCatalogPromise: null, codexModelCatalogLoadedAt: Date.now(),
    codexModelCatalogRuntime: { failure: null },
    codexPlusBackendSettings: { relayProfilesEnabled: true, activeRelayId: "fixed",
      relayProfiles: [{ id: "fixed", relayMode: "aggregate" }] },
    codexRemoteSessionActiveProfile: () => context.codexPlusBackendSettings.relayProfilesEnabled
      ? context.codexPlusBackendSettings.relayProfiles.find((entry: any) =>
        entry.id === context.codexPlusBackendSettings.activeRelayId) : null,
    codexPlusConfiguredDefaultModel: () => defaultModel,
    codexPlusModelMetadata: (model: string) => context.codexModelCatalog.modelMetadata?.[model],
    registerCodexPlusExtensionSelector() {}, queueMicrotask() {}, setTimeout: () => 1, clearTimeout() {},
    MutationObserver: class { observe() {} disconnect() {} },
  };
  vm.runInNewContext(`${source}\n globalThis.testModelPicker = {
    patch: codexPlusModelPickerPatch,
    defaultModel: codexPlusModelPickerDefaultModel,
    refresh: (catalog) => { codexModelCatalog = catalog; codexPlusModelPickerCatalogChanged(); },
    installLegacy: codexPlusModelPickerInstallLabel,
  };`, context);
  return {
    document, defaults, modelList, nativeCalls, pinnedCalls, legacyCalls, api: context.testModelPicker,
    setDefault: (model: string) => { defaultModel = model; context.codexModelCatalog.effective_config_model = model; },
    setCandidateDefault: (model: string) => { defaultModel = model; },
    setSettings: (settings: any) => { context.codexPlusBackendSettings = settings; },
    setLoaded: (loaded: boolean) => { context.codexPlusBackendSettingsLoaded = loaded; },
    setFresh: (fresh: boolean) => { context.codexModelCatalogLoadedAt = fresh ? Date.now() : 0; },
    setPending: (pending: boolean) => { context.codexModelCatalogPromise = pending ? {} : null; },
    setFailed: (failed: boolean) => { context.codexModelCatalogRuntime.failure = failed ? {} : null; },
    addModel: (model: string, label: string) => {
      const item = domRow(label);
      (item.row as any).__reactFiber$fixture = { key: model, memoizedProps: {} };
      modelList.appendChild(item.row);
      return item;
    },
  };
}

test("abbreviated same-name models retain zero-width identities, groups and favorites", () => {
  const metadata: ModelMetadata = {
    "a/sol": { displayName: "GPT-6.1-Sol\u200b", codex_plus_group: "Provider A", codex_plus_group_id: "a" },
    "b/sol": { displayName: "GPT-6.1-Sol\u200b\u200b", codex_plus_group: "Provider B", codex_plus_group_id: "b" },
    sol: { displayName: "GPT-6.1-Sol\u200c", codex_plus_group: "Auto", codex_plus_group_id: "auto" },
    "group/coding": { displayName: "Coding Models", codex_plus_group: "Model Groups", codex_plus_group_id: "groups", codex_plus_favorite: true },
    "a/astra": { displayName: "GPT-Astra", codex_plus_group: "Provider A", codex_plus_group_id: "a" },
  };
  const api = fixture({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata, codex_plus_default_group_id: "a" });
  const rows = [row("Default", { subtitle: "Recommended" }), row("6.1 Sol\u200b"), row("6.1 Sol\u200b\u200b"), row("6.1 Sol\u200c"), row("Coding Models"), row("GPT-Astra")];
  assert.deepEqual(entries(api, rows).map((entry) => [entry.groupId, entry.favorite]), [
    ["a", false], ["b", false], ["auto", false], ["groups", true], ["a", false],
  ]);
  assert.deepEqual(Array.from(api.groups(list(rows)), (group: any) => [group.id, group.entries.length, group.isDefault]), [
    ["a", 2, true], ["b", 1, false], ["auto", 1, false], ["groups", 1, false],
  ]);
});

test("row model identity disambiguates identical names without inheriting the selected parent model", () => {
  const metadata: ModelMetadata = {
    "a/sol": { displayName: "GPT-Sol", codex_plus_group: "Provider A", codex_plus_group_id: "a" },
    "b/sol": { displayName: "GPT-Sol", codex_plus_group: "Provider B", codex_plus_group_id: "b" },
    sol: { displayName: "GPT-Sol", codex_plus_group: "Auto", codex_plus_group_id: "auto" },
  };
  const api = fixture({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata });
  const rows = [row("GPT-Sol", { model: "a/sol" }), row("GPT-Sol", { model: "b/sol" }),
    row("GPT-Sol", { model: "sol" }), row("GPT-Sol", { selectedParent: "a/sol" }), row("GPT-Sol")];
  assert.deepEqual(entries(api, rows).map((entry) => entry.groupId), ["a", "b", "auto"]);
});

test("parenthesized model variants are preserved unless the catalog identifies a provider", () => {
  const metadata: ModelMetadata = {
    "a/variant": { displayName: "GPT-6 (Sol)", codex_plus_group: "Provider A", codex_plus_group_id: "a" },
    "b/sol": { displayName: "GPT-Sol（Provider B）", codex_plus_group: "Provider B", codex_plus_group_id: "b" },
  };
  const api = fixture({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata });
  assert.deepEqual(entries(api, [row("GPT-6 (Sol)"), row("GPT-Sol（Provider B）")]), [
    { name: "GPT-6 (Sol)", groupId: "a", favorite: false },
    { name: "GPT-Sol", groupId: "b", favorite: false },
  ]);
});

test("catalog refresh excludes removed models even when stale metadata remains", () => {
  const metadata: ModelMetadata = {
    "a/sol": { displayName: "GPT-Sol", codex_plus_group: "Provider A", codex_plus_group_id: "a" },
    "b/astra": { displayName: "GPT-Astra（Provider B）", codex_plus_group: "Provider B", codex_plus_group_id: "b" },
  };
  const api = fixture({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata });
  const rows = [row("GPT-Sol"), row("GPT-Astra（Provider B）")];
  assert.deepEqual(entries(api, rows).map((entry) => entry.groupId), ["a", "b"]);
  api.refresh({ status: "ok", models: ["a/sol"], modelMetadata: metadata });
  assert.deepEqual(entries(api, rows).map((entry) => entry.groupId), ["a"]);
});

const providerMetadata: ModelMetadata = {
  "a/sol": { displayName: "Model A", codex_plus_group: "Provider A", codex_plus_group_id: "a" },
  "b/sol": { displayName: "Model B", codex_plus_group: "Provider B", codex_plus_group_id: "b" },
  auto: { displayName: "Auto Model", codex_plus_group: "Auto", codex_plus_group_id: "codex-plus-gateway-auto" },
  grouped: { displayName: "Model Group", codex_plus_group: "Groups", codex_plus_group_id: "codex-plus-gateway-groups" },
};

function catalogFor(models: string[], status = "ok"): Catalog {
  return { status, models, modelMetadata: providerMetadata, codex_plus_default_group_id: "a",
    model: "a/sol", default_model: "a/sol", effective_config_model: "a/sol",
    sources: [{ id: "aggregate:fixed", type: "aggregate_model_catalog" }] };
}

function assertNoDefaultValue(fixture: ReturnType<typeof domFixture>) {
  const { row, title, description, check } = fixture.defaults;
  assert.equal(row.querySelector(".codex-plus-model-default-value"), null);
  assert.equal(title.textContent, "Default");
  assert.equal(description.classList.contains("codex-plus-model-default-recommendation"), true);
  assert.equal(row.getAttribute("aria-checked"), "true");
  assert.equal(check.parentElement !== null, true);
}

function assertNativeDefault(fixture: ReturnType<typeof domFixture>, descriptionAria: string | null = null) {
  const { row, title, description, check, inner } = fixture.defaults;
  assert.equal(row.classList.contains("codex-plus-model-default-row"), false);
  assert.equal(row.querySelector(".codex-plus-model-default-value"), null);
  assert.equal(row.querySelector(".codex-plus-model-group-own-label"), null);
  assert.equal(inner.classList.contains("codex-plus-model-default-layout"), false);
  assert.equal(title.classList.contains("codex-plus-model-default-title"), false);
  assert.equal(description.classList.contains("codex-plus-model-default-recommendation"), false);
  assert.equal(title.getAttribute("aria-hidden"), null);
  assert.equal(description.getAttribute("aria-hidden"), descriptionAria);
  assert.equal(description.classList.contains("codex-plus-model-default-subtitle"), false);
  assert.equal(row.getAttribute("aria-checked"), "true");
  assert.equal(check.parentElement !== null, true);
}

test("zero or one provider leaves the native default row untouched", () => {
  for (const models of [[], ["a/sol"]]) {
    const fixture = domFixture(catalogFor(models));
    if (models.length) fixture.addModel("a/sol", "Model A");
    fixture.api.patch(fixture.modelList);
    assertNativeDefault(fixture);
    assert.equal(fixture.modelList.hasAttribute("data-codex-plus-model-groups"), false);
  }
  const official = domFixture(catalogFor([], "not_configured"), "");
  official.api.patch(official.modelList);
  assertNativeDefault(official);
});

test("auto and custom model groups are not additional providers", () => {
  const fixture = domFixture(catalogFor(["a/sol", "auto", "grouped"]));
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("auto", "Auto Model");
  fixture.addModel("grouped", "Model Group");
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.modelList.getAttribute("data-codex-plus-model-groups"), "true");
  assertNativeDefault(fixture);
});

test("two providers put the full model beside the localized native title and hide known recommendations", () => {
  const fixture = domFixture(catalogFor(["a/sol", "b/sol", "auto"]));
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("b/sol", "Model B");
  fixture.api.patch(fixture.modelList);
  const { row, title, description, check, inner } = fixture.defaults;
  const value = row.querySelector(".codex-plus-model-default-value");
  assert.equal(value?.textContent, "Model A");
  assert.equal(value?.title, "Model A");
  assert.equal(value?.parentElement, inner);
  assert.equal(inner.classList.contains("codex-plus-model-default-layout"), true);
  assert.equal(title.classList.contains("codex-plus-model-default-title"), true);
  assert.equal(description.classList.contains("codex-plus-model-default-recommendation"), true);
  assert.equal(title.textContent, "Default");
  assert.equal(description.textContent, "Recommended set of models");
  assert.equal(title.getAttribute("aria-hidden"), null);
  assert.equal(description.getAttribute("aria-hidden"), null);
  assert.equal(row.getAttribute("aria-checked"), "true");
  assert.equal(check.parentElement !== null, true);
  assert.doesNotMatch(row.textContent, /默认/);

  fixture.document.documentElement.lang = "zh-CN";
  title.textContent = "默认";
  description.textContent = "推荐模型集";
  fixture.api.patch(fixture.modelList);
  assert.equal(title.textContent, "默认");
  assert.equal(description.textContent, "推荐模型集");
  assert.equal(description.classList.contains("codex-plus-model-default-recommendation"), true);
  assert.equal(row.querySelectorAll(".codex-plus-model-default-value").length, 1);
});

test("only exact confirmed recommendation descriptions are hidden and native attributes survive switching", () => {
  for (const originalAria of [null, "false", "true"]) {
    const fixture = domFixture(catalogFor(["a/sol", "b/sol"]));
    fixture.addModel("a/sol", "Model A");
    const second = fixture.addModel("b/sol", "Model B");
    const { description, inner } = fixture.defaults;
    const originalStyle = "display: inline; color: red;";
    description.setAttribute("style", originalStyle);
    inner.setAttribute("style", "flex-grow: 1;");
    if (originalAria !== null) description.setAttribute("aria-hidden", originalAria);
    for (const [text, hidden] of [
      ["Recommended set of models", true], ["  推荐模型集  ", true], ["推荐默认集", true],
      ["推薦模型組合", true], ["推薦的模型組合", true], ["Recommended", false],
      ["Recommended set of models for this workspace", false], ["推荐默认集（其他说明）", false],
    ] as const) {
      description.textContent = text;
      fixture.api.patch(fixture.modelList);
      assert.equal(description.classList.contains("codex-plus-model-default-recommendation"), hidden, text);
      assert.equal(description.getAttribute("aria-hidden"), originalAria);
      assert.equal(description.getAttribute("style"), originalStyle);
      assert.equal(inner.getAttribute("style"), "flex-grow: 1;");
    }
    description.textContent = "推荐默认集";
    fixture.api.patch(fixture.modelList);
    second.row.remove();
    fixture.api.refresh(catalogFor(["a/sol"]));
    fixture.api.patch(fixture.modelList);
    assertNativeDefault(fixture, originalAria);
    assert.equal(description.getAttribute("style"), originalStyle);
    assert.equal(inner.getAttribute("style"), "flex-grow: 1;");
    fixture.api.refresh(catalogFor(["a/sol", "b/sol"], "not_configured"));
    fixture.api.patch(fixture.modelList);
    assertNativeDefault(fixture, originalAria);
    assert.equal(description.getAttribute("style"), originalStyle);
  }
});

test("repeated scans do not rewrite default-row classes or the model title", () => {
  const fixture = domFixture(catalogFor(["a/sol", "b/sol"]));
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("b/sol", "Model B");
  fixture.api.patch(fixture.modelList);
  const { row, title, description, inner } = fixture.defaults;
  const value = row.querySelector(".codex-plus-model-default-value")!;
  const nodes = [row, title, description, inner];
  const initialAdds = nodes.map((node) => node.classAdds);
  const initialTitleWrites = value.titleWrites;
  for (let i = 0; i < 5; i++) fixture.api.patch(fixture.modelList);
  assert.deepEqual(nodes.map((node) => node.classAdds), initialAdds);
  assert.equal(value.titleWrites, initialTitleWrites);
  title.classList.remove("codex-plus-model-default-title");
  fixture.api.patch(fixture.modelList);
  assert.equal(title.classList.contains("codex-plus-model-default-title"), true);
});

test("native default click and keyboard actions survive legacy handler removal", () => {
  const fixture = domFixture(catalogFor(["a/sol", "b/sol"]));
  fixture.addModel("a/sol", "Model A");
  const second = fixture.addModel("b/sol", "Model B");
  for (const state of ["multiple", "single", "official", "multiple"]) {
    if (state === "single") {
      second.row.remove();
      fixture.api.refresh(catalogFor(["a/sol"]));
    } else if (state === "official") {
      fixture.api.refresh(catalogFor(["a/sol"], "not_configured"));
    } else {
      if (!second.row.parentElement) fixture.modelList.appendChild(second.row);
      fixture.api.refresh(catalogFor(["a/sol", "b/sol"]));
    }
    fixture.api.patch(fixture.modelList);
    for (const [type, key] of [["click", ""], ["keydown", "Enter"], ["keydown", " "]]) {
      const event = fixture.document.dispatch(type, fixture.defaults.row, key);
      assert.equal(event.defaultPrevented, false, state);
      assert.equal(event.stopped, false, state);
    }
  }
  assert.deepEqual(fixture.nativeCalls, Array(12).fill("default"));
  assert.deepEqual(fixture.pinnedCalls, []);
  assert.deepEqual(fixture.legacyCalls, []);
});

test("partial visible results use the full provider catalog, not pseudo groups", () => {
  const fixture = domFixture(catalogFor(["a/sol", "b/sol", "auto"]));
  fixture.addModel("a/sol", "Model A");
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model A");
  fixture.api.refresh(catalogFor(["a/sol", "auto"]));
  fixture.api.patch(fixture.modelList);
  assertNativeDefault(fixture);
});

test("legacy aggregate relay IDs and cpx model IDs remain distinct providers", () => {
  const metadata: ModelMetadata = {
    "gpt-6-sol": { displayName: "GPT-6 Sol", codex_plus_group: "Provider A", codex_plus_group_id: "legacy-relay_1" },
    "cpx-ccs-1782292584285/gpt-6-sol": {
      displayName: "GPT-6 Sol（Provider B）", codex_plus_group: "Provider B", codex_plus_group_id: "ccs-1782292584285",
    },
  };
  const fixture = domFixture({
    ...catalogFor(Object.keys(metadata)), modelMetadata: metadata, codex_plus_default_group_id: "legacy-relay_1",
    effective_config_model: "gpt-6-sol",
  }, "gpt-6-sol");
  fixture.addModel("gpt-6-sol", "GPT-6 Sol");
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "GPT-6 Sol");
});

test("only the two exact gateway pseudo IDs are excluded from provider counting", () => {
  const metadata: ModelMetadata = {
    first: { displayName: "First", codex_plus_group: "First Provider", codex_plus_group_id: "auto" },
    second: { displayName: "Second", codex_plus_group: "Second Provider", codex_plus_group_id: "codex-plus-gateway-auto-legacy" },
    grouped: providerMetadata.grouped,
  };
  const fixture = domFixture({ ...catalogFor(Object.keys(metadata)), modelMetadata: metadata,
    effective_config_model: "first" }, "first");
  fixture.addModel("first", "First");
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "First");
});

test("switching to one provider, official, and back fully restores and reapplies state", () => {
  const fixture = domFixture(catalogFor(["a/sol", "b/sol"]));
  fixture.addModel("a/sol", "Model A");
  const second = fixture.addModel("b/sol", "Model B");
  fixture.api.patch(fixture.modelList);
  assert.ok(fixture.defaults.row.querySelector(".codex-plus-model-default-value"));

  second.row.remove();
  fixture.api.refresh(catalogFor(["a/sol"]));
  fixture.api.patch(fixture.modelList);
  assertNativeDefault(fixture);
  assert.equal(fixture.modelList.hasAttribute("data-codex-plus-model-groups"), false);
  assert.equal(fixture.modelList.querySelector(".codex-plus-model-group-toolbar"), null);

  fixture.api.refresh(catalogFor(["a/sol", "b/sol"], "not_configured"));
  fixture.modelList.appendChild(second.row);
  fixture.api.patch(fixture.modelList);
  assertNativeDefault(fixture);
  assert.equal(fixture.modelList.hasAttribute("data-codex-plus-model-groups"), false);

  fixture.api.refresh(catalogFor(["a/sol", "b/sol"]));
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model A");
  fixture.api.refresh(catalogFor(["a/sol", "b/sol"], "not_configured"));
  fixture.api.patch(fixture.modelList);
  assertNativeDefault(fixture);
  assert.equal(fixture.modelList.hasAttribute("data-codex-plus-model-groups"), false);
});

test("missing or changing default models never leave a blank or stale value", () => {
  const fixture = domFixture({ ...catalogFor(["a/sol", "b/sol"]), effective_config_model: "" }, "");
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("b/sol", "Model B");
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
  fixture.setDefault("a/sol");
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model A");
  fixture.setDefault("b/sol");
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model B");
  fixture.setDefault("");
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
});

test("unapplied catalogs never expose a candidate as the actual default", () => {
  const fixture = domFixture(catalogFor(["a/sol", "b/sol"], "not_applied"));
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("b/sol", "Model B");
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
  for (const [type, key] of [["click", ""], ["keydown", "Enter"], ["keydown", " "]]) {
    const event = fixture.document.dispatch(type, fixture.defaults.row, key);
    assert.equal(event.defaultPrevented, false);
    assert.equal(event.stopped, false);
  }
  assert.deepEqual(fixture.nativeCalls, ["default", "default", "default"]);
  assert.deepEqual(fixture.pinnedCalls, []);
});

test("only an effective configured model in the published catalog is displayed", () => {
  const catalog = catalogFor(["a/sol", "b/sol"]);
  const fixture = domFixture(catalog);
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("b/sol", "Model B");
  for (const effective of [undefined, "", "unknown/model"]) {
    fixture.api.refresh({ ...catalog, effective_config_model: effective });
    fixture.api.patch(fixture.modelList);
    assertNoDefaultValue(fixture);
  }
  fixture.api.refresh({ ...catalog, effective_config_model: "b/sol" });
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model B");
  assert.equal(fixture.defaults.description.classList.contains("codex-plus-model-default-recommendation"), true);
});

test("configured display never consults the candidate-default selector", () => {
  const catalog = catalogFor(["a/sol", "b/sol"]);
  const selected = catalog.model;
  const fixture = domFixture(catalog, "a/sol");
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("b/sol", "Model B");
  fixture.setCandidateDefault("a/sol");
  fixture.api.refresh({ ...catalog, effective_config_model: "b/sol" });
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model B");
  assert.equal(catalog.model, selected);
  const value = fixture.defaults.row.querySelector(".codex-plus-model-default-value");
  value?.remove();
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model B");
});

test("effective default supports snake and camel metadata display names", () => {
  const catalog = catalogFor(["a/sol", "b/sol"]);
  catalog.modelMetadata = { ...providerMetadata,
    "a/sol": { ...providerMetadata["a/sol"], displayName: undefined, display_name: "Snake Name" },
    "b/sol": { ...providerMetadata["b/sol"], displayName: "Camel Name" } };
  const fixture = domFixture(catalog);
  fixture.addModel("a/sol", "Snake Name");
  fixture.addModel("b/sol", "Camel Name");
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Snake Name");
  fixture.setDefault("b/sol");
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Camel Name");
});

test("default display follows provider identity and pending or failed catalog updates", () => {
  const catalog = catalogFor(["a/sol", "b/sol"]);
  const fixture = domFixture(catalog);
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("b/sol", "Model B");
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model A");
  fixture.setFresh(false);
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
  fixture.setFresh(true);
  fixture.setPending(true);
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
  fixture.setPending(false);
  fixture.setFailed(true);
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
  fixture.setFailed(false);
  fixture.setSettings({ relayProfilesEnabled: true, activeRelayId: "other",
    relayProfiles: [{ id: "other", relayMode: "aggregate" }] });
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
  fixture.api.refresh({ ...catalog, effective_config_model: "b/sol",
    sources: [{ id: "aggregate:other", type: "aggregate_model_catalog" }] });
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model B");
  fixture.api.refresh({ ...catalog, status: "not_applied", effective_config_model: "b/sol" });
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
  fixture.setLoaded(false);
  fixture.api.patch(fixture.modelList);
  assertNoDefaultValue(fixture);
});

test("a fixed pure API instance resolves only its own effective model and keeps a single-provider row native", () => {
  const catalog = { ...catalogFor(["a/sol"]),
    sources: [{ id: "relay-profile:fixed", type: "relay_profile_model_list" }] };
  const fixture = domFixture(catalog);
  fixture.setSettings({ relayProfilesEnabled: false, activeRelayId: "fixed",
    relayProfiles: [{ id: "fixed", relayMode: "pureApi" }] });
  fixture.addModel("a/sol", "Model A");
  assert.equal(fixture.api.defaultModel(), "a/sol");
  fixture.api.patch(fixture.modelList);
  assertNativeDefault(fixture);
  fixture.api.refresh({ ...catalog, sources: [{ id: "relay-profile:other", type: "relay_profile_model_list" }] });
  assert.equal(fixture.api.defaultModel(), "");
  fixture.api.refresh({ ...catalog, effective_config_model: undefined });
  assert.equal(fixture.api.defaultModel(), "");
});

test("fixed multi-instance profiles require a matching source and reject official profiles", () => {
  const catalog = catalogFor(["a/sol", "b/sol"]);
  const fixture = domFixture(catalog);
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("b/sol", "Model B");
  fixture.setSettings({ relayProfilesEnabled: false, activeRelayId: "fixed",
    relayProfiles: [{ id: "fixed", relayMode: "aggregate" }] });
  fixture.api.patch(fixture.modelList);
  assert.equal(fixture.defaults.row.querySelector(".codex-plus-model-default-value")?.textContent, "Model A");
  for (const sources of [[], [{ id: "aggregate:other", type: "aggregate_model_catalog" }],
    [{ id: "aggregate:fixed", type: "other" }]]) {
    fixture.api.refresh({ ...catalog, sources });
    fixture.api.patch(fixture.modelList);
    assertNoDefaultValue(fixture);
  }
  fixture.api.refresh(catalog);
  fixture.setSettings({ relayProfilesEnabled: false, activeRelayId: "fixed",
    relayProfiles: [{ id: "fixed", relayMode: "official" }] });
  fixture.api.patch(fixture.modelList);
  assertNativeDefault(fixture);
});

test("native title, subtitle and container redraws are reconciled without duplicate values", () => {
  const fixture = domFixture(catalogFor(["a/sol", "b/sol"]));
  fixture.addModel("a/sol", "Model A");
  fixture.addModel("b/sol", "Model B");
  fixture.api.patch(fixture.modelList);
  const { row, inner, title, description } = fixture.defaults;
  const newTitle = new FixtureElement("span");
  newTitle.textContent = "默认";
  inner.insertBefore(newTitle, title);
  title.remove();
  const newDescription = new FixtureElement("span");
  newDescription.textContent = "推荐默认集";
  inner.insertBefore(newDescription, description);
  description.remove();
  const staleValue = new FixtureElement("span");
  staleValue.className = "codex-plus-model-default-value";
  row.querySelector("[data-menu-row-content]")?.appendChild(staleValue);
  fixture.api.patch(fixture.modelList);
  assert.equal(newTitle.classList.contains("codex-plus-model-default-title"), true);
  assert.equal(newDescription.classList.contains("codex-plus-model-default-recommendation"), true);
  assert.equal(row.querySelectorAll(".codex-plus-model-default-value").length, 1);
  assert.equal(staleValue.parentElement, null);

  const replacement = domRow("Default", "Recommended set of models");
  const content = inner.parentElement!;
  content.insertBefore(replacement.inner, inner);
  inner.remove();
  fixture.api.patch(fixture.modelList);
  assert.equal(replacement.inner.classList.contains("codex-plus-model-default-layout"), true);
  assert.equal(replacement.title.classList.contains("codex-plus-model-default-title"), true);
  assert.equal(replacement.description.classList.contains("codex-plus-model-default-recommendation"), true);
  assert.equal(row.querySelectorAll(".codex-plus-model-default-value").length, 1);

  replacement.title.remove();
  fixture.api.patch(fixture.modelList);
  assert.equal(row.querySelector(".codex-plus-model-default-value"), null);
  assert.equal(replacement.description.classList.contains("codex-plus-model-default-recommendation"), false);
  replacement.inner.insertBefore(replacement.title, replacement.description);
  fixture.api.patch(fixture.modelList);
  assert.equal(row.querySelector(".codex-plus-model-default-value")?.textContent, "Model A");
});

test("long model names keep their full accessible text and use bounded grid cells", () => {
  const longName = "Provider A / " + "very-long-model-name-".repeat(12);
  const catalog = catalogFor(["a/sol", "b/sol"]);
  catalog.modelMetadata = { ...providerMetadata, "a/sol": { ...providerMetadata["a/sol"], displayName: longName } };
  const fixture = domFixture(catalog);
  fixture.addModel("a/sol", longName);
  fixture.addModel("b/sol", "Model B");
  fixture.api.patch(fixture.modelList);
  const value = fixture.defaults.row.querySelector(".codex-plus-model-default-value");
  assert.equal(value?.textContent, longName);
  assert.equal(value?.title, longName);
  assert.equal(value?.getAttribute("aria-hidden"), null);
  assert.equal(fixture.defaults.check.parentElement !== null, true);
  const css = fixture.document.head.children.find((node) => node.id === "codex-plus-model-picker-groups-style")?.textContent || "";
  assert.match(css, /grid-template-columns:minmax\(0,max-content\) minmax\(0,1fr\)/);
  assert.match(css, /codex-plus-model-default-value\{[^}]*min-width:0;overflow:hidden;text-overflow:ellipsis/);
  assert.match(css, /codex-plus-model-default-value\{[^}]*text-align:right/);
});

test("legacy default row patches are restored for both single and multiple providers", () => {
  for (const models of [["a/sol"], ["a/sol", "b/sol"]]) {
    const fixture = domFixture(catalogFor(models));
    fixture.addModel("a/sol", "Model A");
    if (models.length > 1) fixture.addModel("b/sol", "Model B");
    const { row, title, description } = fixture.defaults;
    fixture.api.installLegacy({ row, name: "默认" });
    row.classList.add("codex-plus-model-default-row", "codex-plus-model-default-suppress-selection");
    description.classList.add("codex-plus-model-default-subtitle");
    description.setAttribute("aria-hidden", "true");
    fixture.api.patch(fixture.modelList);
    assert.equal(title.getAttribute("aria-hidden"), null);
    assert.equal(description.getAttribute("aria-hidden"), null);
    assert.equal(row.classList.contains("codex-plus-model-default-suppress-selection"), false);
    assert.equal(row.querySelector(".codex-plus-model-group-own-label"), null);
    if (models.length === 1) assertNativeDefault(fixture);
    else assert.equal(row.querySelector(".codex-plus-model-default-value")?.textContent, "Model A");
  }
});

test("legacy aria recovery uses React props and does not guess when original props are unavailable", () => {
  for (const original of [null, "false", "true", "unknown"]) {
    const fixture = domFixture(catalogFor(["a/sol"]));
    fixture.addModel("a/sol", "Model A");
    const { row, description } = fixture.defaults;
    fixture.api.installLegacy({ row, name: "默认" });
    row.classList.add("codex-plus-model-default-row", "codex-plus-model-default-suppress-selection");
    description.classList.add("codex-plus-model-default-subtitle");
    description.setAttribute("aria-hidden", "true");
    description.setAttribute("style", "color: red;");
    if (original === "unknown") delete (description as any).__reactProps$fixture;
    else (description as any).__reactProps$fixture = original === null ? {} : { "aria-hidden": original };
    fixture.api.patch(fixture.modelList);
    assert.equal(description.getAttribute("aria-hidden"), original === "unknown" ? "true" : original);
    assert.equal(description.getAttribute("style"), "color: red;");
    assert.equal(description.classList.contains("codex-plus-model-default-subtitle"), false);
  }
});
