import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const app = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const component = app.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "RelayFold");
assert.ok(component);
const code = ts.transpileModule(component.getText(app), { compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", target: ts.ScriptTarget.ES2022 } }).outputText;

type Tree = { type: string; props: Record<string, any>; children: unknown[] };
function fixture(stored = new Map<string, string>(), storageFails = false) {
  const slots: any[] = [], effects: Function[] = [];
  let cursor = 0;
  const context = vm.createContext({
    window: { localStorage: {
      getItem: (key: string) => { if (storageFails) throw new Error("unavailable"); return stored.get(key) ?? null; },
      setItem: (key: string, value: string) => { if (storageFails) throw new Error("unavailable"); stored.set(key, value); },
    } },
    t: (value: string) => value, ChevronDown: "chevron",
    jsx: (type: string, props: Record<string, any>, ...children: unknown[]) => ({ type, props, children }),
    useState: (initial: any) => { const index = cursor++; if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial; return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }]; },
    useRef: (initial: any) => { const index = cursor++; return slots[index] ??= { current: initial }; },
    useEffect: (callback: Function, deps: any[]) => {
      const index = cursor++; const previous = slots[index];
      if (!previous || deps.some((value, i) => value !== previous[i])) { slots[index] = deps; effects.push(callback); }
    },
  });
  vm.runInContext(code, context);
  const render = (props: Record<string, unknown>) => {
    cursor = 0; let tree = context.RelayFold(props) as Tree;
    while (effects.length) { effects.shift()!(); cursor = 0; tree = context.RelayFold(props); }
    return tree;
  };
  return { render, stored };
}

test("model configuration is open by default while vendor folds use stable identifiers", () => {
  assert.ok(/className="relay-config-section relay-model-settings"[^>]*defaultOpen/.test(source));
  for (const section of ["models", "requests", "channel-protection", "advanced", "config-preview", "common-config", "auth-preview"]) {
    assert.ok(source.includes(`sectionId="${section}"`), section);
  }
});

test("fold preference survives remount and locale changes without storing provider data", () => {
  const f = fixture(); const props = { title: "模型配置", sectionId: "models", defaultOpen: true, children: { draft: "kept" } };
  let tree = f.render(props); assert.equal(tree.props.open, true);
  tree.props.onToggle({ currentTarget: { open: false } });
  tree = f.render(props); assert.equal(tree.props.open, false); assert.equal((tree.children[1] as Tree).children[0], props.children);
  const next = fixture(f.stored).render({ ...props, title: "Models" }); assert.equal(next.props.open, false);
  assert.deepEqual([...f.stored], [["codex-plus-relay-fold:models", "false"]]);
});

test("new validation failures reveal their controls once without overriding a manual collapse", () => {
  const f = fixture(); const props = { title: "请求设置", sectionId: "requests", children: "unsaved draft" };
  assert.equal(f.render(props).props.open, false);
  let tree = f.render({ ...props, issue: "header invalid" }); assert.equal(tree.props.open, true);
  // 浏览器因 open prop 产生的 toggle 不是用户偏好。
  tree.props.onToggle({ currentTarget: { open: true } }); assert.equal(f.stored.size, 0);
  tree.props.onToggle({ currentTarget: { open: false } });
  assert.equal(f.render({ ...props, issue: "header still invalid" }).props.open, false);
  f.render(props); assert.equal(f.render({ ...props, issue: "new error" }).props.open, true);
  assert.equal(fixture(f.stored).render({ ...props, issue: "initial error" }).props.open, true);
});

test("unavailable local storage keeps folds usable and their draft children mounted", () => {
  const f = fixture(new Map(), true); const child = { draft: "unsaved" };
  const props = { title: "模型配置", sectionId: "models", defaultOpen: true, children: child };
  let tree = f.render(props); assert.equal(tree.props.open, true);
  tree.props.onToggle({ currentTarget: { open: false } }); tree = f.render(props);
  assert.equal(tree.props.open, false); assert.equal((tree.children[1] as Tree).children[0], child);
});

test("corrupt fold preferences use section defaults and do not affect other sections", () => {
  const stored = new Map([["codex-plus-relay-fold:models", "broken"], ["codex-plus-relay-fold:requests", "true"]]);
  assert.equal(fixture(stored).render({ title: "Models", sectionId: "models", defaultOpen: true }).props.open, true);
  assert.equal(fixture(stored).render({ title: "Requests", sectionId: "requests" }).props.open, true);
  assert.equal(fixture(stored).render({ title: "Advanced", sectionId: "advanced" }).props.open, false);
});

test("new sharing and protocol hint strings have English translations", () => {
  const dictionary = readFileSync(new URL("./i18n-en.ts", import.meta.url), "utf8");
  for (const key of [
    "分享会话按钮",
    "在当前会话工具栏显示分享按钮，保存后更新显示，无需重启 Codex。",
    "若网关提示 thinking type: adaptive 无效，可启用此项后重试；这会停用厂商私有推理参数。",
  ]) {
    assert.ok(source.includes(`t(${JSON.stringify(key)})`), key);
    assert.ok(dictionary.includes(`${JSON.stringify(key)}:`), key);
  }
});
