import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { parseSkillRepository } from "./skill-repository.ts";

type Element = { type: any; props: Record<string, any>; children: any[] };
const source = readFileSync(new URL("./SkillRepositoryQuickAdd.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("SkillRepositoryQuickAdd.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const declaration = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillRepositoryQuickAdd")!;
const code = ts.transpileModule(declaration.getText(file), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

function harness() {
  const slots: any[] = [];
  const calls: any[] = [];
  const added: any[] = [];
  const busy: boolean[] = [];
  let cursor = 0;
  let status = "failed";
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useRef: (initial: any) => { const index = cursor++; if (!(index in slots)) slots[index] = { current: initial }; return slots[index]; },
    t: (value: string) => value, parseSkillRepository, Action: "Action", Plus: "Plus",
    invoke: async (command: string, args: any) => {
      calls.push({ command, args });
      return { status, message: status === "ok" ? "已验证" : "验证失败，未保存", repos: [args.repo] };
    },
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const existing = [{ owner: "owner", name: "existing", branch: "main", subdir: "", enabled: true }];
  const render = () => {
    cursor = 0;
    const elements: Element[] = [];
    const visit = (node: any) => {
      if (!node || typeof node !== "object") return;
      if (Array.isArray(node)) return node.forEach(visit);
      elements.push(node);
      node.children?.forEach(visit);
    };
    visit(context.exports.SkillRepositoryQuickAdd({ repos: existing, disabled: false, onBusyChange: (value: boolean) => busy.push(value), onAdded: (value: any) => added.push(value) }));
    return elements;
  };
  return { render, calls, added, busy, setStatus: (value: string) => { status = value; } };
}

test("quick add validates input and never writes an already configured repository", async () => {
  const ui = harness();
  let elements = ui.render();
  elements.find((node) => node.type === "input")!.props.onChange({ currentTarget: { value: "not a github url!" } });
  elements = ui.render();
  elements.find((node) => node.type === "Action")!.props.onClick();
  assert.equal(ui.calls.length, 0);
  assert.ok(ui.render().some((node) => node.props.role === "status" && node.children[0].includes("无法识别")));
  ui.render().find((node) => node.type === "input")!.props.onChange({ currentTarget: { value: "OWNER/existing" } });
  ui.render().find((node) => node.type === "Action")!.props.onClick();
  assert.equal(ui.calls.length, 0);
  assert.ok(ui.render().some((node) => node.props.role === "status" && node.children[0].includes("已添加")));
});

test("failed validation keeps the repository out; success reports refreshed repository list", async () => {
  const ui = harness();
  ui.render().find((node) => node.type === "input")!.props.onChange({ currentTarget: { value: "another-owner/repo" } });
  ui.render().find((node) => node.type === "Action")!.props.onClick();
  await tick();
  assert.equal(ui.calls[0].command, "add_skill_repo");
  assert.deepEqual(Object.keys(ui.calls[0].args), ["repo"]);
  assert.equal(ui.added.length, 0);
  assert.ok(ui.render().some((node) => node.props.role === "status" && node.children[0].includes("验证失败")));
  ui.setStatus("ok");
  ui.render().find((node) => node.type === "Action")!.props.onClick();
  await tick();
  assert.equal(ui.added.length, 1);
  assert.equal(ui.added[0][0].owner, "another-owner");
  assert.deepEqual(ui.busy, [true, false, true, false]);
});

test("branch and subdirectory can be corrected before validation", async () => {
  const ui = harness();
  ui.render().find((node) => node.type === "input" && node.props["aria-label"] === "GitHub 仓库地址")!
    .props.onChange({ currentTarget: { value: "https://github.com/owner/repo/tree/feature/team/skills" } });
  let elements = ui.render();
  assert.equal(elements.find((node) => node.type === "input" && node.props["aria-label"] === "分支")!.props.value, "feature");
  assert.equal(elements.find((node) => node.type === "input" && node.props["aria-label"] === "子目录")!.props.value, "team/skills");
  elements.find((node) => node.type === "input" && node.props["aria-label"] === "分支")!
    .props.onChange({ currentTarget: { value: "feature/team" } });
  elements = ui.render();
  elements.find((node) => node.type === "input" && node.props["aria-label"] === "子目录")!
    .props.onChange({ currentTarget: { value: "skills" } });
  ui.render().find((node) => node.type === "Action")!.props.onClick();
  await tick();
  assert.equal(ui.calls[0].args.repo.branch, "feature/team");
  assert.equal(ui.calls[0].args.repo.subdir, "skills");
});
