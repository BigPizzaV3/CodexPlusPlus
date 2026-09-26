import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { browserConnectionLabel, browserHeaderLabel, nativeBrowserStatusLabel } from "./native-browser-status.ts";

test("adapter refusal is distinct from browser availability and header state", () => {
  assert.match(nativeBrowserStatusLabel("runtime_unverified"), /未应用旧版兼容补丁/);
  assert.doesNotMatch(nativeBrowserStatusLabel("blocked"), /未启用兼容/);
  assert.match(nativeBrowserStatusLabel("blocked"), /文件/);
});

test("a connected Edge or Chrome is explicitly available regardless of patch or header state", () => {
  for (const headerEnabled of [true, false, null]) {
    assert.equal(browserConnectionLabel({ state: "available", failedChecks: 0,
      browsers: [{ family: "edge", headerEnabled }] }), "Edge 浏览器可用");
  }
  assert.equal(browserConnectionLabel({ state: "available", failedChecks: 1,
    browsers: [{ family: "chrome", headerEnabled: false }, { family: "edge", headerEnabled: true }] }),
  "Chrome / Edge 浏览器可用");
  assert.equal(browserConnectionLabel({ state: "available", failedChecks: 0, browsers: [] }), "浏览器状态检测失败");
});

test("disconnect, check failure, and header setting have distinct labels", () => {
  assert.equal(browserConnectionLabel({ state: "disconnected", failedChecks: 0, browsers: [] }), "浏览器未连接");
  assert.equal(browserConnectionLabel({ state: "check_failed", failedChecks: 1, browsers: [] }), "浏览器状态检测失败");
  assert.match(browserHeaderLabel(true), /已开启.*扩展报告/);
  assert.match(browserHeaderLabel(false), /已关闭.*扩展报告/);
  assert.match(browserHeaderLabel(null), /未提供/);
});

test("patch diagnostics are separate from live availability with bounded visible refresh", async () => {
  const component = await readFile(new URL("./native-browser-settings.tsx", import.meta.url), "utf8");
  assert.match(component, /browserConnectionLabel\(connection\)/);
  assert.match(component, /<details[^>]*>[\s\S]*nativeBrowserStatusLabel\(compatibility.state\)/);
  assert.match(component, /document.hidden/);
  assert.match(component, /15_000/);
  assert.match(component, /clearInterval/);
  assert.match(component, /if \(inFlight.current\) return/);
});

test("patch diagnostics describe only patch state, not browser availability", () => {
  assert.match(nativeBrowserStatusLabel("prepared"), /兼容服务文件/);
  assert.match(nativeBrowserStatusLabel("restored"), /扩展保留/);
  assert.match(nativeBrowserStatusLabel("stale"), /过期/);
  assert.equal(nativeBrowserStatusLabel("new-state"), "无法读取兼容补丁状态");
  for (const state of ["prepared", "blocked", "runtime_unverified", "new-state"]) {
    assert.doesNotMatch(nativeBrowserStatusLabel(state), /浏览器.*可用/);
  }
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
  assert.match(app, /此兼容补丁仅适配 Windows 上的 Edge \/ Chrome/);
  assert.match(nativeBrowserStatusLabel("unsupported"), /Edge \/ Chrome/);
  assert.match(component, /"native_browser_status"/);
  assert.doesNotMatch(component, /"restart|"save_settings/);
});
