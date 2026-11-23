import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

/** 找到 `marker` 所在的那个 <ExtensionManagerPanel … /> 标签。 */
function panelTag(source: string, marker: string) {
  const at = source.indexOf(marker);
  assert.ok(at >= 0, marker);
  const start = source.lastIndexOf("<ExtensionManagerPanel", at);
  const end = source.indexOf("/>", at);
  assert.ok(start >= 0 && end > at, `${marker} 不在面板标签里`);
  return source.slice(start, end);
}

test("本地拓展行里展开的面板和整页打开的一样：能读清单声明的数据、能跳转", () => {
  // 只声明 manager.page（没有 nav、replaces）的拓展只有行菜单「打开面板」这一个入口，
  // 这里不传 dataSources / onNavigate 的话，data() 会报「清单没有声明」、navigate() 永远失败。
  const inline = panelTag(read("./ExtensionsPage.tsx"), "actions.showExtensionNotice(managerPage.title || displayName, message)");
  assert.match(inline, /dataSources=\{managerPage\.data \?\? \[\]\}/);
  assert.match(inline, /onNavigate=\{actions\.navigateFromExtension\}/);
  const app = read("./App.tsx");
  const fullPage = panelTag(app, "actions.showExtensionNotice(page.title, message)");
  assert.match(fullPage, /dataSources=\{page\.data\}/);
  assert.match(fullPage, /onNavigate=\{navigateFromExtension\}/);
  // navigateFromExtension 定义在 actions 之后：登记进 actions 时包一层，调用时才取，否则渲染时就报未初始化。
  assert.match(app, /navigateFromExtension: \(target: string\) => navigateFromExtension\(target\),/);
  assert.ok(app.indexOf("navigateFromExtension: (target: string) =>") < app.indexOf("const navigateFromExtension = "));
});

test("拓展面板保存配置只覆盖给出的项；设置表单仍整份提交", () => {
  const page = read("./ExtensionsPage.tsx");
  const app = read("./App.tsx");
  // 三个面板入口（行内面板、拓展自己的设置页、整页）都带 merge。
  assert.match(panelTag(page, "actions.showExtensionNotice(managerPage.title || displayName, message)"), /setExtensionConfig\(script\.key, values, \{ merge: true \}\)/);
  assert.match(panelTag(page, 'kind="settings"\n        scriptKey={script.key}'), /setExtensionConfig\(script\.key, values, \{ silent: true, merge: true \}\)/);
  assert.match(panelTag(app, "actions.showExtensionNotice(page.title, message)"), /setExtensionConfig\(page\.key, values, \{ merge: true \}\)/);
  // 通用设置表单提交的是整份配置，不带 merge。
  assert.match(page, /if \(await actions\.setExtensionConfig\(script\.key, values\)\) setConfigOpen\(false\);/);
  assert.match(app, /call<SettingsResult>\("set_extension_config", \{ key, values, merge: options\.merge === true \}\)/);
  // 后端：merge 走 update_extension_config（只覆盖给出的键），否则整份替换。
  const command = read("../src-tauri/src/commands.rs");
  const body = command.slice(command.indexOf("pub fn set_extension_config("), command.indexOf("pub async fn update_extension_config_live("));
  assert.match(body, /merge: Option<bool>/);
  assert.match(body, /if merge\.unwrap_or\(false\) \{\s+manager\.update_extension_config\(trimmed, &values\)\s+\} else \{\s+manager\.set_extension_config\(trimmed, &values\)/);
  // Codex 页内：拓展自己的设置面板拿到的 saveConfig 带 merge，通用表单逐项保存不带。
  const settings = read("../../../assets/inject/renderer-inject/40-backend-settings.js");
  assert.match(settings, /const saveConfig = \(values\) => saveCodexPlusExtensionConfig\(key, values, true\);/);
  assert.match(read("../../../assets/inject/renderer-inject/41-extension-detail.js"), /await saveCodexPlusExtensionConfig\(key, next\);/);
  const routes = read("../../../crates/codex-plus-core/src/routes.rs");
  assert.match(routes, /payload\.get\("merge"\)\.and_then\(Value::as_bool\)\.unwrap_or\(false\)/);
  assert.match(routes, /update_extension_config\(&key, &values\)/);
});
