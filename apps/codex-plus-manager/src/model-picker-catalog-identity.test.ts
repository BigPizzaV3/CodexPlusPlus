import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/71-model-picker-groups.js", import.meta.url), "utf8");
function runtime(catalog: any) {
  const names = ["codexPlusModelPickerNormalize", "codexPlusModelPickerNormalizeStrict",
    "codexPlusModelPickerNormalizeLoose", "codexPlusModelPickerNativeLabelKey",
    "codexPlusModelPickerSplitLabel", "providerNameOf", "codexPlusModelPickerCatalogEntries",
    "codexPlusModelPickerCatalogAliases", "codexPlusModelPickerCatalogProviderMap",
    "codexPlusModelPickerCatalogExactMap", "codexPlusModelPickerRowModel",
    "codexPlusModelPickerRememberProvider", "codexPlusModelPickerRows", "codexPlusModelPickerGroups"];
  const functions = names.map(name => {
    const found = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"));
    assert.ok(found, name);
    return found[0];
  }).join("\n");
  const state = { providerNames: new Map(), defaultProviderName: "" };
  const context = vm.createContext({
    codexModelCatalog: catalog, codexPlusModelPickerGroupsState: state,
    codexPlusModelPickerGroupNameMaxLength: 24,
    codexPlusModelPickerDefaultGroupId: () => "provider:a",
    codexPlusModelPickerDefaultProviderName: () => "Provider A",
    codexPlusModelPickerRowIdentity: (row: any) => ({ text: row.label, groupable: true }),
  });
  const api = vm.runInContext(`${functions}
    ({ exact: codexPlusModelPickerCatalogExactMap, providers: codexPlusModelPickerCatalogProviderMap,
       rows: codexPlusModelPickerRows, rowModel: codexPlusModelPickerRowModel,
       groups: codexPlusModelPickerGroups })`, context);
  return { api, state, update: (next: any) => { context.codexModelCatalog = next; } };
}
const entry = (displayName: string, id: string) => ({
  displayName, codex_plus_group: `Provider ${id.toUpperCase()}`, codex_plus_group_id: `provider:${id}`,
});
const row = (label: string, model?: string) => ({
  label, classList: { contains: () => false },
  __reactFiber$test: {
    key: model, memoizedProps: {},
    return: { memoizedProps: { models: [], model: "a/sol", onSelectModel() {} }, return: null },
  },
});

it("clears the remembered default only when a successful catalog explicitly removes it", () => {
  const definition = source.match(/^  function codexPlusModelPickerDefaultGroupId\([^]*?^  \}/m)?.[0];
  assert.ok(definition);
  const state = { defaultGroupId: "", providerDirty: false };
  const context = vm.createContext({
    codexModelCatalog: { status: "ok", codex_plus_default_group_id: "provider:a" },
    codexPlusModelPickerGroupsState: state,
    codexPlusModelPickerRememberProvider: () => {},
    providerNameOf: () => "",
  });
  const getDefault = vm.runInContext(`${definition}\ncodexPlusModelPickerDefaultGroupId`, context);
  assert.equal(getDefault(), "provider:a");
  context.codexModelCatalog = { status: "failed", codex_plus_default_group_id: "" };
  assert.equal(getDefault(), "provider:a", "failure must retain the last successful default");
  context.codexModelCatalog = { status: "ok" };
  assert.equal(getDefault(), "provider:a", "older backends without the field retain the fallback");
  state.providerDirty = false;
  context.codexModelCatalog = { status: "ok", codex_plus_default_group_id: "",
    modelMetadata: { old: { codex_plus_default: true, codex_plus_group_id: "provider:a" } } };
  assert.equal(getDefault(), "");
  assert.equal(state.defaultGroupId, "");
  assert.equal(state.providerDirty, true, "the cleared default must be persisted");
  context.codexModelCatalog = { status: "not_applied", codex_plus_default_group_id: "provider:b" };
  assert.equal(getDefault(), "provider:b");
  context.codexModelCatalog = { status: "not_applied", codex_plus_default_group_id: "" };
  assert.equal(getDefault(), "");
});

it("canonical row IDs resolve identical labels without borrowing the selected parent model", () => {
  const r = runtime({ status: "ok", models: ["a/sol", "b/sol"],
    modelMetadata: { "a/sol": entry("GPT-6.1-Sol", "a"), "b/sol": entry("GPT-6.1-Sol", "b") } });
  const rows = [row("6.1 Sol", "a/sol"), row("6.1 Sol", "b/sol")];
  const entries = r.api.rows({ querySelectorAll: () => rows });
  assert.deepEqual([...entries].map(entry => entry.groupId), ["provider:a", "provider:b"]);
  assert.equal(r.api.rowModel(row("6.1 Sol")), "", "parent selection cannot identify an individual row");
  assert.equal(r.api.providers().get("6.1-sol"), null, "ambiguous aliases cannot silently pick the first supplier");
});

it("zero-width disambiguators survive matching while ordinary native formatting may differ", () => {
  const r = runtime({ models: ["a/sol", "b/sol"],
    modelMetadata: { "a/sol": entry("GPT-6.1-Sol", "a"), "b/sol": entry("GPT-6.1-Sol\u200b", "b") } });
  const entries = r.api.rows({ querySelectorAll: () => [row("6.1 Sol"), row("6.1 Sol\u200b")] });
  assert.deepEqual([...entries].map(entry => entry.groupId), ["provider:a", "provider:b"]);
});

it("deleted metadata is excluded and fresh supplier renames replace remembered names", () => {
  const r = runtime({ status: "ok", models: ["a/sol", "b/terra"],
    modelMetadata: { "a/sol": entry("Sol", "a"), "b/terra": entry("Terra", "b") } });
  r.api.providers();
  r.update({ status: "ok", models: ["a/sol"],
    modelMetadata: { "a/sol": { ...entry("Sol", "a"), codex_plus_group: "Renamed" },
      "b/terra": entry("Terra", "b") } });
  assert.equal(r.api.exact().has("model\u0000b/terra"), false);
  assert.equal(r.api.providers().has("terra"), false);
  assert.equal(r.api.providers().get("sol").provider, "Renamed");
});

it("temporary missing metadata retains known provider grouping without publishing extra models", () => {
  const r = runtime({ status: "ok", models: ["a/sol"], modelMetadata: { "a/sol": entry("Sol", "a") } });
  r.api.providers();
  r.update({ status: "ok", models: ["a/sol"], modelMetadata: {} });
  const entries = r.api.rows({ querySelectorAll: () => [row("Sol")] });
  assert.equal(entries[0].groupId, "provider:a");
  assert.equal(entries[0].provider, "Provider A");
});

it("missing metadata preserves only known model ownership and leaves unknown bare models unassigned", () => {
  const r = runtime({ status: "ok", models: ["a/sol", "b/terra"],
    modelMetadata: { "a/sol": entry("Sol", "a"), "b/terra": entry("Terra", "b") } });
  r.api.providers();
  r.update({ status: "ok", models: ["a/sol", "b/terra", "unknown"], modelMetadata: {} });
  const entries = r.api.rows({ querySelectorAll: () => [row("Sol"), row("Terra"), row("Unknown")] });
  assert.deepEqual([...entries].map(value => [value.name, value.provider, value.groupId]), [
    ["Sol", "Provider A", "provider:a"],
    ["Terra", "Provider B", "provider:b"],
    ["Unknown", "", ""],
  ]);
  assert.equal(entries[2].authoritativeGroupId, false);
  const coldStart = runtime({ status: "failed", models: [], modelMetadata: {} });
  const unknown = coldStart.api.rows({ querySelectorAll: () => [row("Unseen")] });
  assert.equal(unknown[0].provider, "");
  assert.equal(unknown[0].groupId, "");
});

it("uses uniquely remembered provider identity for a suffix when metadata is temporarily missing", () => {
  const r = runtime({ status: "ok", models: ["a/sol"],
    modelMetadata: { "a/sol": entry("Sol", "a") } });
  r.api.providers();
  r.update({ status: "ok", models: ["a/sol", "a/terra"], modelMetadata: {} });
  const entries = r.api.rows({ querySelectorAll: () => [row("Sol"), row("Terra（Provider A）")] });
  assert.deepEqual([...entries].map(value => value.groupId), ["provider:a", "provider:a"]);
});

it("keeps interleaved model-group and provider rows together after metadata disappears", () => {
  const metadata = {
    "group/1": { displayName: "Group One", codex_plus_group: "模型组", codex_plus_group_id: "groups" },
    "hanhe/1": { displayName: "Hanhe One", codex_plus_group: "寒鹤", codex_plus_group_id: "hanhe" },
    "fusheng/1": { displayName: "Fusheng One", codex_plus_group: "浮生", codex_plus_group_id: "fusheng" },
  };
  const r = runtime({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata });
  r.api.providers();
  r.update({ status: "ok", models: [...Object.keys(metadata), "group/2", "hanhe/2"], modelMetadata: {} });
  const entries = r.api.rows({ querySelectorAll: () => [
    row("Group One"), row("Hanhe One"), row("Group Two（模型组）"),
    row("Fusheng One"), row("Hanhe Two（寒鹤）"),
  ] });
  const groups = r.api.groups(entries);
  assert.deepEqual([...groups].map(group => [group.name, group.entries.length]), [
    ["模型组", 2], ["寒鹤", 2], ["浮生", 1],
  ]);
});

it("does not pick the last provider when two catalog identities have the same name", () => {
  const sameName = (displayName: string, id: string) => ({
    displayName, codex_plus_group: "Same", codex_plus_group_id: id,
  });
  const r = runtime({ status: "ok", models: ["a/sol", "b/terra"],
    modelMetadata: { "a/sol": sameName("Sol", "id-a"), "b/terra": sameName("Terra", "id-b") } });
  const entries = r.api.rows({ querySelectorAll: () => [row("Unknown（Same）")] });
  assert.equal(entries[0].groupId, "");
  assert.equal(entries[0].authoritativeGroupId, false);
});

it("remembered ownership is discarded when its catalog source changes", () => {
  const r = runtime({ status: "ok", models: ["a/sol"], modelMetadata: { "a/sol": entry("Sol", "a") } });
  r.api.providers();
  assert.equal(r.state.providerNames.size > 0, true);
  const changed = source.match(/^  function codexPlusModelPickerCatalogChanged\([^]*?^  \}/m)?.[0];
  assert.ok(changed);
  const notify = new Function("codexPlusModelPickerGroupsState", "codexPlusModelPickerSchedule",
    "codexPlusModelPickerPersist", `${changed}\nreturn codexPlusModelPickerCatalogChanged;`)
    (r.state, () => {}, () => {});
  notify({ sourceChanged: false });
  assert.equal(r.state.providerNames.size > 0, true, "same-source refresh retains ownership");
  notify({ sourceChanged: true });
  assert.equal(r.state.providerNames.size, 0, "a different source cannot inherit cached ownership");
});

it("persisted ownership is read only for its matching initialized source", () => {
  const storeSource = source.match(/^  function codexPlusModelPickerStore\([^]*?^  \}/m)?.[0];
  assert.ok(storeSource);
  for (const [currentSource, cachedSource, shouldLoad] of [
    ["source-a", "source-a", true],
    ["source-b", "source-a", false],
    ["source-a", undefined, false],
    ["", "source-a", false],
  ] as const) {
    const state = { stored: false, providerNames: new Map(), defaultGroupId: "" };
    const window = { __codexPlusModelCatalogRuntime: { source: currentSource },
      localStorage: { getItem: () => JSON.stringify({
        source: cachedSource, defaultGroupId: "provider:a",
        providers: { sol: { name: "Provider A", groupId: "provider:a", parsed: false } },
      }) } };
    const load = new Function("window", "codexPlusModelPickerGroupsState", "codexPlusModelPickerGroupsStoreKey",
      `${storeSource}\nreturn codexPlusModelPickerStore;`)(window, state, "test");
    load();
    assert.equal(state.providerNames.size > 0, shouldLoad, `${currentSource}/${cachedSource}`);
    if (!currentSource) assert.equal(state.stored, false, "wait for initialized settings");
  }
});

it("initialization discards ownership learned before the source was known", () => {
  const storeSource = source.match(/^  function codexPlusModelPickerStore\([^]*?^  \}/m)?.[0];
  assert.ok(storeSource);
  const state = { stored: false, providerNames: new Map([["sol", { name: "Old" }]]),
    defaultGroupId: "old", defaultProviderName: "Old" };
  const window = { __codexPlusModelCatalogRuntime: { source: "" },
    localStorage: { getItem: () => null } };
  const load = new Function("window", "codexPlusModelPickerGroupsState", "codexPlusModelPickerGroupsStoreKey",
    `${storeSource}\nreturn codexPlusModelPickerStore;`)(window, state, "test");
  load();
  assert.equal(state.stored, false);
  window.__codexPlusModelCatalogRuntime.source = "new";
  load();
  assert.equal(state.providerNames.size, 0);
  assert.equal(state.defaultGroupId, "");
  assert.equal(state.defaultProviderName, "");
});

it("persists ownership only after a catalog source is initialized", () => {
  const persistSource = source.match(/^  function codexPlusModelPickerPersist\([^]*?^  \}/m)?.[0];
  assert.ok(persistSource);
  const writes: any[] = [];
  const window = { __codexPlusModelCatalogRuntime: { source: "" },
    localStorage: { setItem: (_key: string, value: string) => writes.push(JSON.parse(value)) } };
  const state = { providerNames: new Map([["sol", { name: "Provider A" }]]), defaultGroupId: "a" };
  const persist = new Function("window", "codexPlusModelPickerGroupsState", "codexPlusModelPickerGroupsStoreKey",
    `${persistSource}\nreturn codexPlusModelPickerPersist;`)(window, state, "test");
  persist();
  assert.equal(writes.length, 0);
  window.__codexPlusModelCatalogRuntime.source = "source-a";
  persist();
  assert.equal(writes[0].source, "source-a");
  assert.equal(writes[0].providers.sol.name, "Provider A");
});

it("a parenthesized model variant is preserved unless the catalog confirms a supplier suffix", () => {
  const r = runtime({ status: "ok", models: ["a/variant"],
    modelMetadata: { "a/variant": entry("GPT-6.1 (Sol)", "a") } });
  const entries = r.api.rows({ querySelectorAll: () => [row("6.1 (Sol)", "a/variant")] });
  assert.equal(entries[0].name, "6.1 (Sol)");
  assert.equal(entries[0].provider, "Provider A");
});
