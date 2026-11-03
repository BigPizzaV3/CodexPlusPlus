import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const text = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const app = ts.createSourceFile("App.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

function functionSource(name: string) {
  const declaration = app.statements.find(
    (statement): statement is ts.FunctionDeclaration => ts.isFunctionDeclaration(statement) && statement.name?.text === name,
  );
  assert.ok(declaration, `${name} should exist`);
  return declaration.getText(app);
}

test("整体设置只放应用级选项，Agent 设置承担工具级选项", () => {
  const global = functionSource("GlobalSettingsScreen");
  const agent = functionSource("AgentSettingsScreen");

  for (const copy of ["基础设置", "配置备份与迁移", "配置文件路径"]) {
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

test("Agent 设置挂在各 Agent 的侧栏菜单，整体设置留在最左栏", () => {
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  assert.match(sections, /"agentSettings"/);

  const globalNavigation = text.slice(text.indexOf("const globalNavigationRoutes"), text.indexOf("const defaultSettings"));
  assert.match(globalNavigation, /"settings"/);
  assert.doesNotMatch(globalNavigation, /"agentSettings"/);
});

test("设置菜单只属于 Codex，Claude / Grok 不再显示只有跳转按钮的空设置页", () => {
  assert.match(text, /\{ id: "agentSettings",[^\n]*tool: "codex" \}/);
  const agent = functionSource("AgentSettingsScreen");
  assert.doesNotMatch(agent, /没有独立设置项|前往 Grok 配置|前往 Claude 增强/);
});

test("Computer Use 放在侧栏「工具」组，不再出现在每个 Agent 的菜单里", () => {
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  assert.doesNotMatch(sections, /"computerUse"/);

  const globalNavigation = text.slice(text.indexOf("const globalNavigationRoutes"), text.indexOf("const defaultSettings"));
  assert.match(globalNavigation, /"computerUse"/);
  // 「工具」组第一个是推荐内容（用户要求置顶），Computer Use 紧随其后；网关自己一组。
  assert.match(globalNavigation, /const toolNavigationRoutes: Route\[\] = \["recommendations", "computerUse",/);
  assert.match(globalNavigation, /const gatewayNavigationRoutes: Route\[\] = \["hub", "gatewayProviders", "gateway", "gatewayRouting",/);
  const sidebar = functionSource("AppSidebar");
  assert.match(sidebar, /gatewayNavigationRoutes\.map/);
  assert.match(sidebar, /visibleRoutes\(focus, toolNavigationRoutes\)\.map/);
  // 只管理 Codex 时不显示：Codex 自带 Computer Use，这一页接入的是其他客户端。
  assert.match(globalNavigation, /const multiAgentRoutes = new Set<Route>\(\["agentCollaboration", "computerUse"\]\);/);
});

test("Agent 设置标题跟随当前 Agent，整体设置标题固定", () => {
  const entry = functionSource("App");
  assert.match(entry, /route === "agentSettings" \? tf\("\{0\} 设置", \[activeToolName\]\) : soloAgent && route === "gateway" \? tf\("\{0\} 接入", \[agentName\(soloAgent\)\]\) : routeTitle\(navAliasRoute\[route\] \?\? route\)/);
  assert.match(entry, /const activeToolName = toolEntries\.find/);
});

test("只用增强功能：隐藏供应商页面，配置保存成功后才提交模式", () => {
  const helpers = text.slice(text.indexOf("const multiAgentRoutes"), text.indexOf("function agentFocusDescription"));
  assert.match(helpers, /const providerRoutes = new Set<Route>\(\["hub", "gatewayProviders", "gateway", "gatewayRouting", "gatewaySettings", "gatewayUsage", "requestLog", "relay", "relayEnvironment", "multiLaunch"\]\);/);
  assert.match(helpers, /providersHidden\(focus\) && providerRoutes\.has\(routeId\)/);
  // 侧栏、页签、命令面板都按同一条规则过滤。
  assert.match(functionSource("AppSidebar"), /enhanceOnly \? null : \(\s*<section className=\{`side-group \$\{collapsed\.includes\("gateway"\)/);
  assert.match(functionSource("AgentPageTabs"), /visibleRoutes\(focus, entry\.routes\)/);
  assert.match(text, /if \(!item \|\| !routeVisible\(agentFocus, routeId\)\) return null;/);
  // 启动默认停在网关总览：模式不让看就回落脚页。
  assert.match(text, /if \(!routeVisible\(agentFocus, route\)\) void navigate\(agentHomeRoute\[activeTool\] \?\? "overview"\);/);
  assert.match(text, /closeAgentFocus = \(mode: FocusMode\) => commitAgentFocusSelection\(mode,/);
  assert.match(text, /normalizeSettings\(\{ \.\.\.latest\.settings, relayProfilesEnabled: false \}\)/);
  assert.match(text, /rebaseSettingsEdits\(normalizeSettings\(current\), previous, saved\)/);
  assert.doesNotMatch(text, /relayOffRequested/);
});
