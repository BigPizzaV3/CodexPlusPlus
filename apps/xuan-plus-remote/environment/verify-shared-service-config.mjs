import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const environmentRoot = path.dirname(fileURLToPath(import.meta.url));
const remoteRoot = path.resolve(environmentRoot, "..");

function validateMobileSource(source, config) {
  const environment = source.match(
    /static readonly environment:\s*RemoteEnvironment\s*=\s*RemoteEnvironment\.([A-Z]+);/,
  )?.[1];
  const enabled = source.match(/static readonly enabled:\s*boolean\s*=\s*(true|false);/)?.[1];
  const apiBaseUrl = source.match(/static readonly apiBaseUrl:\s*string\s*=\s*'([^']*)';/)?.[1];
  const appWssUrl = source.match(
    /static readonly replyStreamWssBaseUrl:\s*string\s*=\s*\r?\n?\s*'([^']*)';/,
  )?.[1];
  assert.equal(environment?.toLowerCase(), config.protocolEnvironment, "手机与共享协议环境不一致");
  assert.equal(enabled, String(config.enabled), "手机与服务配置的启用状态不一致");
  assert.equal(apiBaseUrl, config.apiBaseUrl, "手机与 PC 的 HTTPS endpoint 不一致");
  assert.equal(appWssUrl, config.appWssUrl, "手机与共享配置的实时回复 WSS endpoint 不一致");
}

function validateProtocolFixtures(fixtures, config) {
  for (const name of ["validPairingRegistration", "validPairingConsume"]) {
    const fixture = fixtures[name];
    assert.equal(fixture?.environment, config.protocolEnvironment, `${name} 请求环境不一致`);
    assert.equal(fixture?.pairing?.environment, config.protocolEnvironment, `${name} 二维码环境不一致`);
  }
}

function validateConfig(config) {
  assert.equal(config.schemaVersion, "1.1", "共享远程服务配置版本不受支持");
  assert.equal(config.protocolEnvironment, "dev", "本副本只允许开发环境");
  assert.equal(typeof config.enabled, "boolean", "必须显式配置启用状态");
  if (!config.enabled) {
    for (const field of ["apiBaseUrl", "wssUrl", "appWssUrl"]) {
      assert.equal(config[field], "", "未启用时不得保留原服务地址");
    }
    return;
  }
  assert.match(config.apiBaseUrl, /^https:\/\/[A-Za-z0-9.-]+(?:\/[A-Za-z0-9._~!$&'()*+,;=:@%/-]*)?$/);
  assert.match(config.wssUrl, /^wss:\/\/[A-Za-z0-9.-]+(?:\/[A-Za-z0-9._~!$&'()*+,;=:@%/-]*)?$/);
  assert.match(config.appWssUrl, /^wss:\/\/[A-Za-z0-9.-]+(?:\/[A-Za-z0-9._~!$&'()*+,;=:@%/-]*)?$/);
  assert.ok(!config.apiBaseUrl.endsWith("/"), "HTTPS endpoint 不能以斜杠结尾");
  assert.ok(!config.wssUrl.endsWith("/"), "WSS endpoint 不能以斜杠结尾");
  assert.ok(!config.appWssUrl.endsWith("/"), "实时回复 WSS endpoint 不能以斜杠结尾");
}

const [configText, mobileSource, fixtureText] = await Promise.all([
  readFile(path.join(environmentRoot, "shared-remote-service.json"), "utf8"),
  readFile(
    path.join(
      remoteRoot,
      "app",
      "entry",
      "src",
      "main",
      "ets",
      "remote",
      "RemoteEnvironmentConfig.ets",
    ),
    "utf8",
  ),
  readFile(path.join(remoteRoot, "protocol", "contract-fixtures.json"), "utf8"),
]);

const config = JSON.parse(configText);
validateConfig(config);
validateMobileSource(mobileSource, config);
validateProtocolFixtures(JSON.parse(fixtureText), config);
assert.throws(() => validateConfig({ ...config, enabled: "false" }));
assert.throws(() => validateConfig({ ...config, enabled: false, apiBaseUrl: "https://example.invalid" }));
assert.throws(() => validateConfig({ ...config, enabled: true, apiBaseUrl: "http://example.invalid" }));
validateConfig({
  ...config,
  enabled: true,
  apiBaseUrl: "https://example.invalid/api",
  wssUrl: "wss://example.invalid/gateway",
  appWssUrl: "wss://example.invalid/tasks",
});

process.stdout.write("手机与本副本服务配置一致性检查通过；不代表桌面适配器已接入\n");
