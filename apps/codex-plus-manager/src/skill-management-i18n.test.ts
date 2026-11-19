import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { EN_BACKEND, EN_BACKEND_PATTERNS, EN_PLAIN, EN_TEMPLATE } from "./i18n-en.ts";

test("Skills version management and sync messages have English translations", () => {
  const keys = { t: new Set<string>(), tf: new Set<string>() };
  const collect = (node: ts.Node, into: Set<string>) => {
    if (ts.isStringLiteralLike(node) && /\p{Script=Han}/u.test(node.text)) into.add(node.text);
    ts.forEachChild(node, child => collect(child, into));
  };
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.arguments[0]) {
      const kind = node.expression.text;
      if (kind === "t" || kind === "tf") collect(node.arguments[0], keys[kind]);
    }
    ts.forEachChild(node, visit);
  };
  for (const file of ["SkillVersionPicker.tsx", "ManagementExtra.tsx"]) {
    const tree = ts.createSourceFile(file, readFileSync(new URL(file, import.meta.url), "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    if (file === "SkillVersionPicker.tsx") visit(tree);
    else for (const node of tree.statements) {
      if (ts.isFunctionDeclaration(node) && ["SkillsPage", "SkillDetailDrawer", "SkillBackupList", "SkillBackupsDrawer", "sourceLabel"].includes(node.name?.text || "")) visit(node);
    }
  }
  const missing: string[] = [];
  for (const [kind, values] of Object.entries(keys)) for (const key of values) {
    const english = kind === "tf" ? EN_TEMPLATE[key] : EN_PLAIN[key] ?? EN_BACKEND[key];
    if (!english) { missing.push(key); continue; }
    assert.ok(!/\p{Script=Han}/u.test(english), `Untranslated: ${key}`);
    assert.deepEqual([...english.matchAll(/\{\d+\}/g)].map(([v]) => v).sort(), [...key.matchAll(/\{\d+\}/g)].map(([v]) => v).sort(), key);
  }
  assert.deepEqual(missing, []);
});

test("version comparison and export errors have English explanations", () => {
  for (const message of [
    "Skill 包含无法安全比较的符号链接：C:/skills/example/nested",
    "不能另存到受控源、备份或共享目录，请选择 Agent 安装目录或其他目录",
  ]) {
    const english = EN_BACKEND[message] ?? EN_PLAIN[message]
      ?? EN_BACKEND_PATTERNS.find(([pattern]) => pattern.test(message))?.[1];
    assert.ok(english && !/\p{Script=Han}/u.test(english), message);
  }
});
