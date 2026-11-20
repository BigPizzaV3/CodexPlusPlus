import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";
import { modelFastSupportMode, setModelFastSupportMode, synchronizeModelMetadataFastSupport, parseModelMetadataDocument, createActiveImportDraft, cancelActiveImportDraft } from "./model-metadata.ts";

const fragment = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");
const extract = (source: string, name: string) => {
  const body = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0];
  assert.ok(body, name); return body;
};

test("per-model Fast modes preserve all original metadata across support, denial and inheritance", () => {
  const original = { description: "retained", service_tiers: [{ id: "flex", name: "Flex", description: "Existing tier" }], additional_speed_tiers: ["slow"] };
  let current = original;
  for (const mode of ["supported", "unsupported", "inherit"] as const) {
    current = setModelFastSupportMode(current, mode) as typeof current;
    assert.equal(modelFastSupportMode(current), mode);
    assert.deepEqual(current.service_tiers, original.service_tiers);
    assert.deepEqual(current.additional_speed_tiers, original.additional_speed_tiers);
    assert.equal(current.description, original.description);
  }
  assert.equal("codex_plus_fast_support" in original, false);
  assert.equal(modelFastSupportMode({ service_tiers: [] }), "unsupported");
  assert.equal(modelFastSupportMode({ additional_speed_tiers: ["fast"] }), "supported");
  assert.equal(modelFastSupportMode({}), "inherit");
});

test("Fast editing touches only the selected model and cancellation keeps the saved declaration", () => {
  const source = JSON.stringify({ models: [{ slug: "grok", service_tiers: [{ id: "flex" }] }, { slug: "other", description: "untouched" }] });
  const changed = synchronizeModelMetadataFastSupport(source, "GROK[1M]", "supported")!;
  assert.ok(changed);
  assert.deepEqual(JSON.parse(changed.document).models[1], JSON.parse(source).models[1]);
  assert.deepEqual(JSON.parse(changed.document).models[0].service_tiers, [{ id: "flex" }]);
  assert.equal(changed.preview.metadata.codex_plus_fast_support, "supported");
  const inherited = synchronizeModelMetadataFastSupport(changed.document, "grok", "inherit")!;
  assert.equal(modelFastSupportMode(inherited.preview.metadata), "inherit");
  const draft = createActiveImportDraft({ index: 0, rowName: "grok", window: "1000000", autoCompact: "90%", document: source, preview: parseModelMetadataDocument(source, "grok").ok ? changed.preview : null });
  assert.equal(cancelActiveImportDraft({ ...draft, document: changed.document, preview: changed.preview }).draft, null);
  assert.equal(modelFastSupportMode(parseModelMetadataDocument(source, "grok").ok ? JSON.parse(source).models[0] : null), "unsupported");
  assert.equal(synchronizeModelMetadataFastSupport("not-json", "grok", "supported"), null);
  assert.equal(synchronizeModelMetadataFastSupport(JSON.stringify({ models: [{ slug: "grok" }, { slug: "grok" }] }), "grok", "supported"), null);
});

test("Fast support declarations stay available for model metadata without enforcing a request policy", () => {
  const metadata: Record<string, unknown> = {};
  const context = vm.createContext({
    codexServiceTierSupportedFastModels: new Set(["gpt-5.4"]),
    codexPlusModelMetadata: (model: string) => metadata[model.toLowerCase()],
    codexServiceTierUiModelName: () => "grok-custom",
  });
  vm.runInContext(["normalizeCodexServiceTierModelName", "codexServiceTierFastSupportedForModel", "codexServiceTierFastAvailability"]
    .map((name) => extract(fragment("20-menu.js"), name)).join("\n"), context);
  const supported = (model: string) => context.codexServiceTierFastAvailability(model).supported;
  assert.equal(supported("grok-fast-without-a-declaration"), false);
  metadata["grok-custom"] = { prioritySupportOverride: true };
  assert.equal(supported("grok-custom"), true);
  metadata["gpt-5.4"] = { prioritySupportOverride: false, serviceTiers: [{ id: "priority" }] };
  assert.equal(supported("gpt-5.4"), false);
  metadata["gpt-5.4"] = { serviceTiers: [] };
  assert.equal(supported("gpt-5.4"), true);
});

test("retired Fast preferences cannot rewrite the native service tier or enable native unlock", () => {
  const style = fragment("10-style.js");
  const map = style.match(/  const codexPlusBackendSettingMap = \{[^]*?^  \};/m)?.[0];
  assert.ok(map);
  const stored = { serviceTierControls: true, nativeFastUnlock: false };
  const context = vm.createContext({
    conversationViewDefaultWidth: 900, window: {},
    localStorage: { getItem: () => JSON.stringify(stored) },
    codexPlusSettingsKey: "settings",
    codexPlusBackendSettings: { enhancementsEnabled: true, codexAppServiceTierControls: true },
    applyCodexRemoteSessionProviderOverride: (_method: string, params: unknown) => params,
  });
  vm.runInContext([map, ...["defaultCodexPlusSettings", "backendCodexPlusSettings", "codexPlusSettings"]
    .map((name) => extract(style, name)), extract(fragment("30-service-tier.js"), "applyCodexServiceTierRequestOverride")].join("\n"), context);
  assert.equal(context.codexPlusSettings().serviceTierControls, undefined);
  assert.equal(context.codexPlusSettings().nativeFastUnlock, false);
  for (const params of [{ model: "custom", serviceTier: "priority" }, { model: "custom", service_tier: "flex" }, { model: "custom" }]) {
    assert.equal(context.applyCodexServiceTierRequestOverride("turn/start", params), params);
  }
});

test("reinjection removes the retired composer Fast badge", () => {
  let removed = 0;
  const context = vm.createContext({ document: { querySelectorAll: (selector: string) => {
    assert.equal(selector, '[data-codex-service-tier-badge="true"]');
    return [{ remove: () => { removed += 1; } }];
  } } });
  vm.runInContext(extract(fragment("90-action-groups.js"), "removeLegacyCodexServiceTierBadges"), context);
  context.removeLegacyCodexServiceTierBadges();
  assert.equal(removed, 1);
  assert.doesNotMatch(fragment("90-action-groups.js"), /function installCodexServiceTierBadge/);
});

test("Fast flows into new-chat descriptors without changing native names or reasoning defaults", () => {
  const metadata: Record<string, unknown> = {
    prioritySupportOverride: true,
    serviceTiers: [{ id: "priority", name: "Fast", description: "Provider-declared priority" }],
    additionalSpeedTiers: ["fast"],
    supportedReasoningEfforts: [],
  };
  const context = vm.createContext({
    codexPlusModelMetadata: () => metadata,
    codexModelCatalog: {},
    structuredClone,
    modelReasoningEfforts: () => { throw new Error("Fast does not configure reasoning choices"); },
  });
  vm.runInContext(["applyCodexPlusModelMetadata", "codexPlusModelDescriptor", "codexPlusModelServiceTiers", "codexPlusModelInputModalities", "codexPlusModelImageDetailOriginal"]
    .map((name) => extract(fragment("70-model-catalog.js"), name)).join("\n"), context);
  const descriptor = { displayName: "Native Grok", description: "Native details", defaultReasoningEffort: "high", supportedReasoningEfforts: [{ reasoningEffort: "high" }] };
  const original = structuredClone(descriptor);
  assert.equal(context.applyCodexPlusModelMetadata(descriptor, "gpt-6.2-sol"), true);
  assert.deepEqual(descriptor, { ...original, serviceTiers: metadata.serviceTiers, additionalSpeedTiers: ["fast"] });
  assert.equal(context.applyCodexPlusModelMetadata(descriptor, "gpt-6.2-sol"), false);
  context.modelReasoningEfforts = () => [];
  const inserted = context.codexPlusModelDescriptor("chatgpt/gpt-6.2-astra");
  assert.deepEqual(inserted.serviceTiers, metadata.serviceTiers);
  assert.deepEqual(inserted.additionalSpeedTiers, ["fast"]);
  Object.assign(metadata, { displayName: "Explicit name", description: "Explicit details", defaultReasoningEffort: "low" });
  assert.equal(context.applyCodexPlusModelMetadata(descriptor, "grok-custom"), true);
  assert.equal(descriptor.displayName, "Explicit name");
  assert.equal(descriptor.description, "Explicit details");
  assert.equal(descriptor.defaultReasoningEffort, "low");
  Object.assign(metadata, { prioritySupportOverride: false, serviceTiers: [], additionalSpeedTiers: [] });
  assert.equal(context.applyCodexPlusModelMetadata(descriptor, "gpt-6.2-sol"), true);
  assert.equal(JSON.stringify(Reflect.get(descriptor, "serviceTiers")), "[]");
  assert.equal(JSON.stringify(Reflect.get(descriptor, "additionalSpeedTiers")), "[]");
});
