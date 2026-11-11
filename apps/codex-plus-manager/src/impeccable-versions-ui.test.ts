import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

type Element = { type: any; props: Record<string, any>; children: any[] };
const source = readFileSync(new URL("./ImpeccableVersions.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("ImpeccableVersions.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const component = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "ImpeccableVersions");
assert.ok(component);
const code = ts.transpileModule(component.getText(file), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

test("impeccable details distinguish the skill, actual engine and required engine versions", async () => {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const calls: Array<{ command: string; args: any }> = [];
  let cursor = 0;
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [slots[index], (value: any) => { slots[index] = value; }];
    },
    useEffect: (run: () => void, deps: any[]) => {
      const index = cursor++;
      const previous = slots[index] as any[] | undefined;
      if (!previous || deps.some((value, position) => !Object.is(value, previous[position]))) effects.push(run);
      slots[index] = deps;
    },
    t: (value: string) => value,
    invoke: async (command: string, args: any) => {
      calls.push({ command, args });
      return { status: "ok", message: "", skillVersion: "4.8.0", engineVersion: null, requiredEngineVersion: "0.1.18" };
    },
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const render = (): Element => {
    cursor = 0;
    const result = context.exports.ImpeccableVersions({ instanceKey: "impeccable:shared" });
    effects.splice(0).forEach((run) => run());
    return result;
  };
  render();
  await new Promise<void>((resolve) => setImmediate(resolve));
  const details = render();
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{
    command: "inspect_impeccable_versions", args: { instanceKey: "impeccable:shared" },
  }]);
  assert.deepEqual(details.children.map((node: Element) => [node.type, node.children[0]]), [
    ["dt", "技能版本"], ["dd", "4.8.0"],
    ["dt", "实际引擎版本"], ["dd", "未知"],
    ["dt", "启动器要求的引擎版本"], ["dd", "0.1.18"],
  ]);
});

test("skill details request versions only for the selected impeccable instance", () => {
  const pageSource = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
  const pageFile = ts.createSourceFile("ManagementExtra.tsx", pageSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const detail = pageFile.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillDetailDrawer");
  assert.ok(detail);
  const detailCode = ts.transpileModule(`${detail.getText(pageFile)}\nexports.detail = SkillDetailDrawer;`, {
    compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", ManagementDrawer: "ManagementDrawer", ImpeccableVersions: "ImpeccableVersions",
    t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
  };
  vm.createContext(context);
  vm.runInContext(detailCode, context);
  const render = (id: string) => context.exports.detail({
    skill: { id, name: id, repoKey: "", installed: false, bundled: false, instanceKey: `${id}:shared` },
    backups: [], busy: false,
  }) as Element;
  const impeccable = render("impeccable");
  const versions = (impeccable.children[0] as Element).children.find((node: Element) => node?.type === "ImpeccableVersions");
  assert.equal(versions?.props.instanceKey, "impeccable:shared");
  assert.equal((render("other").children[0] as Element).children.some((node: Element) => node?.type === "ImpeccableVersions"), false);
});
