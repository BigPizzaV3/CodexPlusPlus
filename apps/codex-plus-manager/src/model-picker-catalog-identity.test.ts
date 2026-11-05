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
    "codexPlusModelPickerRememberProvider", "codexPlusModelPickerRows"];
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
       rows: codexPlusModelPickerRows, rowModel: codexPlusModelPickerRowModel })`, context);
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

it("a parenthesized model variant is preserved unless the catalog confirms a supplier suffix", () => {
  const r = runtime({ status: "ok", models: ["a/variant"],
    modelMetadata: { "a/variant": entry("GPT-6.1 (Sol)", "a") } });
  const entries = r.api.rows({ querySelectorAll: () => [row("6.1 (Sol)", "a/variant")] });
  assert.equal(entries[0].name, "6.1 (Sol)");
  assert.equal(entries[0].provider, "Provider A");
});
