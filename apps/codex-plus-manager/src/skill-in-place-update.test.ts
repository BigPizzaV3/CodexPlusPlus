import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = readFileSync(new URL("./SkillInPlaceUpdateDrawer.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("SkillInPlaceUpdateDrawer.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const component = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillInPlaceUpdateDrawer");
assert.ok(component);
const code = ts.transpileModule(component.getText(file), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

function fixture() {
  const slots: any[] = [];
  const effects: Array<() => () => void> = [];
  const calls: Array<{ command: string; args: any }> = [];
  let cursor = 0;
  let resolvePreview!: (result: any) => void;
  const context: Record<string, any> = {
    exports: {}, fragment: "fragment", jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useEffect: (effect: () => () => void) => { const index = cursor++; if (!(index in slots)) { slots[index] = true; effects.push(effect); } },
    invoke: (command: string, args: any) => {
      calls.push({ command, args });
      return command === "preview_skill_in_place_update"
        ? new Promise((resolve) => { resolvePreview = resolve; })
        : Promise.resolve();
    },
    t: (value: string) => value, ManagementDrawer: "ManagementDrawer", RefreshCw: "RefreshCw", console,
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const render = () => {
    cursor = 0;
    return context.exports.SkillInPlaceUpdateDrawer({ id: "alpha", repoKey: "owner/repo@main", onClose: () => {}, onExecute: async () => true });
  };
  render();
  const cleanup = effects[0]();
  return { calls, cleanup, render, resolve: (currentHash: string) => resolvePreview({
    status: "ok", previewId: "preview-1", repoKey: "owner/repo@main", repoPath: "alpha",
    targetPath: "/skills/alpha", currentHash, nextHash: "new", conflicts: [],
  }) };
}

test("closing a loaded update preview releases it", async () => {
  const drawer = fixture();
  drawer.resolve("old");
  await tick();
  drawer.cleanup();
  assert.equal(drawer.calls.at(-1)?.command, "discard_skill_in_place_update_preview");
  assert.equal(drawer.calls.at(-1)?.args.previewId, "preview-1");
});

test("closing while a preview is loading releases its late result", async () => {
  const drawer = fixture();
  drawer.cleanup();
  drawer.resolve("old");
  await tick();
  assert.equal(drawer.calls.at(-1)?.command, "discard_skill_in_place_update_preview");
  assert.equal(drawer.calls.at(-1)?.args.previewId, "preview-1");
});

test("a preview without a matching baseline warns before update", async () => {
  const drawer = fixture();
  drawer.resolve("");
  await tick();
  const root = drawer.render();
  const serialized = JSON.stringify(root);
  assert.match(serialized, /尚无一致的更新基线/);
  assert.match(serialized, /确认后会先备份再替换/);
  drawer.cleanup();
});
