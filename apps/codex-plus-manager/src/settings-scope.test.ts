import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { appDeclarationSource, readAppSource } from "./app-source.test-helper.ts";

const text = readAppSource();
const app = ts.createSourceFile("App.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
// 侧栏路由表在 routes.ts：全局导航那几张表从 globalNavigationRoutes 排到设置模式的路由表之前。
const routesSource = readFileSync(new URL("./routes.ts", import.meta.url), "utf8");

function functionSource(name: string) {
  const declaration = app.statements.find(
    (statement): statement is ts.FunctionDeclaration => ts.isFunctionDeclaration(statement) && statement.name?.text === name,
  );
  assert.ok(declaration, `${name} should exist`);
  return declaration.getText(app).replace(/^export /, "");
}

test("整体设置只放应用级选项，Agent 设置承担工具级选项", () => {
  const global = functionSource("GlobalSettingsScreen");
  const agent = functionSource("AgentSettingsScreen");

  for (const copy of ["界面主题", "配置备份与迁移", "配置文件路径"]) {
    assert.ok(global.includes(`t("${copy}")`), `整体设置应包含 ${copy}`);
  }
  assert.doesNotMatch(global, /Codex 供应商设置|Codex 启动参数/);
  for (const action of ["actions.setThemePreference", "actions.exportSettings", "actions.importSettings"]) {
    assert.ok(global.includes(action), `整体设置应保留 ${action}`);
    assert.ok(!agent.includes(action), `Agent 设置不应包含 ${action}`);
  }

  for (const copy of ["Codex 启动参数"]) {
    assert.ok(agent.includes(`t("${copy}")`), `Agent 设置应包含 ${copy}`);
  }
  // 测试模型已删除：「测试供应商」直接用供应商自己的默认模型。
  assert.doesNotMatch(agent, /供应商测试模型|relayTestModel/);
});

test("Codex 设置与整体设置都进齿轮的设置模式，不占 Agent 的页签", () => {
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  assert.doesNotMatch(sections, /"agentSettings"/);
  assert.match(text, /const settingsModeRoutes = new Set<Route>\(\["settings", "agentSettings",/);

  const globalNavigation = routesSource.slice(routesSource.indexOf("const globalNavigationRoutes"), routesSource.indexOf("const settingsModeRoutes"));
  assert.match(globalNavigation, /"settings"/);
  assert.doesNotMatch(globalNavigation, /"agentSettings"/);
});

test("设置菜单只属于 Codex，Claude / Grok 不再显示只有跳转按钮的空设置页", () => {
  assert.match(text, /\{ id: "agentSettings",[^\n]*tool: "codex" \}/);
  const agent = functionSource("AgentSettingsScreen");
  assert.doesNotMatch(agent, /没有独立设置项|前往 Grok 配置|前往 Claude 增强/);
});

test("Computer Use 在主侧栏第一组（跟在 MCP 与 Skills 后面，仅 macOS），不在设置模式，也不出现在每个 Agent 的菜单里", () => {
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  assert.doesNotMatch(sections, /"computerUse"/);

  const globalNavigation = routesSource.slice(routesSource.indexOf("const globalNavigationRoutes"), routesSource.indexOf("const settingsModeRoutes"));
  assert.match(globalNavigation, /"computerUse"/);
  assert.doesNotMatch(text, /toolNavigationRoutes/);
  assert.match(globalNavigation, /const gatewayNavigationRoutes: Route\[\] = \["gateway", "gatewayRouting"\];/);
  // ⌘K 里归「全局管理」；设置模式和设置分类里都没有它。
  assert.match(globalNavigation, /const managementNavigationRoutes: Route\[\] = \["gatewayUsage", "sessions", "context", "skills", "computerUse"\];/);
  assert.doesNotMatch(routesSource.slice(routesSource.indexOf("const settingsModeRoutes")), /^export const settingsModeRoutes[^\n]*"computerUse"/m);
  const entries = appDeclarationSource("settingsNavEntries");
  assert.doesNotMatch(entries, /key: "computerUse"/);
  // 设置分类只有 Codex 一项看 Codex 可不可用，其余始终显示。
  assert.match(entries, /return entries\.filter\(\(entry\) => entry\.key !== "codex" \|\| codexAvailable\);/);
  const sidebar = appDeclarationSource("AppSidebar");
  const customize = sidebar.indexOf('key: "customize"');
  const computerUse = sidebar.indexOf('key: "computerUse"');
  assert.ok(customize > 0 && computerUse > customize && computerUse < sidebar.indexOf('key: "sessions"'), "Computer Use 紧跟在 MCP 与 Skills 后面");
  assert.match(sidebar, /\.\.\.\(computerUseSupported \? \[\{ key: "computerUse"/);
});

test("标题：设置模式用分类名，Agent 自己的页面和其余页面都用页面名", () => {
  const entry = functionSource("App");
  assert.match(entry, /settingsEntryKey\r?\n\s*\? settingsEntries\.find\(\(entry\) => entry\.key === settingsEntryKey\)\?\.label/);
  // Agent 自己的页面：Agent 名字在侧栏，标题直接是页面名（概览、内置拓展…）。
  assert.match(entry, /onAgentPage\r?\n\s*\? routeTitle\(route\)/);
  assert.match(entry, /<h1 title=\{[^\n]*\}>\{pageTitle\}<\/h1>/);
  assert.match(entry, /const activeToolName = toolEntries\.find/);
});

test("使用模式已经取消：页面不再按模式隐藏，也没有顺带关掉供应商配置切换的逻辑", () => {
  // 侧栏、页签、命令面板都不再过滤路由；停在不属于当前 Agent 的页面上时才回落脚页。
  assert.doesNotMatch(text, /routeVisible|visibleRoutes|providerRoutes|multiAgentRoutes|providersHidden|singleAgent/);
  assert.match(functionSource("AppSidebar"), /\.flatMap\(\(entry\) => entry\.routes\)/);
  assert.match(text, /if \(!item\) return null;\n\s*if \(item\.tools && !pageVisibleForTool\(item, activeTool\)\) return null;/);
  assert.match(text, /if \(currentRoute\?\.tools && !pageVisibleForTool\(currentRoute, activeTool\)\) void navigate\(agentHomeRoute\[activeTool\] \?\? "relay"\);/);
  // 「只用增强功能」会在提交前关闭 relayProfilesEnabled；模式没了，这条写设置的路径也没了。
  assert.doesNotMatch(text, /commitAgentFocusSelection|relayProfilesEnabled: false \}\)/);
  assert.doesNotMatch(text, /relayOffRequested/);
});
