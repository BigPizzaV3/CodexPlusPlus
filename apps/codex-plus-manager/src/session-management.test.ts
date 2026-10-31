import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { matchesManagementQuery, normalizeProjectPath, sessionGroups, type ManagedSession } from "./management-model.ts";

type Tree = { type: string; props: Record<string, any>; children: unknown[] };
const source = readFileSync(new URL("./ProjectSessionPage.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("ProjectSessionPage.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const functions = file.statements.filter((node) => ts.isFunctionDeclaration(node));
const code = ts.transpileModule(functions.map((node) => node.getText(file).replace(/^export /, "")).join("\n"), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", target: ts.ScriptTarget.ES2022 },
}).outputText;

function nodes(tree: unknown): Tree[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  if (!tree || typeof tree !== "object") return [];
  const node = tree as Tree;
  return [node, ...node.children.flatMap(nodes)];
}

function text(tree: unknown): string {
  if (Array.isArray(tree)) return tree.map(text).join("");
  if (!tree || typeof tree === "boolean") return "";
  if (typeof tree !== "object") return String(tree);
  return (tree as Tree).children.map(text).join("");
}

function button(tree: Tree, label: string | RegExp): Tree {
  const found = nodes(tree).find((node) => node.type === "button" && (typeof label === "string" ? text(node) === label : label.test(text(node))));
  assert.ok(found, `未找到按钮：${label}`);
  return found;
}

function fixture(overrides: Record<string, any> = {}) {
  const slots: any[] = [];
  let cursor = 0;
  const jsx = (type: any, props: Record<string, any> | null, ...children: unknown[]): Tree => typeof type === "function"
    ? type({ ...props, children: children.length === 1 ? children[0] : children })
    : { type, props: props ?? {}, children };
  const context = vm.createContext({
    jsx, fragment: "fragment", matchesManagementQuery, sessionGroups,
    t: (value: string) => value,
    tf: (value: string, args: unknown[]) => value.replace(/\{(\d+)\}/g, (_, index: string) => String(args[Number(index)])),
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useMemo: (create: () => unknown) => create(),
    Archive: "Archive", ArrowLeft: "ArrowLeft", ArrowRight: "ArrowRight", ChevronDown: "ChevronDown", Copy: "Copy", History: "History", Info: "Info",
    RefreshCw: "RefreshCw", Search: "Search", Trash2: "Trash2", ManagementDrawer: "ManagementDrawer",
  });
  vm.runInContext(code, context);
  let props = {
    sessions: { sessions: [session("first", "/projects/alpha"), session("second", "/projects/beta")], totalCount: 4, hasMore: true, offset: 0, limit: 2 },
    onRefresh: async () => {}, onDelete: async () => true, onDeleteMany: async () => true,
    onArchiveMany: async () => true, onListProjectSessions: async () => [], ...overrides,
  };
  return { render: (update: Record<string, any> = {}) => { props = { ...props, ...update }; cursor = 0; return context.ProjectSessionPage(props) as Tree; } };
}

function session(id: string, cwd: string): ManagedSession {
  return { id, title: id, cwd, modelProvider: "custom", archived: false, updatedAtMs: 1, rolloutPath: "", dbPath: "" };
}

const flushActions = () => new Promise<void>((resolve) => setImmediate(resolve));
const bulkButtons = (tree: Tree) => nodes(tree).filter((node) => node.type === "button" && /全选当前列表|取消全部选择|归档已选|删除已选/.test(text(node)));

test("session management exposes bulk actions only during selection and finishing clears the selection", () => {
  const f = fixture();
  let tree = f.render();
  assert.equal(bulkButtons(tree).length, 0);
  button(tree, "选择多个…").props.onClick();
  tree = f.render();
  assert.equal(button(tree, /^删除已选/).props.disabled, true);
  button(tree, "全选当前列表").props.onClick();
  tree = f.render();
  assert.equal(button(tree, "删除已选 (2)").props.disabled, false);
  button(tree, "取消全部选择").props.onClick();
  tree = f.render();
  assert.equal(button(tree, "删除已选 (0)").props.disabled, true);
  button(tree, "全选当前列表").props.onClick();
  tree = f.render();
  button(tree, "完成").props.onClick();
  tree = f.render();
  assert.equal(bulkButtons(tree).length, 0);
  button(tree, "选择多个…").props.onClick();
  assert.equal(button(f.render(), "删除已选 (0)").props.disabled, true);
});

test("search and pagination retain selected sessions, and cancelled deletion keeps them available", async () => {
  const deleted: string[][] = []; const offsets: number[] = [];
  let allowDelete = false;
  const f = fixture({
    onRefresh: async (offset: number) => { offsets.push(offset); },
    onDeleteMany: async (rows: ManagedSession[]) => { deleted.push(Array.from(rows, (row) => row.id).sort()); return allowDelete; },
  });
  let tree = f.render(); button(tree, "选择多个…").props.onClick();
  tree = f.render(); button(tree, "全选当前列表").props.onClick();
  tree = f.render(); button(tree, "下一页").props.onClick();
  await flushActions();
  assert.deepEqual(offsets, [2]);
  tree = f.render({ sessions: { sessions: [session("third", "/projects/alpha"), session("fourth", "/projects/beta")], totalCount: 4, hasMore: false, offset: 2, limit: 2 } });
  const search = nodes(tree).find((node) => node.type === "input" && node.props.placeholder);
  assert.ok(search);
  search.props.onChange({ target: { value: "third" } });
  tree = f.render();
  button(tree, "全选当前列表").props.onClick();
  tree = f.render(); button(tree, "删除已选 (3)").props.onClick();
  await flushActions();
  assert.deepEqual(deleted, [["first", "second", "third"]]);
  tree = f.render();
  assert.equal(button(tree, "删除已选 (3)").props.disabled, false);
  allowDelete = true;
  button(tree, "删除已选 (3)").props.onClick();
  await flushActions();
  assert.equal(button(f.render(), "删除已选 (0)").props.disabled, true);
});

test("whole-project selection includes sessions outside the page and archive receives the retained set", async () => {
  const requested: string[] = []; const archived: string[][] = [];
  const f = fixture({
    onListProjectSessions: async (cwd: string) => { requested.push(cwd); return [session("first", cwd), session("off-page", cwd)]; },
    onArchiveMany: async (rows: ManagedSession[], archive: boolean) => { assert.equal(archive, true); archived.push(Array.from(rows, (row) => row.id).sort()); return true; },
  });
  let tree = f.render(); button(tree, "选择多个…").props.onClick();
  tree = f.render();
  const group = nodes(tree).find((node) => node.props.className === "mg-session-group-header" && text(node).includes("/projects/alpha"));
  assert.ok(group);
  button(group, "全选项目").props.onClick();
  await flushActions();
  assert.deepEqual(requested, ["/projects/alpha"]);
  tree = f.render(); button(tree, "归档已选").props.onClick();
  await flushActions();
  assert.deepEqual(archived, [["first", "off-page"]]);
  assert.equal(button(f.render(), "删除已选 (0)").props.disabled, true);
});

test("project headings expose their controlled region and can expand and collapse independently", () => {
  const f = fixture(); let tree = f.render();
  const toggle = (root: Tree) => nodes(root).find((node) => node.type === "button" && node.props.className === "mg-session-group-toggle" && text(node).includes("/projects/alpha"))!;
  const initial = toggle(tree); assert.ok(initial);
  assert.equal(initial.props["aria-expanded"], false);
  const regionId = initial.props["aria-controls"]; assert.ok(regionId);
  assert.equal(nodes(tree).find((node) => node.props.id === regionId)?.props.hidden, true);
  initial.props.onClick(); tree = f.render();
  assert.equal(toggle(tree).props["aria-expanded"], true);
  assert.equal(nodes(tree).find((node) => node.props.id === regionId)?.props.hidden, false);
  assert.equal(nodes(tree).filter((node) => node.props.className === "mg-session-row").length, 1);
  toggle(tree).props.onClick(); tree = f.render();
  assert.equal(toggle(tree).props["aria-expanded"], false);
  assert.equal(nodes(tree).filter((node) => node.props.className === "mg-session-row").length, 0);
});

test("a single project starts expanded and respects a manual collapse", () => {
  const f = fixture({ sessions: { sessions: [session("only", "/projects/alpha")], totalCount: 1, hasMore: false, offset: 0, limit: 2 } });
  let tree = f.render();
  const toggle = nodes(tree).find((node) => node.props.className === "mg-session-group-toggle");
  assert.ok(toggle); assert.equal(toggle.props["aria-expanded"], true);
  toggle.props.onClick(); tree = f.render();
  assert.equal(nodes(tree).find((node) => node.props.className === "mg-session-group-toggle")?.props["aria-expanded"], false);
  assert.equal(nodes(tree).filter((node) => node.props.className === "mg-session-row").length, 0);
});

test("Windows long-path prefixes are stripped from project headings and grouping (#2791)", () => {
  assert.equal(normalizeProjectPath("\\\\?\\C:\\Users\\me\\Documents\\ChatGPT\\my app\\"), "C:/Users/me/Documents/ChatGPT/my app");
  assert.equal(normalizeProjectPath("\\\\?\\UNC\\server\\share\\repo"), "//server/share/repo");
  assert.equal(normalizeProjectPath("\\\\server\\share\\repo"), "//server/share/repo");
  assert.equal(normalizeProjectPath("\\\\?\\E:\\"), "E:");
  assert.equal(normalizeProjectPath("/projects/alpha/"), "/projects/alpha");
  // 不是盘符或 UNC 的命名空间路径不知道如何还原，保持原样。
  assert.equal(normalizeProjectPath("\\\\?\\Volume{abc}\\repo"), "//?/Volume{abc}/repo");
  const groups = sessionGroups([session("long", "\\\\?\\E:\\Work\\new-chat"), session("plain", "E:\\Work\\new-chat\\")], "project");
  assert.equal(groups.length, 1, "带不带前缀是同一个项目");
  assert.equal(groups[0].path, "E:/Work/new-chat");
  assert.equal(groups[0].label, "new-chat");
});
