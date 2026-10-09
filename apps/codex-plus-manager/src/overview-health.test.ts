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

test("overview shows the three live services and retires the latest launch panel", () => {
  const screen = functionSource("OverviewScreen");
  const items = functionSource("healthItems");
  assert.match(screen, /t\("Codex 版本"\)/);
  assert.match(items, /t\("Codex 应用"\)/);
  assert.doesNotMatch(screen + items, /management_shortcut|Codex\+\+ 应用入口|actions\.(checkHealth|repairShortcuts)/);
  for (const title of ["后台服务状态", "调试连接状态", "代理服务器状态"]) {
    assert.ok(items.includes(`t("${title}")`));
  }
  assert.match(items, /runtime_health/);
  assert.match(screen, /overview-health-grid/);
  assert.doesNotMatch(screen, /最近启动|LatestLaunch/);
  const entry = functionSource("App");
  assert.match(entry, /codexAppRunning \? actions\.restart\(\) : actions\.launch\(\)/);
  assert.match(entry, /runtime_health\?\.codex_app/);
  assert.match(entry, /codex-app-state/);
  assert.doesNotMatch(entry, /<span>Codex APP<\/span>/);
  assert.match(entry, /Codex \{statusLabel\(overview\?\.runtime_health\?\.codex_app\?\.status/);
  assert.match(entry, /aria-label=\{t\("刷新状态"\)\}/);
  assert.doesNotMatch(screen, /刷新状态|actions\.refreshCurrent/);
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  assert.match(css, /\.overview-health-grid\s*\{\s*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/);
});

test("maintenance retains application entrypoint checks and repairs", () => {
  const screen = functionSource("MaintenanceScreen");
  assert.match(screen, /management_shortcut/);
  assert.match(screen, /actions\.checkHealth\(\)/);
  assert.match(screen, /actions\.repairShortcuts\(\)/);
});
