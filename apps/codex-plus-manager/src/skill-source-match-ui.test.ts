import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

type Element = { type: any; props: Record<string, any>; children: any[] };

const source = readFileSync(new URL("./SkillSourceMatchDrawer.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("SkillSourceMatchDrawer.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const nodes = file.statements.filter((node) =>
  ts.isFunctionDeclaration(node) && node.name?.text === "SkillSourceMatchDrawer"
  || ts.isVariableStatement(node) && node.declarationList.declarations.some((item) =>
    ts.isIdentifier(item.name) && ["keyOfRepo", "keyOfCandidate", "evidenceLabel"].includes(item.name.text)));
const code = ts.transpileModule(nodes.map((node) => node.getText(file)).join("\n"), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test("batch discovery exposes local evidence and submits only preview-bound choices", async () => {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const calls: Array<{ command: string; args: any }> = [];
  const selections: any[] = [];
  let cursor = 0;
  let elements: Element[] = [];
  let closed = false;
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useEffect: (run: () => void, deps: any[]) => {
      const index = cursor++;
      const previous = slots[index] as any[] | undefined;
      if (!previous || deps.some((value, position) => !Object.is(value, previous[position]))) effects.push(run);
      slots[index] = deps;
    },
    useMemo: (run: () => any) => run(),
    useRef: (value: any) => { const index = cursor++; return slots[index] ??= { current: value }; },
    t: (value: string) => value,
    tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    invoke: async (command: string, args: any) => {
      calls.push({ command, args });
      if (command === "discover_skill_source_evidence") return {
        status: "ok", message: "", evidence: args.id === "alpha"
          ? [{ repo: "owner/alpha", kind: "lock", path: "skills-lock.json" }]
          : [{ repo: "owner/beta", kind: "script-link", path: "start.ps1" }],
      };
      assert.equal(command, "find_skill_repository_matches");
      return { status: "ok", message: "", previewId: "preview-one", warnings: [], manualCandidates: [],
        matches: [{ localId: "alpha", localName: "Alpha", repoKey: "owner/alpha@main",
          repoPath: "skills/alpha", remoteName: "Alpha", exact: true, localHash: "hidden-local",
          snapshot: "hidden-snapshot", remoteHash: "hidden-remote" }] };
    },
    AppSelect: "AppSelect", ManagementDrawer: "ManagementDrawer", Plus: "Plus", RefreshCw: "RefreshCw", Search: "Search",
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    elements.push(node);
    node.children?.forEach(collect);
    if (node.props?.footer) collect(node.props.footer);
  };
  const props = {
    repos: [{ owner: "owner", name: "alpha", branch: "main", subdir: "", enabled: true }],
    localSkillIds: ["alpha", "beta", "alpha"],
    onClose: () => { closed = true; },
    onAddRepo: async () => null,
    onApply: async (items: any[]) => { selections.push(...items); return { status: "ok", message: "" }; },
  };
  const render = () => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.exports.SkillSourceMatchDrawer(props));
    effects.splice(0).forEach((run) => run());
  };
  render();
  await tick();
  await tick();
  render();
  assert.deepEqual(calls.filter((call) => call.command === "discover_skill_source_evidence").map((call) => call.args.id),
    ["alpha", "beta"]);
  assert.ok(elements.some((node) => node.type === "small" && node.children.includes("alpha · 锁文件")));
  assert.ok(elements.some((node) => node.type === "small" && node.children.includes("beta · 脚本中的链接")));

  const checkbox = elements.find((node) => node.type === "input" && node.props.type === "checkbox");
  assert.ok(checkbox);
  checkbox.props.onChange({ currentTarget: { checked: true } });
  render();
  const confirm = elements.find((node) => node.type === "button" && node.children.includes("关联所选 1 项"));
  assert.ok(confirm);
  assert.equal(confirm.props.disabled, false);
  await confirm.props.onClick();
  await tick();
  assert.deepEqual(JSON.parse(JSON.stringify(selections)), [{
    previewId: "preview-one", localId: "alpha", repoKey: "owner/alpha@main", repoPath: "skills/alpha",
  }]);
  assert.equal(closed, true);
});

test("instance matching binds the selected installation path to preview and confirmation", async () => {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const calls: Array<{ command: string; args: any }> = [];
  const selections: any[] = [];
  let cursor = 0;
  let elements: Element[] = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useEffect: (run: () => void, deps: any[]) => {
      const index = cursor++;
      const previous = slots[index] as any[] | undefined;
      if (!previous || deps.some((value, position) => !Object.is(value, previous[position]))) effects.push(run);
      slots[index] = deps;
    },
    useMemo: (run: () => any) => run(),
    useRef: (value: any) => { const index = cursor++; return slots[index] ??= { current: value }; },
    t: (value: string) => value,
    tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    invoke: async (command: string, args: any) => {
      calls.push({ command, args });
      if (command === "discover_skill_source_evidence") return { status: "ok", message: "", evidence: [] };
      return { status: "ok", message: "", previewId: "instance-preview", warnings: [], manualCandidates: [],
        matches: [{ localId: "impeccable", localName: "impeccable", repoKey: "owner/repo@main",
          repoPath: "skills/impeccable", remoteName: "impeccable", exact: true }] };
    },
    AppSelect: "AppSelect", ManagementDrawer: "ManagementDrawer", Plus: "Plus", RefreshCw: "RefreshCw", Search: "Search",
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    elements.push(node);
    node.children?.forEach(collect);
    if (node.props?.footer) collect(node.props.footer);
  };
  const props = {
    id: "impeccable", localPath: "C:/agents/impeccable",
    repos: [{ owner: "owner", name: "repo", branch: "main", subdir: "", enabled: true }],
    localSkillIds: [], onClose: () => {}, onAddRepo: async () => null,
    onApply: async (items: any[]) => { selections.push(...items); return { status: "ok", message: "" }; },
  };
  const render = () => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.exports.SkillSourceMatchDrawer(props));
    effects.splice(0).forEach((run) => run());
  };
  render(); await tick(); await tick(); render();
  assert.equal(calls.find((call) => call.command === "discover_skill_source_evidence")?.args.localPath, props.localPath);
  assert.equal(calls.find((call) => call.command === "find_skill_repository_matches")?.args.localPath, props.localPath);
  const checkbox = elements.find((node) => node.type === "input" && node.props.type === "checkbox");
  assert.ok(checkbox);
  checkbox.props.onChange({ currentTarget: { checked: true } }); render();
  const confirm = elements.find((node) => node.type === "button" && node.children.includes("关联所选 1 项"));
  assert.ok(confirm);
  await confirm.props.onClick(); await tick();
  assert.equal(selections[0].localPath, props.localPath);
  assert.equal(selections[0].previewId, "instance-preview");
});
