const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");
const source = readFileSync(`${__dirname}/renderer-inject.js`, "utf8");

test("retired project enhancements have no UI, dispatch or scanner entry", () => {
  assert.doesNotMatch(source, /data-codex-plus-setting="(?:zedRemoteOpen|upstreamWorktreeCreate)"/);
  assert.doesNotMatch(source, /data-codex-upstream-worktree-open/);
  assert.doesNotMatch(source, /(?:postJson|bridgeCall)\("\/(?:zed-remote|upstream-worktree)\//);
  assert.doesNotMatch(source, /function (?:installUpstream\w+|openUpstreamWorktreeDialog|refreshZedRemote\w+|scheduleZedRemote\w+)\(/);
  assert.doesNotMatch(source, /upstreamBranchDefaultsCache|codexUpstreamBranchSelection|codexUpstreamProjectContext/);

  // 已发布的类名只保留兼容声明，没有入口或事件处理。
  assert.match(source, /zedRemoteButtonClass = "codex-zed-remote-button"/);
  assert.match(source, /upstreamWorktreeDialogClass = "codex-upstream-worktree-dialog"/);
  assert.match(source, /function installCodexRemoteSessionRecoveryListener\(/);
  assert.match(source, /function installSessionShareButton\(/);
  assert.match(source, /function activateSessionCopyMenuItem\(/);
});

test("old backend switches cannot enable retired features", () => {
  const style = readFileSync(`${__dirname}/renderer-inject/10-style.js`, "utf8");
  const begin = style.indexOf("  function defaultCodexPlusSettings()");
  const end = style.indexOf("  // Dream skin runtime", begin);
  assert.ok(begin >= 0 && end > begin);
  const context = vm.createContext({
    window: {}, localStorage: { getItem: () => "{}" },
    conversationViewDefaultWidth: 900, codexPlusSettingsKey: "fixture",
    codexPlusBackendSettings: {
      enhancementsEnabled: true,
      codexAppZedRemoteOpen: true, codexAppUpstreamWorktreeCreate: true,
      codexAppForceChineseLocale: true, codexAppNativeMenuTranslation: true,
      codexAppNativeMenuPlacement: false, codexAppStepwiseEnabled: true,
    },
  });
  vm.runInContext(`${style.slice(begin, end)}\nglobalThis.result = codexPlusSettings();`, context);
  for (const key of ["zedRemoteOpen", "upstreamWorktreeCreate", "forceChineseLocale", "nativeMenuTranslation"]) {
    assert.equal(Object.hasOwn(context.result, key), false, key);
  }
  assert.equal(context.result.nativeMenuPlacement, false, "menu placement keeps its independent mapping");
  assert.equal(context.result.stepwise, true, "Stepwise remains available");
});

test("renderer startup leaves the native locale alone even with an old force flag", () => {
  const prelude = readFileSync(`${__dirname}/renderer-inject/00-prelude.js`, "utf8");
  const end = prelude.indexOf("  const helperBase =");
  assert.ok(end > 0);
  const storageWrites = [];
  const window = {
    __CODEX_PLUS_FORCE_CHINESE_LOCALE__: { enabled: true, locale: "zh-CN" },
    localStorage: {
      getItem: () => "en-US",
      setItem: (...args) => storageWrites.push(args),
      removeItem: (...args) => storageWrites.push(args),
    },
  };
  const navigator = { userAgent: "test", language: "en-US", languages: ["en-US"] };
  vm.runInNewContext(`${prelude.slice(0, end)}\n})();`, { window, navigator, process: { versions: { node: "fixture" } } });
  assert.equal(navigator.language, "en-US");
  assert.deepEqual(navigator.languages, ["en-US"]);
  assert.deepEqual(storageWrites, []);
  assert.doesNotMatch(source, /installCodexPlusForceChineseLocale|codexMenuLocalizationMap|function localizeCodexMenus\(/);
});

test("retired fast startup flags cannot change native fetch or Statsig initialization", () => {
  const prelude = readFileSync(`${__dirname}/renderer-inject/00-prelude.js`, "utf8");
  const end = prelude.indexOf("  const helperBase =");
  assert.ok(end > 0);
  const fetch = async () => "native response";
  const initializeAsync = async () => "native initialization";
  const unexpectedCalls = [];
  const client = {
    initializeAsync,
    loadingStatus: "Loading",
    $emt: (...args) => unexpectedCalls.push(args),
  };
  const window = {
    __CODEX_PLUS_FAST_STARTUP__: { enabled: true, statsigTimeoutMs: 100 },
    __STATSIG__: { firstInstance: client },
    fetch,
    setTimeout: (...args) => unexpectedCalls.push(args),
    setInterval: (...args) => unexpectedCalls.push(args),
  };
  vm.runInNewContext(`${prelude.slice(0, end)}\n})();`, {
    window,
    navigator: { userAgent: "test" },
    process: { versions: { node: "fixture" } },
  });
  assert.equal(window.fetch, fetch);
  assert.equal(client.initializeAsync, initializeAsync);
  assert.equal(client.loadingStatus, "Loading");
  assert.deepEqual(unexpectedCalls, []);
  assert.doesNotMatch(source, /installCodexPlusFastStartup|__CODEX_PLUS_FAST_STARTUP__|__codexPlusFastStartup|statsigTimeoutMs/);
  // 模型白名单仍依赖原生 Statsig 动态配置，与已撤下的启动超时无关。
  assert.match(source, /function patchStatsigModelDynamicConfig\(/);
  assert.match(source, /function patchStatsigModelWhitelist\(/);
  assert.match(source, /patchStatsigModelWhitelist\(\);/);
});

function managedLocaleFixture({ managed, current = "zh-CN", fail } = {}) {
  const managedKey = "codexPlus.forceChineseLocale.managed.v1";
  const reloadKey = "codexPlus.forceChineseLocale.reload.v1";
  const local = new Map(managed ? [[managedKey, JSON.stringify(managed)]] : []);
  const session = new Map([[reloadKey, "old-reload-marker"]]);
  const calls = [];
  const context = vm.createContext({
    window: {
      localStorage: { getItem: key => local.get(key) ?? null, removeItem: key => local.delete(key) },
      sessionStorage: { removeItem: key => session.delete(key) },
    },
    codexStateCall: async (method, { params }) => {
      calls.push({ method, params });
      if (fail === method) throw Error("native API temporarily unavailable");
      if (method === "get-setting") return { value: current };
      assert.equal(method, "set-setting");
      assert.equal(params.key, "localeOverride");
      current = params.value;
      return null;
    },
  });
  const prelude = readFileSync(`${__dirname}/renderer-inject/00-prelude.js`, "utf8");
  const begin = prelude.indexOf("  async function restoreCodexPlusManagedLocale()");
  const end = prelude.indexOf("  const helperBase =", begin);
  assert.ok(begin >= 0 && end > begin);
  vm.runInContext(`${prelude.slice(begin, end)}\nglobalThis.cleanup = restoreCodexPlusManagedLocale;`, context);
  return { cleanup: context.cleanup, local, session, calls, managedKey, reloadKey, current: () => current };
}

test("retired locale migration restores only its managed value once", async () => {
  for (const previousValue of ["en-US", null]) {
    const fixture = managedLocaleFixture({ managed: { appliedLocale: "zh-CN", previousValue } });
    assert.equal(await fixture.cleanup(), true);
    assert.equal(fixture.current(), previousValue);
    assert.deepEqual(fixture.calls.map(call => call.method), ["get-setting", "set-setting"]);
    assert.equal(fixture.local.has(fixture.managedKey), false);
    assert.equal(fixture.session.has(fixture.reloadKey), false);
    assert.equal(await fixture.cleanup(), true);
    assert.equal(fixture.calls.length, 2, "successful cleanup never writes the native setting again");
  }
});

test("retired locale migration preserves a language the user selected later", async () => {
  const fixture = managedLocaleFixture({ managed: { appliedLocale: "zh-CN", previousValue: "en-US" }, current: "ja-JP" });
  assert.equal(await fixture.cleanup(), true);
  assert.equal(fixture.current(), "ja-JP");
  assert.deepEqual(fixture.calls.map(call => call.method), ["get-setting"]);
  assert.equal(fixture.local.has(fixture.managedKey), false);
  assert.equal(fixture.session.has(fixture.reloadKey), false);
});

test("retired locale migration does not access native settings without a managed marker", async () => {
  const fixture = managedLocaleFixture();
  assert.equal(await fixture.cleanup(), true);
  assert.equal(fixture.calls.length, 0);
});

test("retired locale migration retains its marker when the native API fails", async () => {
  for (const fail of ["get-setting", "set-setting"]) {
    const fixture = managedLocaleFixture({ managed: { appliedLocale: "zh-CN", previousValue: "en-US" }, fail });
    assert.equal(await fixture.cleanup(), false);
    assert.equal(fixture.current(), "zh-CN");
    assert.equal(fixture.local.has(fixture.managedKey), true);
    assert.equal(fixture.session.has(fixture.reloadKey), true);
  }
  const invalid = managedLocaleFixture({ managed: { appliedLocale: "zh-CN", previousValue: "not a valid locale" } });
  assert.equal(await invalid.cleanup(), false);
  assert.deepEqual(invalid.calls.map(call => call.method), ["get-setting"]);
  assert.equal(invalid.local.has(invalid.managedKey), true, "invalid old data must not become a native setting");
});
