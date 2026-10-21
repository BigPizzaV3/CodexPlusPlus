import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { parseModelMetadataDocument } from "./model-metadata.ts";
import { isProviderSyncTargetSelectable } from "./provider-sync-target.ts";

test("metadata rejects invalid final critical types and retains empty instructions", () => {
  for (const metadata of [{ base_instructions: null }, { supports_parallel_tool_calls: "true" }]) {
    assert.equal(parseModelMetadataDocument(JSON.stringify({ models: [{ slug: "custom", ...metadata }] }), "custom").ok, false);
  }
  assert.equal(parseModelMetadataDocument(JSON.stringify({ models: [{ slug: "custom", base_instructions: "", supports_parallel_tool_calls: false }] }), "custom").ok, true);
});

test("authoritative model groups merge non-adjacent IDs and separate identical names", () => {
  const source = readFileSync(new URL("../../../assets/inject/renderer-inject/71-model-picker-groups.js", import.meta.url), "utf8");
  const fn = source.match(/^  function codexPlusModelPickerGroups\([^]*?^  \}/m)?.[0];
  assert.ok(fn);
  const context = vm.createContext({});
  vm.runInContext(fn, context);
  const rows = [{ provider: "Same", groupId: "a", name: "one" }, { provider: "Same", groupId: "b", name: "two" }, { provider: "Same", groupId: "a", name: "three" }];
  const groups = context.codexPlusModelPickerGroups(rows);
  assert.deepEqual(Array.from(groups, (group: any) => group.id), ["a", "b"]);
  assert.deepEqual(Array.from(groups[0].entries, (entry: any) => entry.name), ["one", "three"]);
  assert.equal(context.codexPlusModelPickerGroups(rows.map((entry) => ({ ...entry, authoritativeGroupId: false }))).length, 1);
});

test("active ProjectSessionPage owns repair panel and enabled actions call existing callbacks", () => {
  const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  const app = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const panels: ts.JsxSelfClosingElement[] = [];
  const walk = (node: ts.Node) => { if (ts.isJsxSelfClosingElement(node) && node.tagName.getText(app) === "ProjectSessionPage") panels.push(node); ts.forEachChild(node, walk); };
  walk(app);
  assert.equal(panels.length, 1);
  assert.ok(panels[0].attributes.properties.some((attribute) => ts.isJsxAttribute(attribute) && attribute.name.getText(app) === "repairPanel" && attribute.initializer?.getText(app).includes("<SessionRepairPanel")));
  const fn = app.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SessionRepairPanel");
  assert.ok(fn);
  const slots: any[] = []; const effects: Function[] = []; let cursor = 0;
  const context = vm.createContext({
    jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment",
    t: (value: string) => value,
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useEffect: (callback: Function, deps: any[]) => {
      const index = cursor++; const previous = slots[index];
      if (!previous || deps.some((value, i) => value !== previous[i])) { slots[index] = deps; effects.push(callback); }
    },
    formatProgressPercent: (value: number) => String(value),
    Panel: "Panel", CardHead: "CardHead", CardContent: "CardContent", Field: "Field", AppSelect: "AppSelect", Button: "Button", ToggleVisual: "ToggleVisual",
    RefreshCw: "RefreshCw", Wrench: "Wrench", Save: "Save", Info: "Info", ChevronDown: "ChevronDown", isProviderSyncTargetSelectable, providerSyncTargetLabel: () => "人工目标",
  });
  vm.runInContext(ts.transpileModule(fn.getText(app), { compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", target: ts.ScriptTarget.ES2022 } }).outputText, context);
  let synced = 0; let repaired = 0; let saved = 0; let recovered = 0;
  const targets: string[] = []; const drafts: any[] = [];
  const props = { form: { providerSyncEnabled: false }, providerSyncProgress: { active: false, percent: 0 }, sessionIndexRepairActive: false,
    sessionIndexRepairReport: null, sessionIndexRepairReportError: null,
    providerSyncTargets: { targets: [{ id: "target", isResolvable: true, isCurrentProvider: false }] }, selectedProviderSyncTarget: "target",
    onFormChange: (draft: any) => { drafts.push(draft); }, actions: {
      syncProvidersNow: () => { synced++; }, repairSessionIndex: () => { repaired++; }, saveSettings: () => { saved++; },
      recoverProviderSync: () => { recovered++; }, setProviderSyncTarget: (target: string) => { targets.push(target); },
    } };
  const nodes: any[] = [];
  const collect = (node: any) => { if (!node || typeof node !== "object") return; if (Array.isArray(node)) return node.forEach(collect); nodes.push(node); node.children?.forEach(collect); };
  const render = (nextProps: any) => {
    cursor = 0; let tree = context.SessionRepairPanel(nextProps);
    while (effects.length) { effects.shift()!(); cursor = 0; tree = context.SessionRepairPanel(nextProps); }
    nodes.length = 0; collect(tree); return tree;
  };
  let tree = render(props);
  const button = (label: string) => nodes.find((node) => node.type === "Button" && node.children.includes(label));
  assert.equal(tree.type, "details");
  assert.equal(tree.props.open, false);
  assert.ok(nodes.some((node) => node.type === "strong" && node.children.includes("修复与维护")));
  assert.equal(button("刷新会话"), undefined);
  tree.props.onToggle({ currentTarget: { open: true } });
  tree = render(props); assert.equal(tree.props.open, true);
  assert.equal(button("修复历史会话").props.disabled, false);
  button("修复历史会话").props.onClick();
  button("修复会话索引").props.onClick();
  button("保存设置").props.onClick();
  nodes.find((node) => node.type === "AppSelect").props.onChange("another-target");
  nodes.find((node) => node.type === "input" && node.props.type === "checkbox").props.onChange({ currentTarget: { checked: true } });
  assert.equal(synced, 1); assert.equal(repaired, 1); assert.equal(saved, 1);
  assert.deepEqual(targets, ["another-target"]); assert.equal(drafts[0].providerSyncEnabled, true);
  tree.props.onToggle({ currentTarget: { open: false } });
  assert.equal(render(props).props.open, false);
  tree = render({ ...props, providerSyncProgress: { active: true, percent: 25 } });
  assert.equal(tree.props.open, true);
  assert.equal(button("正在修复…").props.disabled, true);
  assert.ok(nodes.some((node) => node.props.role === "status" && node.props["aria-live"] === "polite"));
  render(props);
  tree.props.onToggle({ currentTarget: { open: false } });
  tree = render({ ...props, sessionIndexRepairReportError: "index unavailable" });
  assert.equal(tree.props.open, true);
  assert.ok(nodes.some((node) => node.props.role === "alert" && node.children.includes("index unavailable")));
  render({ ...props, providerSyncProgress: { active: false, percent: 0,
    result: { syncStatus: "partial_failure", backupDir: "/artificial/unreadable", recoveryJournalPath: "/artificial/unreadable", recoveryAvailable: false } } });
  assert.equal(button("恢复未完成修复"), undefined);
  assert.equal(nodes.some((node) => node.children?.includes("仍有改动：")), false);
  render({ ...props, providerSyncProgress: { active: false, percent: 0,
    result: { syncStatus: "partial_failure", backupDir: "/artificial/backup", recoveryJournalPath: "/artificial/journal", recoveryAvailable: true } } });
  button("恢复未完成修复").props.onClick();
  assert.equal(recovered, 1);
});

test("all full saves require their own revision and conflicts preserve the caller draft", async () => {
  const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  const app = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let initializer = "";
  const find = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(app) === "call") initializer = node.initializer?.getText(app) || "";
    ts.forEachChild(node, find);
  };
  find(app);
  assert.ok(initializer);
  let request: any;
  const context = vm.createContext({ invoke: async (_command: string, args: unknown) => { request = args; return { message: "settings_revision_conflict: newer settings exist", status: "failed" }; } });
  vm.runInContext(ts.transpileModule(`const call = ${initializer}; globalThis.callTest = call;`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  const draft = { revision: "old", model: "draft-model" };
  await assert.rejects(context.callTest("save_settings", { settings: draft }), /settings_revision_conflict/);
  assert.equal(request.expectedRevision, "old");
  assert.deepEqual(draft, { revision: "old", model: "draft-model" });
  await assert.rejects(context.callTest("save_settings", { settings: { model: "unversioned" } }));
  assert.equal(request.expectedRevision, "");
});
