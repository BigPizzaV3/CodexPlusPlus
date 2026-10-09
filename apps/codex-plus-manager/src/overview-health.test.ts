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

test("overview health keeps Codex checks without the retired application entrypoint and buttons", () => {
  const screen = functionSource("OverviewScreen");
  const items = functionSource("healthItems");
  assert.match(screen, /t\("Codex 版本"\)/);
  assert.match(items, /t\("Codex 应用"\)/);
  assert.doesNotMatch(screen + items, /management_shortcut|Codex\+\+ 应用入口|actions\.(checkHealth|repairShortcuts)/);
  assert.match(screen, /t\("最近启动"\)/);
  assert.match(screen, /actions\.launch\(\)/);
});

test("maintenance retains application entrypoint checks and repairs", () => {
  const screen = functionSource("MaintenanceScreen");
  assert.match(screen, /management_shortcut/);
  assert.match(screen, /actions\.checkHealth\(\)/);
  assert.match(screen, /actions\.repairShortcuts\(\)/);
});
