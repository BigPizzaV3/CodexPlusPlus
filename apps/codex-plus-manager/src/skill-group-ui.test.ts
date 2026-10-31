import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { customSkillGroupKey, groupSkills, legacySkillCustomGroupName, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName } from "./skill-groups.ts";

type Element = { type: any; props: Record<string, any>; children: any[] };
type FixtureSkill = { id: string; name: string; description: string; repoKey: string; installed: boolean; enabled: boolean; bundled: boolean; updateAvailable: boolean; agents: string[]; managed: boolean };
type FixtureRepo = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };

const source = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("ManagementExtra.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const page = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillsPage");
assert.ok(page);
const code = ts.transpileModule(page.getText(file), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

const skill = (id: string, repoKey: string, bundled = false): FixtureSkill => ({ id, name: id, description: "", repoKey,
  installed: true, enabled: false, bundled, updateAvailable: false, agents: [], managed: true });
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

function createHarness(saved = "{}") {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const storage = new Map([["codex-plus-plus.skill-groups.v1", saved]]);
  const prompts: Array<string | null> = [];
  const calls: string[] = [];
  const server = { status: "ok", message: "ok", skills: [skill("builtin", "", true), skill("alpha", "a/repo@main"), skill("zeta", "z/repo@main")],
    repos: [] as FixtureRepo[], repositorySkills: [], repoCounts: {} as Record<string, number>, repoErrors: [], skillsDir: "", codexSkillsDir: "" };
  let cursor = 0;
  let elements: Element[] = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }), fragment: "fragment",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useEffect: (run: () => void, deps?: any[]) => {
      const index = cursor++;
      const previous = slots[index] as any[] | undefined;
      if (!previous || !deps || deps.some((value, position) => !Object.is(value, previous[position]))) effects.push(run);
      slots[index] = deps;
    },
    useMemo: (run: () => any) => run(), useDeferredValue: (value: any) => value, startTransition: (run: () => void) => run(),
    useSensors: (...values: any[]) => values, useSensor: () => null, PointerSensor: "pointer", KeyboardSensor: "keyboard",
    sortableKeyboardCoordinates: null, closestCenter: null, verticalListSortingStrategy: null,
    localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) },
    invoke: async (command: string) => {
      calls.push(command);
      if (command !== "list_installed_skills" && command !== "refresh_skill_catalog") throw new Error(`Unexpected command: ${command}`);
      return structuredClone(server);
    },
    ok: (result: { status: string }) => result.status === "ok", SKILL_GROUP_STORAGE_KEY: "codex-plus-plus.skill-groups.v1",
    t: (value: string) => value, tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    groupSkills, legacySkillCustomGroupName, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName, customSkillGroupKey,
    matchesManagementQuery: (values: string[], query: string) => !query || values.some((value) => value?.toLowerCase().includes(query.toLowerCase())),
    sourceLabel: (item: FixtureSkill) => item.bundled ? "内置" : item.repoKey === "local" ? "本地导入" : item.repoKey ? item.repoKey.split("@", 1)[0] : "用户",
    repoUrl: () => "", MCP_AGENTS: ["codex", "claude", "grok"],
    appPrompt: async () => prompts.shift() ?? null, appConfirm: async () => true,
  };
  for (const name of ["Header", "Button", "SearchField", "AgentColumn", "MatrixCell", "Menu", "MenuItem", "SortableSkillGroup", "DndContext", "SortableContext",
    "SkillDetailDrawer", "SkillRepoDrawer", "BookOpen", "RefreshCw", "FolderOpen", "Link", "ArrowDownAZ", "ArrowDownZA", "ArrowDownUp", "ChevronDown", "GripVertical"]) context[name] = name;
  vm.createContext(context);
  vm.runInContext(code, context);
  const collect = (node: any, result: Element[]) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach((item) => collect(item, result)); return; }
    result.push(node);
    if (node.type === "SortableSkillGroup") collect(node.children[0]("handle"), result);
    else node.children?.forEach((item: any) => collect(item, result));
    if (node.props?.actions) collect(node.props.actions, result);
  };
  const render = () => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.exports.SkillsPage({ notify: async () => {} }), elements);
    effects.splice(0).forEach((run) => run());
  };
  const find = (predicate: (node: Element) => boolean) => {
    const result = elements.find(predicate);
    assert.ok(result);
    return result;
  };
  const group = (key: string) => find((node) => node.type === "SortableSkillGroup" && node.props.group.key === key);
  const groupButton = (key: string, text: string) => {
    const children: Element[] = [];
    collect(group(key).children[0]("handle"), children);
    const button = children.find((node) => node.type === "Button" && node.children.includes(text));
    assert.ok(button);
    return button;
  };
  const rowElements = (groupKey: string, repoKey: string, id: string) => {
    const children: Element[] = [];
    collect(group(groupKey).children[0]("handle"), children);
    const row = children.find((node) => node.type === "div" && node.props.className === "mg-matrix-row" && node.props.key === skillIdentityKey({ repoKey, id }));
    assert.ok(row);
    const result: Element[] = [];
    collect(row, result);
    return result;
  };
  const sort = () => find((node) => node.type === "button" && String(node.props["aria-label"] || "").startsWith("按名称排序"));
  const order = () => elements.filter((node) => node.type === "SortableSkillGroup").map((node) => node.props.group.key);
  const selectTab = (name: string) => { find((node) => node.type === "button" && node.children.includes(name)).props.onClick(); render(); };
  const drag = (from: string, to: string) => { find((node) => node.type === "DndContext").props.onDragEnd({ active: { id: from }, over: { id: to } }); render(); };
  const savedState = () => JSON.parse(storage.get("codex-plus-plus.skill-groups.v1")!);
  return { render, find, group, groupButton, rowElements, sort, order, selectTab, drag, savedState, prompts, storage, server, calls };
}

test("shared IDs support independent choices while retaining legacy assignments and bundled protection", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Manual", "Other"], assignments: { shared: "Manual" }, future: "keep" }));
  ui.server.skills = [skill("shared", "", true), skill("shared", "a/repo@main"), skill("shared", "z/repo@main"),
    skill("local-import", "local"), skill("cc-import", "cc-switch")];
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  assert.deepEqual(ui.group("builtin").props.group.items.map((item: FixtureSkill) => item.repoKey), [""]);
  assert.deepEqual(ui.group("custom:Manual").props.group.items.map((item: FixtureSkill) => item.repoKey).sort(), ["a/repo@main", "z/repo@main"]);
  assert.deepEqual(ui.group("user").props.group.items.map((item: FixtureSkill) => item.repoKey), ["local", "cc-switch"]);
  assert.ok(ui.rowElements("user", "local", "local-import").some((node) => node.type === "small" && node.children.includes("本地导入")));
  assert.ok(ui.rowElements("user", "cc-switch", "cc-import").some((node) => node.type === "small" && node.children.includes("cc-switch")));
  assert.equal(ui.rowElements("builtin", "", "shared").some((node) => node.props.className === "mg-menu-label"), false);
  assert.ok(ui.rowElements("custom:Manual", "a/repo@main", "shared").some((node) => node.props.className === "mg-menu-label"));
  assert.deepEqual(ui.savedState().assignments, { shared: "Manual" });
  assert.equal(ui.savedState().future, "keep");

  const reset = ui.rowElements("custom:Manual", "a/repo@main", "shared").find((node) => node.type === "MenuItem" && node.children.includes("按来源"));
  assert.ok(reset); reset.props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().assignments, { shared: "Manual" });
  assert.deepEqual(ui.savedState().assignmentsByIdentity, { [skillIdentityKey(skill("shared", "a/repo@main"))]: "" });
  assert.equal(ui.group("repo:a/repo").props.group.items.length, 1);
  assert.equal(ui.group("repo:z/repo").props.group.items.length, 0);
  assert.equal(ui.group("custom:Manual").props.group.items.length, 1);
  assert.equal(ui.group("builtin").props.group.items.length, 1);
  assert.equal(ui.rowElements("repo:a/repo", "a/repo@main", "shared").find((node) => node.type === "MenuItem" && node.children.includes("按来源"))?.props.checked, true);

  const assign = ui.rowElements("repo:a/repo", "a/repo@main", "shared").find((node) => node.type === "MenuItem" && node.children.includes("Other"));
  assert.ok(assign); assign.props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().assignments, { shared: "Manual" });
  assert.deepEqual(ui.savedState().assignmentsByIdentity, { [skillIdentityKey(skill("shared", "a/repo@main"))]: "custom:Other" });
  assert.equal(ui.group("custom:Manual").props.group.items.length, 1);
  assert.equal(ui.group("custom:Other").props.group.items.length, 1);
  assert.equal(ui.group("builtin").props.group.items.length, 1);
  assert.equal(ui.group("repo:a/repo").props.group.items.length, 0);

  const restored = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"));
  restored.server.skills = [...ui.server.skills].reverse();
  restored.render(); await tick(); restored.render(); restored.selectTab("已安装");
  assert.equal(restored.group("builtin").props.group.items.length, 1);
  assert.equal(restored.group("custom:Manual").props.group.items.length, 1);
  assert.equal(restored.group("custom:Other").props.group.items.length, 1);
  assert.deepEqual(restored.savedState().assignments, { shared: "Manual" });

  restored.prompts.push("Renamed");
  await restored.groupButton("custom:Other", "重命名").props.onClick(); restored.render();
  assert.equal(restored.savedState().assignmentsByIdentity[skillIdentityKey(skill("shared", "a/repo@main"))], "custom:Renamed");
  assert.equal(restored.group("custom:Renamed").props.group.items.length, 1);
  assert.equal(restored.group("custom:Manual").props.group.items.length, 1);
  await restored.groupButton("custom:Renamed", "删除分组").props.onClick(); restored.render();
  assert.equal(restored.savedState().assignmentsByIdentity[skillIdentityKey(skill("shared", "a/repo@main"))], "");
  assert.equal(restored.group("repo:a/repo").props.group.items.length, 1);
  assert.equal(restored.group("custom:Manual").props.group.items.length, 1);
  assert.deepEqual(restored.savedState().assignments, { shared: "Manual" });

  const afterDelete = createHarness(restored.storage.get("codex-plus-plus.skill-groups.v1"));
  afterDelete.server.skills = [...restored.server.skills];
  afterDelete.render(); await tick(); afterDelete.render(); afterDelete.selectTab("已安装");
  assert.equal(afterDelete.group("repo:a/repo").props.group.items.length, 1);
  assert.equal(afterDelete.group("custom:Manual").props.group.items.length, 1);
  afterDelete.prompts.push("Legacy renamed");
  await afterDelete.groupButton("custom:Manual", "重命名").props.onClick(); afterDelete.render();
  assert.deepEqual(afterDelete.savedState().assignments, { shared: "Legacy renamed" });
  assert.equal(afterDelete.group("repo:a/repo").props.group.items.length, 1);
  assert.equal(afterDelete.group("custom:Legacy renamed").props.group.items.length, 1);
  await afterDelete.groupButton("custom:Legacy renamed", "删除分组").props.onClick(); afterDelete.render();
  assert.deepEqual(afterDelete.savedState().assignments, {});
  assert.equal(afterDelete.group("repo:a/repo").props.group.items.length, 1);
  assert.equal(afterDelete.group("repo:z/repo").props.group.items.length, 1);
});

test("known repository groups stay empty in installed view after individual moves and sorting", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Manual"], assignments: {} }));
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  const move = ui.rowElements("repo:a/repo", "a/repo@main", "alpha").find((node) => node.type === "MenuItem" && node.children.includes("Manual"));
  assert.ok(move); move.props.onClick(); ui.render();
  assert.equal(ui.group("repo:a/repo").props.group.items.length, 0);
  assert.equal(ui.group("custom:Manual").props.group.items.length, 1);
  ui.sort().props.onClick(); ui.render();
  assert.ok(ui.savedState().order.includes("repo:a/repo"));
  ui.find((node) => node.type === "SearchField").props.onChange("alpha"); ui.render();
  assert.deepEqual(ui.order(), ["custom:Manual"]);
  ui.find((node) => node.type === "SearchField").props.onChange(""); ui.render();
  const restored = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"));
  restored.render(); await tick(); restored.render(); restored.selectTab("已安装");
  assert.equal(restored.group("repo:a/repo").props.group.items.length, 0);
  assert.equal(restored.group("custom:Manual").props.group.items.length, 1);
  assert.ok(restored.order().includes("repo:a/repo"));
});

test("configured empty repositories appear in installed view and retain sort and drag order", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Manual"], aliases: { "repo:a/empty": "First" } }));
  ui.server.skills = [skill("local-import", "local"), skill("cc-import", "cc-switch")];
  ui.server.repos = [
    { owner: "a", name: "empty", branch: "main", subdir: "", enabled: true },
    { owner: "a", name: "empty", branch: "dev", subdir: "skills", enabled: true },
    { owner: "z", name: "empty", branch: "main", subdir: "", enabled: false },
    { owner: "", name: "local", branch: "", subdir: "", enabled: true },
  ];
  // a/empty has no cache entry; z/empty has a cached empty scan.
  ui.server.repoCounts = { "z/empty@main": 0 };
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  assert.equal(ui.order().filter((key: string) => key === "repo:a/empty").length, 1);
  assert.equal(ui.group("repo:a/empty").props.group.label, "First");
  assert.equal(ui.group("repo:a/empty").props.group.items.length, 0);
  assert.equal(ui.group("repo:z/empty").props.group.items.length, 0);
  assert.deepEqual(ui.group("user").props.group.items.map((item: FixtureSkill) => item.repoKey), ["local", "cc-switch"]);
  assert.ok(!ui.order().includes("repo:local") && !ui.order().includes("repo:cc-switch"));

  ui.find((node) => node.type === "SearchField").props.onChange("local-import"); ui.render();
  assert.deepEqual(ui.order(), ["user"]);
  ui.sort().props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().order, ["repo:a/empty", "custom:Manual", "repo:z/empty"]);
  ui.find((node) => node.type === "SearchField").props.onChange(""); ui.render();
  ui.drag("repo:z/empty", "repo:a/empty");
  assert.deepEqual(ui.order(), ["builtin", "user", "repo:z/empty", "repo:a/empty", "custom:Manual"]);
  assert.equal(ui.savedState().sortDirection, null);

  const restored = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"));
  restored.server.skills = ui.server.skills;
  restored.server.repos = ui.server.repos;
  restored.server.repoCounts = ui.server.repoCounts;
  restored.render(); await tick(); restored.render(); restored.selectTab("已安装");
  assert.deepEqual(restored.order(), ui.order());
  assert.equal(restored.group("repo:a/empty").props.group.items.length, 0);
  assert.equal(restored.group("repo:z/empty").props.group.items.length, 0);
  restored.server.repos = restored.server.repos.filter((repo) => repo.owner !== "z");
  restored.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  await tick(); restored.render();
  assert.ok(!restored.order().includes("repo:z/empty"));
  assert.ok(restored.order().includes("repo:a/empty"));
});

test("an empty skill inventory still shows configured repositories", async () => {
  const ui = createHarness();
  ui.server.skills = [];
  ui.server.repos = [{ owner: "empty", name: "repo", branch: "main", subdir: "", enabled: true }];
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  assert.deepEqual(ui.order(), ["builtin", "user", "repo:empty/repo"]);
  assert.equal(ui.group("repo:empty/repo").props.group.items.length, 0);
  ui.find((node) => node.type === "SearchField").props.onChange("empty"); ui.render();
  assert.deepEqual(ui.order(), []);
});

test("new assignments for duplicate IDs without legacy settings affect only one repository", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Manual"], assignments: {} }));
  ui.server.skills = [skill("same", "a/repo@main"), skill("same", "z/repo@main")];
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  const move = ui.rowElements("repo:a/repo", "a/repo@main", "same").find((node) => node.type === "MenuItem" && node.children.includes("Manual"));
  assert.ok(move); move.props.onClick(); ui.render();
  assert.equal(ui.group("repo:z/repo").props.group.items.length, 1);
  assert.equal(ui.group("custom:Manual").props.group.items.length, 1);
  assert.deepEqual(ui.savedState().assignments, {});
  const restored = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"));
  restored.server.skills = [...ui.server.skills].reverse();
  restored.render(); await tick(); restored.render(); restored.selectTab("已安装");
  assert.equal(restored.group("repo:z/repo").props.group.items.length, 1);
  assert.equal(restored.group("custom:Manual").props.group.items.length, 1);
});

test("rows with ambiguous colon-joined identities remain distinct", async () => {
  const first = skill("x", "owner/repo@main:sub");
  const second = skill("sub:x", "owner/repo@main");
  assert.equal(`${first.repoKey}:${first.id}`, `${second.repoKey}:${second.id}`);
  assert.notEqual(skillIdentityKey(first), skillIdentityKey(second));
  const ui = createHarness(JSON.stringify({ names: ["Manual"], assignments: {} }));
  ui.server.skills = [first, second];
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  const move = ui.rowElements("repo:owner/repo", first.repoKey, first.id).find((node) => node.type === "MenuItem" && node.children.includes("Manual"));
  assert.ok(move); move.props.onClick(); ui.render();
  assert.deepEqual(ui.group("custom:Manual").props.group.items.map((item: FixtureSkill) => item.id), ["x"]);
  assert.deepEqual(ui.group("repo:owner/repo").props.group.items.map((item: FixtureSkill) => item.id), ["sub:x"]);
  assert.ok(ui.rowElements("repo:owner/repo", second.repoKey, second.id).length);
});

test("empty custom groups show immediately in both tabs and survive reload", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Empty"], assignments: {}, future: { keep: true } }));
  ui.render(); await tick(); ui.render();
  assert.ok(ui.order().includes("custom:Empty"));
  ui.prompts.push("New");
  await ui.find((node) => node.type === "Button" && node.children.includes("新建分组")).props.onClick();
  ui.render();
  assert.ok(ui.order().includes("custom:New"));
  assert.deepEqual(ui.savedState().future, { keep: true });
  ui.selectTab("已安装");
  assert.ok(ui.order().includes("custom:Empty"));
  assert.equal(ui.group("builtin").props.disabled, true);
  assert.equal(ui.group("user").props.disabled, true);
  const restored = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"));
  restored.render(); await tick(); restored.render();
  assert.ok(restored.order().includes("custom:New"));
  assert.deepEqual(ui.calls, ["list_installed_skills", "refresh_skill_catalog"]);
});

test("sort, reload and manual drag preserve the full order and pinned groups", async () => {
  const ui = createHarness(JSON.stringify({ names: ["middle"], assignments: {} }));
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  assert.equal(ui.sort().children[0].type, "ArrowDownUp");
  ui.sort().props.onClick(); ui.render();
  assert.deepEqual(ui.order(), ["builtin", "user", "repo:a/repo", "custom:middle", "repo:z/repo"]);
  assert.equal(ui.sort().children[0].type, "ArrowDownAZ");
  ui.sort().props.onClick(); ui.render();
  assert.deepEqual(ui.order(), ["builtin", "user", "repo:z/repo", "custom:middle", "repo:a/repo"]);
  assert.equal(ui.savedState().sortDirection, "desc");
  const restored = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"));
  restored.render(); await tick(); restored.render(); restored.selectTab("已安装");
  assert.deepEqual(restored.order(), ui.order());
  assert.equal(restored.sort().children[0].type, "ArrowDownZA");
  restored.drag("repo:z/repo", "user");
  assert.equal(restored.savedState().sortDirection, "desc");
  restored.drag("repo:a/repo", "repo:z/repo");
  assert.deepEqual(restored.order(), ["builtin", "user", "repo:a/repo", "repo:z/repo", "custom:middle"]);
  assert.equal(restored.savedState().sortDirection, null);
  const dragged = createHarness(restored.storage.get("codex-plus-plus.skill-groups.v1"));
  dragged.render(); await tick(); dragged.render(); dragged.selectTab("已安装");
  assert.deepEqual(dragged.order(), restored.order());
  assert.equal(dragged.sort().children[0].type, "ArrowDownUp");
});

test("sorting a filtered list keeps hidden groups and filtered drag is disabled", async () => {
  const ui = createHarness(JSON.stringify({ names: ["middle"], assignments: {} }));
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  ui.find((node) => node.type === "SearchField").props.onChange("alpha"); ui.render();
  assert.deepEqual(ui.order(), ["repo:a/repo"]);
  assert.equal(ui.group("repo:a/repo").props.disabled, true);
  ui.sort().props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().order, ["repo:a/repo", "custom:middle", "repo:z/repo"]);
  ui.find((node) => node.type === "SearchField").props.onChange(""); ui.render();
  assert.deepEqual(ui.order(), ["builtin", "user", "repo:a/repo", "custom:middle", "repo:z/repo"]);
});

test("an active name direction also sorts groups created after restoring it", async () => {
  const ui = createHarness(JSON.stringify({ names: ["middle"], assignments: {}, sortDirection: "desc" }));
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  ui.prompts.push("zzzz");
  await ui.find((node) => node.type === "Button" && node.children.includes("新建分组")).props.onClick();
  ui.render();
  assert.deepEqual(ui.order().slice(0, 3), ["builtin", "user", "custom:zzzz"]);
  assert.equal(ui.savedState().sortDirection, "desc");
  const restored = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"));
  restored.render(); await tick(); restored.render(); restored.selectTab("已安装");
  assert.deepEqual(restored.order(), ui.order());
});

test("renaming and deleting a custom group preserve or clear both assignment formats", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Old"], assignments: { alpha: "Old", zeta: "custom:Old" }, order: ["custom:Old"] }));
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  ui.prompts.push("New");
  await ui.groupButton("custom:Old", "重命名").props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().assignments, { alpha: "New", zeta: "New" });
  assert.deepEqual(ui.savedState().order, ["custom:New"]);
  await ui.groupButton("custom:New", "删除分组").props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().assignments, {});
  assert.ok(ui.order().includes("repo:a/repo"));
  assert.ok(ui.order().includes("repo:z/repo"));
  assert.ok(!ui.order().includes("custom:New"));
});

test("renaming a group to a legacy prefix-like name preserves other legacy assignments", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Old", "Foo"], assignments: { alpha: "Old", zeta: "custom:Foo" } }));
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  ui.prompts.push("custom:Foo");
  await ui.groupButton("custom:Old", "重命名").props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().assignments, { alpha: "custom:Foo", zeta: "Foo" });
  assert.deepEqual(ui.group("custom:custom:Foo").props.group.items.map((item: FixtureSkill) => item.id), ["alpha"]);
  assert.deepEqual(ui.group("custom:Foo").props.group.items.map((item: FixtureSkill) => item.id), ["zeta"]);
  await ui.groupButton("custom:custom:Foo", "删除分组").props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().assignments, { zeta: "Foo" });
  assert.deepEqual(ui.group("custom:Foo").props.group.items.map((item: FixtureSkill) => item.id), ["zeta"]);
});
