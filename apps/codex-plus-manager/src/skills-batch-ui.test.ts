import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { customSkillGroupKey, defaultSkillGroupKey, groupSkills, legacySkillCustomGroupName, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName } from "./skill-groups.ts";
import { mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchState, skillBatchTargets } from "./skill-agent-batch.ts";
import { SkillTaskQueue } from "./skill-task-queue.ts";

type Element = { type: any; props: Record<string, any>; children: any[] };
type FixtureSkill = { id: string; name: string; description: string; repoKey: string; repoPath?: string; installed: boolean; enabled: boolean; bundled: boolean; updateAvailable: boolean; agents: string[]; managed: boolean };

const source = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("ManagementExtra.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const selected = file.statements.filter((node) => ts.isFunctionDeclaration(node) && ["GroupAgentSelect", "SkillRepoDrawer", "SkillsPage"].includes(node.name?.text || ""));
assert.equal(selected.length, 3);
const code = ts.transpileModule(selected.map((node) => node.getText(file)).join("\n"), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

const skill = (id: string, repoKey: string): FixtureSkill => ({ id, name: id, description: "", repoKey, installed: true,
  enabled: false, bundled: false, updateAvailable: false, agents: [], managed: true });
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

function createHarness(saved = "{}") {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const storage = new Map([["codex-plus-plus.skill-groups.v1", saved]]);
  const notices: Array<{ message: string; status?: string }> = [];
  const pending: Array<{ id: string; resolve: (result: any) => void }> = [];
  const pendingLoads: Array<() => void> = [];
  const pendingDataUpdates: Array<() => void> = [];
  let holdLoads = 0;
  let deferDataUpdates = false;
  const windowListeners = new Map<string, () => void>();
  const failures = new Set(["a2"]);
  const updateCalls: Array<{ command: string; args: any }> = [];
  const installCalls: Array<{ command: string; args: any }> = [];
  const updateQueue = new SkillTaskQueue<any>();
  let releasePreview: (() => void) | undefined;
  const previewGate = new Promise<void>((resolve) => { releasePreview = resolve; });
  let cursor = 0;
  let elements: Element[] = [];
  const server = { status: "ok", message: "ok", skills: [skill("a", "a/repo@main"), skill("a2", "a/repo@main"), skill("z", "z/repo@main")],
    repos: [], pendingImportedRepos: [], repositorySkills: [], repoCounts: {}, repoErrors: [], skillsDir: "", codexSkillsDir: "" };
  const snapshot = () => structuredClone(server);
  const invoke = async (command: string, args: any = {}) => {
    if (command === "list_installed_skills") {
      const result = snapshot();
      if (holdLoads > 0) {
        holdLoads--;
        return new Promise((resolve) => pendingLoads.push(() => resolve(result)));
      }
      return result;
    }
    if (command === "preview_skill_update") {
      updateCalls.push({ command, args });
      if (args.id === "a") await previewGate;
      return { status: "ok", message: "ok", snapshot: args.id === "a2" ? "changed-snapshot" : null };
    }
    if (command === "update_skill" || command === "update_skill_confirmed") {
      updateCalls.push({ command, args });
      return snapshot();
    }
    if (command === "install_skill") {
      installCalls.push({ command, args });
      return snapshot();
    }
    if (command !== "set_skill_enabled_for") throw new Error(`unexpected command ${command}`);
    const target = server.skills.find((item) => item.id === args.id)!;
    if (args.id === "z") {
      target.agents = [args.agent];
      return snapshot();
    }
    if (args.id === "a" && !failures.has(args.id)) {
      target.agents = target.agents.filter((agent) => agent !== args.agent);
      if (args.enabled) target.agents.push(args.agent);
    }
    const result = snapshot();
    if (failures.has(args.id)) { result.status = "failed"; result.message = "fixture failure"; }
    return new Promise((resolve) => pending.push({ id: args.id, resolve: () => resolve(result) }));
  };
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }), fragment: "fragment",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => {
        const apply = () => { slots[index] = typeof value === "function" ? value(slots[index]) : value; };
        if (index === 0 && deferDataUpdates) pendingDataUpdates.push(apply);
        else apply();
      }];
    },
    useRef: (initial: any) => { const index = cursor++; if (!(index in slots)) slots[index] = { current: initial }; return slots[index]; },
    useEffect: (run: () => void, deps?: any[]) => {
      const index = cursor++;
      const previous = slots[index] as any[] | undefined;
      if (!previous || !deps || deps.some((value, i) => !Object.is(value, previous[i]))) effects.push(run);
      slots[index] = deps;
    },
    useMemo: (run: () => any) => run(), useDeferredValue: (value: any) => value,
    startTransition: (run: () => void) => run(),
    useSyncExternalStore: (_subscribe: any, getSnapshot: () => any) => getSnapshot(),
    useSensors: (...values: any[]) => values, useSensor: () => null, PointerSensor: "pointer", KeyboardSensor: "keyboard",
    sortableKeyboardCoordinates: null, closestCenter: null, verticalListSortingStrategy: null,
    localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) },
    window: { addEventListener: (name: string, listener: () => void) => windowListeners.set(name, listener),
      removeEventListener: (name: string) => windowListeners.delete(name) },
    invoke, ok: (result: { status: string }) => result.status === "ok", SKILL_GROUP_STORAGE_KEY: "codex-plus-plus.skill-groups.v1",
    t: (value: string) => value, tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    groupSkills, legacySkillCustomGroupName, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName, customSkillGroupKey, defaultSkillGroupKey,
    mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchState, skillBatchTargets,
    matchesManagementQuery: (values: string[], query: string) => !query || values.some((value) => value?.toLowerCase().includes(query.toLowerCase())),
    repoKey: (repo: any) => `${repo.owner}/${repo.name}@${repo.branch}${repo.subdir ? `:${repo.subdir}` : ""}`,
    repoFromKey: () => null,
    repoUrl: () => "", sourceLabel: (item: FixtureSkill) => item.repoKey.split("@", 1)[0], importChannel: () => "",
    MCP_AGENTS: ["codex", "claude", "grok"], skillUpdateQueue: updateQueue,
    checkRepositories: async () => {}, openDialog: async () => null, appConfirm: async () => true, appPrompt: async () => null,
  };
  for (const name of ["Header", "Button", "SearchField", "AgentColumn", "MatrixCell", "SortableSkillGroup", "DndContext", "SortableContext", "SkillActionMenu",
    "SkillDetailDrawer", "SkillImportDrawer", "SkillConflictDialog", "BookOpen", "RefreshCw", "CloudDownload", "FolderOpen", "Link", "FolderPlus", "ArrowDownAZ",
    "ArrowDownZA", "ArrowDownUp", "ChevronsUp", "ChevronsDown", "ChevronDown", "Edit3", "Trash2", "AlertTriangle", "Search", "SkillRepositoryMatchDrawer",
    "ManagementDrawer", "SearchBox", "SkillListViewport", "Plus", "ExternalLink"]) context[name] = name;
  vm.createContext(context);
  vm.runInContext(code, context);
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    elements.push(node);
    if (node.type === "SortableSkillGroup") collect(node.children[0]("handle"));
    else node.children?.forEach(collect);
    if (node.props?.actions) collect(node.props.actions);
  };
  const render = () => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.exports.SkillsPage({ notify: async (_title: string, message: string, status?: string) => { notices.push({ message, status }); } }));
    effects.splice(0).forEach((run) => run());
    return elements;
  };
  const find = (predicate: (node: Element) => boolean) => { const result = elements.find(predicate); assert.ok(result); return result; };
  const group = (key: string) => find((node) => node.type === "SortableSkillGroup" && node.props.group.key === key);
  const groupSelect = (key: string, agent: string) => find((node) => node.type === context.GroupAgentSelect && node.props.group.key === key && node.props.agent === agent);
  const groupInput = (key: string, agent: string) => context.GroupAgentSelect(groupSelect(key, agent).props).children[0] as Element;
  const row = (name: string, agent: string) => find((node) => node.type === "MatrixCell" && node.props.name === name && node.props.agent === agent);
  const sort = () => find((node) => node.type === "button" && String(node.props["aria-label"] || "").startsWith("按名称排序"));
  const order = () => elements.filter((node) => node.type === "SortableSkillGroup").map((node) => node.props.group.key);
  const expand = (key: string) => {
    const subtree: Element[] = [];
    const walk = (node: any) => { if (!node || typeof node !== "object") return; if (Array.isArray(node)) return node.forEach(walk); subtree.push(node); node.children?.forEach(walk); };
    walk(group(key).children[0]("handle"));
    subtree.find((node) => node.type === "button" && node.props.className === "mg-group-head")!.props.onClick();
    render();
  };
  const renderDrawer = (props: any) => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.SkillRepoDrawer(props));
    return elements;
  };
  return { render, renderDrawer, find, groupSelect, groupInput, row, sort, order, expand, pending, pendingLoads, pendingDataUpdates, failures, notices, storage, server, updateCalls, installCalls, updateQueue,
    holdLoads: (count: number) => { holdLoads = count; }, deferDataUpdates: () => { deferDataUpdates = true; },
    focus: () => windowListeners.get("focus")?.(), releasePreview: () => releasePreview?.() };
}

test("discover installs the selected repository path with its distinct local identity", async () => {
  const ui = createHarness();
  const installed = { ...skill("alpha-original", "a/repo@main"), name: "Alpha", repoPath: "a/alpha" };
  const candidate = { ...skill("cpx-skill-alias", "a/repo@main"), name: "Alpha", repoPath: "z/alpha", installed: false };
  ui.server.skills = [installed, candidate];
  ui.render(); await tick(); ui.render();
  ui.find((node) => node.type === "button" && node.children.includes("发现")).props.onClick();
  ui.render(); ui.expand("repo:a/repo");
  ui.find((node) => node.type === "SearchField").props.onChange("z/alpha");
  ui.render();
  const candidateRow = ui.find((node) => node.props.className === "mg-matrix-row");
  assert.equal(candidateRow.props.key, skillIdentityKey(candidate));
  assert.equal(candidateRow.children[0].children[1].props.title, "z/alpha");
  ui.find((node) => node.type === "Button" && node.children.includes("安装")).props.onClick();
  await tick();
  assert.equal(ui.installCalls.length, 1);
  assert.equal(ui.installCalls[0].args.repoKey, "a/repo@main");
  assert.equal(ui.installCalls[0].args.id, "cpx-skill-alias");
  assert.equal(ui.installCalls[0].args.expectedRepoPath, "z/alpha");
  ui.render();
  ui.find((node) => node.type === "SearchField").props.onChange("");
  ui.render();
  ui.find((node) => node.type === "button" && node.children.includes("已安装")).props.onClick();
  ui.render();
  const installedRow = ui.find((node) => node.props.className === "mg-matrix-row");
  assert.equal(installedRow.props.key, skillIdentityKey(installed));
  assert.equal(installedRow.children[0].children[1].props.title, "a/alpha");
});

test("repository drawer displays both paths and installs the selected candidate", () => {
  const ui = createHarness();
  const repo = { owner: "a", name: "repo", branch: "main", subdir: "", enabled: true };
  const installed = { ...skill("alpha-original", "a/repo@main"), name: "Alpha", repoPath: "a/alpha" };
  const candidate = { ...skill("cpx-skill-alias", "a/repo@main"), name: "Alpha", repoPath: "z/alpha", installed: false };
  const calls: Array<{ command: string; args: any }> = [];
  ui.renderDrawer({ repos: [repo], pendingRepos: [], catalog: [installed, candidate], counts: { "a/repo@main": 2 },
    checks: {}, initialKey: "a/repo@main", busy: false, onRun: (command: string, args: any) => calls.push({ command, args }),
    onClose: () => {}, onSave: async () => true, onDelete: () => {}, onInspect: async () => {}, onOpenUrl: () => {}, onOpenSkill: () => {} });
  const viewport = ui.find((node) => node.type === "SkillListViewport");
  assert.equal(viewport.props.items.length, 2);
  const installedRow = viewport.props.renderRow(installed) as Element;
  const candidateRow = viewport.props.renderRow(candidate) as Element;
  assert.notEqual(installedRow.props.key, candidateRow.props.key);
  assert.equal(installedRow.children[0].children[1].props.title, "a/alpha");
  assert.equal(candidateRow.children[0].children[1].props.title, "z/alpha");
  const install = candidateRow.children[1].children[1].children[0];
  assert.equal(install.type, "Button");
  install.props.onClick();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "install_skill");
  assert.equal(calls[0].args.repoKey, "a/repo@main");
  assert.equal(calls[0].args.id, "cpx-skill-alias");
  assert.equal(calls[0].args.expectedRepoPath, "z/alpha");
});

test("batch selection leaves unrelated buttons and agent controls enabled and preserves newer results", async () => {
  const ui = createHarness();
  ui.render(); await tick(); ui.render();
  ui.expand("repo:a/repo"); ui.expand("repo:z/repo");
  ui.groupInput("repo:a/repo", "codex").props.onChange({ target: { checked: true } });
  ui.render();
  assert.equal(ui.pending[0].id, "a");
  assert.equal(ui.groupSelect("repo:a/repo", "codex").props.busy, true);
  assert.equal(ui.groupInput("repo:a/repo", "codex").props.checked, true);
  assert.equal(ui.groupSelect("repo:a/repo", "claude").props.busy, false);
  assert.equal(ui.groupSelect("repo:z/repo", "codex").props.busy, false);
  assert.equal(ui.row("a", "codex").props.disabled, true);
  assert.equal(ui.row("a", "claude").props.disabled, false);
  assert.equal(ui.row("z", "claude").props.disabled, false);
  for (const label of ["从本地目录导入", "从 cc-switch 导入"]) {
    assert.equal(ui.find((node) => node.type === "Button" && node.children.includes(label)).props.disabled, false);
  }

  ui.row("z", "claude").props.onToggle();
  await tick(); ui.render();
  assert.equal(ui.row("z", "claude").props.enabled, true);
  ui.pending.shift()!.resolve(null);
  await tick(); ui.render();
  assert.equal(ui.pending[0].id, "a2");
  assert.equal(ui.row("a", "codex").props.enabled, true);
  assert.equal(ui.row("z", "claude").props.enabled, true);
  assert.equal(ui.groupSelect("repo:a/repo", "codex").props.busy, true);

  ui.pending.shift()!.resolve(null);
  await tick(); ui.render(); await tick(); ui.render();
  assert.equal(ui.groupSelect("repo:a/repo", "codex").props.busy, false);
  assert.equal(ui.row("a", "codex").props.enabled, true);
  assert.equal(ui.row("a2", "codex").props.enabled, false);
  assert.equal(ui.row("z", "claude").props.enabled, true);
  const input = ui.groupInput("repo:a/repo", "codex");
  const element = { indeterminate: false };
  input.props.ref(element);
  assert.equal(element.indeterminate, true);
  assert.ok(ui.notices.some((notice) => notice.status === "failed" && notice.message.includes("fixture failure")));
});

test("a delayed batch response cannot overwrite a newer refresh for the same agent", async () => {
  const ui = createHarness();
  ui.server.skills = [skill("a", "a/repo@main")];
  ui.render(); await tick(); ui.render(); ui.expand("repo:a/repo");
  ui.groupInput("repo:a/repo", "codex").props.onChange({ target: { checked: true } });
  ui.render();
  assert.equal(ui.pending.length, 1);

  // The pending response captured the enabled state; a later external change disables it.
  ui.server.skills[0].agents = [];
  ui.focus(); await tick(); ui.render();
  ui.holdLoads(10);
  ui.pending.shift()!.resolve(null);
  await tick(); ui.render();
  assert.equal(ui.row("a", "codex").props.enabled, false);
  assert.ok(ui.pendingLoads.length > 0);
  ui.pendingLoads.forEach((resolve) => resolve());
  await tick(); ui.render();
  assert.equal(ui.row("a", "codex").props.enabled, false);
});

test("a delayed refresh response cannot overwrite a newer batch result", async () => {
  const ui = createHarness();
  ui.server.skills = [skill("a", "a/repo@main")];
  ui.render(); await tick(); ui.render(); ui.expand("repo:a/repo");
  ui.holdLoads(1);
  ui.focus(); await tick();
  assert.equal(ui.pendingLoads.length, 1);

  ui.groupInput("repo:a/repo", "codex").props.onChange({ target: { checked: true } });
  ui.pending.shift()!.resolve(null);
  await tick(); ui.render();
  assert.equal(ui.row("a", "codex").props.enabled, true);
  ui.pendingLoads.shift()!();
  await tick(); ui.render();
  assert.equal(ui.row("a", "codex").props.enabled, true);
});

test("a successful batch result is not dropped while its follow-up refresh is pending", async () => {
  const ui = createHarness();
  ui.server.skills = [skill("a", "a/repo@main")];
  ui.render(); await tick(); ui.render(); ui.expand("repo:a/repo");
  ui.groupInput("repo:a/repo", "codex").props.onChange({ target: { checked: true } });
  ui.deferDataUpdates();
  ui.holdLoads(10);
  ui.pending.shift()!.resolve(null);
  await tick();
  assert.equal(ui.pendingDataUpdates.length, 1);
  assert.ok(ui.pendingLoads.length > 0);
  ui.pendingDataUpdates.shift()!();
  ui.render();
  assert.equal(ui.row("a", "codex").props.enabled, true);
});

test("filter changes keep the same group and agent locked without projecting untouched skills", async () => {
  const ui = createHarness();
  ui.server.skills[0].name = "first-target";
  ui.server.skills[1].name = "second-visible";
  ui.render(); await tick(); ui.render();
  ui.find((node) => node.type === "SearchField").props.onChange("first-target"); ui.render();
  const initial = ui.groupInput("repo:a/repo", "codex");
  initial.props.onChange({ target: { checked: true } });
  initial.props.onChange({ target: { checked: true } });
  assert.equal(ui.pending.length, 1);
  ui.render();
  ui.find((node) => node.type === "SearchField").props.onChange("second-visible"); ui.render();
  const filtered = ui.groupInput("repo:a/repo", "codex");
  assert.equal(filtered.props.disabled, true);
  assert.equal(filtered.props.checked, false);
  filtered.props.onChange({ target: { checked: true } });
  assert.equal(ui.pending.length, 1);
  ui.find((node) => node.type === "SearchField").props.onChange(""); ui.render();
  const full = ui.groupInput("repo:a/repo", "codex");
  const element = { indeterminate: false };
  full.props.ref(element);
  assert.equal(full.props.checked, false);
  assert.equal(element.indeterminate, true);
  assert.equal(ui.groupInput("repo:a/repo", "claude").props.disabled, false);
  assert.equal(ui.groupInput("repo:z/repo", "codex").props.disabled, false);
  ui.groupInput("repo:z/repo", "codex").props.onChange({ target: { checked: true } });
  await tick(); ui.render();
  assert.equal(ui.server.skills[2].agents.includes("codex"), true);
  ui.pending.shift()!.resolve(null);
  await tick(); ui.render();
  assert.equal(ui.groupInput("repo:a/repo", "codex").props.disabled, false);
});

test("filtering to an already-enabled member keeps its real checked state and blocks a second batch", async () => {
  const ui = createHarness();
  ui.server.skills[0].name = "first-target";
  ui.server.skills[1].name = "second-visible";
  ui.server.skills[1].agents = ["codex"];
  ui.render(); await tick(); ui.render();
  ui.find((node) => node.type === "SearchField").props.onChange("first-target"); ui.render();
  ui.groupInput("repo:a/repo", "codex").props.onChange({ target: { checked: true } });
  ui.render();
  ui.find((node) => node.type === "SearchField").props.onChange("second-visible"); ui.render();
  const filtered = ui.groupInput("repo:a/repo", "codex");
  assert.equal(filtered.props.checked, true);
  assert.equal(filtered.props.disabled, true);
  filtered.props.onChange({ target: { checked: false } });
  assert.equal(ui.pending.length, 1);
  ui.pending.shift()!.resolve(null);
  await tick(); ui.render();
  assert.equal(ui.groupInput("repo:a/repo", "codex").props.checked, true);
  assert.equal(ui.groupInput("repo:a/repo", "codex").props.disabled, false);
});

test("a filtered-out pending disable does not change another enabled member's display", async () => {
  const ui = createHarness();
  ui.server.skills[0].name = "first-target";
  ui.server.skills[1].name = "second-visible";
  ui.server.skills[0].agents = ["codex"];
  ui.server.skills[1].agents = ["codex"];
  ui.render(); await tick(); ui.render();
  ui.find((node) => node.type === "SearchField").props.onChange("first-target"); ui.render();
  ui.groupInput("repo:a/repo", "codex").props.onChange({ target: { checked: false } });
  ui.render();
  ui.find((node) => node.type === "SearchField").props.onChange("second-visible"); ui.render();
  const filtered = ui.groupInput("repo:a/repo", "codex");
  assert.equal(filtered.props.checked, true);
  assert.equal(filtered.props.disabled, true);
  ui.pending.shift()!.resolve(null);
  await tick(); ui.render();
  assert.equal(ui.groupInput("repo:a/repo", "codex").props.checked, true);
});

test("moving a pending target to another group keeps that group locked even when the target is filtered out", async () => {
  const ui = createHarness();
  ui.server.skills[0].name = "first-target";
  ui.server.skills[1].name = "second-visible";
  ui.server.skills[1].repoKey = "";
  ui.render(); await tick(); ui.render();
  ui.find((node) => node.type === "SearchField").props.onChange("first-target"); ui.render();
  ui.groupInput("repo:a/repo", "codex").props.onChange({ target: { checked: true } });
  ui.render();
  ui.find((node) => node.type === "SkillActionMenu" && node.props.skill.id === "a").props.onAssign("user");
  ui.render();
  ui.find((node) => node.type === "SearchField").props.onChange("second-visible"); ui.render();
  const filtered = ui.groupInput("user", "codex");
  assert.equal(filtered.props.disabled, true);
  assert.equal(filtered.props.checked, false);
  filtered.props.onChange({ target: { checked: true } });
  assert.equal(ui.pending.length, 1);
  ui.pending.shift()!.resolve(null);
  await tick(); ui.render();
  assert.equal(ui.groupInput("user", "codex").props.disabled, false);
});

test("a failed completed target shows its actual state while the remaining target is pending", async () => {
  const ui = createHarness();
  ui.failures.add("a");
  ui.render(); await tick(); ui.render();
  ui.groupInput("repo:a/repo", "codex").props.onChange({ target: { checked: true } });
  ui.render();
  assert.equal(ui.groupInput("repo:a/repo", "codex").props.checked, true);
  ui.pending.shift()!.resolve(null);
  await tick(); ui.render();
  assert.equal(ui.pending[0].id, "a2");
  const input = ui.groupInput("repo:a/repo", "codex");
  const element = { indeterminate: false };
  input.props.ref(element);
  assert.equal(input.props.disabled, true);
  assert.equal(input.props.checked, false);
  assert.equal(element.indeterminate, true);
  ui.pending.shift()!.resolve(null);
  await tick(); ui.render();
  assert.equal(ui.groupInput("repo:a/repo", "codex").props.disabled, false);
});

test("group update queues the complete group even when filtered and deduplicates pending tasks", async () => {
  const ui = createHarness();
  ui.server.skills[0].updateAvailable = true;
  ui.server.skills[0].repoPath = "skills/a";
  ui.server.skills[1].updateAvailable = true;
  ui.server.skills[2].updateAvailable = true;
  ui.render(); await tick(); ui.render();
  const queuedSkills = ui.groupSelect("repo:a/repo", "codex").props.group.items as FixtureSkill[];
  ui.find((node) => node.type === "SearchField").props.onChange("a2");
  ui.render();
  const update = ui.find((node) => node.type === "button" && node.props["aria-label"] === "更新分组内全部 Skill a/repo");
  assert.equal(update.children[0].type, "CloudDownload");
  assert.equal(update.props.title, "更新分组内全部 Skill");
  update.props.onClick();
  update.props.onClick();
  ui.render();
  assert.equal(ui.find((node) => node.type === "button" && node.props["aria-label"] === "更新分组内全部 Skill a/repo").props.disabled, true);
  assert.deepEqual(ui.updateQueue.getSnapshot().map((entry) => entry.key), ["a/repo@main\0a", "a/repo@main\0a2"]);
  assert.deepEqual(ui.updateCalls.map((call) => [call.command, call.args.id]), [["preview_skill_update", "a"]]);
  queuedSkills.find((item) => item.id === "a")!.repoPath = "mutated/a";
  queuedSkills.find((item) => item.id === "a2")!.repoPath = "mutated/a2";
  ui.server.skills[0].repoPath = "relinked/a";
  ui.server.skills[1].repoPath = "relinked/a2";
  ui.releasePreview();
  await tick(); await tick(); ui.render();
  assert.deepEqual(ui.updateCalls.map((call) => [call.command, call.args.id]), [
    ["preview_skill_update", "a"], ["update_skill", "a"], ["preview_skill_update", "a2"], ["update_skill_confirmed", "a2"],
  ]);
  assert.equal(ui.updateCalls[1].args.expectedRepoPath, "skills/a");
  assert.equal(ui.updateCalls[3].args.expectedRepoPath, null);
  assert.equal(ui.updateCalls[3].args.expectedSnapshot, "changed-snapshot");
});

test("both update paths pass the original repository path, including missing paths", async () => {
  const ui = createHarness();
  ui.server.skills[0].updateAvailable = true;
  ui.server.skills[1].updateAvailable = true;
  ui.server.skills[1].repoPath = "skills/a2";
  ui.render(); await tick(); ui.render();
  ui.find((node) => node.type === "button" && node.props["aria-label"] === "更新分组内全部 Skill a/repo").props.onClick();
  ui.server.skills[1].repoPath = "relinked/a2";
  ui.releasePreview();
  await tick(); await tick(); ui.render();
  assert.equal(ui.updateCalls[1].args.expectedRepoPath, null);
  assert.equal(ui.updateCalls[3].args.expectedRepoPath, "skills/a2");
  assert.equal(ui.updateCalls[3].args.expectedSnapshot, "changed-snapshot");
});

test("matching refresh keeps the installed skill and its new repository group when an old catalog item comes first", async () => {
  const ui = createHarness();
  ui.server.skills[0].repoPath = "skills/old-a";
  ui.render(); await tick(); ui.render();
  ui.expand("repo:a/repo");
  ui.find((node) => node.type === "button" && node.props.className === "mg-row-main" && node.children[0]?.children[0]?.props.title === "a").props.onClick();
  ui.render();
  ui.find((node) => node.type === "SkillDetailDrawer").props.onEditSource();
  ui.render();

  const oldCatalogItem = { ...ui.server.skills[0], installed: false };
  const installed = { ...ui.server.skills[0], repoKey: "new/repo@main", repoPath: "skills/new-a" };
  ui.server.skills = [oldCatalogItem, installed, ...ui.server.skills.slice(1)];
  ui.find((node) => node.type === "SkillRepositoryMatchDrawer").props.onUpdated();
  await tick(); ui.render();
  const detail = ui.find((node) => node.type === "SkillDetailDrawer").props;
  assert.equal(detail.skill.installed, true);
  assert.equal(detail.skill.repoKey, "new/repo@main");
  assert.equal(detail.skill.repoPath, "skills/new-a");
  assert.equal(detail.group.key, "repo:new/repo");
  assert.equal(detail.originGroup.key, "repo:new/repo");
});

test("matching refresh closes installed details when only an old uninstalled catalog item remains", async () => {
  const ui = createHarness();
  ui.render(); await tick(); ui.render();
  ui.expand("repo:a/repo");
  ui.find((node) => node.type === "button" && node.props.className === "mg-row-main" && node.children[0]?.children[0]?.props.title === "a").props.onClick();
  ui.render();
  ui.find((node) => node.type === "SkillDetailDrawer").props.onEditSource();
  ui.render();
  ui.server.skills[0].installed = false;
  ui.find((node) => node.type === "SkillRepositoryMatchDrawer").props.onUpdated();
  await tick();
  assert.equal(ui.render().some((node) => node.type === "SkillDetailDrawer"), false);
});

test("a pending matching refresh does not reopen details closed by the user", async () => {
  const ui = createHarness();
  ui.render(); await tick(); ui.render();
  ui.expand("repo:a/repo");
  ui.find((node) => node.type === "button" && node.props.className === "mg-row-main" && node.children[0]?.children[0]?.props.title === "a").props.onClick();
  ui.render();
  ui.find((node) => node.type === "SkillDetailDrawer").props.onEditSource();
  ui.render();
  ui.find((node) => node.type === "SkillRepositoryMatchDrawer").props.onUpdated();
  ui.find((node) => node.type === "SkillDetailDrawer").props.onClose();
  await tick();
  assert.equal(ui.render().some((node) => node.type === "SkillDetailDrawer"), false);
});

test("sort button toggles order and icon, persists direction, and respects manual dragging", async () => {
  const ui = createHarness();
  ui.render(); await tick(); ui.render();
  assert.equal(ui.sort().children[0].type, "ArrowDownUp");
  ui.sort().props.onClick(); ui.render();
  assert.deepEqual(ui.order(), ["builtin", "user", "repo:a/repo", "repo:z/repo"]);
  assert.equal(ui.sort().children[0].type, "ArrowDownAZ");
  assert.match(ui.sort().props["aria-label"], /A-Z/);
  ui.sort().props.onClick(); ui.render();
  assert.deepEqual(ui.order(), ["builtin", "user", "repo:z/repo", "repo:a/repo"]);
  assert.equal(ui.sort().children[0].type, "ArrowDownZA");
  assert.match(ui.sort().props["aria-label"], /Z-A/);
  const saved = ui.storage.get("codex-plus-plus.skill-groups.v1")!;
  assert.equal(JSON.parse(saved).sortDirection, "desc");
  const restored = createHarness(saved);
  restored.render(); await tick(); restored.render();
  assert.deepEqual(restored.order(), ["builtin", "user", "repo:z/repo", "repo:a/repo"]);
  assert.equal(restored.sort().children[0].type, "ArrowDownZA");
  const dnd = ui.find((node) => node.type === "DndContext");
  dnd.props.onDragEnd({ active: { id: "repo:z/repo" }, over: { id: "user" } }); ui.render();
  assert.equal(ui.sort().children[0].type, "ArrowDownZA");
  ui.find((node) => node.type === "DndContext").props.onDragEnd({ active: { id: "repo:z/repo" }, over: { id: "repo:a/repo" } }); ui.render();
  assert.deepEqual(ui.order(), ["builtin", "user", "repo:a/repo", "repo:z/repo"]);
  assert.equal(ui.sort().children[0].type, "ArrowDownUp");
  ui.sort().props.onClick(); ui.render();
  assert.equal(ui.sort().children[0].type, "ArrowDownAZ");
});
