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

function policyFixture() {
  const metadata: Record<string, unknown> = {};
  const state = { mode: "global-fast", defaultMode: "inherit", entries: {} };
  const context = vm.createContext({ metadata, state,
    codexServiceTierSupportedFastModels: new Set(["gpt-5.4"]),
    codexPlusModelMetadata: (model: string) => metadata[model.toLowerCase()],
    codexServiceTierUiModelName: () => "grok-custom", codexPlusSettings: () => ({ serviceTierControls: true }),
    codexServiceTierRequestMethods: () => new Set(["turn/start"]), readThreadServiceTierState: () => state,
    normalizeCodexServiceTierControlMode: (mode: string) => mode, normalizeCodexThreadServiceTierMode: (mode: string) => mode,
    codexServiceTierThreadIdForRequest: (_method: string, params: Record<string, string>) => params.threadId,
    codexServiceTierModelForRequest: (params: Record<string, string>) => params.model,
    codexServiceTierRememberThreadModel: () => {}, codexFastServiceTierValue: () => "priority", codexServiceTierInheritedValue: () => null,
    sendCodexPlusDiagnostic: () => {},
  });
  const menu = fragment("20-menu.js"), tier = fragment("30-service-tier.js");
  vm.runInContext([
    ...["normalizeCodexServiceTierModelName", "codexServiceTierFastSupportedForModel", "codexServiceTierFastAvailability", "isFastServiceTierValue"].map((name) => extract(menu, name)),
    ...["codexServiceTierOverrideResult", "codexServiceTierOverrideForRequest", "applyCodexServiceTierRequestOnly"].map((name) => extract(tier, name)),
  ].join("\n"), context);
  return { metadata, state, ui: (model: string) => context.codexServiceTierFastAvailability(model).supported,
    request: (model: string, serviceTier: string | null = null) => context.applyCodexServiceTierRequestOnly("turn/start", { threadId: "thread-12345678", model, service_tier: serviceTier }) };
}

test("Fast UI and request filtering share explicit declarations, including denial of built-in models", () => {
  const f = policyFixture();
  assert.equal(f.ui("grok-fast-without-a-declaration"), false);
  assert.equal(f.request("grok-fast-without-a-declaration").service_tier, null);
  f.metadata["grok-custom"] = { prioritySupportOverride: true };
  assert.equal(f.ui("grok-custom"), true); assert.equal(f.request("grok-custom").service_tier, "priority");
  f.metadata["gpt-5.4"] = { prioritySupportOverride: false, serviceTiers: [{ id: "priority" }] };
  assert.equal(f.ui("gpt-5.4"), false); assert.equal(f.request("gpt-5.4", "priority").service_tier, null);
  f.metadata["gpt-5.4"] = { serviceTiers: [] };
  assert.equal(f.ui("gpt-5.4"), true, "empty built-in metadata must preserve the old default");
  f.metadata["gpt-5.4"] = { prioritySupportOverride: false, serviceTiers: [{ id: "flex" }] };
  assert.equal(f.ui("gpt-5.4"), false, "inheriting original flex-only user metadata must remain consistent with its native catalog");
  f.state.mode = "global-standard";
  assert.equal(f.request("grok-custom", "priority").service_tier, null);
  f.state.mode = "inherit";
  assert.equal(f.request("grok-custom", "priority").service_tier, "priority");
  assert.equal(f.request("gpt-5.4", "priority").service_tier, null);
});

test("a responsive-only composer footer supplies the Fast badge placement without a provider button", () => {
  const source = fragment("90-action-groups.js");
  const footer = { children: [] as unknown[], parentElement: null, textContent: "", matches: (selector: string) => selector.includes("[data-composer-footer-responsive]"), querySelectorAll: () => [], getBoundingClientRect: () => ({ left: 100, top: 600, width: 600, height: 30 }) };
  const button = { visible: true };
  const group = { children: [button], textContent: "", firstChild: button };
  footer.children = [group];
  const composer = { querySelectorAll: (selector: string) => selector.includes("[data-composer-footer-responsive]") ? [footer] : [], matches: () => false };
  const context = vm.createContext({ document: { querySelectorAll: () => [footer] },
    codexServiceTierComposerFooterSelector: ".composer-footer, [data-composer-footer-responsive]",
    codexServiceTierBadgeVisibleElement: () => true, codexServiceTierKnownProviderNames: () => [],
    codexServiceTierBadgeAnchor: () => null, codexServiceTierBadgeText: () => "",
  });
  vm.runInContext(["codexServiceTierVisibleComposerFooters", "codexServiceTierComposerScore", "codexServiceTierBestComposerFooter", "codexServiceTierComposerFooter", "codexServiceTierBadgeFooterGroup", "codexServiceTierBadgePlacement"].map((name) => extract(source, name)).join("\n"), context);
  assert.equal(context.codexServiceTierBadgePlacement(composer).parent, group);
  const withoutFooter = { ...composer, querySelectorAll: () => [] };
  assert.equal(context.codexServiceTierBadgePlacement(withoutFooter), null, "another pane's footer is never the fallback");
});

test("Fast-only UI metadata leaves an existing native descriptor's name and reasoning default intact", () => {
  const metadata: Record<string, unknown> = {
    prioritySupportOverride: true,
    serviceTiers: [{ id: "priority", name: "Fast", description: "Provider-declared priority" }],
    supportedReasoningEfforts: [],
  };
  const context = vm.createContext({
    codexPlusModelMetadata: () => metadata,
    modelReasoningEfforts: () => { throw new Error("Fast does not configure reasoning choices"); },
  });
  vm.runInContext(extract(fragment("70-model-catalog.js"), "applyCodexPlusModelMetadata"), context);
  const descriptor = { displayName: "Native Grok", description: "Native details", defaultReasoningEffort: "high", supportedReasoningEfforts: [{ reasoningEffort: "high" }] };
  const original = structuredClone(descriptor);
  assert.equal(context.applyCodexPlusModelMetadata(descriptor, "grok-custom"), false);
  assert.deepEqual(descriptor, original);
  Object.assign(metadata, { displayName: "Explicit name", description: "Explicit details", defaultReasoningEffort: "low" });
  assert.equal(context.applyCodexPlusModelMetadata(descriptor, "grok-custom"), true);
  assert.equal(descriptor.displayName, "Explicit name");
  assert.equal(descriptor.description, "Explicit details");
  assert.equal(descriptor.defaultReasoningEffort, "low");
});
