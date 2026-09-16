import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { nativeBrowserStatusLabel } from "./native-browser-status.ts";

test("prepared and restored do not claim browser or extension acceptance", () => {
  assert.match(nativeBrowserStatusLabel("prepared"), /尚未验收/);
  assert.match(nativeBrowserStatusLabel("restored"), /扩展保留/);
  assert.match(nativeBrowserStatusLabel("stale"), /过期/);
  assert.match(nativeBrowserStatusLabel("new-state"), /不代表浏览器已可用/);
});

test("consent precedes enabling, default is off, and refresh is read-only", async () => {
  const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const component = await readFile(new URL("./native-browser-settings.tsx", import.meta.url), "utf8");
  assert.match(app, /codexAppNativeBrowserRequireIdentification: false/);
  assert.match(app, /if \(value && !window\.confirm\(nativeBrowserConsent\)\) return;/);
  assert.match(component, /x-browser-agent/);
  assert.match(component, /不会关闭扩展已保存的标识设置/);
  assert.match(component, /Edge \/ Chrome 稳定版扩展/);
  assert.match(app, /原生 Edge \/ Chrome 请求标识兼容（实验）/);
  assert.match(nativeBrowserStatusLabel("unsupported"), /Edge \/ Chrome/);
  assert.match(component, /"native_browser_status"/);
  assert.doesNotMatch(component, /"restart|"save_settings/);
});
