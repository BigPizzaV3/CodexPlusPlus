import assert from "node:assert/strict";
import test from "node:test";
import { hasCodexEnhancements, serializeSetupState, setupDirectState, shouldShowSetup, type SetupRelayProfile, type SetupRelaySettings } from "./onboarding-setup-model.ts";

test("仅启用拓展不能绕过增强总开关，独立额度解锁可单独使用", () => {
  const config = {
    settings: { enhancementsEnabled: false, codexAppStandaloneQuotaUnlock: false },
    user_scripts: { enabled: true, skip_next_start: false, scripts: [{ enabled: true, status: "not_loaded", error: "" }] },
  };
  assert.equal(hasCodexEnhancements(null), false);
  assert.equal(hasCodexEnhancements(config), false);
  config.user_scripts.enabled = false;
  assert.equal(hasCodexEnhancements(config), false);
  config.settings.codexAppStandaloneQuotaUnlock = true;
  assert.equal(hasCodexEnhancements(config), true);
  config.settings.codexAppStandaloneQuotaUnlock = false;
  config.settings.enhancementsEnabled = true;
  assert.equal(hasCodexEnhancements(config), true);
});

test("入门清单只自动面向新用户，完成与跳过都独立记忆，旧导览用户不强制重看", () => {
  assert.equal(shouldShowSetup(null, null), true);
  assert.equal(shouldShowSetup("broken", null), true);
  assert.equal(shouldShowSetup("null", null), true);
  assert.equal(shouldShowSetup('{"version":1,"state":"inProgress"}', null), true);
  for (const state of ["completed", "dismissed"] as const) {
    const stored = serializeSetupState(state);
    assert.deepEqual(JSON.parse(stored), { version: 1, state });
    assert.equal(shouldShowSetup(stored, null), false);
  }
  assert.equal(shouldShowSetup(null, "1"), false);
  assert.equal(shouldShowSetup(null, "0"), false);
  assert.equal(shouldShowSetup("broken", "garbage"), false);
});

function profile(overrides: Partial<SetupRelayProfile> = {}): SetupRelayProfile {
  return { id: "default", name: "默认中转", relayMode: "official", officialMixApiKey: false, baseUrl: "", apiKey: "", noAuth: false, ...overrides };
}
function settings(active: SetupRelayProfile, enabled = true): SetupRelaySettings {
  return { relayProfilesEnabled: enabled, activeRelayId: active.id, relayProfiles: [profile({ id: "other", name: "Other" }), active] };
}
const loggedIn = { authenticated: true, configured: false };
const loggedOut = { authenticated: false, configured: false };

test("直连状态区分未读到、配置切换关闭和没有供应商", () => {
  assert.equal(setupDirectState(null, loggedIn).issue, "loading");
  assert.equal(setupDirectState(settings(profile()), null).issue, "loading");
  assert.equal(setupDirectState(settings(profile(), false), loggedIn).issue, "switching_off");
  assert.equal(setupDirectState({ relayProfilesEnabled: true, activeRelayId: "default", relayProfiles: [] }, loggedIn).issue, "no_profile");
});

test("官方登录看 ChatGPT 登录，混入 API 还要 Base URL 和 Key，纯 API 看写入的配置", () => {
  const official = settings(profile());
  assert.equal(setupDirectState(official, loggedOut).issue, "not_logged_in");
  assert.equal(setupDirectState(official, loggedIn).ready, true);
  assert.equal(setupDirectState(official, loggedIn).profile?.id, "default", "看的是当前供应商，不是列表第一项");
  const mixed = profile({ relayMode: "mixedApi", baseUrl: "https://relay.test/v1" });
  assert.equal(setupDirectState(settings(mixed), loggedOut).issue, "not_logged_in");
  assert.equal(setupDirectState(settings(mixed), loggedIn).issue, "api_missing");
  assert.equal(setupDirectState(settings({ ...mixed, apiKey: "sk-test" }), loggedIn).ready, true);
  assert.equal(setupDirectState(settings(profile({ officialMixApiKey: true })), loggedIn).mode, "mixedApi");
  const pure = profile({ relayMode: "pureApi", baseUrl: "https://relay.test/v1", apiKey: "sk-test" });
  assert.equal(setupDirectState(settings(pure), loggedIn).issue, "not_configured", "ChatGPT 登录不能代替纯 API 的配置");
  assert.equal(setupDirectState(settings(pure), { authenticated: false, configured: true }).ready, true);
  assert.equal(setupDirectState(settings({ ...pure, apiKey: "", noAuth: true }), loggedOut).ready, true, "免鉴权供应商不要求 Key");
});

test("网关托管和聚合供应商按写入的配置判断，聚合供应商不做单条请求测试", () => {
  const gateway = setupDirectState(settings(profile({ id: "codex-plus-gateway", relayMode: "pureApi" })), { authenticated: false, configured: true });
  assert.equal(gateway.mode, "gateway");
  assert.equal(gateway.ready, true);
  assert.equal(gateway.testable, true);
  const aggregate = setupDirectState(settings(profile({ relayMode: "aggregate" })), { authenticated: false, configured: true });
  assert.equal(aggregate.ready, true);
  assert.equal(aggregate.testable, false);
});

test("配置标记随当前供应商、开关和登录状态变化，且不包含明文 Key", () => {
  const active = profile({ relayMode: "pureApi", baseUrl: "https://relay.test/v1", apiKey: "sk-secret-value" });
  const original = setupDirectState(settings(active), loggedIn).fingerprint;
  assert.ok(!original.includes("sk-secret-value"));
  assert.equal(setupDirectState(settings({ ...active }), { ...loggedIn }).fingerprint, original);
  for (const next of [
    setupDirectState(settings({ ...active, apiKey: "sk-other" }), loggedIn),
    setupDirectState(settings({ ...active, baseUrl: "https://changed.test" }), loggedIn),
    setupDirectState(settings(profile()), loggedIn),
    setupDirectState(settings(active, false), loggedIn),
    setupDirectState(settings(active), loggedOut),
  ]) assert.notEqual(next.fingerprint, original);
});
