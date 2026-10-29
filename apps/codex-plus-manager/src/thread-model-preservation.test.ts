import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

const source = readFileSync(new URL(
  "../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
const override = source.slice(0, source.indexOf("  function codexManagedSessionConnection("));

function runtime(mode = "pureApi", providerSyncEnabled = false) {
  return new Function("codexPlusBackendSettings", "codexModelCatalog", "sendCodexPlusDiagnostic",
    `${override}\nreturn applyCodexRemoteSessionProviderOverride;`)(
    { relayProfilesEnabled: true, activeRelayId: "relay", providerSyncEnabled,
      relayProfiles: [{ id: "relay", relayMode: mode, configContents: 'model_provider = "custom"' }] },
    {}, () => {});
}

describe("thread model restoration ownership", () => {
  it("leaves thread-only resumes intact across distinct saved model and reasoning choices", () => {
    const apply = runtime();
    for (const [threadId, model, effort] of [
      ["thread-a", "model-a", "high"], ["thread-b", "model-b", "ultra"],
      ["thread-c", "outside-current-catalog", "low"],
    ]) {
      const params: { threadId: string; model?: string; modelProvider?: string; reasoningEffort?: string } = { threadId };
      const patched = apply("thread/resume", params);
      assert.equal(patched, params);
      assert.equal(Object.hasOwn(patched, "modelProvider"), false);
      assert.deepEqual(params, { threadId });
      const saved = { model, reasoningEffort: effort };
      const native = patched.model || patched.modelProvider || patched.reasoningEffort
        ? { model: "global-default", reasoningEffort: "medium" } : saved;
      assert.equal(native.model, model);
      assert.equal(native.reasoningEffort, effort);
    }
  });

  it("preserves field-free and foreign-provider resumes", () => {
    const apply = runtime();
    for (const params of [
      { threadId: "thread", reasoningEffort: "high" },
      { threadId: "thread", model: "chosen", modelProvider: "other", reasoningEffort: "ultra" },
    ]) {
      assert.equal(apply("thread/resume", params), params);
    }
  });

  it("rewrites explicit openai providers with supplier sync enabled or disabled", () => {
    for (const sync of [false, true]) {
      const apply = runtime("pureApi", sync);
      for (const key of ["modelProvider", "model_provider"]) {
        const params = { threadId: "old-openai", [key]: "openai" };
        const snapshot = { ...params };
        const patched = apply("thread/resume", params);
        assert.equal(patched.modelProvider, "custom");
        assert.equal(Object.hasOwn(patched, "model_provider"), false);
        assert.equal(Object.hasOwn(patched, "model"), false);
        assert.deepEqual(params, snapshot);
      }
    }
  });

  it("treats model and provider fields by presence, including null and empty values", () => {
    const apply = runtime();
    for (const params of [
      { threadId: "thread", model: null },
      { threadId: "thread", model: "" },
      { threadId: "thread", modelProvider: null },
      { threadId: "thread", model_provider: "" },
    ]) {
      assert.equal(apply("thread/resume", params).modelProvider, "custom");
    }
  });

  it("retains provider routing for starts and explicit model switches", () => {
    const apply = runtime();
    const params = { threadId: "thread", model: "chosen", reasoningEffort: "high" };
    assert.deepEqual(apply("thread/resume", params), { ...params, modelProvider: "custom" });
    assert.equal(apply("thread/start", {}).modelProvider, "custom");
    assert.equal(apply("thread/resume", params).model, "chosen");
    assert.equal(runtime("official")("thread/resume", params), params);
  });
});
