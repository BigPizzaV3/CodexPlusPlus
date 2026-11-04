import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { customSkillGroupKey, groupSkills, legacySkillCustomGroupName, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName } from "./skill-groups.ts";
import { mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchTargets, SkillBatchRegistry } from "./skill-agent-batch.ts";

type Element = { type: any; props: Record<string, any>; children: any[] };
type FixtureSkill = { id: string; name: string; description: string; repoKey: string; installed: boolean; enabled: boolean; bundled: boolean; updateAvailable: boolean; agents: string[]; managed: boolean };
type FixtureRepo = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };
type FixtureServer = { status: string; message: string; skills: FixtureSkill[]; repos: FixtureRepo[]; repositorySkills: FixtureSkill[]; repoCounts: Record<string, number>; repoErrors: string[]; skillsDir: string; codexSkillsDir: string };

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

function createHarness(saved = "{}", shared?: { server: FixtureServer; registry: SkillBatchRegistry }) {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const cleanups = new Map<number, () => void>();
  const storage = new Map([["codex-plus-plus.skill-groups.v1", saved]]);
  const prompts: Array<string | null> = [];
  const calls: string[] = [];
  const requests: Array<{ command: string; args: Record<string, unknown> }> = [];
  const notices: Array<{ title: string; message: string; status: string }> = [];
  const hooks: { mutation?: (command: string, args: Record<string, unknown>) => Promise<any>; list?: (command: string) => Promise<any> } = {};
  const server = shared?.server ?? { status: "ok", message: "ok", skills: [skill("builtin", "", true), skill("alpha", "a/repo@main"), skill("zeta", "z/repo@main")],
    repos: [] as FixtureRepo[], repositorySkills: [], repoCounts: {} as Record<string, number>, repoErrors: [], skillsDir: "", codexSkillsDir: "" };
  const registry = shared?.registry ?? new SkillBatchRegistry();
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
      if (!previous || !deps || deps.some((value, position) => !Object.is(value, previous[position]))) effects.push(() => {
        cleanups.get(index)?.();
        const cleanup = run();
        if (typeof cleanup === "function") cleanups.set(index, cleanup);
        else cleanups.delete(index);
      });
      slots[index] = deps;
    },
    useMemo: (run: () => any) => run(), useDeferredValue: (value: any) => value, startTransition: (run: () => void) => run(),
    useRef: (value: any) => { const index = cursor++; return slots[index] ??= { current: value }; },
    useSyncExternalStore: (_subscribe: any, snapshot: () => any) => { cursor++; return snapshot(); },
    useSensors: (...values: any[]) => values, useSensor: () => null, PointerSensor: "pointer", KeyboardSensor: "keyboard",
    sortableKeyboardCoordinates: null, closestCenter: null, verticalListSortingStrategy: null,
    localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) },
    invoke: async (command: string, args: Record<string, unknown> = {}) => {
      calls.push(command);
      requests.push({ command, args });
      if (command === "list_installed_skills" || command === "refresh_skill_catalog") return hooks.list ? hooks.list(command) : structuredClone(server);
      if (hooks.mutation) return hooks.mutation(command, args);
      throw new Error(`Unexpected command: ${command}`);
    },
    ok: (result: { status: string }) => result.status === "ok", SKILL_GROUP_STORAGE_KEY: "codex-plus-plus.skill-groups.v1",
    t: (value: string) => value, tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    groupSkills, legacySkillCustomGroupName, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName, customSkillGroupKey,
    matchesManagementQuery: (values: string[], query: string) => !query || values.some((value) => value?.toLowerCase().includes(query.toLowerCase())),
    sourceLabel: (item: FixtureSkill) => item.bundled ? "内置" : item.repoKey === "local" ? "本地导入" : item.repoKey ? item.repoKey.split("@", 1)[0] : "用户",
    repoUrl: () => "", MCP_AGENTS: ["codex", "claude", "grok"],
    // 使用模式会隐藏部分 Agent 列；这里按全部 Agent 渲染。
    agentVisible: () => true, useAgentFocus: () => "all",
    mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchTargets, skillBatchRegistry: registry,
    appPrompt: async () => prompts.shift() ?? null, appConfirm: async () => true,
  };
  for (const name of ["Header", "Button", "CxToolbar", "CxFilters", "CxRow", "CxBadge", "CxBanner", "CxCard", "CxEmpty", "CxStatusIcon", "AgentToggle", "Menu", "MenuItem", "SortableSkillGroup", "GroupAgentSelect", "DndContext", "SortableContext",
    "SkillDetailDrawer", "SkillRepoDrawer", "BookOpen", "RefreshCw", "FolderOpen", "FolderPlus", "Link", "ArrowDownAZ", "ArrowDownZA", "ArrowDownUp", "ChevronDown", "ChevronsUp", "ChevronsDown", "CloudDownload", "GripVertical", "X"]) context[name] = name;
  vm.createContext(context);
  vm.runInContext(code, context);
  const collect = (node: any, result: Element[]) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach((item) => collect(item, result)); return; }
    result.push(node);
    if (node.type === "SortableSkillGroup") collect(node.children[0]("handle"), result);
    else node.children?.forEach((item: any) => collect(item, result));
    // 骨架组件把一部分内容放在属性里：页面操作、筛选条尾部按钮、行标题旁的小标签与说明。
    for (const key of ["actions", "trailing", "badges", "detail"]) if (node.props?.[key]) collect(node.props[key], result);
  };
  const render = () => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.exports.SkillsPage({ notify: async (title: string, message: string, status: string) => { notices.push({ title, message, status }); } }), elements);
    effects.splice(0).forEach((run) => run());
  };
  const find = (predicate: (node: Element) => boolean) => {
    const result = elements.find(predicate);
    assert.ok(result);
    return result;
  };
  const group = (key: string) => find((node) => node.type === "SortableSkillGroup" && node.props.group.key === key);
  const groupHead = (key: string) => {
    const children: Element[] = [];
    collect(group(key).children[0]("handle"), children);
    const button = children.find((node) => node.type === "button" && node.props.className === "cx-skill-group-toggle");
    assert.ok(button);
    return button;
  };
  const groupButton = (key: string, text: string) => {
    const children: Element[] = [];
    collect(group(key).children[0]("handle"), children);
    const button = children.find((node) => node.type === "MenuItem" && node.children.includes(text));
    assert.ok(button);
    return button;
  };
  const groupSelect = (key: string, agent: string) => {
    const children: Element[] = [];
    collect(group(key).children[0]("handle"), children);
    const select = children.find((node) => node.type === "GroupAgentSelect" && node.props.agent === agent);
    assert.ok(select);
    return select;
  };
  const rowElements = (groupKey: string, repoKey: string, id: string) => {
    const head = groupHead(groupKey);
    if (!head.props["aria-expanded"]) { head.props.onClick(); render(); }
    const children: Element[] = [];
    collect(group(groupKey).children[0]("handle"), children);
    const row = children.find((node) => node.type === "CxRow" && node.props.className === "cx-skill-row" && node.props.key === skillIdentityKey({ repoKey, id }));
    assert.ok(row);
    const result: Element[] = [];
    collect(row, result);
    return result;
  };
  const sort = () => find((node) => node.type === "button" && String(node.props["aria-label"] || "").startsWith("按名称排序"));
  const order = () => elements.filter((node) => node.type === "SortableSkillGroup").map((node) => node.props.group.key);
  // 「已安装 / 发现」是一排筛选胶囊（CxFilters）。
  const filters = () => find((node) => node.type === "CxFilters");
  const tabId = (name: string) => filters().props.items.find((item: { id: string; label: string }) => item.label === name).id;
  const selectTab = (name: string) => { filters().props.onChange(tabId(name)); render(); };
  const activeTab = (name: string) => filters().props.value === tabId(name);
  const search = (value: string) => { find((node) => node.type === "CxToolbar").props.onQuery(value); render(); };
  const drag = (from: string, to: string) => { find((node) => node.type === "DndContext").props.onDragEnd({ active: { id: from }, over: { id: to } }); render(); };
  const savedState = () => JSON.parse(storage.get("codex-plus-plus.skill-groups.v1")!);
  const unmount = () => { for (const cleanup of cleanups.values()) cleanup(); cleanups.clear(); };
  return { render, unmount, find, group, groupHead, groupButton, groupSelect, rowElements, sort, order, selectTab, activeTab, search, drag, savedState, prompts, storage, server, registry, calls, requests, notices, hooks };
}

test("installed groups default closed and preserve interactions as new groups load", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Manual"] }));
  ui.render();
  assert.equal(ui.activeTab("已安装"), true);
  assert.equal(ui.groupHead("custom:Manual").props["aria-expanded"], false);
  ui.groupHead("custom:Manual").props.onClick(); ui.render();
  await tick(); ui.render();
  assert.equal(ui.groupHead("custom:Manual").props["aria-expanded"], true);
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], false);
  ui.prompts.push("Renamed");
  await ui.groupButton("custom:Manual", "重命名").props.onClick(); ui.render();
  assert.equal(ui.groupHead("custom:Renamed").props["aria-expanded"], true);
  ui.find((node) => node.type === "button" && node.props["aria-label"] === "全部展开").props.onClick(); ui.render();
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], true);
  ui.server.skills.push(skill("later", "later/repo@main"));
  ui.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  await tick(); ui.render();
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], true);
  assert.equal(ui.groupHead("repo:later/repo").props["aria-expanded"], false);
  ui.find((node) => node.type === "button" && node.props["aria-label"] === "全部折叠").props.onClick(); ui.render();
  assert.equal(ui.groupHead("custom:Renamed").props["aria-expanded"], false);
  ui.search("alpha");
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], true);
  ui.search("");
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], false);
  ui.selectTab("发现"); ui.selectTab("已安装");
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], false);
  const reopened = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"));
  reopened.render(); await tick(); reopened.render();
  assert.equal(reopened.activeTab("已安装"), true);
  assert.equal(reopened.groupHead("custom:Renamed").props["aria-expanded"], false);
});

test("collapsing search results preserves hidden groups' open and closed states", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Manual"] }));
  ui.server.skills.push(skill("alpha-two", "b/repo@main"));
  ui.render(); await tick(); ui.render();
  ui.groupHead("custom:Manual").props.onClick(); ui.render();
  ui.groupHead("repo:a/repo").props.onClick(); ui.render();
  assert.equal(ui.groupHead("repo:z/repo").props["aria-expanded"], false);
  ui.search("alpha");
  assert.deepEqual(ui.order(), ["repo:a/repo", "repo:b/repo"]);
  ui.find((node) => node.type === "button" && node.props["aria-label"] === "全部折叠").props.onClick(); ui.render();
  ui.search("");
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], false);
  assert.equal(ui.groupHead("repo:b/repo").props["aria-expanded"], false);
  assert.equal(ui.groupHead("repo:z/repo").props["aria-expanded"], false);
  assert.equal(ui.groupHead("custom:Manual").props["aria-expanded"], true);
});

test("collapsed installed groups support per-agent batch enable and disable", async () => {
  const ui = createHarness();
  ui.server.skills = [skill("alpha", "a/repo@main"), { ...skill("beta", "a/repo@main"), agents: ["claude"] }, skill("zeta", "z/repo@main")];
  ui.render(); await tick(); ui.render();
  assert.equal(ui.activeTab("已安装"), true);
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], false);
  ui.hooks.mutation = async (command, args) => {
    assert.equal(command, "set_skill_enabled_for");
    const item = ui.server.skills.find((entry) => entry.id === args.id)!;
    item.agents = item.agents.filter((agent) => agent !== args.agent);
    if (args.enabled) item.agents.push(String(args.agent));
    item.enabled = item.agents.includes("codex");
    return structuredClone(ui.server);
  };
  ui.groupSelect("repo:a/repo", "codex").props.onChange(true);
  for (let index = 0; index < 20 && ui.registry.getSnapshot().length; index++) await tick();
  await tick(); ui.render();
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], false);
  assert.deepEqual(ui.server.skills.map((item) => item.agents), [["codex"], ["claude", "codex"], []]);
  ui.groupSelect("repo:a/repo", "codex").props.onChange(false);
  for (let index = 0; index < 20 && ui.registry.getSnapshot().length; index++) await tick();
  await tick(); ui.render();
  assert.equal(ui.groupHead("repo:a/repo").props["aria-expanded"], false);
  assert.deepEqual(ui.server.skills.map((item) => item.agents), [[], ["claude"], []]);
  assert.deepEqual(ui.requests.filter((request) => request.command === "set_skill_enabled_for")
    .map(({ args }) => [args.agent, args.id, args.enabled]), [
    ["codex", "alpha", true], ["codex", "beta", true],
    ["codex", "alpha", false], ["codex", "beta", false],
  ]);
  ui.unmount();
});

test("group toolbar icons retain accessible labels and fold or expand groups", async () => {
  const ui = createHarness();
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  const control = (label: string) => ui.find((node) => node.type === "button" && node.props["aria-label"] === label);
  for (const [label, icon] of [["新建分组", "FolderPlus"], ["全部折叠", "ChevronsUp"], ["全部展开", "ChevronsDown"]]) {
    const button = control(label);
    assert.equal(button.props.title, label);
    assert.equal(button.children[0].type, icon);
  }
  control("全部折叠").props.onClick(); ui.render();
  for (const key of ui.order()) assert.equal(ui.groupHead(key).props["aria-expanded"], false);
  control("全部展开").props.onClick(); ui.render();
  for (const key of ui.order()) assert.equal(ui.groupHead(key).props["aria-expanded"], true);
});

test("queued updates survive remount and invalidate an older list response", async () => {
  const ui = createHarness();
  ui.server.skills = ["alpha", "beta", "gamma"].map((id) => skill(id, "a/repo@main"));
  ui.server.skills.forEach((item) => { item.updateAvailable = true; });
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  let finishAlpha!: () => void;
  let finishBeta!: () => void;
  ui.hooks.mutation = async (_command, args) => {
    if (args.id === "alpha") await new Promise<void>((resolve) => { finishAlpha = resolve; });
    if (args.id === "beta") await new Promise<void>((resolve) => { finishBeta = resolve; });
    const item = ui.server.skills.find((entry) => entry.id === args.id)!;
    item.updateAvailable = false;
    return structuredClone(ui.server);
  };
  const button = (page: typeof ui, id: string, label: string) => {
    const found = page.rowElements("repo:a/repo", "a/repo@main", id).find((node) => node.type === "Button" && node.children.includes(label));
    assert.ok(found, `${id} should display ${label}`);
    return found;
  };
  button(ui, "alpha", "更新").props.onClick();
  button(ui, "beta", "更新").props.onClick();
  button(ui, "gamma", "更新").props.onClick();
  await tick(); ui.render();
  assert.deepEqual(ui.registry.getUpdates().map(({ id, state }) => [id, state]), [
    ["alpha", "running"], ["beta", "running"], ["gamma", "queued"],
  ]);
  assert.equal(button(ui, "alpha", "更新中").props.disabled, true);
  assert.equal(button(ui, "beta", "更新中").props.disabled, true);
  assert.equal(button(ui, "gamma", "排队中").props.disabled, true);
  ui.unmount();

  const reopened = createHarness("{}", { server: ui.server, registry: ui.registry });
  reopened.render(); await tick(); reopened.render(); reopened.selectTab("已安装");
  assert.equal(button(reopened, "alpha", "更新中").props.disabled, true);
  assert.equal(button(reopened, "beta", "更新中").props.disabled, true);
  assert.equal(button(reopened, "gamma", "排队中").props.disabled, true);

  const stale = structuredClone(ui.server);
  let oldList!: (value: FixtureServer) => void;
  reopened.hooks.list = (command) => command === "refresh_skill_catalog" ? new Promise((resolve) => { oldList = resolve; }) : Promise.resolve(structuredClone(ui.server));
  reopened.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  await tick();
  assert.equal(typeof oldList, "function");
  reopened.hooks.list = undefined;

  finishBeta();
  for (let i = 0; i < 20 && ui.registry.getUpdates().length !== 1; i++) await tick();
  assert.deepEqual(ui.registry.getUpdates().map(({ id, state }) => [id, state]), [["alpha", "running"]]);
  assert.deepEqual(ui.requests.filter((entry) => entry.command === "update_skill").map((entry) => entry.args.id), ["alpha", "beta", "gamma"]);
  reopened.render();
  assert.equal(button(reopened, "alpha", "更新中").props.disabled, true);
  assert.deepEqual(reopened.group("repo:a/repo").props.group.items.map((item: FixtureSkill) => [item.id, item.updateAvailable]),
    [["alpha", true], ["beta", false], ["gamma", false]]);

  finishAlpha();
  for (let i = 0; i < 20 && ui.registry.getUpdates().length; i++) await tick();
  assert.equal(ui.registry.getUpdates().length, 0);
  oldList(stale);
  await tick(); await tick(); reopened.render(); reopened.selectTab("已安装");
  assert.deepEqual(reopened.group("repo:a/repo").props.group.items.map((item: FixtureSkill) => [item.id, item.updateAvailable]),
    [["alpha", false], ["beta", false], ["gamma", false]]);
  for (const id of ["alpha", "beta", "gamma"]) {
    assert.equal(reopened.rowElements("repo:a/repo", "a/repo@main", id).some((node) => node.type === "Button" && node.children.includes("更新")), false);
  }
  reopened.unmount();
});

test("an Agent batch can finish while another Skill update is downloading", async () => {
  const ui = createHarness();
  ui.server.skills = [skill("alpha", "a/repo@main"), skill("beta", "b/repo@main")];
  ui.server.skills[0].updateAvailable = true;
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  let finish!: () => void;
  ui.hooks.mutation = async (command, args) => {
    const item = ui.server.skills.find((entry) => entry.id === args.id)!;
    if (command === "update_skill") {
      await new Promise<void>((resolve) => { finish = resolve; });
      item.updateAvailable = false;
    } else {
      assert.equal(command, "set_skill_enabled_for");
      item.agents = ["codex"];
      item.enabled = true;
    }
    return structuredClone(ui.server);
  };
  const update = ui.rowElements("repo:a/repo", "a/repo@main", "alpha")
    .find((node) => node.type === "Button" && node.children.includes("更新"));
  assert.ok(update);
  update.props.onClick();
  await tick(); ui.render();
  try {
    ui.groupSelect("repo:b/repo", "codex").props.onChange(true);
    for (let i = 0; i < 20 && ui.registry.getSnapshot().length; i++) await tick();
    assert.equal(ui.registry.getSnapshot().length, 0);
    assert.equal(ui.server.skills[1].enabled, true);
    assert.deepEqual(ui.requests.filter((entry) => entry.command === "set_skill_enabled_for").map((entry) => entry.args.id), ["beta"]);
    assert.deepEqual(ui.registry.getUpdates().map(({ id, state }) => [id, state]), [["alpha", "running"]]);
  } finally {
    finish();
    for (let i = 0; i < 20 && (ui.registry.getUpdates().length || ui.registry.getSnapshot().length); i++) await tick();
    ui.unmount();
  }
});

test("batch selection queues other writes and summarizes a large set of failures", async () => {
  const ui = createHarness();
  ui.server.skills = ["alpha", "beta", "gamma", "delta", "epsilon"].map((id) => skill(id, "a/repo@main"));
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  let release!: () => void;
  ui.hooks.mutation = async (command, args) => {
    if (command === "import_ccs_skills") return structuredClone(ui.server);
    assert.equal(command, "set_skill_enabled_for");
    if (args.id === "alpha") {
      await new Promise<void>((resolve) => { release = resolve; });
      const item = ui.server.skills.find((entry) => entry.id === "alpha")!;
      item.enabled = true;
      item.agents = ["codex"];
      return structuredClone(ui.server);
    }
    if (args.id === "gamma") throw new Error("gamma failed");
    return { ...structuredClone(ui.server), status: "error", message: `${args.id} failed ${"x".repeat(300)}` };
  };
  ui.groupSelect("repo:a/repo", "codex").props.onChange(true);
  await tick(); ui.render();
  assert.equal(ui.groupSelect("repo:a/repo", "codex").props.busy, true);
  assert.equal(ui.groupSelect("repo:a/repo", "claude").props.busy, false);
  const importButton = ui.find((node) => node.type === "MenuItem" && node.children.includes("从 cc-switch 导入"));
  assert.notEqual(importButton.props.disabled, true);
  importButton.props.onClick();
  await tick();
  assert.equal(ui.requests.some((request) => request.command === "import_ccs_skills"), false);
  release();
  for (let i = 0; i < 20 && !ui.notices.some((notice) => notice.status === "failed"); i++) await tick();
  const mutations = ui.requests.filter((request) => request.command === "set_skill_enabled_for" || request.command === "import_ccs_skills");
  assert.equal(mutations[0].args.id, "alpha");
  assert.equal(mutations[1].command, "import_ccs_skills");
  assert.deepEqual(mutations.filter((request) => request.command === "set_skill_enabled_for").map((request) => request.args.id),
    ["alpha", "beta", "gamma", "delta", "epsilon"]);
  const failure = ui.notices.find((notice) => notice.status === "failed");
  assert.ok(failure);
  assert.match(failure.message, /已批量启用 1 个 Skill；4 个 Skill 失败/);
  assert.match(failure.message, /beta.*gamma.*delta/);
  assert.doesNotMatch(failure.message, /epsilon/);
  assert.ok(failure.message.length < 800);
});

test("filtered batch survives page remount and changes only visible Skills", async () => {
  const ui = createHarness();
  ui.server.skills = [skill("alpha", "a/repo@main"), skill("beta", "a/repo@main")];
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  ui.search("alpha");
  let release!: () => void;
  ui.hooks.mutation = async (command, args) => {
    assert.equal(command, "set_skill_enabled_for");
    assert.equal(args.id, "alpha");
    await new Promise<void>((resolve) => { release = resolve; });
    ui.server.skills[0].enabled = true;
    ui.server.skills[0].agents = ["codex"];
    return structuredClone(ui.server);
  };
  ui.groupSelect("repo:a/repo", "codex").props.onChange(true);
  await tick();
  ui.unmount();
  const reopened = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"), { server: ui.server, registry: ui.registry });
  reopened.render(); await tick(); reopened.render(); reopened.selectTab("已安装");
  assert.equal(reopened.groupSelect("repo:a/repo", "codex").props.busy, true);
  assert.equal(reopened.groupSelect("repo:a/repo", "claude").props.busy, false);
  release();
  for (let i = 0; i < 20 && ui.registry.getSnapshot().length; i++) await tick();
  await tick(); reopened.render();
  assert.equal(reopened.groupSelect("repo:a/repo", "codex").props.busy, false);
  const alpha = reopened.rowElements("repo:a/repo", "a/repo@main", "alpha").find((node) => node.type === "AgentToggle" && node.props.agent === "codex");
  const beta = reopened.rowElements("repo:a/repo", "a/repo@main", "beta").find((node) => node.type === "AgentToggle" && node.props.agent === "codex");
  assert.equal(alpha?.props.enabled, true);
  assert.equal(beta?.props.enabled, false);
  assert.deepEqual(ui.requests.filter((request) => request.command === "set_skill_enabled_for").map((request) => request.args.id), ["alpha"]);
  reopened.unmount();
});

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
  ui.search("alpha");
  assert.deepEqual(ui.order(), ["custom:Manual"]);
  ui.search("");
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

  ui.search("local-import");
  assert.deepEqual(ui.order(), ["user"]);
  ui.sort().props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().order, ["repo:a/empty", "custom:Manual", "repo:z/empty"]);
  ui.search("");
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
  ui.search("empty");
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
  await ui.find((node) => node.type === "button" && node.props["aria-label"] === "新建分组").props.onClick();
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
  ui.search("alpha");
  assert.deepEqual(ui.order(), ["repo:a/repo"]);
  assert.equal(ui.group("repo:a/repo").props.disabled, true);
  ui.sort().props.onClick(); ui.render();
  assert.deepEqual(ui.savedState().order, ["repo:a/repo", "custom:middle", "repo:z/repo"]);
  ui.search("");
  assert.deepEqual(ui.order(), ["builtin", "user", "repo:a/repo", "custom:middle", "repo:z/repo"]);
});

test("an active name direction also sorts groups created after restoring it", async () => {
  const ui = createHarness(JSON.stringify({ names: ["middle"], assignments: {}, sortDirection: "desc" }));
  ui.render(); await tick(); ui.render(); ui.selectTab("已安装");
  ui.prompts.push("zzzz");
  await ui.find((node) => node.type === "button" && node.props["aria-label"] === "新建分组").props.onClick();
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
