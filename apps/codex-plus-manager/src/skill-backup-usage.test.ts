import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = readFileSync(new URL("./SkillBackupUsage.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("SkillBackupUsage.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const statements = file.statements.filter((node) =>
  ts.isFunctionDeclaration(node) && ["requestBackupUsage", "SkillBackupUsage"].includes(node.name?.text || "")
  || ts.isVariableStatement(node) && node.declarationList.declarations.some((item) => ["cachedUsage", "pendingUsage"].includes(item.name.getText(file))));
const code = ts.transpileModule(statements.map((node) => node.getText(file)).join("\n"), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

function content(node: any): string {
  if (Array.isArray(node)) return node.map(content).join("");
  if (!node || typeof node === "boolean") return "";
  return typeof node === "object" ? content(node.children) : String(node);
}

function find(node: any, predicate: (item: any) => boolean): any {
  if (Array.isArray(node)) return node.map((item) => find(item, predicate)).find(Boolean);
  if (!node || typeof node !== "object") return undefined;
  return predicate(node) ? node : find(node.children, predicate);
}

test("backup usage scans on opening and manual refresh, and labels incomplete results", async () => {
  let slots: any[] = [];
  let effects: Array<() => void> = [];
  let cleanups: Array<() => void> = [];
  let cursor = 0;
  const calls: string[] = [];
  let now = 1_000;
  let resolveUsage!: (value: any) => void;
  let rejectUsage!: (error: Error) => void;
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    RefreshCw: "RefreshCw",
    Date: class extends Date { static now() { return now; } },
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useRef: (value: any) => { const index = cursor++; return slots[index] ??= { current: value }; },
    useEffect: (run: () => () => void) => {
      const index = cursor++;
      if (!(index in slots)) {
        slots[index] = true;
        effects.push(() => { cleanups.push(run()); });
      }
    },
    t: (value: string) => value,
    tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    invoke: (command: string) => {
      calls.push(command);
      return new Promise((resolve, reject) => { resolveUsage = resolve; rejectUsage = reject; });
    },
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const render = (backupRevision = "first") => {
    cursor = 0;
    const root = context.exports.SkillBackupUsage({ backupRevision });
    effects.splice(0).forEach((run) => run());
    return root;
  };
  render();
  assert.deepEqual(calls, ["skill_backup_usage"]);
  let root = render();
  assert.ok(content(root).includes("正在统计备份"));
  assert.equal(find(root, (item) => item.props["aria-label"] === "刷新备份统计").props.disabled, true);

  // Reopening while the first scan is still running reuses it.
  cleanups.splice(0).forEach((cleanup) => cleanup());
  slots = [];
  effects = [];
  render();
  assert.equal(calls.length, 1);
  resolveUsage({ backupCount: 2, measuredCount: 1, bytes: 2048, errors: ["C:/missing: not found"] });
  await tick();
  root = render();
  assert.ok(content(root).includes("备份数量：2"));
  assert.ok(content(root).includes("已统计 1/2 份，文件大小：2 KiB"));
  assert.ok(content(root).includes("C:/missing: not found"));

  // A fresh cached result is displayed when reopening, without another walk.
  cleanups.splice(0).forEach((cleanup) => cleanup());
  slots = [];
  effects = [];
  root = render();
  assert.equal(calls.length, 1);
  assert.ok(content(root).includes("备份数量：2"));

  root = render("second");
  assert.ok(content(root).includes("备份列表已变化，请刷新统计"));
  assert.equal(calls.length, 1);
  find(root, (item) => item.props["aria-label"] === "刷新备份统计").props.onClick();
  assert.equal(calls.length, 2);
  cleanups.splice(0).forEach((cleanup) => cleanup());
  slots = [];
  effects = [];
  render("second");
  assert.equal(calls.length, 2);
  assert.ok(content(render("second")).includes("正在统计备份"));
  rejectUsage(new Error("cannot inspect backups"));
  await tick();
  root = render("second");
  assert.ok(content(root).includes("cannot inspect backups"));
  assert.ok(content(root).includes("上次统计"));
  find(root, (item) => item.props["aria-label"] === "刷新备份统计").props.onClick();
  resolveUsage({ backupCount: 1, measuredCount: 1, bytes: 3, errors: [] });
  await tick();
  root = render("second");
  assert.ok(content(root).includes("备份数量：1"));
  assert.ok(content(root).includes("文件大小：3 B"));
  assert.ok(!content(root).includes("备份列表已变化"));

  now += 60_000;
  cleanups.splice(0).forEach((cleanup) => cleanup());
  slots = [];
  effects = [];
  root = render("second");
  assert.equal(calls.length, 4);
  assert.ok(content(root).includes("文件大小：3 B"));
  root = render("second");
  assert.ok(content(root).includes("正在统计备份"));
  resolveUsage({ backupCount: 1, measuredCount: 1, bytes: 4, errors: [] });
  await tick();
  assert.ok(content(render("second")).includes("文件大小：4 B"));
});

test("clearing a missing backup sends missing-only intent after confirmation", async () => {
  const managementSource = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
  const managementFile = ts.createSourceFile("ManagementExtra.tsx", managementSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let initializer: ts.Expression | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(managementFile) === "deleteBackup") initializer = node.initializer;
    ts.forEachChild(node, visit);
  };
  visit(managementFile);
  assert.ok(initializer);
  const compiled = ts.transpileModule(`const deleteBackup = ${initializer.getText(managementFile)};\nexports.deleteBackup = deleteBackup;`, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const confirmations: any[] = [];
  const calls: any[] = [];
  const context: Record<string, any> = {
    exports: {}, t: (value: string) => value,
    tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    appConfirm: async (message: string, options: any) => { confirmations.push({ message, options }); return true; },
    run: async (command: string, args: any) => { calls.push({ command, args }); },
  };
  vm.createContext(context);
  vm.runInContext(compiled, context);
  await context.exports.deleteBackup({ id: "alpha-1", name: "alpha", missing: true });
  assert.ok(confirmations[0].message.includes("此操作不会删除其他文件"));
  assert.equal(confirmations[0].options.confirmText, "清除记录");
  assert.equal(calls[0].command, "delete_skill_backup");
  assert.equal(calls[0].args.missingOnly, true);
  await context.exports.deleteBackup({ id: "alpha-2", name: "alpha" });
  assert.equal(calls[1].args.missingOnly, false);
});

test("a missing backup can be cleared but cannot be restored", () => {
  const managementSource = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
  const managementFile = ts.createSourceFile("ManagementExtra.tsx", managementSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const component = managementFile.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillBackupList");
  assert.ok(component);
  const compiled = ts.transpileModule(`${component.getText(managementFile)}\nexports.render = SkillBackupList;`, {
    compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const context: Record<string, any> = {
    exports: {}, fragment: "fragment", jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    Button: "Button", Trash2: "Trash2", t: (value: string) => value,
  };
  vm.createContext(context);
  vm.runInContext(compiled, context);
  const root = context.exports.render({
    backups: [{ id: "alpha-1", skillId: "alpha", name: "alpha", backedUpAt: "1", missing: true }],
    busy: false, onRestore: () => {}, onDelete: () => {},
  });
  assert.ok(content(root).includes("备份文件已丢失"));
  assert.equal(find(root, (item) => item.type === "Button").props.disabled, true);
  assert.ok(find(root, (item) => item.props["aria-label"] === "清除记录 alpha"));
});
