import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = readFileSync(new URL("./SkillFileTimes.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("SkillFileTimes.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const component = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillFileTimes");
assert.ok(component);
const code = ts.transpileModule(component.getText(file), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test("file dates are read only on expansion and identify every returned file", async () => {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const calls: any[] = [];
  let cursor = 0;
  let resolveFiles!: (value: any[]) => void;
  let rejectFiles!: (error: Error) => void;
  const context: Record<string, any> = {
    exports: {}, fragment: "fragment", jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    ChevronLeft: "ChevronLeft", ChevronRight: "ChevronRight", RefreshCw: "RefreshCw",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useRef: (value: any) => { const index = cursor++; return slots[index] ??= { current: value }; },
    useEffect: (run: () => void) => { const index = cursor++; if (!(index in slots)) { slots[index] = true; effects.push(run); } },
    t: (value: string) => value,
    invoke: (command: string, args: any) => {
      calls.push({ command, args });
      return new Promise((resolve, reject) => { resolveFiles = resolve; rejectFiles = reject; });
    },
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const render = () => {
    cursor = 0;
    const root = context.exports.SkillFileTimes({ id: "alpha", backupId: "alpha-123" });
    effects.splice(0).forEach((run) => run());
    return root;
  };
  let root = render();
  assert.equal(calls.length, 0);
  root.props.onToggle({ currentTarget: { open: true } });
  root.props.onToggle({ currentTarget: { open: true } });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "list_skill_file_times");
  assert.equal(calls[0].args.backupId, "alpha-123");
  rejectFiles(new Error("cannot read folder"));
  await tick();
  root = render();
  assert.ok(root.children.some((child: any) => child?.props?.role === "alert"));
  root.children.find((child: any) => child?.props?.["aria-label"] === "刷新").props.onClick();
  assert.equal(calls.length, 2);
  resolveFiles([{ path: "SKILL.md", modifiedAtMs: Date.parse("2026-11-10T00:00:00Z") },
    { path: "scripts/run.js", modifiedAtMs: Date.parse("2026-11-09T00:00:00Z"), isLink: true },
    ...Array.from({ length: 248 }, (_, index) => ({ path: `file-${index}`, modifiedAtMs: 0 }))]);
  await tick();
  root = render();
  let content = root.children.find((child: any) => child?.type === "fragment");
  let list = content.children.find((child: any) => child?.type === "ul");
  assert.equal(list.children[0].length, 100);
  assert.deepEqual(Array.from(list.children[0].slice(0, 2), (item: any) => item.children[0].children[0]), ["SKILL.md", "scripts/run.js"]);
  assert.deepEqual(Array.from(list.children[0].slice(0, 2), (item: any) => item.children[1].props.dateTime),
    ["2026-11-10T00:00:00.000Z", "2026-11-09T00:00:00.000Z"]);
  assert.equal(list.children[0][1].children[0].children[1].children[1], "链接（未展开）");
  let controls = content.children.find((child: any) => child?.type === "div");
  assert.equal(controls.children[0].props.disabled, true);
  controls.children[2].props.onClick();
  root = render();
  content = root.children.find((child: any) => child?.type === "fragment");
  list = content.children.find((child: any) => child?.type === "ul");
  assert.equal(list.children[0].length, 100);
  controls = content.children.find((child: any) => child?.type === "div");
  controls.children[2].props.onClick();
  root = render();
  content = root.children.find((child: any) => child?.type === "fragment");
  list = content.children.find((child: any) => child?.type === "ul");
  assert.equal(list.children[0].length, 50);
  controls = content.children.find((child: any) => child?.type === "div");
  assert.equal(controls.children[2].props.disabled, true);
  controls.children[0].props.onClick();
  root = render();
  root.props.onToggle({ currentTarget: { open: true } });
  assert.equal(calls.length, 2);
  root.children.find((child: any) => child?.props?.["aria-label"] === "刷新").props.onClick();
  assert.equal(calls.length, 3);
  resolveFiles([{ path: "updated.md", modifiedAtMs: 0 }]);
  await tick();
  root = render();
  content = root.children.find((child: any) => child?.type === "fragment");
  list = content.children.find((child: any) => child?.type === "ul");
  assert.equal(list.children[0][0].children[0].children[0], "updated.md");
});
