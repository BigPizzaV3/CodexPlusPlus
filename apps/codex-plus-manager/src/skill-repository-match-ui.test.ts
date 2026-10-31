import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { EN_BACKEND_PATTERNS, EN_PLAIN, EN_TEMPLATE } from "./i18n-en.ts";
import { groupSourceMatches, preferredSourceMatch, selectedSourceMatches, sourceMatchKey, sourceMatchLabel, type SkillSourceMatch } from "./skill-source-match.ts";

type Element = { type: any; props: Record<string, any>; children: any[] };
const source = readFileSync(new URL("./SkillRepositoryMatchDrawer.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("SkillRepositoryMatchDrawer.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const declaration = file.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SkillRepositoryMatchDrawer");
assert.ok(declaration);
const code = ts.transpileModule(declaration.getText(file), {
  compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

const longName = "a-very-long-skill-name-".repeat(5);
const longRepo = `organization-with-a-long-name/repository-with-a-long-name@feature/long-branch:plugins/extended/skills`;
const match = (localId: string, repoPath: string, exact: boolean): SkillSourceMatch => ({
  localId, localName: localId === "alpha" ? longName : localId, localHash: `local-${localId}`, snapshot: "snapshot",
  repoKey: longRepo, repoPath, remoteName: repoPath, remoteHash: `remote-${repoPath}`, exact,
});
const alphaExact = match("alpha", `plugins/extended/skills/${"nested-long-name/".repeat(4)}alpha`, true);
const alphaAlternative = match("alpha", "plugins/extended/skills/alternative", false);
const betaExact = match("beta", "plugins/extended/skills/beta", true);
const duplicate = match("duplicate", alphaExact.repoPath, true);
const gammaFirst = match("gamma", "plugins/extended/skills/gamma-1", true);
const gammaSecond = match("gamma", "plugins/extended/skills/gamma-2", true);
const manualAlpha = { ...match("alpha", "other/deeply/nested/unrelated-skill", false), remoteName: "Unrelated Skill" };
const manualOther = match("beta", "another/unrelated", false);
const fixture = [alphaExact, alphaAlternative, betaExact, duplicate, gammaFirst, gammaSecond];
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
type ScanResult = { status: string; message: string; matches: SkillSourceMatch[]; manualCandidates?: SkillSourceMatch[]; examined: number; warnings: string[] };
const scanResult = (matches: SkillSourceMatch[]): ScanResult => ({ status: "ok", message: "ok", matches, examined: 1, warnings: [] });
function deferredScan() {
  let resolve!: (result: ScanResult) => void;
  const promise = new Promise<ScanResult>((done) => { resolve = done; });
  return { promise, resolve };
}

function createHarness({ matches = fixture, manualCandidates = [], id, initialManual, applyStatus = "ok", repos, scanResults }: {
  matches?: SkillSourceMatch[]; manualCandidates?: SkillSourceMatch[]; id?: string; initialManual?: boolean; applyStatus?: string;
  repos?: Array<{ owner: string; name: string; branch: string; subdir: string; enabled: boolean }>;
  scanResults?: Record<string, Promise<ScanResult>>;
} = {}) {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const submissions: any[][] = [];
  const scans: any[] = [];
  const notices: Array<{ message: string; status?: string }> = [];
  let cursor = 0;
  let nodes: Element[] = [];
  let updates = 0;
  let closes = 0;
  let repositories = repos ?? [{ owner: "owner", name: "repo", branch: "main", subdir: "", enabled: true }];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }), fragment: "fragment",
    useState: (initial: any) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === "function" ? value(slots[index]) : value; }];
    },
    useRef: (initial: any) => { const index = cursor++; if (!(index in slots)) slots[index] = { current: initial }; return slots[index]; },
    useEffect: (run: () => void, deps?: any[]) => {
      const index = cursor++;
      const previous = slots[index] as any[] | undefined;
      if (!previous || !deps || deps.some((value, i) => !Object.is(value, previous[i]))) effects.push(run);
      slots[index] = deps;
    },
    useMemo: (run: () => any) => run(),
    invoke: async (command: string, args: any) => {
      if (command === "find_skill_repository_matches") {
        scans.push(args);
        if (scanResults?.[args.repoKey]) return scanResults[args.repoKey];
        return { status: "ok", message: "ok", matches, manualCandidates, examined: 1, warnings: [] };
      }
      if (command === "apply_skill_repository_matches") { submissions.push(args.selections); return { status: applyStatus, message: "applied" }; }
      throw new Error(`unexpected command ${command}`);
    },
    checkRepositories: async (items: any[], _limit: number, run: (item: any) => Promise<void>) => { await Promise.all(items.map(run)); },
    groupSourceMatches, preferredSourceMatch, selectedSourceMatches, sourceMatchKey, sourceMatchLabel,
    repoKey: (repo: { owner: string; name: string; branch: string; subdir: string }) => `${repo.owner}/${repo.name}@${repo.branch}${repo.subdir ? `:${repo.subdir}` : ""}`,
    t: (value: string) => value, tf: (value: string, args: any[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    Action: "Action", ManagementDrawer: "ManagementDrawer", AppSelect: "AppSelect", RefreshCw: "RefreshCw", Search: "Search", SkillRepositoryQuickAdd: "SkillRepositoryQuickAdd",
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const collect = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) { node.forEach(collect); return; }
    nodes.push(node);
    node.children?.forEach(collect);
    if (node.props?.footer) collect(node.props.footer);
  };
  const render = () => {
    cursor = 0; nodes = []; effects.length = 0;
    collect(context.exports.SkillRepositoryMatchDrawer({
      repos: repositories, id, initialManual,
      onClose: () => { closes++; }, onUpdated: () => { updates++; },
      notify: async (_title: string, message: string, status?: string) => { notices.push({ message, status }); },
    }));
    effects.splice(0).forEach((run) => run());
  };
  const find = (predicate: (node: Element) => boolean) => { const node = nodes.find(predicate); assert.ok(node); return node; };
  const row = (id: string) => find((node) => node.type === "div" && node.props.className === "mg-source-match-row" && node.props.key === id);
  const descendants = (root: Element) => {
    const result: Element[] = [];
    const visit = (node: any) => { if (!node || typeof node !== "object") return; if (Array.isArray(node)) { node.forEach(visit); return; } result.push(node); node.children?.forEach(visit); };
    visit(root);
    return result;
  };
  const checkbox = (id: string) => descendants(row(id)).find((node) => node.type === "input")!;
  const select = (id: string) => descendants(row(id)).find((node) => node.type === "AppSelect")!;
  const action = (text: string) => find((node) => node.type === "Action" && node.children.includes(text));
  const submit = () => find((node) => node.type === "Action" && node.props.primary);
  const selectedLabel = () => find((node) => node.type === "span" && /^已选 \d+ 项$/.test(node.children[0])).children[0];
  return { render, find, row, descendants, checkbox, select, action, submit, selectedLabel, submissions, notices, scans,
    setRepos(value: typeof repositories) { repositories = value; },
    get updates() { return updates; }, get closes() { return closes; } };
}

async function ready(options?: Parameters<typeof createHarness>[0]) {
  const ui = createHarness(options);
  ui.render();
  await tick();
  ui.render();
  return ui;
}

test("unchecked candidates can be previewed but only checked items are submitted", async () => {
  const ui = await ready();
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.select("alpha").props.disabled, false);
  assert.equal(ui.select("alpha").props.value, "");
  assert.equal(ui.selectedLabel(), "已选 0 项");
  assert.equal(ui.submit().props.disabled, true);

  ui.select("alpha").props.onChange(sourceMatchKey(alphaAlternative));
  ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(alphaAlternative));
  assert.equal(ui.submit().props.disabled, true);

  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(alphaAlternative));
  assert.equal(ui.selectedLabel(), "已选 1 项");
  assert.equal(ui.submit().props.disabled, false);

  ui.select("beta").props.onChange(sourceMatchKey(betaExact));
  ui.render();
  ui.submit().props.onClick();
  await tick();
  assert.equal(ui.submissions.length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(ui.submissions[0])), [{
    localId: "alpha", localHash: alphaAlternative.localHash, snapshot: alphaAlternative.snapshot,
    repoKey: alphaAlternative.repoKey, repoPath: alphaAlternative.repoPath, remoteHash: alphaAlternative.remoteHash,
  }]);
  assert.equal(ui.updates, 1);
  assert.equal(ui.closes, 1);
  assert.deepEqual(ui.notices, [{ message: "applied", status: "ok" }]);
});

test("unchecking, clearing and select-all preserve previews while defaulting only untouched choices", async () => {
  const ui = await ready();
  ui.select("alpha").props.onChange(sourceMatchKey(alphaAlternative));
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: false } });
  ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(alphaAlternative));
  assert.equal(ui.submit().props.disabled, true);

  ui.action("全选").props.onClick();
  ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, true);
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(alphaAlternative));
  assert.equal(ui.select("beta").props.value, sourceMatchKey(betaExact));
  assert.equal(ui.select("gamma").props.value, "");
  assert.equal(ui.submit().props.disabled, true);
  assert.ok(ui.find((node) => node.props.role === "alert"));

  ui.action("清空选择").props.onClick();
  ui.render();
  assert.equal(ui.selectedLabel(), "已选 0 项");
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(alphaAlternative));
  assert.equal(ui.select("beta").props.value, sourceMatchKey(betaExact));
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(alphaAlternative));
});

test("a checked automatic preview follows later, better scan results and becomes ambiguous when necessary", async () => {
  const first = deferredScan();
  const second = deferredScan();
  const third = deferredScan();
  const repos = ["first", "second", "third"].map((owner) => ({ owner, name: "repo", branch: "main", subdir: "", enabled: true }));
  const early = { ...alphaAlternative, repoKey: "first/repo@main" };
  const exact = { ...alphaExact, repoKey: "second/repo@main" };
  const anotherExact = { ...alphaExact, repoKey: "third/repo@main" };
  const ui = createHarness({ repos, scanResults: {
    "first/repo@main": first.promise, "second/repo@main": second.promise, "third/repo@main": third.promise,
  } });
  ui.render();
  first.resolve(scanResult([early]));
  await tick(); ui.render();
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(early));
  assert.equal(ui.submit().props.disabled, true);

  second.resolve(scanResult([exact]));
  await tick(); ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, true);
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(exact));

  third.resolve(scanResult([anotherExact]));
  await tick(); ui.render();
  assert.equal(ui.select("alpha").props.value, "");
  assert.equal(ui.submit().props.disabled, true);
  assert.ok(ui.find((node) => node.props.role === "alert"));
});

test("an explicitly chosen preview survives later scans, unchecking, and rechecking", async () => {
  const first = deferredScan();
  const second = deferredScan();
  const repos = ["first", "second"].map((owner) => ({ owner, name: "repo", branch: "main", subdir: "", enabled: true }));
  const early = { ...alphaAlternative, repoKey: "first/repo@main" };
  const exact = { ...alphaExact, repoKey: "second/repo@main" };
  const ui = createHarness({ repos, scanResults: { "first/repo@main": first.promise, "second/repo@main": second.promise } });
  ui.render();
  first.resolve(scanResult([early]));
  await tick(); ui.render();
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  ui.select("alpha").props.onChange(sourceMatchKey(early));
  ui.render();
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: false } });
  ui.render();

  second.resolve(scanResult([exact]));
  await tick(); ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(early));
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(early));
  assert.equal(ui.submit().props.disabled, false);
  ui.submit().props.onClick();
  await tick();
  assert.equal(ui.submissions[0][0].repoKey, early.repoKey);
  assert.equal(ui.submissions[0][0].repoPath, early.repoPath);
});

test("a checked explicit preview is not replaced by a later exact match", async () => {
  const first = deferredScan();
  const second = deferredScan();
  const repos = ["first", "second"].map((owner) => ({ owner, name: "repo", branch: "main", subdir: "", enabled: true }));
  const early = { ...alphaAlternative, repoKey: "first/repo@main" };
  const exact = { ...alphaExact, repoKey: "second/repo@main" };
  const ui = createHarness({ repos, scanResults: { "first/repo@main": first.promise, "second/repo@main": second.promise } });
  ui.render();
  first.resolve(scanResult([early]));
  await tick(); ui.render();
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  ui.select("alpha").props.onChange(sourceMatchKey(early));
  ui.render();
  second.resolve(scanResult([exact]));
  await tick(); ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, true);
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(early));
  assert.equal(ui.submit().props.disabled, false);
});

test("an untouched automatic preview updates even while temporarily unchecked", async () => {
  const first = deferredScan();
  const second = deferredScan();
  const repos = ["first", "second"].map((owner) => ({ owner, name: "repo", branch: "main", subdir: "", enabled: true }));
  const early = { ...alphaAlternative, repoKey: "first/repo@main" };
  const exact = { ...alphaExact, repoKey: "second/repo@main" };
  const ui = createHarness({ repos, scanResults: { "first/repo@main": first.promise, "second/repo@main": second.promise } });
  ui.render();
  first.resolve(scanResult([early]));
  await tick(); ui.render();
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: false } });
  ui.render();
  second.resolve(scanResult([exact]));
  await tick(); ui.render();
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(exact));
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(exact));
  ui.submit().props.onClick();
  await tick();
  assert.equal(ui.submissions[0][0].repoKey, exact.repoKey);
  assert.equal(ui.submissions[0][0].repoPath, exact.repoPath);
});

test("conflicts are checked only among selected skills and disappear when one is unchecked", async () => {
  const ui = await ready();
  ui.select("alpha").props.onChange(sourceMatchKey(alphaExact));
  ui.select("duplicate").props.onChange(sourceMatchKey(duplicate));
  ui.render();
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.submit().props.disabled, false);
  ui.checkbox("duplicate").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.submit().props.disabled, true);
  assert.ok(ui.find((node) => node.props.role === "alert"));
  ui.checkbox("duplicate").props.onChange({ currentTarget: { checked: false } });
  ui.render();
  assert.equal(ui.select("duplicate").props.value, sourceMatchKey(duplicate));
  assert.equal(ui.submit().props.disabled, false);
});

test("full skill names and repository labels use scoped wrapping in trigger and portal menu", async () => {
  const ui = await ready();
  assert.equal(ui.descendants(ui.row("alpha")).find((node) => node.type === "strong")!.children[0], longName);
  const select = ui.select("alpha");
  assert.equal(select.props.className, "mg-source-match-select");
  assert.equal(select.props.menuWidth, 420);
  assert.equal(select.props.options.find((option: any) => option.value === sourceMatchKey(alphaExact)).label, sourceMatchLabel(alphaExact));

  const css = readFileSync(new URL("./SkillRepositoryMatchDrawer.css", import.meta.url), "utf8");
  assert.match(css, /\.mg-source-match-row label strong, \.mg-source-match-row label small \{[^}]*overflow-wrap:anywhere;[^}]*white-space:normal;/);
  assert.match(css, /\.app-select\.mg-source-match-select \.app-select-trigger > span,\s*\.app-select-menu\.mg-source-match-select \.app-select-option-text > span \{[^}]*overflow:visible;[^}]*overflow-wrap:anywhere;[^}]*white-space:normal;/);
  const sharedSelect = readFileSync(new URL("./AppSelect.tsx", import.meta.url), "utf8");
  assert.match(sharedSelect, /app-select-menu app-select-menu-portal \$\{sizeClass\} \$\{className\}/);
});

test("single-skill manual mode searches unmatched paths and submits one explicit manual selection", async () => {
  const ui = await ready({ id: "alpha", initialManual: true, matches: [], manualCandidates: [manualAlpha, manualOther] });
  assert.equal(ui.scans[0].id, "alpha");
  assert.equal(ui.find((node) => node.type === "button" && node.children.includes("手动选择")).props["aria-pressed"], true);
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.select("alpha").props.disabled, false);
  assert.equal(ui.select("alpha").props.options.length, 1);
  assert.equal(ui.select("alpha").props.options[0].hint, "手动选择 · 内容不一致");
  const search = () => ui.find((node) => node.type === "input" && node.props.type === "search");
  search().props.onChange({ currentTarget: { value: "UNRELATED nested" } });
  ui.render();
  assert.equal(ui.select("alpha").props.options.length, 1);
  ui.select("alpha").props.onChange(sourceMatchKey(manualAlpha));
  ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.submit().props.disabled, true);
  assert.ok(ui.descendants(ui.row("alpha")).some((node) => node.type === "small" && node.children.includes("手动选择 · 内容不一致")));
  search().props.onChange({ currentTarget: { value: "no result" } });
  ui.render();
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(manualAlpha));
  assert.equal(ui.select("alpha").props.options.length, 1);
  assert.ok(ui.find((node) => node.props.role === "status" && node.children.includes("没有匹配的仓库路径。")));

  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.submit().props.disabled, false);
  ui.submit().props.onClick();
  await tick();
  assert.deepEqual(JSON.parse(JSON.stringify(ui.submissions[0])), [{
    localId: "alpha", localHash: manualAlpha.localHash, snapshot: manualAlpha.snapshot,
    repoKey: manualAlpha.repoKey, repoPath: manualAlpha.repoPath, remoteHash: manualAlpha.remoteHash, manual: true,
  }]);
});

test("single-skill mode switches preserve separate previews but require a fresh checkbox confirmation", async () => {
  const ui = await ready({ id: "alpha", matches: [alphaExact], manualCandidates: [manualAlpha] });
  const mode = (label: string) => ui.find((node) => node.type === "button" && node.children.includes(label));
  assert.equal(mode("自动匹配").props["aria-pressed"], true);
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(alphaExact));
  mode("手动选择").props.onClick();
  ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.select("alpha").props.value, "");
  assert.equal(ui.select("alpha").props.options.length, 2);
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  assert.equal(ui.select("alpha").props.value, "");
  assert.equal(ui.submit().props.disabled, true);
  ui.select("alpha").props.onChange(sourceMatchKey(manualAlpha));
  ui.render();
  assert.equal(ui.submit().props.disabled, false);
  mode("自动匹配").props.onClick();
  ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(alphaExact));
  mode("手动选择").props.onClick();
  ui.render();
  assert.equal(ui.checkbox("alpha").props.checked, false);
  assert.equal(ui.select("alpha").props.value, sourceMatchKey(manualAlpha));
});

test("bulk matching ignores manual candidates and never sends a manual flag", async () => {
  const ui = await ready({ initialManual: true, matches: [alphaExact], manualCandidates: [manualAlpha, manualOther] });
  assert.equal(ui.scans[0].id, null);
  assert.equal(ui.select("alpha").props.options.length, 1);
  assert.equal(ui.select("alpha").props.options[0].value, sourceMatchKey(alphaExact));
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  ui.submit().props.onClick();
  await tick();
  assert.equal(ui.submissions[0].length, 1);
  assert.equal(Object.hasOwn(ui.submissions[0][0], "manual"), false);
});

test("single mode can reach manual candidates when automatic matching has no results", async () => {
  const ui = await ready({ id: "alpha", matches: [], manualCandidates: [manualAlpha] });
  assert.ok(ui.find((node) => node.type === "p" && node.children.includes("未找到匹配来源。")));
  ui.find((node) => node.type === "button" && node.children.includes("手动选择")).props.onClick();
  ui.render();
  assert.equal(ui.select("alpha").props.options[0].value, sourceMatchKey(manualAlpha));
});

test("adding a repository blocks conflicting actions and rescans the updated list", async () => {
  const ui = await ready({ id: "alpha", initialManual: true, matches: [alphaExact] });
  let add = ui.find((node) => node.type === "SkillRepositoryQuickAdd");
  add.props.onBusyChange(true);
  ui.render();
  assert.equal(ui.action("重新匹配").props.disabled, true);
  assert.equal(ui.action("取消").props.disabled, true);
  assert.equal(ui.select("alpha").props.disabled, true);
  add = ui.find((node) => node.type === "SkillRepositoryQuickAdd");
  add.props.onAdded([{ owner: "owner", name: "repo", branch: "main", subdir: "", enabled: true },
    { owner: "other", name: "repo", branch: "main", subdir: "", enabled: true }]);
  add.props.onBusyChange(false);
  ui.render(); await tick(); ui.render();
  assert.ok(ui.scans.some((scan) => scan.repoKey === "other/repo@main"));
  assert.equal(ui.updates, 1);
  assert.equal(ui.action("取消").props.disabled, false);
});

test("a failed application keeps the drawer open and rescans without stale selections", async () => {
  const ui = await ready({ matches: [alphaExact], applyStatus: "failed" });
  ui.checkbox("alpha").props.onChange({ currentTarget: { checked: true } });
  ui.render();
  ui.submit().props.onClick();
  await tick();
  assert.equal(ui.updates, 1);
  assert.equal(ui.closes, 0);
  assert.equal(ui.notices[0].status, "failed");
  ui.render();
  await tick();
  ui.render();
  assert.equal(ui.selectedLabel(), "已选 0 项");
  assert.equal(ui.submit().props.disabled, true);
  assert.equal(ui.scans.length, 2);
});

test("re-matching uses repositories refreshed by the parent", async () => {
  const ui = await ready({ matches: [alphaExact] });
  ui.setRepos([{ owner: "owner", name: "repo", branch: "master", subdir: "", enabled: true }]);
  ui.render();
  ui.action("重新匹配").props.onClick();
  ui.render();
  await tick();
  ui.render();
  assert.equal(ui.scans.at(-1).repoKey, "owner/repo@master");
});

test("matching and quick-add interface strings have English translations", () => {
  const quickAdd = readFileSync(new URL("./SkillRepositoryQuickAdd.tsx", import.meta.url), "utf8");
  for (const text of [source, quickAdd]) {
    for (const [, kind, key] of text.matchAll(/\b(t|tf)\("([^"]+)"/g)) {
      assert.ok((kind === "t" ? EN_PLAIN : EN_TEMPLATE)[key], `missing English translation: ${key}`);
    }
  }
});

test("quick-add backend results have English translations", () => {
  for (const message of [
    "仓库已验证并添加，发现 2 个 Skill。",
    "添加仓库失败：仓库已存在，未修改现有配置",
    "添加仓库失败：仓库验证失败，未保存: HTTP 404",
  ]) {
    assert.ok(EN_BACKEND_PATTERNS.some(([pattern]) => pattern.test(message)), `missing translation: ${message}`);
  }
});
