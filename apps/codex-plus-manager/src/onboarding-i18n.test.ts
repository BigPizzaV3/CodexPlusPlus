import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { EN_BACKEND, EN_PLAIN, EN_TEMPLATE } from "./i18n-en.ts";
import { tourSteps } from "./onboarding-model.ts";

type Keys = { plain: Set<string>; template: Set<string> };
const chinese = /\p{Script=Han}/u;
const source = (name: string) => ts.createSourceFile(name, readFileSync(new URL(name, import.meta.url), "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const emptyKeys = (): Keys => ({ plain: new Set(), template: new Set() });

function collectLiterals(node: ts.Node, keys: Set<string>) {
  if (ts.isStringLiteralLike(node) && chinese.test(node.text)) keys.add(node.text);
  ts.forEachChild(node, (child) => collectLiterals(child, keys));
}

/** 条件表达式里的按钮状态也必须翻译，不能只扫描 t("字面量")。 */
function collectCalls(node: ts.Node, keys: Keys) {
  if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.arguments[0]) {
    if (node.expression.text === "t") collectLiterals(node.arguments[0], keys.plain);
    if (node.expression.text === "tf") collectLiterals(node.arguments[0], keys.template);
  }
  ts.forEachChild(node, (child) => collectCalls(child, keys));
}

function checkKeys(keys: Keys) {
  for (const [kind, values] of Object.entries(keys) as Array<[keyof Keys, Set<string>]>) {
    for (const key of values) {
      const english = kind === "template" ? EN_TEMPLATE[key] : EN_PLAIN[key] ?? EN_BACKEND[key];
      assert.ok(english, `${kind} 缺英文：${key}`);
      assert.ok(!chinese.test(english), `英文仍含中文：${key}`);
      assert.deepEqual([...english.matchAll(/\{\d+\}/g)].map(([value]) => value).sort(),
        [...key.matchAll(/\{\d+\}/g)].map(([value]) => value).sort(), `占位符不一致：${key}`);
    }
  }
}

test("模式问卷与入门清单的所有状态、按钮和错误均有英文", () => {
  const keys = emptyKeys();
  for (const file of ["./AgentFocusDialog.tsx", "./OnboardingSetup.tsx", "./OnboardingTour.tsx"]) {
    const tree = source(file);
    collectCalls(tree, keys);
    if (file === "./AgentFocusDialog.tsx") {
      const visit = (node: ts.Node) => {
        if (ts.isPropertyAssignment(node) && ["name", "hint"].includes(node.name.getText(tree))) {
          collectLiterals(node.initializer, keys.plain);
        }
        ts.forEachChild(node, visit);
      };
      visit(tree);
    }
  }
  assert.ok(keys.plain.has("重试保存"), "需要扫描嵌套条件分支中的失败状态");
  assert.ok(keys.plain.has("管理 Codex 的供应商、增强、皮肤和拓展。隐藏其他 Agent 的界面，已有网关连接保持不变。"), "需要扫描动态读取的模式说明");
  checkKeys(keys);
});

test("全部、单 Agent 和增强模式的实际导览步骤均有英文且保留占位符", () => {
  const keys = emptyKeys();
  for (const steps of [tourSteps(null), tourSteps("Codex"), tourSteps(null, true)]) {
    for (const step of steps) {
      (step.args && !step.plainTitle ? keys.template : keys.plain).add(step.title);
      (step.args ? keys.template : keys.plain).add(step.body);
    }
  }
  checkKeys(keys);
});

test("App 的模式影响说明、事务保存提示和重新打开引导说明均有英文", () => {
  const tree = source("./App.tsx");
  const keys = emptyKeys();
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === "agentFocusDescription") collectCalls(node, keys);
    if (ts.isVariableDeclaration(node) && node.name.getText(tree) === "closeAgentFocus" && node.initializer) collectCalls(node.initializer, keys);
    if (ts.isFunctionDeclaration(node) && node.name?.text === "GlobalSettingsScreen") {
      const settings = emptyKeys();
      collectCalls(node, settings);
      for (const key of settings.plain) if (/引导|导览/.test(key)) keys.plain.add(key);
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
  assert.ok(keys.plain.has("完成配置、验证连接并开始使用，也可以查看各个入口的导览。"));
  checkKeys(keys);
});
