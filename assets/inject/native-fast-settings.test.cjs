const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const extract = (file, name) => {
  const source = fs.readFileSync(require.resolve(`./renderer-inject/${file}`), "utf8");
  const body = source.match(new RegExp(`^  (?:async )?function ${name}\\([^]*?^  \\}`, "m"))?.[0];
  assert.ok(body, name);
  return body;
};

async function runNativeFastSettingsTests() {
  let response = {
    enhancementsEnabled: true,
    codexAppNativeFastUnlock: true,
    nativeFastLocalApiConfigured: true,
    relayProfiles: [{ configContents: "private backend fixture" }],
  };
  const context = vm.createContext({
    window: { __CODEX_PLUS_TEST_SERVICE_TIER__: true }, module: { exports: {} },
    codexPlusBackendSettings: {}, codexPlusBackendSettingsSeq: 0,
    codexPlusBackendSettingsLoaded: false,
    postJson: async (_route, changes) => { response = { ...response, ...changes }; return response; },
    refreshCodexPlusBackendToggles() {}, runScanStep() {}, installCodexPlusDictation() {},
  });
  vm.runInContext(fs.readFileSync(require.resolve("./native-fast-unlock.js"), "utf8"), context);
  const runtime = context.window.__codexPlusNativeFastUnlock.runtime;
  vm.runInContext([
    extract("30-service-tier.js", "loadBackendSettingsState"),
    extract("40-backend-settings.js", "setBackendSetting"),
  ].join("\n"), context);

  assert.equal(await context.loadBackendSettingsState(), true);
  assert.equal(runtime.allow("local", null), true, "首次加载保留后端的 API 配置投影");
  assert.deepEqual(Object.keys(context.window.__codexPlusSettings).sort(), [
    "codexAppNativeFastUnlock", "enhancementsEnabled", "nativeFastLocalApiConfigured",
  ]);
  assert.equal(JSON.stringify(context.window.__codexPlusSettings).includes("private backend fixture"), false);

  response.nativeFastLocalApiConfigured = false;
  assert.equal(await context.loadBackendSettingsState(), true);
  assert.equal(runtime.allow("local", null), false, "心跳应撤销已失效的本机配置投影");

  response.nativeFastLocalApiConfigured = true;
  await context.setBackendSetting("codexAppNativeFastUnlock", true);
  assert.equal(runtime.allow("local", null), true, "保存开关不能丢失配置投影");
  await context.setBackendSetting("codexAppNativeFastUnlock", false);
  assert.equal(runtime.allow("local", null), false);
  await context.setBackendSetting("codexAppNativeFastUnlock", true);
  await context.setBackendSetting("enhancementsEnabled", false);
  assert.equal(runtime.allow("local", null), false, "增强总开关关闭后立即停止放行");
}

module.exports = runNativeFastSettingsTests;
if (require.main === module) {
  runNativeFastSettingsTests().then(() => console.log("Native Fast settings refresh and safe projection passed")).catch(error => {
    console.error(error);
    process.exitCode = 1;
  });
}
