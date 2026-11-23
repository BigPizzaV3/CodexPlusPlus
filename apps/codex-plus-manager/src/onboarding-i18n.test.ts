import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { readAppSource } from "./app-source.test-helper.ts";
import { EN_BACKEND, EN_PLAIN, EN_TEMPLATE } from "./i18n-en.ts";
import { TOUR_STEPS } from "./onboarding-model.ts";

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

test("启动引导与界面导览的所有状态、按钮和错误均有英文", () => {
  const keys = emptyKeys();
  for (const file of ["./OnboardingSetup.tsx", "./OnboardingTour.tsx"]) collectCalls(source(file), keys);
  assert.ok(keys.plain.has("测试没有通过"), "需要扫描嵌套条件分支中的失败状态");
  assert.ok(keys.template.has("当前供应商：{0} · {1}"), "需要扫描带占位符的状态");
  checkKeys(keys);
});

test("导览每一步的标题和正文均有英文", () => {
  const keys = emptyKeys();
  for (const step of TOUR_STEPS) {
    keys.plain.add(step.title);
    keys.plain.add(step.body);
  }
  checkKeys(keys);
});

test("设置页里重新打开引导、开始导览的说明均有英文", () => {
  const tree = ts.createSourceFile("App.tsx", readAppSource(), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const keys = emptyKeys();
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === "GlobalSettingsScreen") {
      const settings = emptyKeys();
      collectCalls(node, settings);
      // 标题、按钮和各自那一行的说明。
      for (const key of settings.plain) if (/引导|导览|重新走一遍|认识侧栏/.test(key)) keys.plain.add(key);
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
  for (const key of ["启动引导", "重新打开引导", "界面导览", "开始导览"]) assert.ok(keys.plain.has(key), key);
  checkKeys(keys);
});
