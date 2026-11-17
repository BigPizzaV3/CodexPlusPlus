import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

type Element = { type: string; props: Record<string, any>; children: any[] };
const tick = () => new Promise<void>(resolve => setImmediate(resolve));
const healthy = { root: "codex", path: "C:/codex/alpha", revision: "current", modifiedAtMs: 1_700_000_000_000, comparisonError: null, linked: false, managed: true };
const broken = { root: "claude", path: "C:/claude/alpha", revision: null, modifiedAtMs: null, comparisonError: "nested link", linked: false, managed: false };
const code = ts.transpileModule(readFileSync(new URL("./SkillVersionPicker.tsx", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React, jsxFactory: "jsx" },
}).outputText;

function collect(node: any, result: Element[] = []): Element[] {
  if (Array.isArray(node)) node.forEach(value => collect(value, result));
  else if (node && typeof node === "object") { result.push(node); node.children?.forEach((value: any) => collect(value, result)); }
  return result;
}

async function picker(mode: string, versions = [healthy, broken]) {
  const state: any[] = [];
  const actions: any[] = [];
  let cursor = 0;
  let started = false;
  let effect: (() => void) | undefined;
  const context = {
    exports: {},
    jsx: (type: string, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    require: (name: string) => {
      if (name === "react") return {
        useState: (initial: any) => {
          const index = cursor++;
          if (!(index in state)) state[index] = initial;
          return [state[index], (value: any) => { state[index] = value; }];
        },
        useEffect: (run: () => void) => { if (!started) { effect = run; started = true; } },
      };
      if (name === "@tauri-apps/api/core") return { invoke: async () => versions };
      if (name === "./ManagementPages") return { ManagementDrawer: "drawer" };
      if (name === "./app-dialog") return { appConfirm: async () => true };
      if (name === "./i18n") return { t: (text: string) => text };
      if (name === "lucide-react") return { FolderOpen: "folder", Trash2: "trash" };
      if (name === "@tauri-apps/plugin-dialog") return { open: async () => null };
      throw new Error(`Unexpected import: ${name}`);
    },
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const render = () => {
    cursor = 0;
    return collect((context.exports as any).SkillVersionPicker({
      task: { id: "alpha", mode, agent: "grok" }, roots: {}, onClose: () => {},
      onRun: async (action: any) => { actions.push(action); return true; },
    }));
  };
  render(); effect!(); await tick();
  return { elements: render(), actions };
}

test("a comparison error stays visible while another version can be enabled", async () => {
  const { elements, actions } = await picker("enable");
  const bad = elements.find(node => node.type === "button" && collect(node).some(child => child.children.includes(broken.path)))!;
  assert.equal(bad.props.disabled, true);
  assert.ok(collect(bad).some(node => node.props.role === "alert" && node.children.includes("nested link")));
  assert.equal(collect(bad).some(node => node.type === "time"), false);
  const good = elements.find(node => node.type === "button" && collect(node).some(child => child.children.includes(healthy.path)))!;
  assert.equal(good.props.disabled, false);
  await good.props.onClick();
  assert.equal(actions[0].revision, "current");
  assert.equal(actions[0].root, "codex");
});

test("conflict resolution stays blocked when any version cannot be compared", async () => {
  for (const versions of [[healthy, broken], [healthy, { ...broken, root: "source" }]]) {
    const { elements, actions } = await picker("resolve", versions);
    assert.ok(elements.filter(node => node.type === "button").every(node => node.props.disabled));
    assert.ok(elements.some(node => node.props.role === "alert" && node.children.includes("部分安装位置无法比较，请先处理后重试。")));
    assert.equal(actions.length, 0);
  }
});

test("a healthy local version can be retained without hiding an unrelated comparison error", async () => {
  const { elements, actions } = await picker("local");
  const good = elements.find(node => node.type === "button" && collect(node).some(child => child.children.includes(healthy.path)))!;
  assert.equal(good.props.disabled, false);
  await good.props.onClick();
  await tick();
  assert.equal(actions[0].kind, "keepLocal");
  assert.equal(actions[0].revisions.codex, "current");
});
