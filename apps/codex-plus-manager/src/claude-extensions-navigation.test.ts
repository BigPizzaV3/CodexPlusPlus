import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { agentVisible, providersHidden, singleAgent } from "./agent-focus-model.ts";
import { pageVisibleForTool } from "./page-groups.ts";

const text = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const app = ts.createSourceFile("App.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

function declaration(name: string): ts.VariableDeclaration {
  const topLevel = app.statements.flatMap((node) => ts.isVariableStatement(node) ? Array.from(node.declarationList.declarations) : [])
    .find((node) => node.name.getText(app) === name);
  if (topLevel) return topLevel;
  let result: ts.VariableDeclaration | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(app) === name) result = node;
    ts.forEachChild(node, visit);
  };
  visit(app);
  assert.ok(result, name);
  return result;
}

function functionSource(name: string) {
  const node = app.statements.find(
    (statement): statement is ts.FunctionDeclaration => ts.isFunctionDeclaration(statement) && statement.name?.text === name,
  );
  assert.ok(node, name);
  return node.getText(app);
}

const navigationNames = [
  "routes", "routeTool", "globalNavigationRoutes", "gatewayNavigationRoutes", "managementNavigationRoutes",
  "navAliasRoute", "toolNavigationRoutes", "multiAgentRoutes", "providerRoutes", "routeVisible", "visibleRoutes",
];
const navigationSource = navigationNames.map((name) => `const ${declaration(name).getText(app)};`).join("\n");
const icons = [...text.matchAll(/icon: (\w+)/g)].map((match) => match[1]);
const transpile = (source: string) => ts.transpileModule(source, {
  compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 },
}).outputText;

type Element = { type: string; props: Record<string, any> };
function elements(node: any): Element[] {
  if (!node || typeof node !== "object") return [];
  if (Array.isArray(node)) return node.flatMap(elements);
  return [node, ...elements(node.props?.children)];
}

function harness() {
  const navigations: string[] = [];
  const context = vm.createContext({
    ...Object.fromEntries([...icons, "ChevronDown", "Search", "Plus", "AgentGlyph", "Loader2", "Square", "Play", "Sun", "Moon", "Languages", "ArrowUp"].map((name) => [name, name])),
    React: { createElement: (type: string, props: object, ...children: unknown[]) => ({ type, props: { ...props, children } }) },
    t: (value: string) => value,
    tf: (value: string, args: unknown[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    agentName: (value: string) => value,
    singleAgent, providersHidden, pageVisibleForTool,
    useGatewayPayload: () => null,
    useState: () => [[], () => {}],
    loadCollapsedGroups: () => [],
    getLanguage: () => "zh",
    paletteShortcut: "Ctrl+K",
    APP_LOGO_URL: "logo.png",
    managerNavPages: () => [],
  });
  vm.runInContext(transpile(`${navigationSource}\n${functionSource("AppSidebar")}
    globalThis.sidebar = AppSidebar;
    globalThis.routes = routes;
    globalThis.codexLaunchVisible = (route, activeTool) => ${declaration("onCodexPage").initializer!.getText(app)};`), context);
  return {
    context,
    routes: context.routes as Array<{ id: string; tool?: string; tools?: string[] }>,
    navigations,
    sidebar: (activeTool: string, focus: "all" | "enhance" | "codex" = "all") => context.sidebar({
      tools: [{ id: activeTool, name: activeTool, switchable: true, relayCount: 0 }],
      activeTool, focus, soloAgent: singleAgent(focus), route: "overview", theme: "light", version: "test",
      codexRun: { running: true, busy: false, unavailableReason: null, onStart: () => {}, onStop: () => {} },
      onNavigate: (route: string) => navigations.push(route),
    }) as Element,
  };
}

test("拓展管理只向 Codex 和 Claude 开放，保留原本 Codex 主归属", () => {
  const h = harness();
  const page = h.routes.find((route) => route.id === "userScripts");
  assert.ok(page);
  assert.equal(page.tool, "codex");
  assert.deepEqual(Array.from(page.tools!), ["codex", "claude"]);
  assert.deepEqual(Array.from(h.routes).filter((route) => route.tools).map((route) => route.id), ["userScripts"]);
  for (const tool of ["codex", "claude"]) assert.equal(pageVisibleForTool(page, tool), true);
  for (const tool of ["grok", "opencode", "unknown"]) assert.equal(pageVisibleForTool(page, tool), false);
  assert.equal(pageVisibleForTool(h.routes.find((route) => route.id === "enhance"), "claude"), false);
});

test("实际侧栏在 Claude 下可打开同一拓展管理页，Grok 保持隐藏", () => {
  for (const activeTool of ["codex", "claude", "grok", "opencode"]) {
    const h = harness();
    const entry = elements(h.sidebar(activeTool)).find((node) => node.props["data-tour"] === "nav-userScripts");
    assert.equal(Boolean(entry), activeTool === "codex" || activeTool === "claude", activeTool);
    if (entry) {
      entry.props.onClick();
      assert.deepEqual(h.navigations, ["userScripts"]);
    }
  }
  const h = harness();
  assert.ok(elements(h.sidebar("claude", "enhance")).some((node) => node.props["data-tour"] === "nav-userScripts"));
  assert.equal(agentVisible("codex", "claude"), false, "原来的只管理 Codex 模式仍不显示 Claude");
});

test("只有显式 tools 限定的入口按宿主过滤，其余全局管理保持原行为", () => {
  const h = harness();
  const mcp = h.routes.find((route) => route.id === "context")!;
  // 旧入口即使保留主归属，也不因此改变全局管理的可见行为。
  mcp.tool = "codex";
  const entries = elements(h.sidebar("claude"));
  for (const route of ["context", "skills", "sessions"]) {
    assert.ok(entries.some((node) => node.props["data-tour"] === `nav-${route}`), route);
  }
});

test("Codex 的共享拓展页仍有重启按钮，Claude 下不显示 Codex 重启", () => {
  const { context } = harness();
  assert.equal(context.codexLaunchVisible("userScripts", "codex"), true);
  assert.equal(context.codexLaunchVisible("userScripts", "claude"), false);
  assert.equal(context.codexLaunchVisible("enhance", "codex"), true);
  assert.equal(context.codexLaunchVisible("overview", "codex"), true);
  assert.equal(context.codexLaunchVisible("overview", "grok"), false);
  assert.equal(context.codexLaunchVisible("claude", "claude"), false);
  assert.match(text, /onRestart: \(\) => void actions\.restart\(\)/);
});

test("共享宿主范围同时用于切换 Agent、迟到导航和命令面板", () => {
  const switchTool = declaration("switchTool").getText(app);
  assert.match(switchTool, /if \(!pageVisibleForTool\(currentRoute, toolId\)\)/);
  assert.match(switchTool, /if \(destination\) await navigate\(destination\);\s*else if \(globalNavigationRoutes\.includes\(route\)\) await navigate\("overview"\)/);
  assert.match(text, /if \(currentRoute\?\.tools && !pageVisibleForTool\(currentRoute, activeTool\)\) void navigate\(agentHomeRoute\[activeTool\] \?\? "overview"\)/);
  assert.match(declaration("routeCommand").getText(app), /if \(item\.tools && !pageVisibleForTool\(item, activeTool\)\) return null/);
});

test("只有明确声明 Claude 适配的拓展才显示 Claude Code 宿主标签", () => {
  const row = functionSource("ScriptRow");
  assert.match(row, /pkg\?\.claudeDesktop \? \{ text: "Claude Code" \} : null/);
});

test("Claude 共享拓展页不显示 Codex 专属热重载和下次启动跳过", () => {
  const screen = functionSource("UserScriptsScreen");
  assert.match(text, /<UserScriptsScreen host=\{activeTool\}/);
  assert.match(screen, /const codexHost = host === "codex"/);
  assert.match(screen, /\{codexHost \? <Button onClick=\{\(\) => void reload\(\)\}/);
  assert.match(screen, /\{codexHost \? <label[\s\S]*?aria-label=\{t\("下次启动跳过拓展"\)\}/);
});
