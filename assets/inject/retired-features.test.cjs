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
      codexAppNativeMenuTranslation: true,
      codexAppNativeMenuPlacement: false, codexAppStepwiseEnabled: true,
    },
  });
  vm.runInContext(`${style.slice(begin, end)}\nglobalThis.result = codexPlusSettings();`, context);
  for (const key of ["zedRemoteOpen", "upstreamWorktreeCreate", "nativeMenuTranslation"]) {
    assert.equal(Object.hasOwn(context.result, key), false, key);
  }
  assert.equal(context.result.nativeMenuPlacement, false, "menu placement keeps its independent mapping");
  assert.equal(context.result.stepwise, true, "Stepwise remains available");
});

function installForceChinese(fixture) {
  const prelude = readFileSync(`${__dirname}/renderer-inject/00-prelude.js`, "utf8");
  const begin = prelude.indexOf("  function installCodexPlusForceChineseLocale()");
  const end = prelude.indexOf("  const helperBase =", begin);
  assert.ok(begin >= 0 && end > begin);
  vm.runInContext(
    `${prelude.slice(begin, end)}\nglobalThis.install = installCodexPlusForceChineseLocale;`,
    fixture.context,
  );
  fixture.context.install();
  return fixture;
}

test("renderer startup forces zh-CN through the native locale setting", async () => {
  const fixture = installForceChinese(forceChineseContext({ enabled: true }));
  const { context, calls } = fixture;
  assert.equal(context.navigator.language, "zh-CN");
  // languages 是沙箱 realm 里造的数组，摊平到本 realm 再比，否则 deepStrictEqual 会
  // 因为 Array.prototype 不同而判「结构相同但非同一引用」。
  assert.deepEqual([...context.navigator.languages], ["zh-CN", "zh", "en-US", "en"]);
  await fixture.settled();
  assert.equal(context.window.localStorage.getItem(fixture.managedKey),
    JSON.stringify({ appliedLocale: "zh-CN", previousValue: "en-US" }),
    "the native value it overwrites is remembered so it can be restored");
  assert.deepEqual(calls.map(call => [call.method, call.params.key, call.params.value]), [
    ["get-setting", "localeOverride", undefined],
    ["set-setting", "localeOverride", "zh-CN"],
  ]);
  // 菜单汉化仍未恢复，这两个入口都不该回来。
  assert.doesNotMatch(source, /codexMenuLocalizationMap|function localizeCodexMenus\(/);
});

test("force Chinese locale installs once per configuration", async () => {
  const fixture = installForceChinese(forceChineseContext({ enabled: true }));
  await fixture.settled();
  const afterFirst = fixture.calls.length;
  fixture.context.install();
  await fixture.settled();
  assert.equal(fixture.calls.length, afterFirst, "re-running with the same config does not resync");
  assert.equal(fixture.context.window.__codexPlusForceChineseLocaleInstalled, "2:on:zh-CN");
});

test("force Chinese locale leaves everything alone while it is switched off", async () => {
  const fixture = installForceChinese(forceChineseContext({ enabled: false }));
  const { context } = fixture;
  assert.equal(context.navigator.language, "en-US");
  assert.deepEqual(context.navigator.languages, ["en-US"]);
  await fixture.settled();
  assert.deepEqual(fixture.calls, [], "no native read or write without a managed marker");
  assert.deepEqual(fixture.writes, []);
});

test("force Chinese locale retries while the Codex state API is still unavailable", async () => {
  // 注入早于 App asset 就绪时，state API 的头几次调用必然失败；重试要把它救回来，
  // 否则第一次失败会被永久缓存的 promise 记住，语言包再也打不上。
  const fixture = installForceChinese(forceChineseContext({ enabled: true, failFirst: 3 }));
  await fixture.settled();
  await fixture.settled();
  const methods = fixture.calls.map(call => call.method);
  assert.deepEqual(methods, ["get-setting", "get-setting", "get-setting", "get-setting", "set-setting"],
    "three failed reads are retried before the write lands");
  assert.equal(fixture.current(), "zh-CN");
  assert.deepEqual(fixture.reloads, ["reload"]);
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

function forceChineseContext({ enabled = true, managed = null, current = "en-US", failFirst = 0 } = {}) {
  const managedKey = "codexPlus.forceChineseLocale.managed.v1";
  const local = new Map(managed ? [[managedKey, JSON.stringify(managed)]] : []);
  const session = new Map();
  const calls = [];
  const writes = [];
  const reloads = [];
  const context = vm.createContext({
    window: {
      __CODEX_PLUS_FORCE_CHINESE_LOCALE__: { enabled, locale: "zh-CN" },
      localStorage: {
        getItem: (key) => local.get(key) ?? null,
        setItem: (key, value) => { writes.push([key, value]); local.set(key, value); },
        removeItem: (key) => { writes.push([key, null]); local.delete(key); },
      },
      sessionStorage: {
        getItem: (key) => session.get(key) ?? null,
        setItem: (key, value) => session.set(key, value),
        removeItem: (key) => session.delete(key),
      },
      location: { reload: () => reloads.push("reload") },
      setInterval: () => 0,
      clearInterval: () => {},
      // 重试退避靠 window.setTimeout：夹具里立刻回调，否则用例会一直等真实计时。
      setTimeout: (callback) => { callback(); return 0; },
      clearTimeout: () => {},
    },
    navigator: { userAgent: "test", language: "en-US", languages: ["en-US"] },
    codexStateCall: async (method, { params }) => {
      calls.push({ method, params });
      // failFirst 用来模拟注入早于 asset 就绪：state API 头几次调用必然失败。
      if (calls.length <= failFirst) throw Error("Codex 状态 API 不可用 (test)");
      if (method === "get-setting") return { value: current };
      assert.equal(method, "set-setting");
      assert.equal(params.key, "localeOverride");
      current = params.value;
      return null;
    },
  });
  return {
    context, calls, writes, reloads, local, session, managedKey,
    current: () => current,
    // 安装函数里同步读设置是 fire-and-forget，等微任务清空后再断言。
    settled: () => new Promise((resolve) => setImmediate(resolve)),
  };
}

