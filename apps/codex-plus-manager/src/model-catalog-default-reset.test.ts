import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import vm from "node:vm";

// 网关模式下模型选择器「重置为默认」回归（#3058）：
// 原生重置读 defaultModelSlug 和 versionOptions 里 id=latest 的版本选项，
// patchModelDefaults 只补 defaultModel/default_model 时，重置会落到
// 网关目录之外的模型（如 gpt-5.6-sol）。
const source = readFileSync(
  new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");

function runtime(catalog: Record<string, unknown>) {
  const slice = (start: string, end: string) =>
    source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)));
  const body = [
    slice("  function codexPlusModelDescriptor(", "  function modelArrayLooksPatchable("),
    slice("  function modelFilterPolicy(", "  function sameModelCollection("),
    slice("  function patchModelDefaults(", "  // 只收窄当前纯 API 供应商的有效目录"),
    "return patchModelDefaults;",
  ].join("\n");
  return new Function("codexModelCatalog", "codexModelCatalogActiveProfile",
    "codexPlusModelMetadata", "modelReasoningEfforts", "codexPlusModelDefaultSnapshots", body)(
    catalog, () => ({ id: "gw" }), () => null, () => ["medium"], new Map());
}

const allowed = new Set(["provider-a/model-a", "provider-a/model-b"]);

describe("gateway model default patch (#3058)", () => {
  it("rewrites defaultModelSlug and versionOptions defaults to the gateway fallback", () => {
    const patch = runtime({ model: "provider-a/model-a" });
    const latest = {
      id: "latest", defaultModelSlug: "gpt-5.6-sol",
      options: [{ model: "gpt-5.6-sol", slug: "gpt-5.6-sol" }, { model: "provider-a/model-b", slug: "provider-a/model-b" }],
    };
    const value = { defaultModel: "provider-a/model-a", defaultModelSlug: "gpt-5.6-sol", versionOptions: [latest] };
    assert.equal(patch(value, allowed), true);
    assert.equal(value.defaultModelSlug, "provider-a/model-a");
    assert.equal(value.defaultModel, "provider-a/model-a", "allowed default stays untouched");
    assert.equal(value.versionOptions[0].defaultModelSlug, "provider-a/model-a");
    assert.deepEqual(value.versionOptions[0].options.map((option: any) => option.model),
      ["provider-a/model-a", "provider-a/model-b"]);
    assert.equal(value.versionOptions[0].options[0].slug, "provider-a/model-a");
  });

  it("keeps snake_case slug fields and leaves untouched versions referenced as-is", () => {
    const patch = runtime({ default_model: "provider-a/model-a" });
    const version = { id: "latest", default_model_slug: "gpt-5.6-sol", options: [] };
    const value = { default_model_slug: "gpt-5.6-sol", versionOptions: [version] };
    assert.equal(patch(value, allowed), true);
    assert.equal(value.default_model_slug, "provider-a/model-a");
    assert.equal(value.versionOptions[0].default_model_slug, "provider-a/model-a");
  });

  it("restores the original fields when the gateway allowlist goes away", () => {
    const patch = runtime({ model: "provider-a/model-a" });
    const latest = { id: "latest", defaultModelSlug: "gpt-5.6-sol", options: [{ model: "gpt-5.6-sol" }] };
    const value = { defaultModelSlug: "gpt-5.6-sol", versionOptions: [latest] };
    patch(value, allowed);
    const originalOptions = latest.options;
    assert.equal(patch(value, null), true);
    assert.equal(value.defaultModelSlug, "gpt-5.6-sol");
    assert.equal(value.versionOptions[0], latest, "the original version array comes back");
    assert.equal(value.versionOptions[0].options, originalOptions);
  });

  it("does nothing when every default already belongs to the gateway catalog", () => {
    const patch = runtime({ model: "provider-a/model-a" });
    const value = {
      defaultModel: "provider-a/model-a", defaultModelSlug: "provider-a/model-a",
      versionOptions: [{ id: "latest", defaultModelSlug: "provider-a/model-a", options: [{ model: "provider-a/model-a" }] }],
    };
    assert.equal(patch(value, allowed), false);
  });
});
