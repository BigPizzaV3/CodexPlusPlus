import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

type Element = { type: any; props: Record<string, any>; children: any[] };
const source = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("ManagementExtra.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const declarations = file.statements.filter((node) =>
  ts.isFunctionDeclaration(node) && ["SkillDetailDrawer", "SkillActionMenu", "SkillImportDrawer"].includes(node.name?.text || ""));
assert.equal(declarations.length, 3);
const code = ts.transpileModule(declarations.map((node) => node.getText(file)).join("\n"), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const selectSource = readFileSync(new URL("./AppSelect.tsx", import.meta.url), "utf8");
const selectFile = ts.createSourceFile("AppSelect.tsx", selectSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const selectDeclaration = selectFile.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "AppSelect")!;
const selectCode = ts.transpileModule(selectDeclaration.getText(selectFile), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

const skill = { id: "sample", name: "Sample", repoKey: "owner/repo@main", repoPath: "skills/sample", installed: true, bundled: false };
const groups = [
  { key: "builtin", label: "内置", kind: "builtin", items: [] },
  { key: "user", label: "用户", kind: "user", items: [] },
  { key: "repo:owner/repo", label: "来源仓库", kind: "repo", items: [] },
  { key: "custom:工作", label: "工作", kind: "custom", items: [skill] },
];

function createHarness() {
  const slots: any[] = [];
  let cursor = 0;
  const assignments: string[] = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }), fragment: "fragment",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useEffect: () => {},
    usePopoverLayer: () => ({ rootRef: null, menuRef: null, menuStyle: {} }),
    createPortal: (node: any) => node, document: { body: {} },
    repoUrl: () => "https://github.com/owner/repo/tree/main/skills/sample",
    sourceLabel: () => "owner/repo", importChannel: () => "", t: (value: string) => value,
    tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
  };
  for (const name of ["ManagementDrawer", "AppSelect", "Button", "RefreshCw", "ArrowRight", "ExternalLink", "Search", "Edit3", "MoreHorizontal", "ArrowLeft", "Check", "BookOpen", "FolderOpen", "ChevronRight", "Trash2"]) context[name] = name;
  vm.createContext(context);
  vm.runInContext(code, context);
  const props = { skill, groups, group: groups[3], originGroup: groups[2], assignment: "custom:工作", busy: false, canUninstall: true,
    onAssign: (value: string) => assignments.push(value), onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {}, onEditSource: () => {}, onMatch: () => {},
    onDetail: () => {}, onUninstall: () => {} };
  const render = (component: string, overrides = {}) => {
    cursor = 0;
    const elements: Element[] = [];
    const visit = (node: any) => {
      if (!node || typeof node !== "object") return;
      if (Array.isArray(node)) return node.forEach(visit);
      elements.push(node);
      node.children?.forEach(visit);
      if (node.props?.footer) visit(node.props.footer);
    };
    visit(context[component]({ ...props, ...overrides }));
    return elements;
  };
  return { render, assignments };
}

function createSelectHarness() {
  let open = false;
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    useState: () => [open, (value: any) => { open = typeof value === "function" ? value(open) : value; }],
    usePopoverLayer: () => ({ rootRef: null, menuRef: null, menuStyle: {} }),
    createPortal: (node: any) => node, document: { body: {} }, Check: "Check", ChevronDown: "ChevronDown",
  };
  vm.createContext(context);
  vm.runInContext(selectCode, context);
  return (props: any) => {
    const elements: Element[] = [];
    const visit = (node: any) => {
      if (!node || typeof node !== "object") return;
      if (Array.isArray(node)) return node.forEach(visit);
      elements.push(node);
      node.children?.forEach(visit);
    };
    visit(context.AppSelect(props));
    return elements;
  };
}

test("detail displays current group and can choose another group or automatic source", () => {
  const ui = createHarness();
  const elements = ui.render("SkillDetailDrawer");
  const select = elements.find((node) => node.type === "AppSelect")!;
  assert.equal(select.props.value, "custom:工作");
  assert.deepEqual(Array.from(select.props.options, (option: any) => option.label), ["用户", "来源仓库", "工作", "按来源自动分组 · 来源仓库"]);
  select.props.onChange("user");
  select.props.onChange("");
  assert.deepEqual(ui.assignments, ["user", ""]);
  assert.equal(ui.render("SkillDetailDrawer", { skill: { ...skill, bundled: true }, group: groups[0] }).some((node) => node.type === "AppSelect"), false);
});

test("long detail group names remain distinct in the trigger and the selectable portal options", () => {
  const ui = createHarness();
  const first = `${"long-group-".repeat(12)}A`;
  const second = `${"long-group-".repeat(12)}B`;
  const longGroups = [{ key: "custom:a", label: first, kind: "custom", items: [skill] },
    { key: "custom:b", label: second, kind: "custom", items: [] }];
  const select = ui.render("SkillDetailDrawer", { groups: longGroups, group: longGroups[0], originGroup: longGroups[1] })
    .find((node) => node.type === "AppSelect")!;
  assert.equal(select.props.className, "mg-skill-long-select");
  assert.equal(select.props.title, first);
  const renderSelect = createSelectHarness();
  let elements = renderSelect(select.props);
  const trigger = elements.find((node) => node.props.className === "app-select-trigger")!;
  assert.equal(trigger.props.title, first);
  assert.equal(trigger.children[0].children[0], first);
  trigger.props.onClick();
  elements = renderSelect(select.props);
  assert.ok(elements.some((node) => node.props.role === "listbox" && node.props.className.includes("mg-skill-long-select")));
  const choices = elements.filter((node) => node.props.role === "option");
  assert.equal(choices[0].props.title, first);
  assert.equal(choices[1].props.title, second);
  assert.equal(choices[1].children[0].children[0], second);
  choices[1].props.onClick();
  assert.deepEqual(ui.assignments, ["custom:b"]);
});

test("long import overwrite targets remain distinct and the selected target is submitted", () => {
  const ui = createHarness();
  const name = "long-skill-name-".repeat(12);
  const matches = [{ id: `${name}A`, name: `${name}A`, snapshot: "snapshot-a" },
    { id: `${name}B`, name: `${name}B`, snapshot: "snapshot-b" }];
  const candidate = { id: "incoming", name: "Incoming", sourceHash: "source", repoKey: "local", repoPath: "incoming", matches, duplicateCandidates: [] };
  const submitted: any[] = [];
  const props = { request: { preview: { candidates: [candidate], warning: "" } }, busy: false,
    onClose: () => {}, onRefresh: () => {}, onApply: (items: any[]) => submitted.push(...items) };
  let elements = ui.render("SkillImportDrawer", props);
  elements.find((node) => node.type === "input" && node.props.type === "checkbox")!.props.onChange({ currentTarget: { checked: true } });
  elements = ui.render("SkillImportDrawer", props);
  const select = elements.find((node) => node.type === "AppSelect")!;
  assert.equal(select.props.className, "mg-skill-long-select");
  const renderSelect = createSelectHarness();
  let rendered = renderSelect(select.props);
  rendered.find((node) => node.props.className === "app-select-trigger")!.props.onClick();
  rendered = renderSelect(select.props);
  const choices = rendered.filter((node) => node.props.role === "option");
  assert.equal(choices[0].props.title, `${matches[0].name} (${matches[0].id})`);
  assert.equal(choices[1].props.title, `${matches[1].name} (${matches[1].id})`);
  choices[1].props.onClick();
  elements = ui.render("SkillImportDrawer", props);
  const selected = elements.find((node) => node.type === "AppSelect")!;
  assert.equal(selected.props.value, matches[1].id);
  assert.equal(selected.props.title, `${matches[1].name} (${matches[1].id})`);
  const trigger = createSelectHarness()(selected.props).find((node) => node.props.className === "app-select-trigger")!;
  assert.equal(trigger.children[0].children[0], selected.props.title);
  elements.find((node) => node.type === "Button" && node.children.includes("导入所选 1 项"))!.props.onClick();
  assert.equal(submitted[0].targetId, matches[1].id);
  assert.equal(submitted[0].targetSnapshot, "snapshot-b");
});

test("long-name styles wrap both select triggers and portal option labels", () => {
  const css = readFileSync(new URL("./ManagementPages.css", import.meta.url), "utf8");
  assert.match(css, /\.app-select\.mg-skill-long-select \.app-select-trigger > span,\s*\.app-select-menu\.mg-skill-long-select \.app-select-option-label\s*\{[^}]*overflow-wrap:anywhere;[^}]*white-space:normal;/);
  assert.match(css, /\.app-select\.mg-skill-long-select \.app-select-trigger\s*\{[^}]*height:auto;/);
});

test("installed skill can edit and rematch an existing source; bundled and uninstalled entries cannot", () => {
  const ui = createHarness();
  const labels = (overrides = {}) => ui.render("SkillDetailDrawer", overrides)
    .filter((node) => node.type === "button").map((node) => node.props["aria-label"]);
  assert.ok(labels().includes("修改仓库来源"));
  assert.ok(labels().includes("匹配仓库来源"));
  assert.ok(!labels({ skill: { ...skill, bundled: true } }).includes("修改仓库来源"));
  assert.ok(!labels({ skill: { ...skill, installed: false } }).includes("匹配仓库来源"));
});

test("group submenu shows the current group first and retains automatic source reset", () => {
  const ui = createHarness();
  let elements = ui.render("SkillActionMenu");
  elements.find((node) => node.type === "button" && node.props["aria-haspopup"] === "menu")!.props.onClick();
  elements = ui.render("SkillActionMenu");
  elements.find((node) => node.type === "button" && node.children.some((child: any) => child?.children?.includes("切换分组")))!.props.onClick();
  elements = ui.render("SkillActionMenu");
  const choices = elements.filter((node) => node.props.role === "menuitemradio");
  assert.equal(choices[0].props["aria-checked"], "true");
  assert.equal(choices[0].children[0].children[0], "工作");
  const reset = elements.find((node) => node.props.role === "menuitem" && node.children[0]?.children?.includes("按来源自动分组"))!;
  reset.props.onClick();
  assert.deepEqual(ui.assignments, [""]);
});
