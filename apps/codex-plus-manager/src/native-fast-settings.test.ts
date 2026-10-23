import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/10-style.js", import.meta.url), "utf8");
const settingsPage = readFileSync(new URL("../../../assets/inject/renderer-inject/40-backend-settings.js", import.meta.url), "utf8");
const serviceTier = readFileSync(new URL("../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
const extract = (name: string) => {
  const body = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0];
  assert.ok(body, name);
  return body;
};

test("native Fast renderer preference defaults off, follows backend settings and respects the master switch", () => {
  const mapping = source.match(/^  const codexPlusBackendSettingMap = \{[^]*?^  \};/m)?.[0];
  assert.ok(mapping);
  const backend: Record<string, boolean> = {};
  const stored: Record<string, boolean> = {};
  const context = vm.createContext({
    codexPlusBackendSettings: backend,
    codexPlusSettingsKey: "settings",
    conversationViewDefaultWidth: 960,
    window: {},
    localStorage: { getItem: () => JSON.stringify(stored) },
  });
  vm.runInContext([mapping, ...["defaultCodexPlusSettings", "backendCodexPlusSettings", "codexPlusSettings"].map(extract)].join("\n"), context);
  assert.equal(context.codexPlusSettings().nativeFastUnlock, false);
  backend.codexAppNativeFastUnlock = true;
  assert.equal(context.codexPlusSettings().nativeFastUnlock, true);
  assert.equal(context.codexPlusSettings().serviceTierControls, undefined);
  backend.codexAppNativeFastUnlock = false;
  stored.nativeFastUnlock = true;
  assert.equal(context.codexPlusSettings().nativeFastUnlock, false, "the backend value must override a stale browser preference");
  backend.codexAppNativeFastUnlock = true;
  backend.enhancementsEnabled = false;
  assert.equal(context.codexPlusSettings().nativeFastUnlock, false);
  backend.enhancementsEnabled = true;
  assert.equal(context.codexPlusSettings().nativeFastUnlock, true);
});

test("in-app settings expose native Fast unlock without the retired service tier badge", () => {
  assert.match(settingsPage, /data-codex-plus-setting="nativeFastUnlock"/);
  assert.match(settingsPage, /原生 Fast 强制解锁/);
  assert.match(settingsPage, /纯 API Key 模式恢复原生 Fast 开关；保存后重新启动 Codex 生效，实际支持与计费由供应商决定。/);
  assert.doesNotMatch(settingsPage, /data-codex-plus-setting="serviceTierControls"/);
});

test("backend loads and saves publish only native Fast policy flags and tolerate an absent helper", async () => {
  const extractAsync = (fragment: string, name: string) => {
    const body = fragment.match(new RegExp(`^  async function ${name}\\([^]*?^  \\}`, "m"))?.[0];
    assert.ok(body, name);
    return body;
  };
  for (const surface of ["absent-window", "absent-helper", "helper"]) {
    const publications: Array<Record<string, boolean>> = [];
    let response = { enhancementsEnabled: true, codexAppNativeFastUnlock: true, nativeFastLocalApiConfigured: true, privateField: "fixture-only" };
    const window = {
      __CODEX_PLUS_TEST_SERVICE_TIER__: true,
      ...(surface === "helper" ? { __codexPlusNativeFastUnlock: { runtime: { update: (value: Record<string, boolean>) => publications.push({ ...value }) } } } : {}),
    };
    const context = vm.createContext({
      window,
      codexPlusBackendSettings: { enhancementsEnabled: true },
      codexPlusBackendSettingsLoaded: false,
      codexPlusBackendSettingsSeq: 0,
      postJson: async () => response,
      refreshCodexPlusBackendToggles: () => {},
    });
    vm.runInContext([
      extractAsync(serviceTier, "loadBackendSettingsState"),
      extractAsync(settingsPage, "setBackendSetting"),
    ].join("\n"), context);
    assert.equal(await context.loadBackendSettingsState(), true);
    if (surface === "absent-window") delete context.window;
    await context.setBackendSetting("codexAppNativeFastUnlock", true);
    response = { ...response, enhancementsEnabled: false, codexAppNativeFastUnlock: false, nativeFastLocalApiConfigured: false };
    await context.setBackendSetting("codexAppNativeFastUnlock", false);
    assert.deepEqual(publications, surface === "helper" ? [
      { enhancementsEnabled: true, codexAppNativeFastUnlock: true, nativeFastLocalApiConfigured: true },
      { enhancementsEnabled: true, codexAppNativeFastUnlock: true, nativeFastLocalApiConfigured: true },
      { enhancementsEnabled: false, codexAppNativeFastUnlock: false, nativeFastLocalApiConfigured: false },
    ] : []);
  }
});
