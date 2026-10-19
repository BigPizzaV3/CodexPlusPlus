import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { parseModelMetadataMap, synchronizeModelMetadataLiteSupport } from "./model-metadata.ts";
import { reasoningParameterMode, setModelReasoningParameterMode } from "./model-reasoning.ts";
import { requestCompatibilityEnabled, settingsRequireLocalHelper } from "./model-routes.ts";

test("reasoning modes retain level drafts, round trip and require the helper only when opted in", () => {
  const original = JSON.stringify({ model: { custom: "kept", supported_reasoning_levels: [{ effort: "high" }] }, other: { custom: true } });
  const omitted = setModelReasoningParameterMode(original, "model", "omit");
  assert.equal(reasoningParameterMode(parseModelMetadataMap(omitted).model), "omit");
  assert.deepEqual(parseModelMetadataMap(omitted).model.supported_reasoning_levels, [{ effort: "high" }]);
  const reset = setModelReasoningParameterMode(omitted, "model", "auto");
  assert.deepEqual(JSON.parse(reset), JSON.parse(original));
  const profile = { id: "p", name: "p", protocol: "responses" as const, relayMode: "pureApi" as const, officialMixApiKey: false, modelMetadata: original, apiKey: "fixture-only", baseUrl: "https://example.test" };
  const settings = { activeRelayId: "p", relayProfilesEnabled: true, enhancementsEnabled: false, relayProfiles: [profile] };
  assert.equal(settingsRequireLocalHelper(settings), false);
  assert.equal(settingsRequireLocalHelper({ ...settings, relayProfiles: [{ ...profile, modelMetadata: omitted }] }), true);
  assert.equal(requestCompatibilityEnabled({ ...profile, strictResponsesReasoning: true }), true);
  assert.equal(requestCompatibilityEnabled({ ...profile, relayMode: "official", strictResponsesReasoning: true }), false);
});

test("Lite controls edit one selected model and preserve the rest of an imported document", () => {
  const original = { models: [{ slug: "one", custom: 42 }, { slug: "two", use_responses_lite: false }] };
  const changed = synchronizeModelMetadataLiteSupport(JSON.stringify(original), "one", "supported");
  assert.ok(changed);
  assert.equal(changed.preview.metadata.use_responses_lite, true);
  assert.deepEqual(JSON.parse(changed.document).models[1], original.models[1]);
  const reset = synchronizeModelMetadataLiteSupport(changed.document, "one", "inherit");
  assert.ok(reset);
  assert.deepEqual(JSON.parse(reset.document), original);
  assert.equal(synchronizeModelMetadataLiteSupport('{"models":[{"slug":"one"},{"slug":"one"}]}', "one", "supported"), null);
});

test("language switches notify React without reload or storing drafts", () => {
  const source = readFileSync(new URL("./i18n.ts", import.meta.url), "utf8");
  const saved = new Map(); let reloads = 0; let calls = 0;
  const exports: Record<string, any> = {};
  const context = vm.createContext({ exports, require: () => ({ EN_PLAIN: { "概览": "Overview" }, EN_BACKEND: {}, EN_BACKEND_PATTERNS: [], EN_TEMPLATE: {} }),
    window: { localStorage: { getItem: (k: string) => saved.get(k), setItem: (k: string, v: string) => saved.set(k, v) }, location: { reload: () => { reloads++; } } } });
  vm.runInContext(ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  const unsubscribe = exports.subscribeLanguage(() => { calls++; });
  assert.equal(exports.t("概览"), "概览");
  exports.setLanguage("en"); assert.equal(exports.t("概览"), "Overview");
  exports.setLanguage("en"); assert.equal(calls, 1);
  exports.setLanguage("zh"); assert.equal(exports.t("概览"), "概览");
  unsubscribe(); exports.setLanguage("en"); assert.equal(calls, 2);
  assert.equal(reloads, 0);
  assert.deepEqual([...saved.keys()], ["codex-plus-lang"]);
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  assert.match(app, /useSyncExternalStore\(subscribeLanguage, getLanguage, getLanguage\)/);
  for (const name of ["App.tsx", "components/ProviderPresetSelector.tsx"]) {
    const file = ts.createSourceFile(name, readFileSync(new URL(`./${name}`, import.meta.url), "utf8"), ts.ScriptTarget.Latest, true);
    const visit = (node: ts.Node) => {
      if (ts.isCallExpression(node) && node.expression.getText(file) === "t") {
        let p: ts.Node | undefined = node.parent; let lazy = false;
        while (p && !ts.isSourceFile(p)) { if (ts.isFunctionLike(p)) { lazy = true; break; } p = p.parent; }
        // 默认供应商名称是配置数据，保持创建时快照；导航/提示标签才需动态翻译。
        const defaultName = node.arguments[0] && ts.isStringLiteral(node.arguments[0]) && node.arguments[0].text === "默认中转";
        if (name === "App.tsx" && defaultName) assert.ok(ts.isPropertyAssignment(node.parent), "配置名称不能变成动态 getter");
        const dataSnapshot = name === "App.tsx" && ts.isPropertyAssignment(node.parent)
          && node.parent.name.getText(file) === "name" && defaultName;
        assert.equal(lazy || dataSnapshot, true, `${name} contains a frozen UI label`);
      }
      ts.forEachChild(node, visit);
    }; visit(file);
  }
});

test("session selection cancellation, search and pagination never delete hidden rows", async () => {
  const { sessionGroups, matchesManagementQuery } = await import("./management-model.ts");
  const source = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
  const file = ts.createSourceFile("ManagementExtra.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const fn = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SessionPage");
  assert.ok(fn);
  const slots: any[] = []; let cursor = 0; let effects: Array<() => void> = []; const deleted: string[] = [];
  const scope = vm.createContext({
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }), fragment: "fragment",
    useState: (initial: any) => { const index = cursor++; if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (next: any) => { slots[index] = typeof next === "function" ? next(slots[index]) : next; }]; },
    useEffect: (run: () => void) => effects.push(run),
    t: (x: string) => x, tf: (x: string, args: any[]) => x.replace(/\{(\d+)\}/g, (_, n) => args[n]),
    sessionGroups, matchesManagementQuery,
    Header: "Header", Button: "Button", SearchBox: "SearchBox", ManagementDrawer: "Drawer",
    History: "Icon", RefreshCw: "Icon", Trash2: "Icon", ArrowLeft: "Icon", ArrowRight: "Icon", ChevronDown: "Icon", Copy: "Icon",
  });
  vm.runInContext(ts.transpileModule(fn.getText(file), { compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, scope);
  const first = { id: "one", title: "first", cwd: "/fixture", modelProvider: "custom" };
  let props: any = { sessions: { sessions: [first, { ...first, id: "two", title: "second" }], offset: 0, limit: 50, totalCount: 2, hasMore: false },
    onRefresh: async () => {}, onDelete: async () => {}, onDeleteMany: async (rows: any[]) => { deleted.push(...rows.map((r) => r.id)); }, notify: async () => {} };
  let nodes: any[] = [];
  const render = () => { cursor = 0; effects = []; nodes = [];
    const collect = (node: any) => { if (!node || typeof node !== "object") return; if (Array.isArray(node)) return node.forEach(collect); nodes.push(node); node.children?.forEach(collect); collect(node.props?.actions); };
    collect(scope.exports.SessionPage(props)); effects.forEach((run) => run()); };
  const button = (label: string) => { const node = nodes.find((n) => n.type === "Button" && n.children?.includes(label)); assert.ok(node, label); return node; };
  render(); button("选择多个…").props.onClick(); render(); button("全选当前列表").props.onClick(); render();
  nodes.find((n) => n.type === "SearchBox").props.onChange("first"); render();
  await button("删除已选").props.onClick(); assert.deepEqual(deleted, ["one"]);
  button("取消全部选择").props.onClick(); render(); assert.equal(button("删除已选").props.disabled, true);
  button("全选当前列表").props.onClick(); render();
  props = { ...props, sessions: { ...props.sessions, sessions: [{ ...first, id: "three", title: "first next page" }], offset: 50, totalCount: 51 } };
  render(); assert.equal(button("删除已选").props.disabled, true);
});
