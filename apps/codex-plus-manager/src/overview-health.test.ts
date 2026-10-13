import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const text = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const app = ts.createSourceFile("App.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

function functionSource(name: string) {
  const declaration = app.statements.find(
    (statement): statement is ts.FunctionDeclaration => ts.isFunctionDeclaration(statement) && statement.name?.text === name,
  );
  assert.ok(declaration, `${name} should exist`);
  return declaration.getText(app);
}

test("overview merges the local server and protocol conversion and keeps the debugger", () => {
  const screen = functionSource("OverviewScreen");
  const items = functionSource("healthItems");
  assert.match(screen, /t\("Codex 版本"\)/);
  assert.match(items, /t\("Codex 应用"\)/);
  assert.doesNotMatch(screen + items, /management_shortcut|Codex\+\+ 应用入口|actions\.(checkHealth|repairShortcuts)/);
  for (const title of ["本地服务器状态", "调试连接状态"]) {
    assert.ok(items.includes(`t("${title}")`));
  }
  assert.match(items, /runtime_health/);
  assert.match(items, /协议转换已启用/);
  assert.match(items, /协议转换未启用/);
  assert.doesNotMatch(items, /后台服务状态|代理服务器状态|proxy_server/);
  assert.match(screen, /overview-health-grid/);
  assert.doesNotMatch(screen, /最近启动|LatestLaunch/);
  const entry = functionSource("App");
  assert.match(entry, /codexAppRunning \? actions\.restart\(\) : actions\.launch\(\)/);
  assert.match(entry, /runtime_health\?\.codex_app/);
  assert.match(entry, /codex-app-state/);
  assert.doesNotMatch(entry, /<span>Codex APP<\/span>/);
  assert.match(entry, /Codex \{statusLabel\(overview\?\.runtime_health\?\.codex_app\?\.status/);
  assert.doesNotMatch(entry, /aria-label=\{t\("刷新状态"\)\}/);
  assert.match(screen, /overview-health-header-actions/);
  assert.match(screen, /aria-label=\{t\("刷新状态"\)\}/);
  assert.match(screen, /actions\.refreshCurrent/);
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  assert.match(css, /\.overview-health-grid\s*\{\s*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/);
});

test("maintenance retains application entrypoint checks and repairs", () => {
  const screen = functionSource("MaintenanceScreen");
  assert.match(screen, /management_shortcut/);
  assert.match(screen, /actions\.checkHealth\(\)/);
  assert.match(screen, /actions\.repairShortcuts\(\)/);
});

test("maintenance no longer offers the removed entrypoint management panel", () => {
  const screen = functionSource("MaintenanceScreen");
  assert.doesNotMatch(screen, /入口管理|安装入口|卸载入口|修复入口|卸载时移除 Codex\+\+ 托管数据/);
  assert.doesNotMatch(screen, /actions\.installEntrypoints|actions\.uninstallEntrypoints|removeOwnedData/);
  const entry = functionSource("App");
  assert.doesNotMatch(entry, /install_entrypoints|uninstall_entrypoints|removeOwnedData|installEntrypoints|uninstallEntrypoints/);
});

test("maintenance drives login startup and auto-takeover with switches", () => {
  const screen = functionSource("MaintenanceScreen");
  assert.match(screen, /switch-row/);
  assert.match(screen, /actions\.setWatcherEnabled\(event\.currentTarget\.checked\)/);
  assert.match(screen, /actions\.setAutoTakeover\(event\.currentTarget\.checked\)/);
  // 开关取代了这两组按钮，别再退回去。
  assert.doesNotMatch(
    screen,
    /actions\.(installWatcher|uninstallWatcher|enableWatcher|disableWatcher)|设置开机启动|移除开机启动|关闭自动接管/,
  );
});

test("watcher switch command is wired through invoke, handler and backend", () => {
  const entry = functionSource("App");
  assert.match(entry, /call<WatcherResult>\("set_watcher_enabled", \{ enabled \}\)/);
  const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.match(lib, /commands::set_watcher_enabled,/);
  const commands = readFileSync(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");
  assert.match(commands, /pub fn set_watcher_enabled\(enabled: bool\)/);
});
