import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { aggregateSkillGroupItems, customSkillGroupKey, groupSkills, legacySkillCustomGroupName, migrateSkillGroupAssignments, migrateSkillInstanceAssignments, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillGroupIdentityKey, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName } from "./skill-groups.ts";
import { mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchTargets, skillCodexStatusError, SkillBatchRegistry } from "./skill-agent-batch.ts";

type Element = { type: any; props: Record<string, any>; children: any[] };
type FixtureSkill = { id: string; name: string; description: string; repoKey: string; installed: boolean; enabled: boolean; bundled: boolean; updateAvailable: boolean; agents: string[]; managed: boolean; originalDir?: string | null; updateOwner?: string; instanceKey?: string; conflict?: string | null; conflictKind?: "independent" | "equal-independent" | "managed-modified" | null; safeDisableAgents?: string[]; locations?: Array<{ manifestPath: string; rootKind: string; managed: boolean; enabled: boolean; statusError?: string | null; originalPath?: string | null; associationPath?: string | null; selectable?: boolean }>; displayInstances?: FixtureSkill[] };
type FixtureRepo = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };
type FixtureBackup = { id: string; skillId: string; name: string; backedUpAt: string; backupPath?: string | null; restorePath?: string | null; purpose?: string | null; restoreBlockedReason?: string | null; deleteWarning?: string | null };
type FixtureServer = { status: string; message: string; skills: FixtureSkill[]; inventoryOk: boolean; repos: FixtureRepo[]; repositorySkills: FixtureSkill[]; repoCounts: Record<string, number>; repoErrors: string[]; skillsDir: string; codexSkillsDir: string; backups: FixtureBackup[]; backupsOk: boolean };

const source = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("ManagementExtra.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const page = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillsPage");
assert.ok(page);
const code = ts.transpileModule(page.getText(file), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const cacheListener = file.statements.find((node) => ts.isExpressionStatement(node)
  && node.getText(file).startsWith("skillBatchRegistry.subscribeSettled("));
assert.ok(cacheListener);
const cacheListenerCode = ts.transpileModule(cacheListener.getText(file), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;
const sourceLabelNode = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "sourceLabel");
assert.ok(sourceLabelNode);
const sourceLabelCode = ts.transpileModule(sourceLabelNode.getText(file), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const detailNode = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillDetailDrawer");
assert.ok(detailNode);
const detailCode = ts.transpileModule(detailNode.getText(file), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", target: ts.ScriptTarget.ES2022 },
}).outputText;
const backupListNode = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillBackupList");
assert.ok(backupListNode);
const backupListCode = ts.transpileModule(backupListNode.getText(file), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", target: ts.ScriptTarget.ES2022 },
}).outputText;

const skill = (id: string, repoKey: string, bundled = false): FixtureSkill => ({ id, name: id, description: "", repoKey,
  installed: true, enabled: false, bundled, updateAvailable: false, agents: [], managed: true });
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test("legacy cc-switch markers do not claim a verified upstream source", () => {
  const context = { t: (value: string) => value, LOCAL_IMPORT_REPO_KEY: "local" };
  vm.createContext(context);
  vm.runInContext(sourceLabelCode, context);
  const label = (context as any).sourceLabel;
  assert.equal(label(skill("legacy", "cc-switch")), "本地");
  assert.equal(label(skill("local", "")), "本地");
  assert.equal(label({ ...skill("conflict", ""), conflict: "内容不同" }), "本地");
  assert.equal(label(skill("import", "local")), "本地导入");
  assert.equal(label(skill("bundled", "", true)), "内置");
  assert.equal(label(skill("repository", "owner/repo@main")), "owner/repo");
});

test("backup list shows the backup location separately from its restore destination", () => {
  const context = { jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, Button: "Button", Trash2: "Trash2", SkillFileTimes: "SkillFileTimes" };
  vm.createContext(context);
  const render = vm.runInContext(`${backupListCode}\nSkillBackupList`, context);
  const selected: string[] = [];
  const backup = { id: "backup", skillId: "alpha", name: "alpha", backedUpAt: "today",
    backupPath: "C:/backups/alpha", restorePath: "C:/skills/alpha" };
  const root = render({ backups: [backup], busy: false, onRestore: () => {}, onDelete: (item: FixtureBackup) => selected.push(item.id) });
  const nodes: Element[] = [];
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    nodes.push(node);
    node.children?.forEach(collect);
  };
  collect(root);
  assert.ok(nodes.some((node) => node.type === "small" && node.children.includes("C:/backups/alpha")));
  assert.ok(nodes.some((node) => node.type === "small" && node.children.includes("C:/skills/alpha")));
  assert.equal(nodes.find((node) => node.type === "SkillFileTimes")?.props.backupId, "backup");
  nodes.find((node) => node.type === "button" && node.props.title === "删除备份")!.props.onClick();
  assert.deepEqual(selected, ["backup"]);
});

test("an incomplete update backup remains deletable but cannot be restored", () => {
  const context = { jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, Button: "Button", Trash2: "Trash2", SkillFileTimes: "SkillFileTimes" };
  vm.createContext(context);
  const render = vm.runInContext(`${backupListCode}\nSkillBackupList`, context);
  const root = render({ backups: [{ id: "backup", skillId: "alpha", name: "alpha", backedUpAt: "today",
    backupPath: "C:/backups/alpha", restoreBlockedReason: "更新备份组已有成员被删除，无法整体恢复" }],
  busy: false, onRestore: () => {}, onDelete: () => {} });
  const nodes: Element[] = [];
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    nodes.push(node);
    node.children?.forEach(collect);
  };
  collect(root);
  assert.equal(nodes.find((node) => node.type === "Button" && node.children.includes("恢复"))?.props.disabled, true);
  assert.equal(nodes.find((node) => node.type === "button" && node.props.title === "删除备份")?.props.disabled, false);
});

test("grouped details keep source matching, updates and impeccable versions tied to their locations", () => {
  const first = { ...skill("impeccable", ""), instanceKey: "first", installChannel: "local",
    locations: [{ manifestPath: "C:/shared/impeccable/SKILL.md", originalPath: "C:/shared/impeccable", selectable: true, rootKind: "shared", managed: false, enabled: true }] };
  const second = { ...skill("impeccable", "owner/repo@main"), instanceKey: "second", repoPath: "skills/impeccable",
    updateOwner: "in_place", installChannel: "github",
    locations: [{ manifestPath: "C:/claude/impeccable/SKILL.md", originalPath: "C:/claude/impeccable", selectable: true, rootKind: "claude", managed: false, enabled: true }] };
  const selected: string[] = [];
  const updates: string[][] = [];
  const context: Record<string, any> = {
    jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }), fragment: "fragment",
    t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: (item: { installChannel: string }) => item.installChannel,
    repoFromKey: (value: string) => value.includes("/") ? {} : null, skillIdentityKey,
  };
  for (const name of ["ManagementDrawer", "Button", "ImpeccableVersions", "CloudDownload", "Search", "ArrowRight", "ExternalLink", "SkillBackupList", "SkillFileTimes"]) context[name] = name;
  vm.createContext(context);
  const render = vm.runInContext(`${detailCode}\nSkillDetailDrawer`, context);
  const root = render({ skill: { ...first, conflict: "内容不同", locations: [...first.locations, ...second.locations], displayInstances: [first, second] },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: (path: string) => selected.push(path), onRestore: () => {}, onUpdate: (repo: string, path: string) => updates.push([repo, path]), onReaPreview: () => {} });
  const elements: Element[] = [];
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    elements.push(node);
    node.children?.forEach(collect);
  };
  collect(root);
  const matchButtons = elements.filter((node) => node.type === "Button" && node.children.some((child) =>
    child === "关联此安装位置" || child === "重新关联此安装位置"));
  assert.equal(matchButtons.length, 2);
  assert.deepEqual(elements.filter((node) => node.type === "SkillFileTimes").map((node) => node.props.rootKind), ["shared", "claude"]);
  matchButtons[0].props.onClick();
  matchButtons[1].props.onClick();
  assert.deepEqual(selected, ["C:/shared/impeccable", "C:/claude/impeccable"]);
  elements.find((node) => node.type === "Button" && node.children.includes("预览此位置更新"))!.props.onClick();
  assert.deepEqual(updates, [["owner/repo@main", "C:/claude/impeccable"]]);
  assert.deepEqual(elements.filter((node) => node.type === "ImpeccableVersions").map((node) => node.props.instanceKey), ["first", "second"]);
});

test("conflicting managed installs can associate their explicit source directory", () => {
  const context: Record<string, any> = {
    jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }), fragment: "fragment",
    t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "local",
    repoFromKey: () => null, skillIdentityKey,
  };
  for (const name of ["ManagementDrawer", "Button", "ImpeccableVersions", "CloudDownload", "Search", "ArrowRight", "ExternalLink", "SkillBackupList", "SkillFileTimes"]) context[name] = name;
  vm.createContext(context);
  const render = vm.runInContext(`${detailCode}\nSkillDetailDrawer`, context);
  const selected: string[] = [];
  const root = render({ skill: { ...skill("alpha", ""), conflict: "内容不同", conflictKind: "managed-modified",
    locations: [{ manifestPath: "C:/codex/alpha/SKILL.md", associationPath: "C:/managed/alpha",
      rootKind: "codex", managed: true, enabled: true }] },
  backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
  onMatch: (path: string) => selected.push(path), onRestore: () => {}, onUpdate: () => {}, onReaPreview: () => {} });
  const elements: Element[] = [];
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    elements.push(node);
    node.children?.forEach(collect);
  };
  collect(root);
  assert.deepEqual(elements.filter((node) => node.type === "SkillFileTimes").map((node) => node.props.rootKind), ["codex", "source"]);
  const button = elements.find((node) => node.type === "Button" && node.children.includes("关联受控源"));
  assert.ok(button);
  button.props.onClick();
  assert.deepEqual(selected, ["C:/managed/alpha"]);
});

test("shared skills do not offer uninstall while local Codex skills still do", async () => {
  const ui = createHarness();
  const shared = { manifestPath: "C:/agents/shared/SKILL.md", rootKind: "shared", managed: false, enabled: true };
  const codex = { manifestPath: "C:/codex/shared/SKILL.md", rootKind: "codex", managed: true, enabled: true };
  ui.server.skills = [
    { ...skill("shared-only", ""), managed: false, agents: ["codex"], locations: [shared] },
    { ...skill("shared-and-copy", ""), agents: ["codex"], locations: [shared, codex] },
    { ...skill("codex-only", ""), agents: ["codex"], locations: [codex] },
  ];
  ui.render(); await tick(); ui.render();
  for (const id of ["shared-only", "shared-and-copy"]) {
    assert.equal(ui.rowElements("user", "", id).some((node) => node.type === "MenuItem" && node.children.includes("卸载")), false);
  }
  assert.equal(ui.rowElements("user", "", "codex-only").some((node) => node.type === "MenuItem" && node.children.includes("卸载")), true);
  ui.unmount();
});

type FixtureCache = { value: FixtureServer | null; localStartedAt: number; catalogStartedAt: number;
  localPending: Promise<FixtureServer> | null; catalogPending: Promise<FixtureServer> | null; mounted: number };

function createHarness(saved = "{}", shared?: { server: FixtureServer; registry: SkillBatchRegistry; cache?: FixtureCache }) {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const cleanups = new Map<number, () => void>();
  const storage = new Map([["codex-plus-plus.skill-groups.v1", saved]]);
  const prompts: Array<string | null> = [];
  const calls: string[] = [];
  const requests: Array<{ command: string; args: Record<string, unknown> }> = [];
  const notices: Array<{ title: string; message: string; status: string }> = [];
  const hooks: { mutation?: (command: string, args: Record<string, unknown>) => Promise<any>; list?: (command: string) => Promise<any>; confirm?: (message: string) => Promise<boolean> } = {};
  const server = shared?.server ?? { status: "ok", message: "ok", skills: [skill("builtin", "", true), skill("alpha", "a/repo@main"), skill("zeta", "z/repo@main")],
    inventoryOk: true, repos: [] as FixtureRepo[], repositorySkills: [], repoCounts: {} as Record<string, number>, repoErrors: [], skillsDir: "", codexSkillsDir: "", backups: [] as FixtureBackup[], backupsOk: true };
  const registry = shared?.registry ?? new SkillBatchRegistry();
  const cache: FixtureCache = shared?.cache ?? { value: null, localStartedAt: 0, catalogStartedAt: 0, localPending: null, catalogPending: null, mounted: 0 };
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
    aggregateSkillGroupItems, groupSkills, legacySkillCustomGroupName, migrateSkillGroupAssignments, migrateSkillInstanceAssignments, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillGroupIdentityKey, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName, customSkillGroupKey,
    matchesManagementQuery: (values: string[], query: string) => !query || values.some((value) => value?.toLowerCase().includes(query.toLowerCase())),
    sourceLabel: (item: FixtureSkill) => item.displayInstances ? "多个 Agent 安装" : item.bundled ? "内置" : item.repoKey === "cc-switch" || !item.repoKey ? "本地" : item.repoKey === "local" ? "本地导入" : item.repoKey.split("@", 1)[0],
    repoUrl: () => "", MCP_AGENTS: ["codex", "claude", "grok"],
    // 使用模式会隐藏部分 Agent 列；这里按全部 Agent 渲染。
    agentVisible: () => true, useAgentFocus: () => "all",
    mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchTargets, skillCodexStatusError, skillBatchRegistry: registry, skillPageCache: cache,
    appPrompt: async () => prompts.shift() ?? null, appConfirm: async (message: string) => hooks.confirm ? hooks.confirm(message) : true,
  };
  for (const name of ["Header", "Button", "CxToolbar", "CxFilters", "CxRow", "CxBadge", "CxBanner", "CxCard", "CxEmpty", "CxStatusIcon", "AgentToggle", "Menu", "MenuItem", "SortableSkillGroup", "GroupAgentSelect", "DndContext", "SortableContext",
    "SkillDetailDrawer", "SkillBackupsDrawer", "SkillRepoDrawer", "SkillSourceMatchDrawer", "SkillDuplicatesDrawer", "SkillInPlaceUpdateDrawer", "ReaUpdatePreviewDrawer", "BookOpen", "RefreshCw", "Search", "FolderOpen", "FolderPlus", "Link", "ArrowDownAZ", "ArrowDownZA", "ArrowDownUp", "ChevronDown", "ChevronsUp", "ChevronsDown", "CloudDownload", "GripVertical", "X"]) context[name] = name;
  vm.createContext(context);
  if (!shared?.cache) vm.runInContext(cacheListenerCode, context);
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
  const flushEffects = () => effects.splice(0).forEach((run) => run());
  const render = (flush = true) => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.exports.SkillsPage({ notify: async (title: string, message: string, status: string) => { notices.push({ title, message, status }); } }), elements);
    if (flush) flushEffects();
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
  const rowElements = (groupKey: string, repoKey: string, id: string, identity?: Pick<FixtureSkill, "instanceKey" | "conflict">) => {
    const head = groupHead(groupKey);
    if (!head.props["aria-expanded"]) { head.props.onClick(); render(); }
    const children: Element[] = [];
    collect(group(groupKey).children[0]("handle"), children);
    const row = children.find((node) => node.type === "CxRow" && node.props.className === "cx-skill-row" && node.props.key === skillIdentityKey({ repoKey, id, ...identity }));
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
  return { render, flushEffects, unmount, find, group, groupHead, groupButton, groupSelect, rowElements, sort, order, selectTab, activeTab, search, drag, savedState, prompts, storage, server, registry, cache, calls, requests, notices, hooks };
}

test("reopening shows the last committed inventory until the background scan completes", async () => {
  const first = createHarness();
  let finishFirstScan!: (result: FixtureServer) => void;
  first.hooks.list = (command) => command === "list_installed_skills"
    ? new Promise((resolve) => { finishFirstScan = resolve; }) : Promise.resolve(structuredClone(first.server));
  first.render();
  assert.equal(first.order().includes("repo:a/repo"), false);
  finishFirstScan(structuredClone(first.server));
  await tick(); first.render();
  assert.ok(first.order().includes("repo:a/repo"));
  first.unmount();

  first.server.skills = [skill("new", "new/repo@main")];
  first.cache.localStartedAt = 0;
  first.cache.catalogStartedAt = 0;
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  let finishScan!: (result: FixtureServer) => void;
  reopened.hooks.list = (command) => command === "list_installed_skills"
    ? new Promise((resolve) => { finishScan = resolve; }) : Promise.resolve(structuredClone(first.server));
  reopened.render();
  assert.ok(reopened.order().includes("repo:a/repo"));
  assert.equal(reopened.order().includes("repo:new/repo"), false);
  finishScan(structuredClone(first.server));
  await tick(); reopened.render();
  assert.ok(reopened.order().includes("repo:new/repo"));
  assert.equal(reopened.order().includes("repo:a/repo"), false);
  reopened.unmount();
});

test("failed reads preserve the previous inventory and backups while valid partial results update", async () => {
  const first = createHarness();
  first.server.backups = [{ id: "backup", skillId: "alpha", name: "alpha", backedUpAt: "today" }];
  first.server.repos = [{ owner: "empty", name: "repo", branch: "main", subdir: "", enabled: true }];
  first.render(); await tick(); first.render(); first.unmount();
  first.cache.localStartedAt = 0;
  first.cache.catalogStartedAt = 0;
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  let finishRefresh!: (result: FixtureServer) => void;
  reopened.hooks.list = (command) => command === "list_installed_skills"
    ? Promise.resolve({ ...structuredClone(first.server), status: "failed", message: "读取失败", skills: [], repos: [], inventoryOk: false,
      backups: [], backupsOk: false, repoErrors: ["读取失败"] })
    : new Promise((resolve) => { finishRefresh = resolve; });
  reopened.render(); await tick(); reopened.render();
  assert.ok(reopened.order().includes("repo:a/repo"));
  assert.ok(reopened.order().includes("repo:empty/repo"));
  assert.equal(reopened.find((node) => node.type === "CxBanner").props.title, "Skill 数据读取失败");
  reopened.find((node) => node.type === "MenuItem" && node.children.includes("查看 Skill 备份")).props.onClick();
  reopened.render();
  assert.equal(reopened.find((node) => node.type === "SkillBackupsDrawer").props.backups[0].id, "backup");
  assert.ok(reopened.notices.some((notice) => notice.message === "读取失败"));
  finishRefresh({ ...structuredClone(first.server), status: "failed", message: "仓库读取失败", skills: [],
    backups: [], repoErrors: ["仓库读取失败"] });
  await tick(); reopened.render();
  assert.equal(reopened.order().includes("repo:a/repo"), false);
  assert.equal(reopened.find((node) => node.type === "SkillBackupsDrawer").props.backups.length, 0);
  assert.ok(reopened.notices.some((notice) => notice.message === "仓库读取失败"));
  reopened.unmount();
});

test("a rejected scan retains the cached view and reports the failure", async () => {
  const first = createHarness();
  first.render(); await tick(); first.render(); first.unmount();
  first.cache.localStartedAt = 0;
  first.cache.catalogStartedAt = 0;
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  let finishRefresh!: (result: FixtureServer) => void;
  reopened.hooks.list = (command) => command === "list_installed_skills"
    ? Promise.reject(new Error("scan unavailable")) : new Promise((resolve) => { finishRefresh = resolve; });
  reopened.render(); await tick(); reopened.render();
  assert.ok(reopened.order().includes("repo:a/repo"));
  assert.ok(reopened.notices.some((notice) => notice.message.includes("scan unavailable")));
  finishRefresh(structuredClone(first.server));
  await tick(); reopened.render();
  reopened.unmount();
});

test("an old scan after a cached page mutation cannot replace the new state", async () => {
  const first = createHarness();
  first.server.skills = [skill("alpha", "a/repo@main")];
  first.render(); await tick(); first.render(); first.unmount();
  first.cache.localStartedAt = 0;
  first.cache.catalogStartedAt = 0;
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  const stale = structuredClone(first.server);
  let finishScan!: (result: FixtureServer) => void;
  let initial = true;
  reopened.hooks.list = (command) => {
    if (initial && command === "list_installed_skills") {
      initial = false;
      return new Promise((resolve) => { finishScan = resolve; });
    }
    return Promise.resolve(structuredClone(first.server));
  };
  reopened.hooks.mutation = async (_command, args) => {
    first.server.skills[0].agents = args.enabled ? ["codex"] : [];
    first.server.skills[0].enabled = Boolean(args.enabled);
    return structuredClone(first.server);
  };
  reopened.render();
  const toggle = reopened.rowElements("repo:a/repo", "a/repo@main", "alpha")
    .find((node) => node.type === "AgentToggle" && node.props.agent === "codex")!;
  await toggle.props.onToggle(); reopened.render();
  finishScan(stale);
  await tick(); reopened.render();
  const enabled = reopened.rowElements("repo:a/repo", "a/repo@main", "alpha")
    .find((node) => node.type === "AgentToggle" && node.props.agent === "codex")!;
  assert.equal(enabled.props.enabled, true);
  assert.equal(reopened.cache.value?.skills[0].enabled, true);
  reopened.unmount();
});

test("quick page switches reuse recent scans, while a manual refresh runs immediately", async () => {
  const first = createHarness();
  first.render(); await tick(); first.render(); first.unmount();
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  reopened.render(); await tick(); reopened.render();
  assert.ok(reopened.order().includes("repo:a/repo"));
  assert.deepEqual(reopened.calls, []);
  reopened.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  await tick();
  assert.deepEqual(reopened.calls, ["refresh_skill_catalog"]);
  reopened.unmount();
});

test("catalog refresh shows progress without confusing a local scan, and clears it on failure", async () => {
  const ui = createHarness();
  let finishLocal!: (result: FixtureServer) => void;
  let finishCatalog!: (result: FixtureServer) => void;
  ui.hooks.list = (command) => new Promise((resolve) => {
    if (command === "list_installed_skills") finishLocal = resolve;
    else finishCatalog = resolve;
  });
  ui.render(); ui.render();
  assert.equal(ui.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.disabled, true);
  finishLocal(structuredClone(ui.server));
  await tick(); ui.render();
  assert.equal(ui.find((node) => node.type === "Button" && node.children.includes("刷新目录中")).props.disabled, true);
  finishCatalog(structuredClone(ui.server));
  await tick(); ui.render();
  assert.equal(ui.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.disabled, false);

  let failRefresh!: (error: Error) => void;
  ui.hooks.list = () => new Promise((_resolve, reject) => { failRefresh = reject; });
  ui.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  ui.render();
  assert.equal(ui.find((node) => node.type === "Button" && node.children.includes("刷新目录中")).props.disabled, true);
  failRefresh(new Error("catalog unavailable"));
  await tick(); ui.render();
  assert.equal(ui.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.disabled, false);
  assert.ok(ui.notices.some((notice) => notice.message.includes("catalog unavailable")));
  ui.unmount();
});

test("a catalog refresh remains visible when reopening the page", async () => {
  const first = createHarness();
  first.render(); await tick(); first.render();
  let finishRefresh!: (result: FixtureServer) => void;
  first.hooks.list = () => new Promise((resolve) => { finishRefresh = resolve; });
  first.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  first.unmount();

  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  reopened.render();
  assert.equal(reopened.find((node) => node.type === "Button" && node.children.includes("刷新目录中")).props.disabled, true);
  assert.deepEqual(reopened.calls, []);
  finishRefresh(structuredClone(first.server));
  await tick(); reopened.render();
  assert.equal(reopened.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.disabled, false);
  reopened.unmount();
});

test("a refresh settling before mount effects does not leave its progress stuck", async () => {
  const first = createHarness();
  first.render(); await tick(); first.render();
  let finishRefresh!: (result: FixtureServer) => void;
  first.hooks.list = () => new Promise((resolve) => { finishRefresh = resolve; });
  first.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  first.unmount();

  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  reopened.render(false);
  assert.equal(reopened.find((node) => node.type === "Button" && node.children.includes("刷新目录中")).props.disabled, true);
  finishRefresh(structuredClone(first.server));
  await tick();
  reopened.flushEffects();
  await tick(); reopened.render();
  assert.equal(reopened.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.disabled, false);
  reopened.unmount();
});

test("local scans and repository refreshes have independent intervals", async () => {
  const first = createHarness();
  first.render(); await tick(); first.render(); first.unmount();
  first.cache.localStartedAt = Date.now() - 31_000;
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  reopened.render(); await tick(); reopened.render();
  assert.deepEqual(reopened.calls, ["list_installed_skills"]);
  reopened.unmount();
  first.cache.catalogStartedAt = Date.now() - 301_000;
  const later = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  later.render(); await tick(); later.render();
  assert.deepEqual(later.calls, ["refresh_skill_catalog"]);
  later.unmount();
});

test("a scan in progress is shared across page remounts", async () => {
  const first = createHarness();
  let finishScan!: (result: FixtureServer) => void;
  first.hooks.list = () => new Promise((resolve) => { finishScan = resolve; });
  first.render(); first.unmount();
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  reopened.render();
  assert.deepEqual(reopened.calls, []);
  finishScan(structuredClone(first.server));
  await tick(); reopened.render();
  assert.ok(reopened.order().includes("repo:a/repo"));
  assert.deepEqual(reopened.calls, ["refresh_skill_catalog"]);
  reopened.unmount();
});

test("a repository refresh finishing during a reopened page's local scan is not lost", async () => {
  const first = createHarness();
  first.render(); await tick(); first.render(); first.unmount();
  first.cache.catalogStartedAt = 0;
  const oldPage = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  let finishCatalog!: (result: FixtureServer) => void;
  oldPage.hooks.list = () => new Promise((resolve) => { finishCatalog = resolve; });
  oldPage.render(); await tick(); oldPage.unmount();

  first.cache.localStartedAt = 0;
  const stale = structuredClone(first.server);
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  let finishLocal!: (result: FixtureServer) => void;
  let localScans = 0;
  reopened.hooks.list = (command) => {
    assert.equal(command, "list_installed_skills");
    localScans++;
    return localScans === 1
      ? new Promise((resolve) => { finishLocal = resolve; })
      : Promise.resolve(structuredClone(first.server));
  };
  reopened.render();
  first.server.skills = [skill("new", "new/repo@main")];
  finishCatalog(structuredClone(first.server));
  await tick(); reopened.render();
  assert.equal(reopened.find((node) => node.props.className === "cx-skill-groups").props["aria-busy"], true);
  finishLocal(stale);
  await tick(); await tick(); reopened.render();
  assert.ok(reopened.order().includes("repo:new/repo"));
  assert.equal(reopened.order().includes("repo:a/repo"), false);
  assert.equal(localScans, 2);
  reopened.unmount();
});

test("a settled background operation waits for the initial local scan before rescanning", async () => {
  const ui = createHarness();
  let finishInitial!: (result: FixtureServer) => void;
  let finishFinal!: (result: FixtureServer) => void;
  let scans = 0;
  ui.hooks.list = (command) => {
    if (command !== "list_installed_skills") return Promise.resolve(structuredClone(ui.server));
    scans++;
    return new Promise((resolve) => { if (scans === 1) finishInitial = resolve; else finishFinal = resolve; });
  };
  ui.render();
  ui.registry.notifySettled(Symbol("other"));
  ui.registry.notifySettled(Symbol("other"));
  ui.registry.notifySettled(Symbol("other"));
  assert.equal(scans, 1);
  finishInitial(structuredClone(ui.server));
  await tick();
  assert.equal(scans, 2);
  assert.deepEqual(ui.calls, ["list_installed_skills", "list_installed_skills"]);
  finishFinal(structuredClone(ui.server));
  await tick(); ui.render();
  assert.equal(scans, 2);
  assert.equal(ui.calls.filter((command) => command === "refresh_skill_catalog").length, 1);
  ui.unmount();
});

test("an operation finishing while the page is closed invalidates the cached inventory", async () => {
  const first = createHarness();
  first.render(); await tick(); first.render(); first.unmount();
  first.server.skills = [skill("new", "new/repo@main")];
  first.registry.notifySettled(Symbol("other"));
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  reopened.render();
  assert.ok(reopened.order().includes("repo:a/repo"));
  await tick(); reopened.render();
  assert.ok(reopened.calls.includes("list_installed_skills"));
  assert.ok(reopened.order().includes("repo:new/repo"));
  reopened.unmount();
});

test("an invalidated scan reused after reopening is followed by a fresh local read", async () => {
  const first = createHarness();
  first.render(); await tick(); first.render(); first.unmount();
  first.cache.localStartedAt = 0;
  const oldPage = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  const stale = structuredClone(first.server);
  let finishOldScan!: (result: FixtureServer) => void;
  oldPage.hooks.list = () => new Promise((resolve) => { finishOldScan = resolve; });
  oldPage.render(); oldPage.unmount();

  first.server.skills = [skill("new", "new/repo@main")];
  first.registry.notifySettled(Symbol("other"));
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  reopened.render();
  assert.deepEqual(reopened.calls, []);
  finishOldScan(stale);
  await tick(); await tick(); reopened.render();
  assert.deepEqual(reopened.calls, ["list_installed_skills"]);
  assert.ok(reopened.order().includes("repo:new/repo"));
  reopened.unmount();
});

test("a repository refresh finishing after an edit is merged with fresh local state", async () => {
  const first = createHarness();
  first.server.skills = [skill("alpha", "a/repo@main")];
  first.render(); await tick(); first.render(); first.unmount();
  first.cache.catalogStartedAt = 0;
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  let finishRefresh!: (result: FixtureServer) => void;
  reopened.hooks.list = (command) => command === "refresh_skill_catalog"
    ? new Promise((resolve) => { finishRefresh = resolve; }) : Promise.resolve(structuredClone(first.server));
  reopened.hooks.mutation = async (_command, args) => {
    first.server.skills[0].agents = args.enabled ? ["codex"] : [];
    first.server.skills[0].enabled = Boolean(args.enabled);
    return structuredClone(first.server);
  };
  reopened.render(); await tick(); reopened.render();
  const toggle = reopened.rowElements("repo:a/repo", "a/repo@main", "alpha")
    .find((node) => node.type === "AgentToggle" && node.props.agent === "codex")!;
  await toggle.props.onToggle(); reopened.render();
  first.server.skills.push(skill("remote", "new/repo@main"));
  finishRefresh({ ...structuredClone(first.server), skills: [skill("alpha", "a/repo@main"), skill("remote", "new/repo@main")] });
  await tick(); await tick(); reopened.render();
  assert.equal(reopened.calls.filter((command) => command === "list_installed_skills").length, 1);
  assert.ok(reopened.order().includes("repo:new/repo"));
  assert.equal(reopened.rowElements("repo:a/repo", "a/repo@main", "alpha")
    .find((node) => node.type === "AgentToggle" && node.props.agent === "codex")!.props.enabled, true);
  reopened.unmount();
});

test("a background repository refresh does not block actions on cached skills", async () => {
  const first = createHarness();
  first.render(); await tick(); first.render(); first.unmount();
  first.cache.catalogStartedAt = 0;
  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  let finishRefresh!: (result: FixtureServer) => void;
  reopened.hooks.list = (command) => command === "refresh_skill_catalog"
    ? new Promise((resolve) => { finishRefresh = resolve; }) : Promise.resolve(structuredClone(first.server));
  reopened.render(); await tick(); reopened.render();
  assert.equal(reopened.find((node) => node.type === "button" && node.props["aria-label"] === "匹配仓库").props.disabled, false);
  assert.equal(reopened.sort().props.disabled, false);
  assert.equal(reopened.group("repo:a/repo").props.disabled, false);
  reopened.drag("repo:z/repo", "repo:a/repo");
  assert.ok(reopened.order().indexOf("repo:z/repo") < reopened.order().indexOf("repo:a/repo"));
  finishRefresh(structuredClone(first.server));
  await tick(); reopened.unmount();
});

test("background operation bursts coalesce local rescans and still perform a final read", async () => {
  const ui = createHarness();
  ui.render(); await tick(); ui.render();
  let finishScan!: (result: FixtureServer) => void;
  let scans = 0;
  ui.hooks.list = (command) => {
    assert.equal(command, "list_installed_skills");
    scans++;
    return scans === 1 ? new Promise((resolve) => { finishScan = resolve; }) : Promise.resolve(structuredClone(ui.server));
  };
  ui.registry.notifySettled(Symbol("other"));
  ui.registry.notifySettled(Symbol("other"));
  ui.registry.notifySettled(Symbol("other"));
  assert.equal(scans, 1);
  ui.server.skills = [skill("new", "new/repo@main")];
  finishScan({ ...structuredClone(ui.server), skills: [skill("old", "old/repo@main")] });
  await tick(); ui.render();
  assert.equal(scans, 2);
  assert.ok(ui.order().includes("repo:new/repo"));
  ui.unmount();
});

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
    return args.includeInventory === false ? { status: "ok", message: "ok" } : structuredClone(ui.server);
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
    .map(({ args }) => [args.agent, args.id, args.enabled, args.includeInventory]), [
    ["codex", "alpha", true, false], ["codex", "beta", true, undefined],
    ["codex", "alpha", false, false], ["codex", "beta", false, undefined],
  ]);
  assert.equal(ui.requests.filter((request) => request.command === "list_installed_skills").length, 1);
  ui.unmount();
});

test("a rejected final batch request refreshes once and preserves the successful changes", async () => {
  const ui = createHarness();
  ui.server.skills = [skill("alpha", "a/repo@main"), skill("beta", "a/repo@main")];
  ui.render(); await tick(); ui.render();
  ui.hooks.mutation = async (_command, args) => {
    if (args.id === "beta") throw new Error("beta failed");
    ui.server.skills[0].agents = ["codex"];
    ui.server.skills[0].enabled = true;
    return { status: "ok", message: "ok" };
  };
  ui.groupSelect("repo:a/repo", "codex").props.onChange(true);
  for (let index = 0; index < 20 && ui.registry.getSnapshot().length; index++) await tick();
  await tick(); ui.render();
  assert.equal(ui.requests.filter((request) => request.command === "list_installed_skills").length, 2);
  assert.equal(ui.rowElements("repo:a/repo", "a/repo@main", "alpha")
    .find((node) => node.type === "AgentToggle" && node.props.agent === "codex")!.props.enabled, true);
  assert.equal(ui.rowElements("repo:a/repo", "a/repo@main", "beta")
    .find((node) => node.type === "AgentToggle" && node.props.agent === "codex")!.props.enabled, false);
  assert.ok(ui.notices.some((notice) => notice.status === "failed" && notice.message.includes("beta failed")));
  ui.unmount();
});

test("a remounted page keeps batch progress until an overlapping refresh and its follow-up complete", async () => {
  const first = createHarness();
  first.server.skills = [skill("alpha", "a/repo@main"), skill("beta", "a/repo@main")];
  first.render(); await tick(); first.render();
  let finishBatch!: () => void;
  first.hooks.mutation = async (_command, args) => {
    const item = first.server.skills.find((entry) => entry.id === args.id)!;
    if (args.id === "beta") await new Promise<void>((resolve) => { finishBatch = resolve; });
    item.agents = ["codex"];
    item.enabled = true;
    return args.includeInventory === false ? { status: "ok", message: "ok" } : structuredClone(first.server);
  };
  first.groupSelect("repo:a/repo", "codex").props.onChange(true);
  await tick();
  first.unmount();

  const reopened = createHarness("{}", { server: first.server, registry: first.registry, cache: first.cache });
  let finishInitial!: (result: FixtureServer) => void;
  let finishFollowUp!: (result: FixtureServer) => void;
  let scans = 0;
  reopened.hooks.list = (command) => {
    assert.equal(command, "list_installed_skills");
    scans++;
    return new Promise((resolve) => { if (scans === 1) finishInitial = resolve; else finishFollowUp = resolve; });
  };
  reopened.render(); await tick(); reopened.render();
  assert.equal(scans, 0);
  const stale = structuredClone(first.server);
  reopened.registry.notifySettled(Symbol("other"));
  assert.equal(scans, 1);
  finishBatch();
  await tick(); await tick(); reopened.render();
  assert.equal(reopened.registry.getSnapshot().length, 1);
  assert.equal(reopened.groupSelect("repo:a/repo", "codex").props.busy, true);
  finishInitial(stale);
  await tick(); reopened.render();
  assert.equal(scans, 2);
  assert.equal(reopened.registry.getSnapshot().length, 1);
  finishFollowUp(structuredClone(first.server));
  for (let index = 0; index < 20 && reopened.registry.getSnapshot().length; index++) await tick();
  reopened.render();
  assert.equal(reopened.registry.getSnapshot().length, 0);
  assert.equal(reopened.rowElements("repo:a/repo", "a/repo@main", "beta")
    .find((node) => node.type === "AgentToggle" && node.props.agent === "codex")!.props.enabled, true);
  assert.equal(scans, 2);
  reopened.unmount();
});

test("a failed batch completion notification releases the operation and reports the error", async () => {
  const ui = createHarness();
  ui.server.skills = [skill("alpha", "a/repo@main")];
  ui.render(); await tick(); ui.render();
  const unsubscribe = ui.registry.subscribeSettled((_owner, progress) => {
    if (!progress) throw new Error("refresh notification failed");
  });
  ui.hooks.mutation = async () => structuredClone(ui.server);
  ui.groupSelect("repo:a/repo", "codex").props.onChange(true);
  for (let index = 0; index < 20 && !ui.notices.some((notice) => notice.status === "failed"); index++) await tick();
  assert.equal(ui.registry.getSnapshot().length, 0);
  assert.ok(ui.notices.some((notice) => notice.message.includes("refresh notification failed")));
  unsubscribe();
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

test("repository matching is immediately before new group and migrates only successful associations", async () => {
  const original = skill("first", "local");
  const failed = skill("second", "local");
  const ui = createHarness(JSON.stringify({ names: ["Manual"], assignmentsByIdentity: {
    [skillIdentityKey(original)]: "custom:Manual", [skillIdentityKey(failed)]: "",
  } }));
  ui.server.skills = [original, failed];
  ui.render(); await tick(); ui.render();
  const trailing = ui.find((node) => node.type === "CxFilters").props.trailing;
  const controls = trailing.children.filter((node: Element) => node?.type === "button");
  assert.equal(controls.findIndex((node: Element) => node.props["aria-label"] === "匹配仓库") + 1,
    controls.findIndex((node: Element) => node.props["aria-label"] === "新建分组"));
  ui.find((node) => node.type === "button" && node.props["aria-label"] === "匹配仓库").props.onClick(); ui.render();
  const drawer = ui.find((node) => node.type === "SkillSourceMatchDrawer");
  assert.deepEqual(Array.from(drawer.props.localSkillIds), ["first", "second"]);
  ui.hooks.mutation = async (command) => {
    assert.equal(command, "apply_skill_repository_matches");
    ui.server.skills = [{ ...original, repoKey: "owner/repo@main" }, failed];
    return { ...structuredClone(ui.server), status: "failed", message: "one failed", applied: [
      { localId: "first", fromRepoKey: "local", toRepoKey: "owner/repo@main" },
    ] };
  };
  await drawer.props.onApply([]);
  ui.render();
  assert.deepEqual(ui.savedState().assignmentsByIdentity, {
    [skillIdentityKey(skill("first", "owner/repo@main"))]: "custom:Manual",
    [skillIdentityKey(failed)]: "",
  });
  assert.equal(ui.notices.at(-1)?.status, "failed");
  ui.unmount();
});

test("a split physical installation retains the previously assigned group after source association", async () => {
  const previous: FixtureSkill = { ...skill("alpha", ""), instanceKey: "alpha:C:/agents/alpha",
    conflict: "多个独立安装", conflictKind: "equal-independent", locations: [
      { manifestPath: "C:/agents/alpha/SKILL.md", originalPath: "C:/agents/alpha", selectable: true, rootKind: "shared", managed: false, enabled: true },
      { manifestPath: "C:/claude/alpha/SKILL.md", originalPath: "C:/claude/alpha", selectable: true, rootKind: "claude", managed: false, enabled: true },
    ] };
  const selected: FixtureSkill = { ...previous, repoKey: "other/repo@main", instanceKey: "alpha:C:/claude/alpha",
    conflictKind: "independent", locations: [previous.locations![1]] };
  const retained: FixtureSkill = { ...previous, conflictKind: "independent", locations: [previous.locations![0]] };
  const ui = createHarness(JSON.stringify({ names: ["Manual"], assignmentsByIdentity: {
    [skillIdentityKey(previous)]: "custom:Manual",
  } }));
  ui.server.skills = [previous];
  ui.render(); await tick(); ui.render();
  ui.find((node) => node.type === "button" && node.props["aria-label"] === "匹配仓库").props.onClick(); ui.render();
  const drawer = ui.find((node) => node.type === "SkillSourceMatchDrawer");
  ui.hooks.mutation = async (command) => {
    assert.equal(command, "apply_skill_repository_matches");
    ui.server.skills = [retained, selected];
    return { ...structuredClone(ui.server), applied: [{
      localId: "alpha", fromRepoKey: "", toRepoKey: selected.repoKey, localPath: "C:/claude/alpha",
    }] };
  };
  await drawer.props.onApply([{ localId: "alpha", repoKey: selected.repoKey, repoPath: "alpha",
    previewId: "preview", localPath: "C:/claude/alpha" }]);
  ui.render();
  assert.equal(ui.savedState().assignmentsByIdentity[skillIdentityKey(retained)], "custom:Manual");
  assert.equal(ui.savedState().assignmentsByIdentity[skillIdentityKey(selected)], "custom:Manual");
  const items = ui.group("custom:Manual").props.group.items as FixtureSkill[];
  assert.equal(items.length, 1);
  assert.deepEqual(items[0].displayInstances?.map((item) => item.instanceKey), [retained.instanceKey, selected.instanceKey]);
  ui.unmount();
});

test("legacy grouping of an equal aggregate follows only the explicitly associated location after splitting", async () => {
  const previous: FixtureSkill = { ...skill("alpha", ""), instanceKey: "alpha:C:/agents/alpha",
    conflict: "多个独立安装", conflictKind: "equal-independent", locations: [
      { manifestPath: "C:/agents/alpha/SKILL.md", originalPath: "C:/agents/alpha", selectable: true, rootKind: "shared", managed: false, enabled: true },
      { manifestPath: "C:/claude/alpha/SKILL.md", originalPath: "C:/claude/alpha", selectable: true, rootKind: "claude", managed: false, enabled: true },
    ] };
  const selected: FixtureSkill = { ...previous, repoKey: "other/repo@main", instanceKey: "alpha:C:/claude/alpha",
    conflictKind: "independent", locations: [previous.locations![1]] };
  const retained: FixtureSkill = { ...previous, conflictKind: "independent", locations: [previous.locations![0]] };
  const ui = createHarness(JSON.stringify({ names: ["Manual"], assignments: { alpha: "Manual" } }));
  ui.server.skills = [previous];
  ui.render(); await tick(); ui.render(); ui.render();
  assert.equal(ui.savedState().assignmentsByIdentity[skillGroupIdentityKey(previous)], "custom:Manual");
  assert.equal(ui.group("custom:Manual").props.group.items.length, 1);
  ui.find((node) => node.type === "button" && node.props["aria-label"] === "匹配仓库").props.onClick(); ui.render();
  const drawer = ui.find((node) => node.type === "SkillSourceMatchDrawer");
  ui.hooks.mutation = async () => {
    ui.server.skills = [retained, selected];
    return { ...structuredClone(ui.server), applied: [{
      localId: "alpha", fromRepoKey: "", toRepoKey: selected.repoKey, localPath: "C:/claude/alpha",
    }] };
  };
  await drawer.props.onApply([{ localId: "alpha", repoKey: selected.repoKey, repoPath: "alpha",
    previewId: "preview", localPath: "C:/claude/alpha" }]);
  ui.render(); ui.render();
  assert.equal(ui.savedState().assignmentsByIdentity[skillIdentityKey(retained)], undefined);
  assert.equal(ui.savedState().assignmentsByIdentity[skillIdentityKey(selected)], "custom:Manual");
  assert.deepEqual(ui.group("user").props.group.items.map((item: FixtureSkill) => item.instanceKey), [retained.instanceKey]);
  assert.deepEqual(ui.group("custom:Manual").props.group.items.map((item: FixtureSkill) => item.instanceKey), [selected.instanceKey]);
  ui.unmount();
});

test("updates of an associated original open a preview instead of entering the old update queue", async () => {
  const ui = createHarness();
  ui.server.skills = [{ ...skill("original", "owner/repo@main"), originalDir: "C:\\skills\\original", updateOwner: "in_place", updateAvailable: true }];
  ui.render(); await tick(); ui.render();
  const update = ui.rowElements("repo:owner/repo", "owner/repo@main", "original")
    .find((node) => node.type === "Button" && node.children.includes("更新"));
  assert.ok(update);
  update.props.onClick(); ui.render();
  const preview = ui.find((node) => node.type === "SkillInPlaceUpdateDrawer");
  assert.equal(preview.props.id, "original");
  assert.equal(preview.props.localPath, "C:\\skills\\original");
  assert.equal(ui.registry.getUpdates().length, 0);
  ui.unmount();
});

test("different Agent installations share a group row without losing paths, status or assignments", async () => {
  const instances: FixtureSkill[] = [
    { ...skill("alpha", ""), instanceKey: "alpha:C:/agents/alpha", conflict: "内容不同", conflictKind: "independent",
      agents: ["codex"], enabled: true, description: "Codex copy",
      locations: [{ manifestPath: "C:/agents/alpha/SKILL.md", originalPath: "C:/agents/alpha", selectable: true, rootKind: "shared", managed: false, enabled: true }] },
    { ...skill("alpha", ""), instanceKey: "alpha:C:/claude/alpha", conflict: "内容不同", conflictKind: "independent",
      agents: ["claude"], description: "Claude copy",
      locations: [{ manifestPath: "C:/claude/alpha/SKILL.md", originalPath: "C:/claude/alpha", selectable: true, rootKind: "claude", managed: false, enabled: true }] },
    { ...skill("alpha", ""), instanceKey: "alpha:C:/grok/alpha", conflict: "内容不同", conflictKind: "independent",
      agents: ["grok"], description: "Grok unique",
      locations: [{ manifestPath: "C:/grok/alpha/SKILL.md", originalPath: "C:/grok/alpha", selectable: true, rootKind: "grok", managed: false, enabled: true }] },
  ];
  const ui = createHarness(JSON.stringify({ names: ["Manual"] }));
  ui.server.skills = instances;
  ui.render(); await tick(); ui.render();
  assert.equal(ui.group("user").props.group.items.length, 1);
  assert.equal(ui.find((node) => node.type === "CxFilters").props.items.find((item: { id: string }) => item.id === "installed").count, 1);
  const row = ui.rowElements("user", "", "alpha", instances[0]);
  const toggles = row.filter((node) => node.type === "AgentToggle");
  assert.ok(toggles.every((node) => node.props.enabled));
  const codex = toggles.find((node) => node.props.agent === "codex")!;
  assert.equal(codex.props.disabled, false);
  assert.ok(toggles.filter((node) => node.props.agent !== "codex").every((node) => node.props.disabled));
  ui.hooks.mutation = async (command, args) => {
    assert.equal(command, "set_skill_enabled_for");
    assert.equal(args.agent, "codex");
    assert.equal(args.id, "alpha");
    instances[0].agents = args.enabled ? ["codex"] : [];
    instances[0].enabled = Boolean(args.enabled);
    instances[0].locations![0].enabled = Boolean(args.enabled);
    return structuredClone(ui.server);
  };
  await codex.props.onToggle(); ui.render();
  assert.deepEqual(instances.slice(1).map((item) => item.agents), [["claude"], ["grok"]]);
  ui.groupSelect("user", "claude").props.onChange(false);
  await tick();
  assert.equal(ui.requests.filter((request) => request.command === "set_skill_enabled_for").length, 1);
  ui.groupSelect("user", "codex").props.onChange(true);
  for (let index = 0; index < 20 && ui.registry.getSnapshot().length; index++) await tick();
  await tick(); ui.render();
  assert.deepEqual(ui.requests.filter((request) => request.command === "set_skill_enabled_for").map((request) =>
    JSON.parse(JSON.stringify(request.args))), [
    { agent: "codex", id: "alpha", enabled: false },
    { agent: "codex", id: "alpha", enabled: true },
  ]);
  assert.deepEqual(instances.slice(1).map((item) => item.agents), [["claude"], ["grok"]]);
  ui.search("Grok unique");
  const found = ui.group("user").props.group.items[0] as FixtureSkill;
  assert.equal(found.displayInstances?.length, 3);
  assert.deepEqual(found.locations?.map((location) => location.manifestPath),
    instances.flatMap((item) => item.locations!.map((location) => location.manifestPath)));
  ui.search("");
  ui.rowElements("user", "", "alpha", instances[0]).find((node) => node.type === "MenuItem" && node.children.includes("Manual"))!.props.onClick();
  ui.render();
  assert.equal(ui.group("custom:Manual").props.group.items.length, 1);
  for (const item of instances) assert.equal(ui.savedState().assignmentsByIdentity[skillGroupIdentityKey(item)], "custom:Manual");
  ui.unmount();
});

test("conflicting same-name locations remain separate and cannot perform id-based writes", async () => {
  const ui = createHarness(JSON.stringify({ names: ["Manual"] }));
  const first: FixtureSkill = { ...skill("same", ""), instanceKey: "shared:C:/agents/same", conflict: "内容不同",
    locations: [{ manifestPath: "C:/agents/same/SKILL.md", rootKind: "shared", managed: false, enabled: true }],
    agents: ["codex"], updateAvailable: true };
  const second: FixtureSkill = { ...skill("same", ""), instanceKey: "same:C:/codex/same", conflict: "内容不同",
    locations: [{ manifestPath: "C:/codex/same/SKILL.md", rootKind: "codex", managed: true, enabled: true }],
    agents: ["codex"], updateAvailable: true };
  ui.server.skills = [first, second];
  ui.render(); await tick(); ui.render();
  const firstRow = ui.rowElements("user", "", "same", first);
  const secondRow = ui.rowElements("user", "", "same", second);
  assert.ok(firstRow.some((node) => node.props.title === "C:/agents/same/SKILL.md"));
  assert.ok(secondRow.some((node) => node.props.title === "C:/codex/same/SKILL.md"));
  for (const row of [firstRow, secondRow]) {
    assert.ok(row.filter((node) => node.type === "AgentToggle").every((node) => node.props.disabled));
    assert.ok(row.find((node) => node.type === "MenuItem" && node.children.includes("重新关联来源"))?.props.disabled);
    assert.equal(row.some((node) => node.type === "MenuItem" && node.children.includes("卸载")), false);
    assert.equal(row.some((node) => node.type === "Button" && node.children.includes("更新")), false);
  }
  firstRow.find((node) => node.type === "MenuItem" && node.children.includes("Manual"))!.props.onClick();
  ui.render();
  assert.deepEqual(ui.group("custom:Manual").props.group.items.map((item: FixtureSkill) => item.instanceKey), [first.instanceKey]);
  assert.deepEqual(ui.group("user").props.group.items.map((item: FixtureSkill) => item.instanceKey), [second.instanceKey]);
  ui.groupSelect("user", "codex").props.onChange(false);
  await tick();
  assert.equal(ui.calls.includes("set_skill_enabled_for"), false);
  ui.unmount();
});

test("equal independent installs allow only the Codex path switch", async () => {
  const ui = createHarness();
  ui.server.skills = [{ ...skill("reverse-engineer-anything", ""), agents: ["codex", "claude"],
    conflict: "多个独立安装，当前内容一致", conflictKind: "equal-independent",
    locations: [
      { manifestPath: "C:/agents/rea/SKILL.md", rootKind: "shared", managed: false, enabled: true },
      { manifestPath: "C:/claude/rea/SKILL.md", rootKind: "claude", managed: false, enabled: true },
    ],
  }];
  ui.render(); await tick(); ui.render();
  const row = ui.rowElements("user", "", "reverse-engineer-anything");
  assert.ok(row.some((node) => node.type === "CxBadge" && node.children.includes("多个独立安装")));
  const toggles = row.filter((node) => node.type === "AgentToggle");
  const codex = toggles.find((node) => node.props.agent === "codex");
  assert.ok(codex);
  assert.equal(codex.props.disabled, false);
  assert.ok(toggles.filter((node) => node.props.agent !== "codex").every((node) => node.props.disabled));
  ui.hooks.mutation = async (command, args) => {
    assert.equal(command, "set_skill_enabled_for");
    assert.deepEqual(JSON.parse(JSON.stringify(args)), { agent: "codex", id: "reverse-engineer-anything", enabled: false });
    return structuredClone(ui.server);
  };
  await codex.props.onToggle();
  assert.equal(ui.requests.filter((request) => request.command === "set_skill_enabled_for").length, 1);
  ui.unmount();
});

test("unknown native status disables only the Codex control and batch", async () => {
  const ui = createHarness();
  ui.server.skills = [{ ...skill("alpha", ""), agents: ["claude"],
    locations: [{ manifestPath: "C:/agents/alpha/SKILL.md", rootKind: "shared", managed: false,
      enabled: false, statusError: "无法确定名称规则的启用状态" }] }];
  ui.render(); await tick(); ui.render();
  const row = ui.rowElements("user", "", "alpha");
  assert.ok(row.some((node) => node.type === "CxBadge" && node.children.includes("Codex 状态未知")));
  const toggles = row.filter((node) => node.type === "AgentToggle");
  const codex = toggles.find((node) => node.props.agent === "codex")!;
  const claude = toggles.find((node) => node.props.agent === "claude")!;
  assert.equal(codex.props.disabled, true);
  assert.equal(codex.props.statusUnknown, true);
  assert.match(codex.props.hint, /无法确定名称规则的启用状态/);
  assert.equal(claude.props.disabled, false);
  ui.groupSelect("user", "codex").props.onChange(true);
  await tick();
  assert.equal(ui.requests.some((request) => request.command === "set_skill_enabled_for"), false);
  ui.unmount();
});

test("a grouped independent instance opens source matching for its exact path", async () => {
  const ui = createHarness();
  const original: FixtureSkill = { ...skill("impeccable", ""), instanceKey: "impeccable:C:/agents/impeccable",
    conflict: "内容不同", conflictKind: "independent", originalDir: "C:/agents/impeccable",
    locations: [{ manifestPath: "C:/agents/impeccable/SKILL.md", originalPath: "C:/agents/impeccable", selectable: true, rootKind: "shared", managed: false, enabled: true }],
    agents: ["codex"] };
  const other: FixtureSkill = { ...original, instanceKey: "impeccable:C:/claude/impeccable",
    originalDir: "C:/claude/impeccable", agents: ["claude"],
    locations: [{ manifestPath: "C:/claude/impeccable/SKILL.md", originalPath: "C:/claude/impeccable", selectable: true, rootKind: "claude", managed: false, enabled: true }] };
  ui.server.skills = [original, other];
  ui.render(); await tick(); ui.render();
  const menu = ui.rowElements("user", "", "impeccable", original)
    .find((node) => node.type === "MenuItem" && node.children.includes("重新关联来源"));
  assert.ok(menu);
  assert.equal(menu.props.disabled, true);
  ui.rowElements("user", "", "impeccable", original).find((node) => node.type === "CxRow")!.props.onClick(); ui.render();
  const detail = ui.find((node) => node.type === "SkillDetailDrawer");
  assert.deepEqual(detail.props.skill.locations.map((location: { manifestPath: string }) => location.manifestPath),
    original.locations!.concat(other.locations!).map((location) => location.manifestPath));
  detail.props.onMatch(original.originalDir); ui.render();
  const drawer = ui.find((node) => node.type === "SkillSourceMatchDrawer");
  assert.equal(drawer.props.id, "impeccable");
  assert.equal(drawer.props.localPath, original.originalDir);
  ui.unmount();
});

test("an equal independent install can select a physical location for matching", async () => {
  const ui = createHarness();
  ui.server.skills = [{ ...skill("alpha", ""), conflict: "多个独立安装", conflictKind: "equal-independent",
    locations: [
      { manifestPath: "C:/agents/alpha/SKILL.md", originalPath: "C:/agents/alpha", selectable: true, rootKind: "shared", managed: false, enabled: true },
      { manifestPath: "C:/claude/alpha/SKILL.md", originalPath: "C:/claude/alpha", selectable: true, rootKind: "claude", managed: false, enabled: true },
    ],
    agents: ["codex", "claude"] }];
  ui.render(); await tick(); ui.render();
  ui.rowElements("user", "", "alpha").find((node) => node.type === "CxRow")!.props.onClick();
  ui.render();
  ui.find((node) => node.type === "SkillDetailDrawer").props.onMatch("C:/claude/alpha");
  ui.render();
  const drawer = ui.find((node) => node.type === "SkillSourceMatchDrawer");
  assert.equal(drawer.props.id, "alpha");
  assert.equal(drawer.props.localPath, "C:/claude/alpha");
  ui.unmount();
});

test("a location-specific update opens the preview for that location and repository", async () => {
  const ui = createHarness();
  ui.server.skills = [{ ...skill("alpha", ""), conflict: "多个独立安装", conflictKind: "equal-independent",
    locations: [
      { manifestPath: "C:/agents/alpha/SKILL.md", originalPath: "C:/agents/alpha", selectable: true, rootKind: "shared", managed: false, enabled: true },
      { manifestPath: "C:/claude/alpha/SKILL.md", originalPath: "C:/claude/alpha", selectable: true, rootKind: "claude", managed: false, enabled: true },
    ],
    agents: ["codex", "claude"] }];
  ui.render(); await tick(); ui.render();
  ui.rowElements("user", "", "alpha").find((node) => node.type === "CxRow")!.props.onClick();
  ui.render();
  ui.find((node) => node.type === "SkillDetailDrawer").props.onUpdate("other/repo@main", "C:/claude/alpha");
  ui.render();
  const drawer = ui.find((node) => node.type === "SkillInPlaceUpdateDrawer");
  assert.equal(drawer.props.id, "alpha");
  assert.equal(drawer.props.repoKey, "other/repo@main");
  assert.equal(drawer.props.localPath, "C:/claude/alpha");
  ui.unmount();
});

test("different confirmed repositories keep split rows, groups and update targets independent", async () => {
  const first: FixtureSkill = { ...skill("alpha", "one/repo@main"), instanceKey: "alpha:C:/agents/alpha",
    originalDir: "C:/agents/alpha", conflict: "不同来源", conflictKind: "independent",
    locations: [{ manifestPath: "C:/agents/alpha/SKILL.md", originalPath: "C:/agents/alpha", selectable: true, rootKind: "shared", managed: false, enabled: true }] };
  const second: FixtureSkill = { ...first, repoKey: "two/repo@main", instanceKey: "alpha:C:/claude/alpha",
    originalDir: "C:/claude/alpha",
    locations: [{ manifestPath: "C:/claude/alpha/SKILL.md", originalPath: "C:/claude/alpha", selectable: true, rootKind: "claude", managed: false, enabled: true }] };
  const ui = createHarness(JSON.stringify({ names: ["Manual"], assignmentsByIdentity: {
    [skillIdentityKey(first)]: "custom:Manual",
  } }));
  ui.server.skills = [first, second];
  ui.render(); await tick(); ui.render();
  for (const [item, group] of [[first, "custom:Manual"], [second, "repo:two/repo"]] as const) {
    const row = ui.rowElements(group, item.repoKey, item.id, item);
    assert.ok(row.some((node) => node.type === "small" && node.children.includes(item.repoKey.split("@")[0])));
    row.find((node) => node.type === "CxRow")!.props.onClick(); ui.render();
    const detail = ui.find((node) => node.type === "SkillDetailDrawer");
    assert.equal(detail.props.skill.repoKey, item.repoKey);
    assert.equal(detail.props.skill.originalDir, item.originalDir);
    detail.props.onUpdate(item.repoKey, item.originalDir); ui.render();
    const preview = ui.find((node) => node.type === "SkillInPlaceUpdateDrawer");
    assert.equal(preview.props.repoKey, item.repoKey);
    assert.equal(preview.props.localPath, item.originalDir);
    preview.props.onClose(); ui.render();
  }
  assert.deepEqual(ui.group("custom:Manual").props.group.items.map((item: FixtureSkill) => item.instanceKey), [first.instanceKey]);
  assert.deepEqual(ui.group("repo:two/repo").props.group.items.map((item: FixtureSkill) => item.instanceKey), [second.instanceKey]);
  ui.unmount();
});

test("different confirmed repositories stay separate even in the same custom group", async () => {
  const first: FixtureSkill = { ...skill("alpha", "one/repo@main"), instanceKey: "alpha:C:/agents/alpha",
    conflict: "不同来源", conflictKind: "independent", agents: ["codex"],
    locations: [{ manifestPath: "C:/agents/alpha/SKILL.md", rootKind: "shared", managed: false, enabled: true }] };
  const second: FixtureSkill = { ...first, repoKey: "two/repo@main", instanceKey: "alpha:C:/claude/alpha", agents: ["claude"],
    locations: [{ manifestPath: "C:/claude/alpha/SKILL.md", rootKind: "claude", managed: false, enabled: true }] };
  const ui = createHarness(JSON.stringify({ names: ["Manual"], assignmentsByIdentity: {
    [skillIdentityKey(first)]: "custom:Manual", [skillIdentityKey(second)]: "custom:Manual",
  } }));
  ui.server.skills = [first, second];
  ui.render(); await tick(); ui.render();
  assert.deepEqual(ui.group("custom:Manual").props.group.items.map((item: FixtureSkill) => item.repoKey),
    ["one/repo@main", "two/repo@main"]);
  ui.unmount();
});

test("a modified managed copy allows only its listed agent to be disabled and backed up", async () => {
  const ui = createHarness();
  const original: FixtureSkill = { ...skill("same", ""), instanceKey: "same:C:/shared/same",
    conflict: "内容不同", conflictKind: "managed-modified", safeDisableAgents: [],
    locations: [{ manifestPath: "C:/shared/same/SKILL.md", rootKind: "shared", managed: false, enabled: true }],
    agents: ["codex"] };
  const copy: FixtureSkill = { ...skill("same", ""), instanceKey: "same:C:/claude/same",
    conflict: "内容不同", conflictKind: "managed-modified", safeDisableAgents: ["claude"],
    locations: [{ manifestPath: "C:/claude/same/SKILL.md", rootKind: "claude", managed: true, enabled: true }],
    agents: ["claude"] };
  ui.server.skills = [original, copy];
  ui.render(); await tick(); ui.render();
  const row = ui.rowElements("user", "", "same", original);
  assert.equal(ui.group("user").props.group.items.length, 1);
  const toggles = row.filter((node) => node.type === "AgentToggle");
  const claude = toggles.find((node) => node.props.agent === "claude");
  assert.ok(claude);
  assert.equal(claude.props.disabled, false);
  assert.equal(claude.props.hint, "停用并备份已修改的受控副本");
  assert.equal(toggles.find((node) => node.props.agent === "codex")?.props.disabled, false);
  assert.equal(toggles.find((node) => node.props.agent === "grok")?.props.disabled, true);

  ui.hooks.confirm = async () => false;
  await claude.props.onToggle();
  assert.equal(ui.requests.some((request) => request.command === "set_skill_enabled_for"), false);
  ui.hooks.confirm = async () => true;
  ui.hooks.mutation = async (command, args) => {
    assert.equal(command, "set_skill_enabled_for");
    assert.deepEqual(JSON.parse(JSON.stringify(args)), { agent: "claude", id: "same", enabled: false });
    return { ...structuredClone(ui.server), skills: [original], message: "已停用并备份" };
  };
  await claude.props.onToggle();
  assert.equal(ui.requests.filter((request) => request.command === "set_skill_enabled_for").length, 1);
  assert.equal(ui.notices.at(-1)?.message, "已停用并备份");
  ui.unmount();
});

test("a saved assignment follows its physical skill through conflict and resolution", async () => {
  const original = { ...skill("same", "owner/repo@main"), instanceKey: "same:C:/agents/same" };
  const ui = createHarness(JSON.stringify({ names: ["Manual"], assignmentsByIdentity: {
    [skillIdentityKey(skill("same", "owner/repo@main"))]: "custom:Manual",
  } }));
  ui.server.skills = [original];
  ui.render(); await tick(); ui.render(); ui.render();
  assert.equal(ui.savedState().assignmentsByIdentity[skillIdentityKey(original)], "custom:Manual");
  assert.deepEqual(ui.group("custom:Manual").props.group.items.map((item: FixtureSkill) => item.instanceKey), [original.instanceKey]);

  const conflicted = { ...original, repoKey: "", conflict: "内容不同" };
  const other = { ...conflicted, instanceKey: "same:C:/codex/same" };
  ui.server.skills = [conflicted, other];
  ui.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  await tick(); ui.render(); ui.render();
  assert.deepEqual(ui.group("custom:Manual").props.group.items.map((item: FixtureSkill) => item.instanceKey), [original.instanceKey]);
  assert.deepEqual(ui.group("user").props.group.items.map((item: FixtureSkill) => item.instanceKey), [other.instanceKey]);
  ui.rowElements("user", "", "same", other).find((node) => node.type === "MenuItem" && node.children.includes("Manual"))!.props.onClick();
  ui.render();

  ui.server.skills = [{ ...skill("same", "owner/repo@main"), instanceKey: other.instanceKey }];
  ui.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  await tick(); ui.render(); ui.render();
  assert.deepEqual(ui.group("custom:Manual").props.group.items.map((item: FixtureSkill) => item.instanceKey), [other.instanceKey]);
  ui.unmount();
});

test("backups remain accessible without a skill and while its instances conflict", async () => {
  const ui = createHarness();
  const backup: FixtureBackup = { id: "backup", skillId: "same", name: "same", backedUpAt: "1",
    purpose: "duplicate", backupPath: "C:/backups/same", restorePath: "C:/codex/same" };
  ui.server.skills = [];
  ui.server.backups = [backup];
  ui.render(); await tick(); ui.render();
  ui.find((node) => node.type === "MenuItem" && node.children.includes("查看 Skill 备份")).props.onClick();
  ui.render();
  const drawer = ui.find((node) => node.type === "SkillBackupsDrawer");
  assert.equal(drawer.props.backups[0].restorePath, backup.restorePath);
  assert.equal(drawer.props.backups[0].backupPath, backup.backupPath);
  ui.hooks.mutation = async (command, args) => {
    assert.equal(command, "restore_skill_backup");
    assert.equal(args.backupId, backup.id);
    return { ...structuredClone(ui.server), status: "failed", message: "原位置已有内容，未覆盖" };
  };
  drawer.props.onRestore(backup);
  await tick(); await tick(); ui.render();
  assert.equal(ui.notices.at(-1)?.status, "failed");
  assert.equal(ui.notices.at(-1)?.message, "原位置已有内容，未覆盖");
  ui.hooks.confirm = async () => false;
  drawer.props.onDelete(backup);
  await tick();
  assert.equal(ui.calls.includes("delete_skill_backup"), false);
  let confirmation = "";
  ui.hooks.confirm = async (message) => { confirmation = message; return true; };
  backup.deleteWarning = "删除后，此更新备份组将无法整体恢复；其他备份文件会保留";
  ui.hooks.mutation = async (command, args) => {
    assert.equal(command, "delete_skill_backup");
    assert.equal(args.backupId, backup.id);
    return { ...structuredClone(ui.server), message: "备份已删除" };
  };
  drawer.props.onDelete(backup);
  await tick(); await tick(); ui.render();
  assert.ok(confirmation.includes(backup.backupPath!));
  assert.ok(confirmation.includes(backup.deleteWarning));
  assert.ok(ui.calls.includes("delete_skill_backup"));
  drawer.props.onClose(); ui.render();

  const conflict = { ...skill("same", ""), instanceKey: "same:C:/agents/same", conflict: "内容不同" };
  ui.server.skills = [conflict, { ...conflict, instanceKey: "same:C:/codex/same" }];
  ui.find((node) => node.type === "Button" && node.children.includes("刷新目录")).props.onClick();
  await tick(); ui.render();
  ui.rowElements("user", "", "same", conflict).find((node) => node.type === "CxRow")!.props.onClick();
  ui.render();
  assert.equal(ui.find((node) => node.type === "SkillDetailDrawer").props.backups[0].id, backup.id);
  ui.unmount();
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

  const reopened = createHarness("{}", { server: ui.server, registry: ui.registry, cache: ui.cache });
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
  const reopened = createHarness(ui.storage.get("codex-plus-plus.skill-groups.v1"), { server: ui.server, registry: ui.registry, cache: ui.cache });
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
  assert.ok(ui.rowElements("user", "cc-switch", "cc-import").some((node) => node.type === "small" && node.children.includes("本地")));
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
