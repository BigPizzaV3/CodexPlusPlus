import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { discardSkillSourceScan, getSkillSourceScan, skillSourceRepoKey as keyOfRepo } from "./skill-source-scan.ts";

type Element = { type: any; props: Record<string, any>; children: any[] };

const source = readFileSync(new URL("./SkillSourceMatchDrawer.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("SkillSourceMatchDrawer.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const nodes = file.statements.filter((node) =>
  ts.isFunctionDeclaration(node) && node.name?.text === "SkillSourceMatchDrawer"
  || ts.isVariableStatement(node) && node.declarationList.declarations.some((item) =>
    ts.isIdentifier(item.name) && ["keyOfRepo", "keyOfCandidate", "evidenceLabel"].includes(item.name.text)));
const code = ts.transpileModule(nodes.map((node) => node.getText(file)).join("\n"), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test("batch discovery exposes local evidence and submits only preview-bound choices", async () => {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const calls: Array<{ command: string; args: any }> = [];
  const selections: any[] = [];
  let cursor = 0;
  let elements: Element[] = [];
  let closed = false;
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useEffect: (run: () => void, deps: any[]) => {
      const index = cursor++;
      const previous = slots[index] as any[] | undefined;
      if (!previous || deps.some((value, position) => !Object.is(value, previous[position]))) effects.push(run);
      slots[index] = deps;
    },
    useMemo: (run: () => any) => run(),
    useRef: (value: any) => { const index = cursor++; return slots[index] ??= { current: value }; },
    t: (value: string) => value,
    tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    invoke: async (command: string, args: any) => {
      calls.push({ command, args });
      if (command === "discover_skill_source_evidence") return {
        status: "ok", message: "", evidence: args.id === "alpha"
          ? [{ repo: "owner/alpha", kind: "lock", path: "skills-lock.json" }]
          : [{ repo: "owner/beta", kind: "script-link", path: "start.ps1" }],
      };
      assert.equal(command, "find_skill_repository_matches");
      return { status: "ok", message: "", previewId: "preview-one", warnings: [], manualCandidates: [],
        matches: [{ localId: "alpha", localName: "Alpha", repoKey: "owner/alpha@main",
          repoPath: "skills/alpha", remoteName: "Alpha", exact: true, localHash: "hidden-local",
          snapshot: "hidden-snapshot", remoteHash: "hidden-remote" }] };
    },
    AppSelect: "AppSelect", ManagementDrawer: "ManagementDrawer", Pause: "Pause", Play: "Play", Plus: "Plus", RefreshCw: "RefreshCw", Search: "Search",
    discardSkillSourceScan, getSkillSourceScan, keyOfRepo,
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    elements.push(node);
    node.children?.forEach(collect);
    if (node.props?.footer) collect(node.props.footer);
  };
  const props = {
    repos: [{ owner: "owner", name: "alpha", branch: "main", subdir: "", enabled: true }],
    localSkillIds: ["alpha", "beta", "alpha"],
    onClose: () => { closed = true; },
    onAddRepo: async () => null,
    onApply: async (items: any[]) => { selections.push(...items); return { status: "ok", message: "" }; },
  };
  const render = () => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.exports.SkillSourceMatchDrawer(props));
    effects.splice(0).forEach((run) => run());
  };
  render();
  await tick();
  await tick();
  render();
  assert.deepEqual(calls.filter((call) => call.command === "discover_skill_source_evidence").map((call) => call.args.id),
    ["alpha", "beta"]);
  assert.ok(elements.some((node) => node.type === "small" && node.children.includes("alpha · 锁文件")));
  assert.ok(elements.some((node) => node.type === "small" && node.children.includes("beta · 脚本中的链接")));

  const checkbox = elements.find((node) => node.type === "input" && node.props.type === "checkbox");
  assert.ok(checkbox);
  checkbox.props.onChange({ currentTarget: { checked: true } });
  render();
  const confirm = elements.find((node) => node.type === "button" && node.children.includes("关联所选 1 项"));
  assert.ok(confirm);
  assert.equal(confirm.props.disabled, false);
  await confirm.props.onClick();
  await tick();
  assert.deepEqual(JSON.parse(JSON.stringify(selections)), [{
    previewId: "preview-one", localId: "alpha", repoKey: "owner/alpha@main", repoPath: "skills/alpha",
  }]);
  assert.equal(closed, true);
});

test("single-skill matching binds each installation to the selected repository path", async () => {
  for (const { localPath, currentRepoKey, currentRepoPath, single } of [
    { localPath: "C:/agents/impeccable", currentRepoKey: undefined, currentRepoPath: undefined, single: false },
    { localPath: undefined, currentRepoKey: undefined, currentRepoPath: undefined, single: false },
    { localPath: "C:/claude/impeccable", currentRepoKey: "owner/repo@main", currentRepoPath: ".codex/skills/impeccable", single: false },
    { localPath: "C:/grok/impeccable", currentRepoKey: undefined, currentRepoPath: undefined, single: true },
  ]) {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const calls: Array<{ command: string; args: any }> = [];
  const selections: any[] = [];
  let cursor = 0;
  let elements: Element[] = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useEffect: (run: () => void, deps: any[]) => {
      const index = cursor++;
      const previous = slots[index] as any[] | undefined;
      if (!previous || deps.some((value, position) => !Object.is(value, previous[position]))) effects.push(run);
      slots[index] = deps;
    },
    useMemo: (run: () => any) => run(),
    useRef: (value: any) => { const index = cursor++; return slots[index] ??= { current: value }; },
    t: (value: string) => value,
    tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    invoke: async (command: string, args: any) => {
      calls.push({ command, args });
      if (command === "discover_skill_source_evidence") return { status: "ok", message: "", evidence: [] };
      return { status: "ok", message: "", previewId: "instance-preview", warnings: [],
        manualCandidates: [{ localId: "impeccable", localName: "impeccable", repoKey: "owner/aaa@main",
          repoPath: "skills/unrelated", remoteName: "unrelated", exact: false, localPaths: ["C:/agents/other"] }],
        matches: [
          ...single ? [] : [{ localId: "impeccable", localName: "impeccable", repoKey: "owner/repo@main",
            repoPath: ".agents/skills/impeccable", remoteName: "impeccable", exact: true, localPaths: ["C:/agents/other"] }],
          { localId: "impeccable", localName: "impeccable", repoKey: "owner/repo@main",
            repoPath: ".codex/skills/impeccable", remoteName: "impeccable", exact: false, localPaths: ["C:/agents/other"] }] };
    },
    AppSelect: "AppSelect", ManagementDrawer: "ManagementDrawer", Pause: "Pause", Play: "Play", Plus: "Plus", RefreshCw: "RefreshCw", Search: "Search",
    discardSkillSourceScan, getSkillSourceScan, keyOfRepo,
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    elements.push(node);
    node.children?.forEach(collect);
    if (node.props?.footer) collect(node.props.footer);
  };
  const props = {
    id: "impeccable", localPath, agentLabel: "Agent", currentRepoKey, currentRepoPath,
    repos: [{ owner: "owner", name: "repo", branch: "main", subdir: "", enabled: true }],
    localSkillIds: [], onClose: () => {}, onAddRepo: async () => null,
    onApply: async (items: any[]) => { selections.push(...items); return { status: "ok", message: "" }; },
  };
  const render = () => {
    cursor = 0; elements = []; effects.length = 0;
    collect(context.exports.SkillSourceMatchDrawer(props));
    effects.splice(0).forEach((run) => run());
  };
  render(); await tick(); await tick(); render();
  for (const command of ["discover_skill_source_evidence", "find_skill_repository_matches"]) {
    const request = calls.find((call) => call.command === command);
    assert.equal(request?.args.id, props.id);
    assert.equal(request?.args.localPath, props.localPath || null);
  }
  const checkbox = elements.find((node) => node.type === "input" && node.props.type === "checkbox");
  assert.ok(checkbox);
  checkbox.props.onChange({ currentTarget: { checked: true } }); render();
  const select = elements.find((node) => node.type === "AppSelect");
  assert.ok(select);
  assert.deepEqual(Array.from(select.props.options, (option: any) => option.label), [
    ...single ? [] : ["owner/repo@main / .agents/skills/impeccable"],
    "owner/repo@main / .codex/skills/impeccable",
    "owner/aaa@main / skills/unrelated",
  ]);
  const wanted = JSON.stringify(["owner/repo@main", ".codex/skills/impeccable"]);
  assert.equal(select.props.value, single || currentRepoKey ? wanted : "");
  if (localPath) {
    assert.ok(elements.some((node) => node.type === "dd" && node.children.includes(`Agent · ${localPath}`)));
    assert.ok(elements.some((node) => node.type === "small" && node.children.includes(localPath)));
    assert.ok(!elements.some((node) => node.type === "small" && node.children.includes("C:/agents/other")));
  } else {
    assert.ok(elements.some((node) => node.type === "small" && node.children.includes("C:/agents/other")));
  }
  let confirm = elements.find((node) => node.type === "button" && node.children.includes("关联所选 1 项"));
  assert.ok(confirm);
  if (!single && !currentRepoKey) {
    assert.equal(confirm.props.disabled, true);
    select.props.onChange(wanted);
    render();
    confirm = elements.find((node) => node.type === "button" && node.children.includes("关联所选 1 项"));
    assert.ok(confirm);
  }
  assert.equal(confirm.props.disabled, false);
  await confirm.props.onClick(); await tick();
  assert.equal(selections[0].localPath, props.localPath);
  assert.equal(selections[0].previewId, "instance-preview");
  assert.equal(selections[0].localId, props.id);
  assert.equal(selections[0].repoKey, "owner/repo@main");
  assert.equal(selections[0].repoPath, ".codex/skills/impeccable");
  }
});

test("background scanning can be paused for a manual association and restarted after an expired preview", async () => {
  let slots: any[] = [];
  let effects: Array<() => void> = [];
  let cleanups: Array<() => void> = [];
  let cursor = 0;
  let elements: Element[] = [];
  const started: string[] = [];
  const pending: Array<(value: unknown) => void> = [];
  const selections: any[][] = [];
  let closed = false;
  const repos = ["one", "two"].map((name) => ({ owner: "owner", name, branch: "main", subdir: "", enabled: true }));
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useEffect: (run: () => void | (() => void), deps: any[]) => {
      const index = cursor++;
      const previous = slots[index] as any[] | undefined;
      if (!previous || deps.some((value, position) => !Object.is(value, previous[position]))) {
        effects.push(() => { const cleanup = run(); if (cleanup) cleanups.push(cleanup); });
      }
      slots[index] = deps;
    },
    useMemo: (run: () => any) => run(),
    useRef: (value: any) => { const index = cursor++; return slots[index] ??= { current: value }; },
    t: (value: string) => value,
    tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    invoke: async (command: string, args: any) => {
      if (command === "discover_skill_source_evidence") return { status: "ok", message: "", evidence: [] };
      assert.equal(command, "find_skill_repository_matches");
      started.push(args.repoKey);
      return new Promise((resolve) => pending.push(resolve));
    },
    AppSelect: "AppSelect", ManagementDrawer: "ManagementDrawer", Pause: "Pause", Play: "Play", Plus: "Plus", RefreshCw: "RefreshCw", Search: "Search",
    discardSkillSourceScan, getSkillSourceScan, keyOfRepo,
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    elements.push(node);
    node.children?.forEach(collect);
    if (node.props?.footer) collect(node.props.footer);
  };
  const props = {
    id: "manual-background-fixture", localPath: "C:/fixture/manual-background",
    repos, localSkillIds: [], onClose: () => { closed = true; }, onAddRepo: async () => null,
    onApply: async (items: any[]) => {
      selections.push(JSON.parse(JSON.stringify(items)));
      return selections.length === 1
        ? { status: "failed", message: "来源匹配预览已失效，请重新匹配" }
        : { status: "ok", message: "" };
    },
  };
  const render = () => {
    cursor = 0; elements = []; effects = [];
    collect(context.exports.SkillSourceMatchDrawer(props));
    effects.splice(0).forEach((run) => run());
  };
  const unmount = () => {
    cleanups.splice(0).forEach((cleanup) => cleanup());
    slots = [];
  };
  const button = (label: string) => {
    const found = elements.find((node) => node.type === "button" && node.children.includes(label));
    assert.ok(found, `missing button: ${label}`);
    return found;
  };
  const candidate = (previewId: string) => ({
    status: "ok", message: "", previewId, warnings: [], matches: [],
    manualCandidates: [{
      localId: props.id, localName: props.id, repoKey: "owner/one@main",
      repoPath: "skills/chosen", remoteName: "chosen", exact: false,
    }],
  });
  const selectManualCandidate = () => {
    const checkbox = elements.find((node) => node.type === "input" && node.props.type === "checkbox");
    assert.ok(checkbox);
    assert.equal(checkbox.props.disabled, false);
    checkbox.props.onChange({ currentTarget: { checked: true } });
    render();
    const select = elements.find((node) => node.type === "AppSelect");
    assert.ok(select);
    assert.equal(select.props.options[0].hint, "手动选择，内容不同");
    select.props.onChange(JSON.stringify(["owner/one@main", "skills/chosen"]));
    render();
  };

  render();
  assert.deepEqual(started, ["owner/one@main"]);
  button("关闭").props.onClick();
  assert.equal(closed, true);
  unmount();
  pending[0](candidate("old-preview"));
  await tick();
  assert.deepEqual(started, ["owner/one@main", "owner/two@main"]);

  closed = false;
  render();
  assert.ok(elements.some((node) => node.props.role === "status"
    && node.children.some((child: unknown) => typeof child === "string" && child.includes("owner/two@main"))));
  button("暂停扫描").props.onClick();
  render();
  assert.ok(button("继续扫描"));
  selectManualCandidate();
  const confirm = button("关联所选 1 项");
  assert.equal(confirm.props.disabled, false);
  confirm.props.onClick();
  await tick();
  render();
  assert.ok(elements.some((node) => node.props.role === "alert" && node.children.includes("来源匹配预览已失效，请重新匹配")));
  assert.equal(closed, false);
  assert.equal(selections[0][0].previewId, "old-preview");
  assert.equal(selections[0][0].manual, true);
  assert.equal(selections[0][0].localPath, props.localPath);

  const retry = elements.find((node) => node.type === "button" && node.props["aria-label"] === "重新匹配");
  assert.ok(retry);
  retry.props.onClick();
  render();
  assert.ok(!elements.some((node) => node.props.role === "alert"
    && node.children.includes("来源匹配预览已失效，请重新匹配")));
  pending[1](candidate("discarded-preview"));
  await tick();
  assert.deepEqual(started, ["owner/one@main", "owner/two@main", "owner/one@main"]);
  pending[2](candidate("new-preview"));
  await tick();
  render();
  button("暂停扫描").props.onClick();
  render();
  selectManualCandidate();
  button("关联所选 1 项").props.onClick();
  await tick();
  assert.equal(closed, true);
  assert.equal(selections[1][0].previewId, "new-preview");
  assert.equal(selections[1][0].manual, true);
  unmount();
  pending[3](candidate("finished-after-close"));
  await tick();
});
