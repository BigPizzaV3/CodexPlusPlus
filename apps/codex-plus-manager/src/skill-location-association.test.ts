import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

type Element = { type: any; props: Record<string, any>; children: any[] };

test("each Agent installation associates its own repository path", () => {
  const source = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
  const file = ts.createSourceFile("ManagementExtra.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const nodes = file.statements.filter((node) => ts.isFunctionDeclaration(node)
    && (node.name?.text === "repoFromKey" || node.name?.text === "SkillDetailDrawer"));
  const code = ts.transpileModule(nodes.map((node) => node.getText(file)).join("\n"), {
    compilerOptions: { jsx: ts.JsxEmit.React, jsxFactory: "jsx", jsxFragmentFactory: "fragment", module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const selected: Array<[string, Record<string, string>]> = [];
  const context: Record<string, any> = {
    exports: {}, jsx: (type: any, props: any, ...children: any[]) => ({ type, props: props || {}, children }),
    fragment: "fragment", t: (value: string) => value, repoUrl: () => "", sourceLabel: () => "本地",
    skillChannelLabel: () => "本地安装", SkillFileTimes: "SkillFileTimes", CloudDownload: "CloudDownload",
    ManagementDrawer: "ManagementDrawer", Button: "Button", Edit3: "Edit3", ExternalLink: "ExternalLink", ArrowRight: "ArrowRight",
  };
  vm.createContext(context);
  vm.runInContext(code, context);
  const locations = [
    { manifestPath: "C:/agents/alpha/SKILL.md", rootKind: "shared", managed: false, enabled: true,
      selectable: true, originalPath: "C:/agents/alpha", repoKey: "owner/repo@main", repoPath: ".agents/skills/alpha" },
    { manifestPath: "C:/claude/alpha/SKILL.md", rootKind: "claude", managed: false, enabled: true,
      selectable: true, originalPath: "C:/claude/alpha", repoKey: "owner/repo@main", repoPath: ".claude/skills/alpha" },
  ];
  const element = context.SkillDetailDrawer({
    skill: { id: "alpha", name: "alpha", description: "", repoKey: "", installed: true, bundled: false,
      locations, displayInstances: locations.map((location) => ({ repoKey: location.repoKey, repoPath: location.repoPath, locations: [location] })) },
    backups: [], busy: false, onClose: () => {}, onOpenUrl: () => {}, onOpenRepo: () => {}, onUpdate: () => {},
    onMatch: (path: string, details: Record<string, string>) => selected.push([path, details]),
  });
  const collect = (node: any, result: Element[] = []): Element[] => {
    if (!node || typeof node !== "object") return result;
    if (Array.isArray(node)) { node.forEach((child) => collect(child, result)); return result; }
    result.push(node);
    node.children?.forEach((child: any) => collect(child, result));
    return result;
  };
  const buttons = collect(element).filter((item) => item.type === "Button" && item.children.includes("重新关联此安装位置"));
  assert.equal(buttons.length, 2);
  buttons.forEach((button) => button.props.onClick());
  assert.deepEqual(selected.map(([path, detail]) => [path, detail.currentRepoPath]), [
    ["C:/agents/alpha", ".agents/skills/alpha"],
    ["C:/claude/alpha", ".claude/skills/alpha"],
  ]);
});
