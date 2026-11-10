import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

type Element = { type: any; props: Record<string, any>; children: any[] };
const source = readFileSync(new URL("./SkillDuplicatesDrawer.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("SkillDuplicatesDrawer.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const nodes = file.statements.filter((node) =>
  ts.isFunctionDeclaration(node) && node.name?.text === "SkillDuplicatesDrawer"
  || ts.isVariableStatement(node) && node.declarationList.declarations.some((item) =>
    ts.isIdentifier(item.name) && item.name.text === "statusLabel"));
const code = ts.transpileModule(nodes.map((node) => node.getText(file)).join("\n"), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test("a verified managed duplicate can be archived despite other independent same-name skills", async () => {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const archives: Array<[string, string, string]> = [];
  const confirmations: string[] = [];
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
    t: (value: string) => value, appConfirm: async (message: string) => { confirmations.push(message); return true; },
    invoke: async () => ({ status: "ok", previewId: "verified-preview", candidates: [
      { id: "rea", sharedPath: "C:/agents/rea", codexPath: "C:/codex/rea", status: "identical-managed" },
      { id: "rea", kind: "source", sharedPath: "C:/codex/rea", codexPath: "C:/managed/rea", originalRootKind: "codex", status: "identical-source" },
      { id: "other", sharedPath: "C:/agents/other", codexPath: "C:/codex/other", status: "different" },
      { id: "unknown", sharedPath: "C:/agents/unknown", codexPath: "C:/codex/unknown", status: "unsupported", detail: "无法确定名称规则的启用状态" },
    ] }),
    ManagementDrawer: "ManagementDrawer", RefreshCw: "RefreshCw", SkillFileTimes: "SkillFileTimes",
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
    onClose: () => {},
    onArchive: async (previewId: string, id: string, kind: string) => { archives.push([previewId, id, kind]); return true; },
  };
  const render = () => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.exports.SkillDuplicatesDrawer(props));
    effects.splice(0).forEach((run) => run());
  };
  render(); await tick(); render();
  const buttons = elements.filter((node) => node.type === "button" && node.children.includes("归档副本"));
  assert.equal(buttons.length, 2);
  assert.ok(elements.some((node) => node.type === "small" && node.children.includes("无法确定名称规则的启用状态")));
  assert.equal(elements.filter((node) => node.type === "SkillFileTimes").length, 8);
  assert.equal(buttons[0].props.disabled, false);
  await buttons[0].props.onClick();
  await buttons[1].props.onClick();
  await tick();
  assert.deepEqual(archives, [["verified-preview", "rea", "shared"], ["verified-preview", "rea", "source"]]);
  assert.ok(confirmations[1].includes("C:/managed/rea"));
  assert.ok(confirmations[1].includes("C:/codex/rea"));
});
