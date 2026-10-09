import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const navigation = readFileSync(new URL("../../../assets/inject/renderer-inject/50-navigation.js", import.meta.url), "utf8");
const marketplace = readFileSync(new URL("../../../assets/inject/renderer-inject/60-plugin-marketplace.js", import.meta.url), "utf8");

function fixture() {
  let now = 1_000_000;
  const window: Record<string, unknown> = {};
  const context = vm.createContext({
    window, Date: { now: () => now },
    codexPluginUsesBroadCatalogKinds: () => true,
    restorePluginMarketplaceName: (name: string) => name,
    sendCodexPlusDiagnostic: () => {},
    pluginMarketplaceErrorText: (value: unknown) => String(value),
  });
  const requestsStart = navigation.indexOf("  const codexPluginRemoteOnlyMarketplaceKinds");
  const requestsEnd = navigation.indexOf("  function displayNameForPluginMarketplaceName", requestsStart);
  const recoveryStart = marketplace.indexOf("  const codexPluginRemoteAuthRetryMs");
  const recoveryEnd = marketplace.indexOf("  function pluginMarketplaceFallbackResult", recoveryStart);
  assert.ok(requestsStart >= 0 && requestsEnd > requestsStart && recoveryStart >= 0 && recoveryEnd > recoveryStart);
  vm.runInContext(`${navigation.slice(requestsStart, requestsEnd)}\n${marketplace.slice(recoveryStart, recoveryEnd)}`, context);
  return {
    window, context, advance: (ms: number) => { now += ms; },
    patch: (params: Record<string, unknown> = {}) => context.patchPluginMarketplaceRequestParams("list-plugins", params),
    reject: () => context.markPluginMarketplaceRemoteCatalogUnavailable("fixture API auth rejection"),
  };
}

test("remote authentication fallback expires without a background polling loop", () => {
  const f = fixture();
  f.reject();
  assert.deepEqual(Array.from(f.patch().marketplaceKinds), ["local", "vertical"]);
  f.advance(59_999);
  assert.deepEqual(Array.from(f.patch().marketplaceKinds), ["local", "vertical"]);
  f.advance(1);
  assert.equal("marketplaceKinds" in f.patch(), false, "native broad request can retry the current account");
  assert.equal(f.window.__codexPluginMarketplaceRemoteCatalogUnavailable, undefined);
});

test("an explicit refetch retries native catalogs while remote-only scope stays remote-only", () => {
  const f = fixture();
  f.reject();
  assert.deepEqual(Array.from(f.patch({ marketplaceKinds: ["shared-with-me"] }).marketplaceKinds), ["shared-with-me"]);
  const refreshed = f.patch({ forceRefetch: true });
  assert.equal(refreshed.forceRefetch, true);
  assert.equal("marketplaceKinds" in refreshed, false);
  assert.equal(f.window.__codexPluginMarketplaceRemoteCatalogUnavailable, undefined);
});

test("legacy flags and a clock correction cannot suppress remote catalogs indefinitely", () => {
  const f = fixture();
  f.window.__codexPluginMarketplaceRemoteCatalogUnavailable = true;
  assert.deepEqual(Array.from(f.patch().marketplaceKinds), ["local", "vertical"]);
  f.advance(60_000);
  assert.equal("marketplaceKinds" in f.patch(), false);
  f.reject();
  f.advance(-1);
  assert.equal("marketplaceKinds" in f.patch(), false);
});
