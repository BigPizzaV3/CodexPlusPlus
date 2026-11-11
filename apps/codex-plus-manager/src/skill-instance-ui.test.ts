import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

type Element = { type: any; props: Record<string, any>; children: any[] };
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

function component(path: string, name: string): string {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
  const file = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const node = file.statements.find((item) => ts.isFunctionDeclaration(item) && item.name?.text === name);
  assert.ok(node);
  return ts.transpileModule(node.getText(file), {
    compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
}

function collect(node: any, result: Element[] = []): Element[] {
  if (!node || typeof node !== "object") return result;
  if (Array.isArray(node)) { node.forEach((item) => collect(item, result)); return result; }
  result.push(node);
  node.children?.forEach((item: any) => collect(item, result));
  if (node.props?.footer) collect(node.props.footer, result);
  return result;
}

test("independent installations retain their own source and actions", () => {
  const matched: string[] = [];
  const updates: Array<[string, string]> = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: (skill: { repoKey: string }) => skill.repoKey ? "https://github.com/example/repo" : "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", Search: "Search", CloudDownload: "CloudDownload", ExternalLink: "ExternalLink", ArrowRight: "ArrowRight", SkillFileTimes: "SkillFileTimes",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  const skill = {
    id: "alpha", name: "Alpha", repoKey: "", description: "", installed: true, bundled: false,
    conflict: "多个独立安装", conflictKind: "equal-independent",
    locations: [
      { manifestPath: "C:/agents/alpha/SKILL.md", originalPath: "C:/agents/alpha", rootKind: "shared", managed: false, enabled: true, selectable: true, repoKey: "one/repo@main", repoPath: "skills/alpha" },
      { manifestPath: "C:/claude/alpha/SKILL.md", originalPath: "C:/claude/alpha", rootKind: "claude", managed: false, enabled: true, selectable: true },
      { manifestPath: "C:/codex/alpha/SKILL.md", rootKind: "codex", managed: true, enabled: true, selectable: false },
      { manifestPath: "C:/legacy/alpha/SKILL.md", originalPath: "C:/legacy/alpha", rootKind: "grok", managed: false, enabled: true, selectable: true, repoKey: "cc-switch" },
    ],
  };
  const elements = collect(context.SkillDetailDrawer({
    skill, backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: (path: string) => matched.push(path), onRestore: () => {},
    onUpdate: (repo: string, path: string) => updates.push([repo, path]), onReaPreview: () => {},
  }));
  const location = (path: string) => {
    const node = elements.find((item) => item.type === "dd" && item.children.includes(`${path}/SKILL.md`));
    assert.ok(node);
    return collect(node);
  };
  const first = location("C:/agents/alpha");
  const second = location("C:/claude/alpha");
  const managed = location("C:/codex/alpha");
  const legacy = location("C:/legacy/alpha");
  const sourceField = elements.findIndex((item) => item.type === "dt" && item.children.includes("来源"));
  assert.ok(sourceField >= 0);
  assert.equal(elements[sourceField + 1].type, "dd");
  assert.ok(elements[sourceField + 1].children.includes("本地"));
  assert.ok(first.some((item) => item.children.includes("one/repo@main / skills/alpha")));
  assert.ok(second.some((item) => item.children.includes("本地")));
  first.find((item) => item.type === "Button" && item.children.includes("重新关联此安装位置"))!.props.onClick();
  second.find((item) => item.type === "Button" && item.children.includes("关联此安装位置"))!.props.onClick();
  first.find((item) => item.type === "Button" && item.children.includes("预览此位置更新"))!.props.onClick();
  assert.equal(second.some((item) => item.type === "Button" && item.children.includes("预览此位置更新")), false);
  assert.ok(legacy.some((item) => item.children.includes("本地")));
  assert.equal(legacy.some((item) => item.type === "Button" && item.children.includes("预览此位置更新")), false);
  assert.deepEqual(matched, ["C:/agents/alpha", "C:/claude/alpha"]);
  assert.deepEqual(updates, [["one/repo@main", "C:/agents/alpha"]]);
  assert.equal(managed.some((item) => item.type === "Button"), false);
  const split = collect(context.SkillDetailDrawer({
    skill: { ...skill, repoKey: "two/repo@main", repoPath: "alpha", originalDir: "C:/claude/alpha",
      conflictKind: "independent", locations: [{ ...skill.locations[1], repoKey: "two/repo@main", repoPath: "alpha" }] },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: () => {}, onRestore: () => {}, onUpdate: (repo: string, path: string) => updates.push([repo, path]), onReaPreview: () => {},
  }));
  split.find((item) => item.type === "Button" && item.children.includes("预览原位更新"))!.props.onClick();
  assert.deepEqual(updates[1], ["two/repo@main", "C:/claude/alpha"]);
});

test("a selectable physical installation associates its exact path with or without an existing source", () => {
  const matched: string[] = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value,
    repoUrl: (skill: { repoKey: string }) => skill.repoKey ? "https://github.com/owner/repo" : "",
    sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", Search: "Search", SkillFileTimes: "SkillFileTimes",
    ArrowRight: "ArrowRight", ExternalLink: "ExternalLink", Edit3: "Edit3",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  for (const [repoKey, label] of [
    ["", "关联 GitHub 仓库"],
    ["owner/repo@main", "修改 GitHub 仓库关联"],
  ]) {
    const elements = collect(context.SkillDetailDrawer({
      skill: { id: "alpha", name: "Alpha", repoKey, description: "", installed: true, bundled: false,
        repoPath: repoKey ? "skills/alpha" : "", locations: [
          { manifestPath: "C:/agents/alpha/SKILL.md", originalPath: "C:/agents/alpha", rootKind: "shared",
            managed: false, enabled: true, selectable: true, repoKey },
        ] },
      backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
      onMatch: (path: string) => matched.push(path), onRestore: () => {}, onUpdate: () => {}, onReaPreview: () => {},
    }));
    const repoPath = elements.findIndex((item) => item.type === "dt" && item.children.includes("仓库路径"));
    assert.ok(repoPath >= 0);
    const button = collect(elements[repoPath + 1]).find((item) => item.type === "button" && item.props["aria-label"] === label);
    assert.ok(button);
    assert.equal(button.props.title, label);
    assert.ok(button.children.some((item: Element) => item.type === "Edit3"));
    assert.equal(elements.some((item) => item.type === "Button" && item.children.includes("关联此安装位置")), false);
    assert.equal(elements.some((item) => item.type === "Button" && item.children.includes("重新关联此安装位置")), false);
    button.props.onClick();
  }
  assert.deepEqual(matched, ["C:/agents/alpha", "C:/agents/alpha"]);
});

test("multiple association targets stay tied to their locations", () => {
  const matched: string[] = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", SkillFileTimes: "SkillFileTimes", FolderOpen: "FolderOpen",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  const elements = collect(context.SkillDetailDrawer({
    skill: { id: "alpha", name: "Alpha", repoKey: "", description: "", installed: true, bundled: false,
      managedSourceDir: "C:/managed/alpha", locations: [
        { manifestPath: "C:/agents/alpha/SKILL.md", originalPath: "C:/agents/alpha", rootKind: "shared",
          managed: false, enabled: true, selectable: true },
        { manifestPath: "C:/codex/alpha/SKILL.md", associationPath: "C:/managed/alpha", rootKind: "codex",
          managed: true, enabled: true },
      ] },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: (path: string) => matched.push(path), onRestore: () => {}, onUpdate: () => {}, onReaPreview: () => {},
  }));
  const repoPath = elements.findIndex((item) => item.type === "dt" && item.children.includes("仓库路径"));
  assert.equal(collect(elements[repoPath + 1]).some((item) => item.type === "button"), false);
  elements.filter((item) => item.type === "Button" && item.children.includes("关联此安装位置"))
    .forEach((item) => item.props.onClick());
  assert.deepEqual(matched, ["C:/agents/alpha", "C:/managed/alpha"]);
});

test("Agent locations that share a source expose one association target", () => {
  const matched: string[] = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", SkillFileTimes: "SkillFileTimes", Edit3: "Edit3",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  const elements = collect(context.SkillDetailDrawer({
    skill: { id: "alpha", name: "Alpha", repoKey: "", description: "", installed: true, bundled: false,
      locations: ["shared", "claude"].map((rootKind) => ({
        manifestPath: `C:/${rootKind}/alpha/SKILL.md`, originalPath: "C:/shared/alpha",
        selectable: true, rootKind, managed: false, enabled: true,
      })) },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: (path: string) => matched.push(path), onRestore: () => {}, onUpdate: () => {}, onReaPreview: () => {},
  }));
  assert.equal(elements.filter((item) => item.type === "small" && item.children.includes("此来源由多个安装位置共用")).length, 2);
  elements.find((item) => item.type === "button" && item.props["aria-label"] === "关联 GitHub 仓库")!.props.onClick();
  assert.deepEqual(matched, ["C:/shared/alpha"]);
});

test("a managed Agent copy points to the shared original alongside an independent installation", () => {
  const matched: string[] = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", SkillFileTimes: "SkillFileTimes", CloudDownload: "CloudDownload",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  const shared = { repoKey: "owner/repo@main", repoPath: ".agents/skills/alpha", locations: [
    { manifestPath: "C:/shared/alpha/SKILL.md", originalPath: "C:/shared/alpha", selectable: true,
      rootKind: "shared", managed: false, enabled: true },
    { manifestPath: "C:/codex/alpha/SKILL.md", associationPath: "C:/shared/alpha",
      rootKind: "codex", managed: true, enabled: true },
  ] };
  const independent = { repoKey: "owner/repo@main", repoPath: ".claude/skills/alpha", locations: [
    { manifestPath: "C:/claude/alpha/SKILL.md", originalPath: "C:/claude/alpha", selectable: true,
      rootKind: "claude", managed: false, enabled: true },
  ] };
  const elements = collect(context.SkillDetailDrawer({
    skill: { id: "alpha", name: "Alpha", repoKey: "", description: "", installed: true, bundled: false,
      locations: [...shared.locations, ...independent.locations], displayInstances: [shared, independent] },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: (path: string) => matched.push(path), onRestore: () => {}, onUpdate: () => {}, onReaPreview: () => {},
  }));
  assert.equal(elements.filter((item) => item.type === "small" && item.children.includes("此来源由多个安装位置共用")).length, 2);
  assert.equal(elements.some((item) => item.type === "dt" && item.children.includes("受控源目录")), false);
  elements.filter((item) => item.type === "Button" && item.children.includes("重新关联此安装位置"))
    .forEach((item) => item.props.onClick());
  assert.deepEqual(matched, ["C:/shared/alpha", "C:/shared/alpha", "C:/claude/alpha"]);
});

test("the detail view displays the reason for an unknown location status", () => {
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", Search: "Search",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  const elements = collect(context.SkillDetailDrawer({
    skill: { id: "alpha", name: "Alpha", repoKey: "", description: "", installed: true, bundled: false,
      locations: [{ manifestPath: "C:/agents/alpha/SKILL.md", rootKind: "shared", managed: false,
        enabled: false, statusError: "无法确定名称规则的启用状态" }] },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: () => {}, onRestore: () => {}, onUpdate: () => {}, onReaPreview: () => {},
  }));
  const location = elements.find((item) => item.type === "dd" && item.children.includes("C:/agents/alpha/SKILL.md"))!;
  assert.ok(location.children.includes("状态未知"));
  assert.equal(location.children.includes("未启用"), false);
  assert.ok(collect(location).some((item) => item.props.role === "alert" && item.children.includes("无法确定名称规则的启用状态")));
});

test("a conflicting unclaimed managed source has a path and file dates without an association action", () => {
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", Search: "Search", SkillFileTimes: "SkillFileTimes", FolderOpen: "FolderOpen",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  const elements = collect(context.SkillDetailDrawer({
    skill: { id: "alpha", name: "Alpha", repoKey: "", description: "", installed: true, bundled: false,
      conflict: "内容不同", conflictKind: "independent", managedSourceDir: "C:/managed/alpha",
      locations: [{ manifestPath: "C:/agents/alpha/SKILL.md", rootKind: "shared", managed: false, enabled: true }] },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: () => {}, onRestore: () => {}, onDeleteBackup: () => {}, onUpdate: () => {}, onReaPreview: () => {},
  }));
  const source = elements.find((item) => item.type === "dd" && item.children.includes("C:/managed/alpha"))!;
  assert.ok(source);
  assert.ok(collect(source).some((item) => item.type === "SkillFileTimes" && item.props.rootKind === "source"));
  assert.equal(collect(source).some((item) => item.type === "Button" && item.children.includes("关联受控源")), false);
});

test("a managed source directory is visible without a conflict", () => {
  const matched: Array<string | undefined> = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", Search: "Search", SkillFileTimes: "SkillFileTimes", Edit3: "Edit3", FolderOpen: "FolderOpen",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  const elements = collect(context.SkillDetailDrawer({
    skill: { id: "alpha", name: "Alpha", repoKey: "", description: "", installed: true, bundled: false,
      managedSourceDir: "C:/managed/alpha", locations: [
        { manifestPath: "C:/codex/alpha/SKILL.md", rootKind: "codex", managed: true, enabled: true,
          associationPath: "C:/managed/alpha" },
      ] },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: (path: string | undefined) => matched.push(path), onRestore: () => {}, onUpdate: () => {}, onReaPreview: () => {},
  }));
  const source = elements.find((item) => item.type === "dd" && item.children.includes("C:/managed/alpha"));
  assert.ok(source);
  assert.ok(collect(source).some((item) => item.type === "SkillFileTimes" && item.props.rootKind === "source"));
  assert.equal(collect(source).some((item) => item.type === "Button"), false);
  const icon = elements.find((item) => item.type === "button" && item.props["aria-label"] === "关联 GitHub 仓库");
  assert.ok(icon);
  icon.props.onClick();
  assert.deepEqual(matched, ["C:/managed/alpha"]);
});

test("an unused managed source reports its status without a name conflict", () => {
  let opened = false;
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", Search: "Search", SkillFileTimes: "SkillFileTimes", Edit3: "Edit3", FolderOpen: "FolderOpen",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  const elements = collect(context.SkillDetailDrawer({
    skill: { id: "alpha", name: "Alpha", repoKey: "", description: "", installed: true, bundled: false,
      managedSourceDir: "C:/managed/alpha", managedSourceStatus: "different",
      locations: [{ manifestPath: "C:/codex/alpha/SKILL.md", rootKind: "codex", managed: true, enabled: true }] },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
    onMatch: () => {}, onRestore: () => {}, onDeleteBackup: () => {}, onUpdate: () => {}, onReaPreview: () => {},
    onManageSource: () => { opened = true; },
  }));
  const source = elements.find((item) => item.type === "dd" && item.children.includes("C:/managed/alpha"))!;
  assert.ok(collect(source).some((item) => item.children.includes("未被目录链接引用，内容与安装位置不同；启用到其他 Agent 前需处理")));
  assert.equal(elements.some((item) => item.props.role === "alert"), false);
  collect(source).find((item) => item.props["aria-label"] === "管理受控源")!.props.onClick();
  assert.equal(opened, true);
});

test("different agent variants describe the managed source without hiding real conflicts", () => {
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "", skillChannelLabel: () => "本地安装",
    ManagementDrawer: "ManagementDrawer", Button: "Button", Search: "Search", SkillFileTimes: "SkillFileTimes", FolderOpen: "FolderOpen",
  };
  vm.createContext(context);
  vm.runInContext(component("./ManagementExtra.tsx", "repoFromKey"), context);
  vm.runInContext(component("./ManagementExtra.tsx", "SkillDetailDrawer"), context);
  const locations = [
    { manifestPath: "C:/codex/alpha/SKILL.md", rootKind: "codex", managed: true, enabled: true },
    { manifestPath: "C:/claude/alpha/SKILL.md", rootKind: "claude", managed: false, enabled: true },
  ];
  for (const conflict of [null, "受控副本已修改"]) {
    const elements = collect(context.SkillDetailDrawer({
      skill: { id: "alpha", name: "Alpha", repoKey: "", description: "", installed: true, bundled: false,
        managedSourceDir: "C:/managed/alpha", managedSourceStatus: "different", locations,
        displayInstances: locations.map((location) => ({ locations: [location] })), conflict },
      backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {},
      onMatch: () => {}, onRestore: () => {}, onDeleteBackup: () => {}, onUpdate: () => {}, onReaPreview: () => {}, onManageSource: () => {},
    }));
    const source = elements.find((item) => item.type === "dd" && item.children.includes("C:/managed/alpha"))!;
    const text = conflict ? "未被目录链接引用，内容与安装位置不同；启用到其他 Agent 前需处理"
      : "受控源与部分 Agent 的独立安装内容不同";
    const note = collect(source).find((item) => item.type === "small" && item.children.includes(text));
    assert.ok(note);
    assert.equal(note.props.className, conflict ? "mg-warning" : undefined);
  }
});

test("update preview separates managed copies, following paths and skipped locations", async () => {
  const slots: any[] = [];
  const effects: Array<() => void> = [];
  const calls: Array<{ command: string; args: any }> = [];
  const executions: string[] = [];
  let cursor = 0;
  let elements: Element[] = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, ManagementDrawer: "ManagementDrawer", RefreshCw: "RefreshCw",
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
    invoke: async (command: string, args: any) => {
      calls.push({ command, args });
      return { status: "ok", message: "", previewId: "confirmed-preview", repoKey: args.repoKey, repoPath: "alpha",
        targetPath: args.localPath, currentHash: "old", nextHash: "new", conflicts: [],
        managedPaths: ["C:/codex/alpha"], followedPaths: ["C:/gemini/alpha"], skippedPaths: ["C:/claude/alpha"] };
    },
  };
  vm.createContext(context);
  vm.runInContext(component("./SkillInPlaceUpdateDrawer.tsx", "SkillInPlaceUpdateDrawer"), context);
  const props = { id: "alpha", repoKey: "one/repo@main", localPath: "C:/agents/alpha", onClose: () => {},
    onExecute: async (id: string) => { executions.push(id); return true; } };
  const render = () => {
    cursor = 0; effects.length = 0;
    elements = collect(context.exports.SkillInPlaceUpdateDrawer(props));
    effects.splice(0).forEach((run) => run());
  };
  render(); await tick(); render();
  assert.equal(calls[0].command, "preview_skill_in_place_update");
  assert.equal(calls[0].args.localPath, "C:/agents/alpha");
  assert.ok(elements.some((item) => item.type === "strong" && item.children.includes("将同步的受控副本")));
  assert.ok(elements.some((item) => item.type === "strong" && item.children.includes("将随原件或受控副本更新的路径")));
  assert.ok(elements.some((item) => item.type === "li" && item.children.includes("C:/gemini/alpha")));
  assert.ok(elements.some((item) => item.type === "strong" && item.children.includes("独立或归属未确认的安装，保持不变")));
  const confirm = elements.find((item) => item.type === "button" && item.children.includes("确认接管并更新"));
  assert.ok(confirm);
  assert.equal(confirm.props.disabled, false);
  await confirm.props.onClick(); await tick(); render();
  assert.deepEqual(executions, ["confirmed-preview"]);
  assert.ok(elements.some((item) => item.type === "p" && item.children.includes("原位更新已完成，原版本已保存到备份。")));
});
