import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/71-model-picker-groups.js", import.meta.url), "utf8");
function runtime(catalog: any) {
  const names = ["codexPlusModelPickerNormalize", "codexPlusModelPickerNormalizeStrict",
    "codexPlusModelPickerNormalizeLoose", "codexPlusModelPickerNativeLabelKey",
    "codexPlusModelPickerSplitLabel", "providerNameOf", "codexPlusModelPickerCatalogEntries",
    "codexPlusModelPickerCatalogAliases", "codexPlusModelPickerCanRestoreProvider", "codexPlusModelPickerCatalogProviderMap",
    "codexPlusModelPickerCatalogExactMap", "codexPlusModelPickerPrice",
    "codexPlusModelPickerCatalogPriceMap", "codexPlusModelPickerRowSlug", "codexPlusModelPickerRowModel",
    "codexPlusModelPickerRememberProvider", "codexPlusModelPickerRows", "codexPlusModelPickerGroups"];
  const functions = names.map(name => {
    const found = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"));
    assert.ok(found, name);
    return found[0];
  }).join("\n");
  const state = { providerNames: new Map(), defaultProviderName: "" };
  const context = vm.createContext({
    codexModelCatalog: catalog, codexPlusModelPickerGroupsState: state,
    codexPlusModelPickerDomState: new WeakMap(),
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

it("keeps five interleaved models per group together through transient metadata loss", () => {
  const sources = [
    { prefix: "group", name: "模型组", id: "groups" },
    { prefix: "hanhe", name: "寒鹤", id: "provider:hanhe" },
    { prefix: "fusheng", name: "浮生", id: "provider:fusheng" },
  ];
  const metadata: Record<string, any> = {};
  const rows: ReturnType<typeof row>[] = [];
  for (let index = 1; index <= 5; index++) {
    for (const source of sources) {
      const model = `${source.prefix}/${index}`;
      const label = `${source.prefix} ${index}`;
      metadata[model] = { displayName: label, codex_plus_group: source.name, codex_plus_group_id: source.id };
      rows.push(row(label, model));
    }
  }
  const models = Object.keys(metadata);
  const r = runtime({ status: "ok", models, modelMetadata: metadata });
  const assertGroups = (phase: string) => {
    const entries = r.api.rows({ querySelectorAll: () => rows });
    const groups = [...r.api.groups(entries)];
    assert.deepEqual(groups.map((group: any) => [group.id, group.entries.length]),
      sources.map(source => [source.id, 5]), phase);
    for (const group of groups) {
      assert.equal(new Set(group.entries.map((entry: any) => entry.row)).size, 5, phase);
      const source = sources.find(value => value.id === group.id)!;
      assert.deepEqual(Array.from(group.entries, (entry: any) => entry.row.label),
        Array.from({ length: 5 }, (_, index) => `${source.prefix} ${index + 1}`), phase);
    }
  };

  assertGroups("complete metadata");
  r.update({ status: "ok", models, modelMetadata: Object.fromEntries(
    models.map(model => [model, { displayName: metadata[model].displayName }]),
  ) });
  assertGroups("grouping metadata temporarily missing");
  r.update({ status: "failed", models: [] });
  assertGroups("catalog temporarily unavailable");
  r.update({ status: "ok", models, modelMetadata: metadata });
  assertGroups("metadata recovered");
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

it("retains same-source identities when successful metadata omits grouping fields", () => {
  const metadata = {
    "a/sol": entry("Sol", "a"),
    "b/terra": entry("Terra", "b"),
    "a/luna": entry("Luna", "a"),
  };
  const r = runtime({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata });
  r.api.rows({ querySelectorAll: () => Object.entries(metadata).map(([id, value]) => row(value.displayName, id)) });
  r.update({ status: "ok", models: Object.keys(metadata), modelMetadata: {
    "a/sol": { displayName: "Sol" },
    "b/terra": { displayName: "Terra" },
    "a/luna": { displayName: "Luna" },
  } });
  const entries = r.api.rows({ querySelectorAll: () => [
    row("Sol", "a/sol"), row("Terra", "b/terra"), row("Luna", "a/luna"),
  ] });
  assert.deepEqual([...entries].map((value: any) => value.groupId), ["provider:a", "provider:b", "provider:a"]);
  assert.deepEqual([...r.api.groups(entries)].map((group: any) => [group.name, group.entries.length]), [
    ["Provider A", 2], ["Provider B", 1],
  ]);
});

it("does not restore an identity when partial metadata explicitly changes its grouping or name", () => {
  for (const replacement of [
    { displayName: "Sol", codex_plus_group_id: "" },
    { displayName: "Sol", codex_plus_group: "" },
    { displayName: "Renamed" },
    { displayName: "Sol", description: "Upstream · Provider B" },
  ]) {
    const r = runtime({ status: "ok", models: ["a/sol"], modelMetadata: { "a/sol": entry("Sol", "a") } });
    r.api.rows({ querySelectorAll: () => [row("Sol", "a/sol")] });
    r.update({ status: "ok", models: ["a/sol"], modelMetadata: { "a/sol": replacement } });
    const current = r.api.rows({ querySelectorAll: () => [row(replacement.displayName, "a/sol")] });
    assert.equal(current[0].groupId, "", JSON.stringify(replacement));
  }
});

it("keeps same-label identities distinct with partial metadata and forgets removed models", () => {
  const metadata = { "a/sol": entry("Sol", "a"), "b/sol": entry("Sol", "b") };
  const r = runtime({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata });
  r.api.rows({ querySelectorAll: () => [row("Sol", "a/sol"), row("Sol", "b/sol")] });
  r.update({ status: "ok", models: Object.keys(metadata), modelMetadata: {
    "a/sol": { displayName: "Sol" }, "b/sol": { displayName: "Sol" },
  } });
  const current = r.api.rows({ querySelectorAll: () => [
    row("Sol", "a/sol"), row("Sol", "b/sol"), row("Sol"),
  ] });
  assert.deepEqual([...current].map((value: any) => value.groupId), ["provider:a", "provider:b", ""]);
  r.update({ status: "ok", models: ["a/sol"], modelMetadata: { "a/sol": { displayName: "Sol" } } });
  assert.equal(r.api.exact().has("model\u0000b/sol"), false);
  assert.equal(r.api.rows({ querySelectorAll: () => [row("Sol", "a/sol")] })[0].groupId, "provider:a");
});

it("restores same-source ownership after a cold catalog failure without treating failure as an empty directory", () => {
  const metadata = {
    "a/sol": entry("Sol", "a"),
    "b/sol": entry("Sol", "b"),
    "group/one": { displayName: "Group One", codex_plus_group: "模型组", codex_plus_group_id: "groups" },
    "a/terra": entry("Terra", "a"),
  };
  const previous = runtime({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata });
  previous.api.rows({ querySelectorAll: () => Object.entries(metadata).map(([model, value]) => row(value.displayName, model)) });
  const r = runtime({ status: "loading", models: [] });
  for (const [key, value] of previous.state.providerNames) {
    r.state.providerNames.set(key, value);
  }
  const menu = () => r.api.rows({ querySelectorAll: () => [
    row("Sol", "a/sol"), row("Group One", "group/one"), row("Sol", "b/sol"), row("Terra", "a/terra"),
  ] });
  for (const status of ["loading", "failed"]) {
    r.update({ status, models: [] });
    const entries = menu();
    assert.deepEqual([...entries].map((value: any) => value.groupId), ["provider:a", "groups", "provider:b", "provider:a"], status);
    assert.deepEqual([...r.api.groups(entries)].map((group: any) => [group.name, group.entries.length]), [
      ["Provider A", 2], ["模型组", 1], ["Provider B", 1],
    ], status);
    const unidentified = r.api.rows({ querySelectorAll: () => [row("Sol")] });
    assert.equal(unidentified[0].groupId, "", "same-label rows without model IDs remain ambiguous");
  }
  r.update({ status: "ok", models: ["a/sol"], modelMetadata: { "a/sol": metadata["a/sol"] } });
  const afterRemoval = menu();
  assert.equal(afterRemoval[1].groupId, "", "successful catalog removal must exclude remembered group");
  assert.equal(afterRemoval[2].groupId, "", "successful catalog removal must exclude remembered supplier");
  assert.equal(afterRemoval[3].groupId, "", "successful catalog removal must exclude remembered model");
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

it("retains distinct same-label model identities when metadata temporarily disappears", () => {
  const metadata = {
    "a/sol": entry("Sol", "a"),
    "b/sol": entry("Sol", "b"),
    "group/one": { displayName: "Group One", codex_plus_group: "模型组", codex_plus_group_id: "groups" },
    "a/terra": entry("Terra", "a"),
  };
  const r = runtime({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata });
  r.api.rows({ querySelectorAll: () => [
    row("Sol", "a/sol"), row("Sol", "b/sol"), row("Group One", "group/one"), row("Terra", "a/terra"),
  ] });
  r.update({ status: "ok", models: Object.keys(metadata), modelMetadata: {} });
  const entries = r.api.rows({ querySelectorAll: () => [
    row("Sol", "a/sol"), row("Group One", "group/one"),
    row("Sol", "b/sol"), row("Terra", "a/terra"),
  ] });
  assert.deepEqual([...entries].map(value => value.groupId), ["provider:a", "groups", "provider:b", "provider:a"]);
  assert.deepEqual([...r.api.groups(entries)].map(group => [group.name, group.entries.length]), [
    ["Provider A", 2], ["模型组", 1], ["Provider B", 1],
  ]);
  const ambiguous = r.api.rows({ querySelectorAll: () => [row("Sol")] });
  assert.equal(ambiguous[0].groupId, "", "an ambiguous label must not inherit another model's owner");
  r.update({ status: "ok", models: Object.keys(metadata), modelMetadata: { "a/sol": metadata["a/sol"] } });
  const partial = r.api.rows({ querySelectorAll: () => [row("Sol"), row("Sol", "b/sol")] });
  assert.equal(partial[0].groupId, "", "partial metadata must not turn an ambiguous label into a match");
  assert.equal(partial[1].groupId, "provider:b");
  r.update({ status: "ok", models: Object.keys(metadata) });
  const missing = r.api.rows({ querySelectorAll: () => [row("Sol", "a/sol"), row("Sol", "b/sol")] });
  assert.deepEqual([...missing].map(value => value.groupId), ["provider:a", "provider:b"]);
  r.update({ status: "ok", models: ["a/sol"], modelMetadata: {} });
  const removed = r.api.rows({ querySelectorAll: () => [row("Sol", "a/sol")] });
  assert.equal(removed[0].groupId, "provider:a", "removed models must not make remaining aliases ambiguous");
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

it("collects interleaved ambiguous owners without hiding their original labels", () => {
  const sameName = (displayName: string, id: string) => ({
    displayName, codex_plus_group: "Same", codex_plus_group_id: id,
  });
  const metadata = {
    "a/one": sameName("One", "id-a"),
    "b/two": sameName("Two", "id-b"),
  };
  const r = runtime({ status: "ok", models: Object.keys(metadata), modelMetadata: metadata });
  const entries = r.api.rows({ querySelectorAll: () => [
    row("Unknown 1（Same）"), row("One", "a/one"),
    row("Unknown 2（Same）"), row("Two", "b/two"),
    row("Unknown 3（Same）"),
  ] });
  assert.deepEqual([...entries].filter(value => !value.groupId).map(value => value.name),
    ["Unknown 1（Same）", "Unknown 2（Same）", "Unknown 3（Same）"]);
  assert.deepEqual([...r.api.groups(entries)].map(group => [group.name, group.id, group.entries.length]), [
    ["", "", 3], ["Same", "id-a", 1], ["Same", "id-b", 1],
  ]);
});

it("moves previously ambiguous rows into their verified groups when metadata arrives", () => {
  const r = runtime({ status: "ok", models: ["a/one", "b/two"],
    modelMetadata: {
      "a/one": { displayName: "One", codex_plus_group: "Same", codex_plus_group_id: "id-a" },
      "b/two": { displayName: "Two", codex_plus_group: "Same", codex_plus_group_id: "id-b" },
    } });
  const menu = () => r.api.rows({ querySelectorAll: () => [
    row("One", "a/one"), row("Three（Same）"), row("Two", "b/two"),
  ] });
  let groups = r.api.groups(menu());
  assert.deepEqual([...groups].map(group => [group.id, group.entries.length]), [
    ["id-a", 1], ["", 1], ["id-b", 1],
  ]);
  r.update({ status: "ok", models: ["a/one", "a/three", "b/two"],
    modelMetadata: {
      "a/one": { displayName: "One", codex_plus_group: "Same", codex_plus_group_id: "id-a" },
      "a/three": { displayName: "Three（Same）", codex_plus_group: "Same", codex_plus_group_id: "id-a" },
      "b/two": { displayName: "Two", codex_plus_group: "Same", codex_plus_group_id: "id-b" },
    } });
  groups = r.api.groups(menu());
  assert.deepEqual([...groups].map(group => [group.id, group.entries.length]), [
    ["id-a", 2], ["id-b", 1],
  ]);
});

it("does not reuse a model's old display name after its authoritative rename", () => {
  const r = runtime({ status: "ok", models: ["a/one"],
    modelMetadata: { "a/one": entry("Old Name", "a") } });
  r.api.providers();
  r.update({ status: "ok", models: ["a/one"],
    modelMetadata: { "a/one": entry("New Name", "a") } });
  r.api.providers();
  r.update({ status: "ok", models: ["a/one", "b/two"], modelMetadata: {} });
  const entries = r.api.rows({ querySelectorAll: () => [row("Old Name"), row("New Name", "a/one")] });
  assert.equal(entries[0].groupId, "", "a stale label cannot claim a new model");
  assert.equal(entries[1].groupId, "provider:a");
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

it("catalog changes only reschedule an already active picker", () => {
  const changed = source.match(/^  function codexPlusModelPickerCatalogChanged\([^]*?^  \}/m)?.[0];
  assert.ok(changed);
  const state = { disposed: false, timer: 0 };
  const delays: number[] = [];
  const notify = new Function("codexPlusModelPickerGroupsState", "codexPlusModelPickerSchedule",
    "codexPlusModelPickerPersist", `${changed}\nreturn codexPlusModelPickerCatalogChanged;`)
    (state, (delay: number) => delays.push(delay), () => {});

  notify();
  assert.deepEqual(delays, [], "catalog loading must not start the picker");
  state.timer = 1;
  notify();
  assert.deepEqual(delays, [0], "an active picker refreshes immediately");
  state.disposed = true;
  notify();
  assert.deepEqual(delays, [0], "a disposed picker stays stopped");
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
        providers: {
          sol: { name: "Provider A", groupId: "provider:a", parsed: false },
          "model\u0000a/sol": { name: "Provider A", groupId: "provider:a", parsed: false, slug: "a/sol", displayName: "Sol" },
        },
      }) } };
    const load = new Function("window", "codexPlusModelPickerGroupsState", "codexPlusModelPickerGroupsStoreKey",
      `${storeSource}\nreturn codexPlusModelPickerStore;`)(window, state, "test");
    load();
    assert.equal(state.providerNames.size > 0, shouldLoad, `${currentSource}/${cachedSource}`);
    if (shouldLoad) assert.equal(state.providerNames.get("model\u0000a/sol")?.displayName, "Sol");
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
