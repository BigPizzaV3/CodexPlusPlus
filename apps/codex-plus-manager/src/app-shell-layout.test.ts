/** 外壳布局：一条侧栏（网关 / Agent / 工具），⌘K 面板，右下角提示卡。直接扫源码，防止结构被无意改回。 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

// 源码契约与 checkout 行尾无关，统一后仍保留全部结构与空白断言。
function readSource(path: string) {
  return readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
}

const text = readSource("./App.tsx");
const styles = readSource("./styles.css");
const app = ts.createSourceFile("App.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

function functionSource(name: string) {
  const declaration = app.statements.find(
    (statement): statement is ts.FunctionDeclaration => ts.isFunctionDeclaration(statement) && statement.name?.text === name,
  );
  assert.ok(declaration, `${name} should exist`);
  return declaration.getText(app);
}

test("只保留一条侧栏，旧的图标栏和 Agent 子侧栏都已移除", () => {
  assert.doesNotMatch(text, /function ApplicationRail|function ToolSwitcher|className="sidebar"|className="app-rail"/);
  assert.match(functionSource("App"), /<AppSidebar\b/);
  assert.match(styles, /\.shell \{\n  grid-template-columns: 220px minmax\(0, 1fr\);/);
  assert.doesNotMatch(styles, /\.app-rail\b|\.tool-chip\b|\.rail-brand\b/);
});

test("侧栏底部是网关状态卡，点它进入总览；Agent 点名字回到上次看的页面，没有就到各自的落脚页", () => {
  const sidebar = functionSource("AppSidebar");
  assert.match(sidebar, /className=\{`cx-side-card[^`]*`\} data-tour="hub" onClick=\{\(\) => onNavigate\("hub"\)\}/);
  assert.match(sidebar, /useGatewayPayload\(/);
  assert.match(text, /const agentHomeRoute: Record<string, Route> = \{ codex: "overview", claude: "claude", grok: "overview" \}/);
  assert.match(text, /const agentEntryRoute = \(toolId: ToolId\) => agentTarget\(agentRoutes\(toolId\), lastAgentRoute\.get\(toolId\), agentHomeRoute\[toolId\] \?\? "overview"\)/);
  assert.match(functionSource("App"), /onOpenAgent=\{\(toolId, destination\) => void switchTool\(toolId, destination \?\? agentEntryRoute\(toolId\)\)\}/);
  assert.match(functionSource("App"), /if \(!globalNavigationRoutes\.includes\(route\)\) lastAgentRoute\.set\(activeTool, route\);/);
});

test("Agent 一行一个始终可见（与设计图一致）；点开 Agent 后它的全部页面都在内容区顶部的页签里", () => {
  const sidebar = functionSource("AppSidebar");
  // 在网关、全局管理、工具这些通用页面时 Agent 行不显示选中，侧栏同一时间只有一个选中项。
  assert.match(sidebar, /const current = tool\.id === activeTool && !onGlobalPage;/);
  assert.match(sidebar, /className=\{`cx-side-item \$\{current \? "active" : ""\} \$\{expanded && agentTabGroups/);
  // 当前 Agent 的页面缩进列在侧栏里它那一行下面，不再是内容区顶部的胶囊。
  assert.match(sidebar, /className="cx-side-subnav" role="group" aria-label=\{t\("Agent 页面"\)\}/);
  assert.match(sidebar, /\{expanded \? \(\(\) => \{\n\s*const pages = agentTabGroups\(navigationSections, routeTool, activeTool\)/);
  // #2913：再点当前 Agent 收起 / 展开它的页面列表，不跳页；切到别的 Agent 时重新展开。
  assert.match(sidebar, /const expanded = current && collapsedAgent !== tool\.id;/);
  assert.match(sidebar, /if \(current\) \{\n\s*setCollapsedAgent\(\(value\) => \(value === tool\.id \? null : tool\.id\)\);\n\s*return;/);
  assert.match(sidebar, /aria-expanded=\{current \? expanded : undefined\}/);
  // 状态点表示接入方式而不是运行状态（运行状态在顶栏和 Codex 行的播放 / 停止键上）。
  assert.match(sidebar, /tone: "direct"/);
  const access = sidebar.slice(sidebar.indexOf("const access ="), sidebar.indexOf("return (", sidebar.indexOf("const access =")));
  assert.doesNotMatch(access, /running/);
  // 只有 Codex 行有播放 / 停止：命令行 Agent 没有常驻进程可启停。
  assert.match(sidebar, /\{tool\.id === "codex" && tool\.switchable \? \(\(\) => \{/);
  assert.match(sidebar, /const label = unattached \? t\("重启并挂载"\) : codexRun\.running \? t\("停止 Codex"\) : t\("启动 Codex"\);/);
  assert.match(sidebar, /aria-label=\{label\}/);
  assert.match(sidebar, /onClick=\{unattached \? codexRun\.onRestart : codexRun\.running \? codexRun\.onStop : codexRun\.onStart\}/);
  // 主动停止写 user_stopped，崩溃检测不把它当成意外停止。
  assert.match(text, /const stoppedByUser = result\.latest_launch\?\.phase === "user_stopped";/);
  const lib = readSource("../src-tauri/src/lib.rs");
  assert.match(lib, /commands::stop_codex_plus,/);
  // 侧栏里当前 Agent 下面缩进列出它的页面（概览 / 增强 / 皮肤 / 插件市场 / 微信 / 多开），仍按页面的 tool 归属和使用模式过滤。
  assert.match(sidebar, /\.flatMap\(\(entry\) => visibleRoutes\(focus, entry\.routes\)\);\n\s*if \(pages\.length < 2\) return null;/);
  // 「连接与供应商」（原旧版供应商配置 + 旧版代理设置）紧跟概览，是 Codex 自己的页面：直连用户每天在这里切供应商，
  // 不藏进设置；Codex 设置（启动参数、维护）在设置模式里。
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  assert.match(sections, /\{ id: "overview", routes: \["overview", "relay"\] \}/);
  for (const id of ["overview", "enhance"]) assert.match(sections, new RegExp(`id: "${id}"`));
  assert.doesNotMatch(sections, /"agentSettings"|"relayEnvironment"/);
  assert.match(text, /const settingsModeRoutes = new Set<Route>\(\["settings", "about", "agentSettings", "maintenance", "gatewaySettings",/);
  assert.doesNotMatch(text, /key: "codexConnection"|key: "legacyProxy"|key: "legacyRelay"/);
  assert.match(text, /id: "relay", get label\(\) \{ return t\("连接与供应商"\); \}/);
  // 内容区顶部不再画 Agent 页签。
  assert.doesNotMatch(functionSource("App"), /<AgentPageTabs\b/);
});

test("⌘K 打开命令面板，总览页渲染在 hub 路由", () => {
  const entry = functionSource("App");
  assert.match(entry, /event\.key\.toLowerCase\(\) === "k"/);
  assert.match(entry, /<CommandPalette commands=\{paletteCommands\}/);
  assert.match(entry, /route === "hub" \? \(\s*<HubOverview/);
});

test("启动 Codex 一眼可见：Codex 页面顶栏最右侧一个主按钮，侧栏播放键常显，都复用 launch / restart（#2744 / #2745）", () => {
  const entry = functionSource("App");
  const button = readSource("./CodexLaunchButton.tsx");
  const hub = readSource("./HubOverview.tsx");
  // 一份状态喂给顶栏、侧栏：运行中是「重启 Codex」，没找到 Codex 时置灰并说明原因。
  assert.match(entry, /const codexLaunch: CodexLaunchState = \{\n\s*running: codexAppRunning,\n\s*busy: launchPending,/);
  assert.match(entry, /overview\?\.codex_app\?\.status === "missing"\n\s*\? t\("没有找到 Codex 应用，请先安装 Codex"\)/);
  assert.match(entry, /onLaunch: \(\) => void actions\.launch\(\),\n\s*onRestart: \(\) => void actions\.restart\(\),/);
  // ⌘K 里的「启动 / 重启 Codex」保留。
  assert.match(entry, /label: codexAppRunning \? t\("重启 Codex"\) : t\("启动 Codex"\)/);
  // 按钮本身：文字随运行状态切换；置灰时提示挂在外层 span 上，悬停仍能看到原因。
  assert.match(button, /const label = state\.busy \? t\("处理中…"\) : state\.running \? t\("重启 Codex"\) : t\("启动 Codex"\);/);
  assert.match(button, /onClick=\{state\.running \? state\.onRestart : state\.onLaunch\}/);
  assert.match(button, /disabled=\{state\.busy \|\| unavailable\}/);
  assert.match(button, /<span className=\{`codex-launch \$\{className\}`\.trim\(\)\} data-running=\{state\.running\} data-unavailable=\{unavailable\} title=\{hint\}>/);
  assert.match(button, /variant="default"/);
  // 顶栏：只在 Codex 的页面里、排在页面操作挂载点之后。Codex++ 不只服务 Codex，
  // 网关、全局管理、工具类页面和 Claude Code / Grok 的页面都不放。
  assert.match(entry, /const onCodexPage = \(routeTool\(route\) === "codex" && \(route !== "userScripts" \|\| extensionHost === "codex"\)\) \|\| \(route === "overview" && activeTool === "codex"\);/);
  assert.match(
    entry,
    /<div className="topbar-page-actions" data-topbar-actions \/>\n[\s\S]{0,400}\{onCodexPage \? <CodexLaunchButton className="topbar-codex-launch" size="sm" state=\{codexLaunch\} \/> : null\}/,
  );
  assert.equal(entry.match(/<CodexLaunchButton\b/g)?.length, 1, "启动按钮只在顶栏出现一次");
  const codexRoutes = text.slice(text.indexOf("const routes: Array"), text.indexOf("const navigationSections")).split("\n")
    .filter((line) => line.includes('tool: "codex"')).map((line) => /id: "(\w+)"/.exec(line)?.[1]);
  for (const id of ["relay", "multiLaunch", "enhance", "dreamSkin", "userScripts", "pluginMarket", "weixin", "agentSettings"]) {
    assert.ok(codexRoutes.includes(id), `${id} 属于 Codex，顶栏要显示启动按钮`);
  }
  for (const id of ["hub", "gatewayProviders", "gateway", "gatewayUsage", "sessions", "claude", "settings"]) {
    assert.ok(!codexRoutes.includes(id), `${id} 不属于 Codex，顶栏不放启动按钮`);
  }
  // 网关总览面向所有 Agent，不放专门的「启动 Codex」按钮。
  assert.doesNotMatch(hub, /CodexLaunchButton|codexLaunch/);
  assert.doesNotMatch(entry, /codexLaunch=\{codexLaunch\}/);
  // 侧栏播放键：常显、点击区放大、不依赖悬停；没找到 Codex 时置灰。
  const sidebar = functionSource("AppSidebar");
  assert.match(sidebar, /tool\.id === "codex" && tool\.switchable \? "has-run" : ""/);
  assert.match(sidebar, /disabled=\{codexRun\.busy \|\| \(!codexRun\.running && Boolean\(codexRun\.unavailableReason\)\)\}/);
  assert.match(entry, /codexRun=\{\{ running: codexAppRunning, busy: launchPending, unavailableReason: codexLaunch\.unavailableReason, onStart: codexLaunch\.onLaunch,/);
  assert.match(styles, /\.shell \.side-agent-run \{\n(?:  [^\n]*\n)*  width: 24px;\n  height: 24px;/);
  assert.match(styles, /\.shell \.side-agent-run\[data-running="false"\] \{\n  background: hsl\(var\(--status-success\) \/ 0\.14\);\n  color: hsl\(var\(--status-success\)\);/);
  assert.doesNotMatch(styles, /:hover \.side-agent-run|\.side-agent-run \{[^}]*(?:opacity: 0;|visibility: hidden)/);
  // 不用 :has()：旧版系统 WebKit 不认，整条规则会失效。
  assert.doesNotMatch(styles, /:has\(\.side-agent-run\)/);
  assert.match(styles, /\.shell \.side-agent-wrap\.has-run \.side-item-label \{/);
});

test("提示卡按语气分色、悬停暂停、不显示进度条，失败可复制详情", () => {
  const notice = functionSource("NoticeDialog");
  assert.match(notice, /noticeTone\(notice\.status\)/);
  assert.match(notice, /onMouseEnter=\{\(\) => setPaused\(true\)\}/);
  assert.match(notice, /tone === "error" && notice\.message/);
  assert.doesNotMatch(notice, /toast-progress/);
  assert.doesNotMatch(styles, /\.toast-progress\b|@keyframes toast-progress/);
});

test("Claude / Grok 的供应商配置页已去掉：要么经网关接入供应商池，要么照 Agent 自己的配置直连", () => {
  assert.doesNotMatch(text, /id: "claudeRelay"/);
  assert.doesNotMatch(text, /id: "grok", get label/);
  assert.doesNotMatch(text, /<ClaudeProviderPage|function GrokScreen/);
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  assert.doesNotMatch(sections, /"claudeRelay"|"grok"/);
  assert.match(sections, /"claude"/, "Claude 增强保留");
});

test("页面不再画第二条标题栏：PageHeader 和工具页的操作都送进外壳顶栏", () => {
  const management = readSource("./ManagementPages.tsx");
  const extra = readSource("./ManagementExtra.tsx");
  const cache = readSource("./agent-cache.tsx");
  assert.match(management, /export function PageHeader\([^)]*\) \{\n  return <TopbarActions>\{actions\}<\/TopbarActions>;/);
  assert.match(extra, /function Header\([^)]*\) \{ return <TopbarActions>\{actions\}<\/TopbarActions>; \}/);
  for (const source of [management, extra]) assert.doesNotMatch(source, /className="mg-header"/);
  assert.match(cache, /<TopbarActions>/);
  assert.doesNotMatch(cache, /CardHeader|className="panel agent-cache-panel"/);
  // 顶栏里给页面操作留了单独的挂载点，排在「启动 / 重启 Codex」按钮左边。
  assert.match(text, /<div className="topbar-page-actions" data-topbar-actions \/>/);
});

test("紧凑密度层：管理页不再套外框，覆盖规则带 .shell 前缀不依赖加载顺序", () => {
  const layer = styles.slice(styles.indexOf("/* ── Graphite · 紧凑密度"));
  assert.match(layer, /\.shell \.mg-page \{\n  margin: 0 auto;\n  overflow: visible;\n  border: 0;/);
  assert.match(layer, /\.shell \.topbar \{\n  min-height: 44px;/);
  // 层内每条规则都以 .shell 开头
  const selectors = [...layer.matchAll(/^([^\s/@}][^{]*)\{$/gm)].map((match) => match[1]);
  assert.ok(selectors.length > 20);
  for (const selector of selectors) assert.match(selector, /^\.shell /, selector);
});

test("请求日志、会话、MCP、Skills 拎成「全局管理」，管理所有 Agent，不再挂在 Codex 下面", () => {
  // 万物皆拓展是全局管理的第一个入口。
  assert.match(text, /id: "userScripts", get label\(\) \{ return t\("万物皆拓展"\); \}/);
  // 「用量」入口在全局管理里：统计（gatewayUsage）与请求日志（requestLog）是同一页的两个页签。
  assert.match(text, /const managementNavigationRoutes: Route\[\] = \["userScripts", "gatewayUsage", "sessions", "context", "skills"\]/);
  assert.match(text, /const navAliasRoute: Partial<Record<Route, Route>> = \{ requestLog: "gatewayUsage" \};/);
  for (const id of ["requestLog", "sessions", "context", "skills"]) {
    const line = text.split("\n").find((row) => row.includes(`{ id: "${id}",`));
    assert.ok(line, id);
    assert.doesNotMatch(line, /tool: "codex"/, `${id} 不再只属于 Codex`);
  }
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  assert.doesNotMatch(sections, /"requestLog"|"sessions"|"context"|"skills"/);
  // 侧栏第一组：万物皆拓展、MCP 与 Skills（两个胶囊）、会话；第二组「聚合网关」：总览、供应商池、Agent 接入、路由、用量。
  const sidebar = functionSource("AppSidebar");
  assert.match(sidebar, /\{ key: "extensions", route: "userScripts" as Route, label: t\("万物皆拓展"\)/);
  assert.ok(sidebar.indexOf('key: "extensions"') < sidebar.indexOf('key: "customize"'), "万物皆拓展排在第一个");
  assert.match(sidebar, /match: customizeRoutes/);
  assert.match(sidebar, /<span>\{t\("聚合网关"\)\}<\/span>/);
  for (const key of ["hub", "providers", "gateway", "routing", "usage"]) assert.match(sidebar, new RegExp(`key: "${key}"`));
  assert.match(sidebar, /match: \["gatewayUsage", "requestLog"\]/);
  assert.match(sidebar, /const shownGeneral = general\.filter\(\(item\) => routeVisible\(focus, item\.route\)\);/);
  assert.match(sidebar, /const shownGateway = gatewayEntries\.filter\(\(item\) => routeVisible\(focus, item\.route\)\);/);
  assert.match(text, /const customizeRoutes: Route\[\] = \["context", "skills"\];/);
  // 路由是聚合网关的一部分，不进设置模式。
  assert.doesNotMatch(text.slice(text.indexOf("const settingsModeRoutes"), text.indexOf("const customizeRoutes")), /"gatewayRouting"/);
  // 网关的会话页并进「会话」：Codex 页签是完整的会话管理，其余页签是多 Agent 会话列表。
  assert.doesNotMatch(text, /"gatewaySessions"/);
  const entry = functionSource("App");
  // 只管理 Codex 时没有 Agent 页签，固定显示 Codex 的完整会话管理。
  assert.match(entry, /const shownSessionTab: SessionTab = soloAgent === "codex" \? "codex" : sessionTab;/);
  assert.match(entry, /route === "sessions" && !soloAgent \? <SessionAgentTabs/);
  assert.match(entry, /route === "sessions" && shownSessionTab !== "codex" \? \(/);
  assert.match(entry, /<SessionsView agent=\{shownSessionTab\}/);
  assert.match(entry, /route === "sessions" && shownSessionTab === "codex" \? \(\s*<ProjectSessionPage/);
});

test("请求日志按 Agent 筛选；网关记录带 agent 字段，没有的算 Codex", () => {
  const screen = functionSource("RequestLogScreen");
  // 只管理 Codex 时固定看 Codex 的请求，不出现 Agent 筛选。
  assert.match(screen, /const agentScope = codexOnly \? "codex" : agentFilter;/);
  assert.match(screen, /agentScope !== "all" && requestLogAgent\(entry\) !== agentScope/);
  assert.match(screen, /\[entries, filter, query, sessionFilter, agentScope\]/);
  assert.match(functionSource("requestLogAgent"), /return entry\.agent \|\| "codex";/);
});

test("「Codex 连接」页顶部是连接方式（经网关 / 直连）：没接入时建议改用新版网关，接入后显示状态并能断开还原", () => {
  const app = functionSource("App");
  assert.match(app, /<LegacyRelayGatewayHint[\s\S]*onOpenGateway=\{\(\) => void navigate\("gatewayProviders"\)\}/);
  // 连接方式作为 connection 传进 RelayScreen，只在列表视图显示；旧版代理设置作为 advanced 收进同一页底部。
  assert.match(app, /\{route === "relay" \|\| route === "relayEnvironment" \? \(\n\s*<RelayScreen/);
  assert.match(app, /connection=\{\(\n\s*<LegacyRelayGatewayHint/);
  assert.match(app, /advanced=\{\(\n\s*<ProxySettingsScreen/);
  assert.match(app, /advancedOpen=\{route === "relayEnvironment" \|\| relayAdvancedOpen\}/);
  assert.match(app, /onDisconnect=\{\(\) => setLegacyDisconnectOpen\(true\)\}/);
  assert.match(app, /legacyDisconnectOpen \? \(\s*<AgentDisconnectDialog/);
  assert.match(app, /invoke<GatewayResult>\("disable_gateway_binding", \{ agent: "codex", previewToken \}\)/);
  assert.doesNotMatch(app, /"disable_gateway_binding", \{ agent: "codex" \}/);
  const hint = functionSource("LegacyRelayGatewayHint");
  assert.match(hint, /t\("建议改用新版网关"\)/);
  assert.match(hint, /form\.activeRelayId === CODEX_GATEWAY_RELAY_ID/);
  assert.match(hint, /t\("Codex 当前经 Codex\+\+ 网关（新版）"\)/);
  assert.match(hint, /t\("Codex 已从网关切回直连"\)/);
  // 一行：标题写现在怎么连（经 Codex++ 网关 / 直连 · 供应商名），右侧切换 + 三点菜单；断开并还原在菜单里。
  assert.match(hint, /\? t\("经 Codex\+\+ 网关"\)/);
  assert.match(hint, /tf\("直连 · \{0\}", \[direct\.name \|\| t\("未命名供应商"\)\]\)/);
  assert.match(hint, /\{viaGateway \? <MenuItem danger onClick=\{onDisconnect\}>\{t\("断开并还原"\)\}<\/MenuItem> : null\}/);
  // 列表里托管的网关供应商单独标出，只能「使用」，不能编辑、删除。
  const card = functionSource("SortableRelayProfileCard");
  assert.match(card, /const gatewayManaged = profile\.id === CODEX_GATEWAY_RELAY_ID;/);
  assert.match(card, /data-tone="gateway">\{t\("新版网关"\)\}/);
  assert.match(card, /\{gatewayManaged \? null : \(\s*<span className="relay-card-extra" onClick=\{\(event\) => event\.stopPropagation\(\)\}>/);
  // 次要操作在三点菜单里；删除仍先查占用。
  assert.match(card, /<Menu label=\{t\("供应商操作"\)\}>/);
  assert.match(card, /const deleteIssue = relayProfileDeleteIssue\(form, profile\.id\);/);
  // 被网关接管时可切换、可编辑的列表整块隐藏，要先断开并还原；只读列出直连供应商，供迁移进供应商池。
  const relay = functionSource("RelayScreen");
  assert.match(relay, /const takenOverByGateway = normalized\.activeRelayId === CODEX_GATEWAY_RELAY_ID;/);
  assert.match(relay, /\{normalized\.relayProfilesEnabled && !takenOverByGateway \? \(/);
  assert.match(relay, /\{normalized\.relayProfilesEnabled && takenOverByGateway \? \(/);
  // 直连用户可以关掉「加入供应商池」的提醒；网关接管时总是提示。
  assert.match(relay, /poolLink\.pending\.length && \(takenOverByGateway \|\| !nudgeDismissed\)/);
  assert.match(relay, /window\.localStorage\.setItem\(DIRECT_POOL_NUDGE_KEY, "1"\)/);
});

test("Skills 与 MCP 同一种行：每个 Skill 一行，右侧每个 Agent 一枚开关胶囊，不再有页面自己的配色和按钮尺寸", () => {
  const extra = readSource("./ManagementExtra.tsx");
  const css = readSource("./ManagementPages.css");
  assert.match(extra, /<CxRow className="cx-skill-row"/);
  assert.match(extra, /const agentColumns: McpAgentId\[\] = MCP_AGENTS\.filter\(\(agent\) => agentVisible\(focus, agent\)\);/);
  // 行里每个可见 Agent 一枚开关；分组标题同一列是这一组的批量开关。
  assert.match(extra, /agentColumns\.map\(\(agent\) => <AgentToggle /);
  assert.match(extra, /agentColumns\.map\(\(agent\) => <GroupAgentSelect /);
  assert.doesNotMatch(extra, /mg-agent-toggle|mg-skill-group-select/);
  assert.doesNotMatch(css, /\.dark \.mg-skills-page|\.mg-skills-page \.mg-primary|\.mg-skills-page :is\(\.mg-button/);
});

test("Agent 页签条不能是滚动容器：否则在 .screen 的 grid 里会被长页面挤成 1px，点进去就看不到页签", () => {
  const css = readSource("./ToolPages.css");
  const rules = [...css.matchAll(/\.agent-page-tabs\s*\{([^}]*)\}/g)].map((match) => match[1]);
  assert.ok(rules.length > 0);
  for (const body of rules) assert.doesNotMatch(body, /overflow/);
  assert.match(rules.join("\n"), /flex-wrap:\s*wrap/);
});

test("安装维护并进 Codex 设置：在设置模式的「Codex」分类里，设置页下方渲染维护内容，旧路由跳到设置", () => {
  const app = functionSource("App");
  assert.match(app, /const next: Route = target === "maintenance" \? "agentSettings" : target === "about" \? "settings" : target === "relayEnvironment" \? "relay" : target === "recommendations" \? "home" : target;/);
  assert.match(app, /\{route === "agentSettings" \|\| route === "maintenance" \? \(/);
  assert.match(text, /\{ key: "codex", label: "Codex", icon: AppWindow, route: "agentSettings", group: "agent"/);
  // 分类目录交给侧栏，页内不再有左侧目录；分组卡片 id 仍然保留，深链照常。
  assert.doesNotMatch(app, /sections=\{codexSettingsSections\(\)\}/);
  const ids = [...text.slice(text.indexOf("function codexSettingsSections"), text.indexOf("const EXPERIMENTAL_TITLE")).matchAll(/id: "([^"]+)"/g)].map((match) => match[1]);
  assert.deepEqual(ids, ["codex-settings-check", "codex-settings-startup", "codex-settings-takeover", "codex-settings-manual-launch"]);
  // 每个分组是一节（CxSection），id 挂在节的最外层。
  for (const id of ids) assert.match(text, new RegExp(`<CxSection(?:(?!<CxSection)[\\s\\S])*?\\bid="${id}"`));
  // 常用的在上，不常用的额外启动参数放进维护页最后的「高级」卡片，和手动启动在一起。
  assert.match(app, /<MaintenanceScreen[\s\S]*?advanced=\{\(\n\s*<AgentSettingsScreen/);
  const maintenance = functionSource("MaintenanceScreen");
  assert.match(maintenance, /<CxSection id="codex-settings-manual-launch" title=\{t\("高级"\)\}>\n\s*<CxCard>\n\s*\{advanced\}/);
  // 同一个 Codex 路径只显示一次：换路径、清除保存的路径在「Codex 应用」那一行的菜单里。
  assert.doesNotMatch(maintenance, /t\("当前识别"\)|t\("保存的应用路径"\)/);
  assert.match(maintenance, /actions\.chooseCodexAppPath\("folder"\)/);
  assert.match(maintenance, /actions\.clearCodexAppPath\(\)/);
});

test("页内目录收成单列：增强页的页签横排成胶囊、分组锚点不再显示；设置模式内容限宽", () => {
  for (const page of ["settings-page enhance-page sectioned-page cx-enhance-page", "settings-page codex-settings-page", "weixin-connect-page sectioned-page cx-page"]) {
    assert.ok(text.includes(`className="${page}"`), page);
  }
  const layer = readSource("./cursor-ui.css");
  assert.match(layer, /:root \.shell \.sectioned-page \{\n  display: flex;\n  flex-direction: column;/);
  assert.match(layer, /:root \.shell \.page-section-nav-list \{\n  display: none;/);
  assert.match(layer, /:root \.shell \.screen\[data-settings-mode="true"\] > \* \{\n  width: min\(100%, var\(--cx-content-narrow\)\);/);
  assert.match(text, /className="enhance-tabs tp-tabs cx-pills" role="tablist"/);
  // 「（实验）」和「需重启」从文字里拿出来做成标题旁的小标签。
  const toggle = functionSource("FeatureToggle");
  assert.match(toggle, /EXPERIMENTAL_TITLE\.test\(title\)/);
  assert.match(toggle, /RESTART_HINT\.test\(detail\)/);
});

test("插件市场紧凑布局的规则带 .shell 前缀：正式版 styles.css 后加载，同权重的 .panel { padding: 0 } 会清掉卡片内边距", () => {
  const css = readSource("./PluginMarketScreen.css");
  const block = css.slice(css.lastIndexOf("/*", css.indexOf("紧凑布局"))).replace(/\/\*[\s\S]*?\*\//g, "");
  const selectors = [...block.matchAll(/(?:^|\})\s*([^{}]+?)\s*\{/g)].map((match) => match[1].trim()).filter(Boolean);
  assert.ok(selectors.length > 10);
  for (const group of selectors) for (const selector of group.split(",")) assert.match(selector.trim(), /^\.shell /, selector);
  assert.match(block, /\.shell \.plugin-market-card \{[^}]*padding:\s*14px 16px/);
});

test("导入供应商弹窗：只有文字那块占满宽度，头像（也是 span）保持固定尺寸", () => {
  const css = readSource("./GatewayPage.css");
  assert.doesNotMatch(css, /\.gw-import-item > span \{/);
  assert.match(css, /\.gw-import-item > span:not\(\.gw-avatar\) \{[^}]*flex: 1/);
  assert.match(css, /\.gw-import-item > \.gw-avatar \{[^}]*flex: none/);
});

test("代理设置按归属拆开：本地服务与请求避震移到网关设置（新旧共用），旧版页只留旧版专用的两项", () => {
  assert.match(text, /id: "relayEnvironment", get label\(\) \{ return t\("旧版代理设置"\); \}/);
  const shared = functionSource("SharedServiceSettings");
  // 两节：本地服务、请求避震；标题悬停说明新旧共用。
  assert.match(shared, /t\("本地服务"\)/);
  assert.match(shared, /t\("网关和旧版本地协议代理共用这些设置"\)/);
  assert.match(shared, /<CxSection /);
  assert.match(shared, /protocolProxyAutoRetry/);
  assert.match(shared, /helperMaxRequestBodyMib/);
  const legacy = functionSource("ProxySettingsScreen");
  assert.doesNotMatch(legacy, /helperMaxRequestBodyMib|protocolProxyAutoRetryMaxAttempts/);
  assert.match(legacy, /const takenOverByGateway = form\.activeRelayId === CODEX_GATEWAY_RELAY_ID;/);
  assert.match(legacy, /t\("目录外模型映射（旧版）"\)/);
  // 跳去网关设置的那一行没用，删掉；环境检测全部正常时只占一行。
  assert.doesNotMatch(legacy, /本地服务与请求避震|onOpenGatewaySettings/);
  const env = functionSource("RelayEnvironmentScreen");
  assert.match(env, /const failed = result \? checks\.filter\(\(check\) => !check\.passed\) : \[\];/);
  assert.match(env, /\{failed\.map\(\(check\) => \(/);
  // 「高级」全是直连专用：被网关接管时整块不显示（配置切换被关掉时仍显示，好重新打开）。
  assert.match(functionSource("RelayScreen"), /\{takenOverByGateway && normalized\.relayProfilesEnabled \? null : \(\n\s*<CxSection>\n\s*<CxCard>\n\s*<details className="cx-set-disclosure cx-set-disclosure-first cx-codex-advanced"/);
  assert.match(functionSource("App"), /<SharedServiceSettings[\s\S]*?form=\{settingsForm\}/);
});

test("首页：状态、精选推荐、其他赞助商、常用入口；广告只在首页，首页是侧栏第一项和启动落脚页", () => {
  const app = functionSource("App");
  assert.match(app, /\{route === "home" \? \(\n\s*<HomeScreen\n\s*ads=\{ads\}/);
  assert.equal((text.match(/<HomeScreen\b/g) ?? []).length, 1);
  assert.doesNotMatch(text, /SponsorBoard|FloatingSponsor/);
  assert.doesNotMatch(functionSource("OverviewScreen"), /featuredAds|AdGrid/);
  // 侧栏主模式第一项；旧「推荐内容」路由跳到首页，设置模式里不再有这一项。
  assert.match(functionSource("AppSidebar"), /const general: SideEntry\[\] = \[\n\s*\{ key: "home", route: "home", label: t\("首页"\), icon: House, match: \["home", "recommendations"\] \},/);
  assert.match(app, /target === "recommendations" \? "home" : target/);
  assert.match(app, /if \(next === "home"\) await refreshAds\(true\);/);
  assert.doesNotMatch(text.slice(text.indexOf("const settingsModeRoutes"), text.indexOf("const customizeRoutes")), /"recommendations"/);
  const home = functionSource("HomeScreen");
  // 首页不能全是广告：先是状态，再是精选推荐和其他赞助商，最后常用入口。
  const order = ['title={t("状态")}', 'title={t("精选推荐")}', 'title={t("更多赞助商")}', 'title={t("常用")}'].map((needle) => home.indexOf(needle));
  assert.ok(order.every((index, position) => index > 0 && (position === 0 || index > order[position - 1])), String(order));
  // 只管增强时不显示网关一行，入口也按使用模式过滤。
  assert.match(home, /\{enhanceOnly \? null : \(/);
  assert.match(home, /shortcuts\.filter\(\(item\) => routeVisible\(focus, item\.route\)\)/);
  // 推荐：置顶位不管几个都上横幅、一行均分宽度，不挪去别处；推荐池的赞助商不能上横幅；其他赞助商全部列出不折叠；同一家去重。卡片上不再单独标「推广」，赞助性质由这一节标题旁的说明交代。
  assert.match(home, /const heroes = topAds;/);
  assert.match(home, /const rest = pool\.filter\(\(ad\) => ad\.type === "sponsor"\);/);
  assert.match(home, /gridTemplateColumns: `repeat\(\$\{heroes\.length\}, minmax\(0, 1fr\)\)`/);
  assert.match(home, /\{rest\.map\(\(ad\) => \(/);
  assert.doesNotMatch(home, /showAllAds|rest\.slice\(0, 3\)/);
  assert.match(home, /!topIdentities\.has\(adIdentity\(ad\)\)/);
  assert.match(home, /\.filter\(\(ad\) => ad\.type === "sponsor"\)/);
  assert.doesNotMatch(home, /t\("推广"\)/);
  assert.match(home, /aside=\{<span>\{t\("赞助商推荐：支持 Codex\+\+ 的服务"\)\}<\/span>\}/);
  // 精选卡片不用 button：旧版 WebKit 里 button 做网格项会被大图撑宽、整页错位。
  assert.match(home, /<div\n\s*className="home-hero"/);
  assert.doesNotMatch(home, /<button[^>]*className="home-hero"/);
  assert.match(functionSource("featuredAds"), /filter\(\(ad\) => !isExpiredAd\(ad\)\)/);
});

test("Codex 在「经网关」和「直连」之间一键切换：切到网关沿用上次接入设置，切回直连先预览还原", () => {
  const app = functionSource("App");
  // 切到网关：和「Agent 接入」开关同一条命令，沿用上次的绑定；池空或模型不在池里时说明原因并带去对应页面。
  assert.match(app, /const draft = draftFromBinding\(previous\);\n\s*const issue = validateDraft\(draft, payload\);/);
  assert.match(app, /invoke<GatewayResult>\("save_gateway_binding", \{ agent: "codex", binding: \{ \.\.\.bindingFromDraft\(draft, previous\), enabled: true \} \}\)/);
  assert.match(app, /onConnect=\{\(\) => void connectCodexGateway\(\)\}/);
  const hint = functionSource("LegacyRelayGatewayHint");
  // 两段式切换在「连接方式」标题右侧；切回直连走 onDisconnect（断开并还原的预览对话框）。
  assert.match(hint, /className="mg-segmented cx-connection-switch" role="radiogroup"/);
  assert.match(hint, /onClick=\{connect\}\n\s*role="radio"/);
  assert.match(hint, /onClick=\{onDisconnect\}\n\s*role="radio"/);
  // 网关里没有接入记录（孤立状态）时切换按钮换成「切回直连」。
  assert.match(hint, /\{orphan \? <button className="cx-primary" onClick=\{onDisconnect\} type="button">\{t\("切回直连"\)\}<\/button> : modeSwitch\}/);
});

test("「连接与供应商」页的 hook 都在编辑页 / 目录页的提前返回之前（否则点「添加供应商」整页白屏）", () => {
  const screen = functionSource("RelayScreen");
  const earlyReturn = screen.indexOf("if (detailProfile) {");
  assert.ok(earlyReturn > 0);
  const after = screen.slice(earlyReturn);
  assert.doesNotMatch(after, /\n  (?:const|let) [^\n]*\buse[A-Z]\w*\(/, "提前返回之后不能再调用 hook");
  assert.ok(screen.indexOf("const [nudgeDismissed") < earlyReturn);
});

test("直连供应商和网关供应商池打通：每行标出是否已在池里，没进池的可单个或全部加入，接管期间也能迁移", () => {
  const link = functionSource("useCodexPoolLink");
  // 复制走网关「导入供应商」同一条命令；只认直连供应商（codex）和聚合供应商（aggregate）两种来源。
  assert.match(link, /importCandidates\(payload\)\.filter\(\(item\) => \(item\.source === "codex" \|\| item\.source === "aggregate"\) && profileIds\.includes\(item\.id\)\)/);
  assert.match(link, /invoke<GatewayResult>\("import_gateway_providers", \{/);
  assert.match(link, /publishGatewayPayload\(result\);/);
  const control = functionSource("PoolLinkControl");
  assert.match(control, /t\("已在供应商池"\)/);
  assert.match(control, /t\("加入供应商池"\)/);
  const screen = functionSource("RelayScreen");
  assert.match(screen, /const poolLink = useCodexPoolLink\(actions, normalized\.relayProfiles\.map\(\(profile\) => profile\.id\)\);/);
  assert.match(screen, /onClick=\{\(\) => void poolLink\.addAll\(\)\}/);
  // 提示收在「直连供应商」标题右侧一行，不再单独一张卡。
  assert.match(screen, /<span className="cx-pool-hint"/);
  assert.doesNotMatch(screen, /\{poolBanner\}/);
  // 接管期间直连供应商整块隐藏，只留「被网关接管」一行说明，没有使用 / 编辑 / 删除。
  const takenOver = screen.slice(screen.indexOf("{normalized.relayProfilesEnabled && takenOverByGateway ? ("), screen.indexOf("{normalized.relayProfilesEnabled && !takenOverByGateway ? ("));
  assert.match(takenOver, /title=\{t\("Codex 正由 Codex\+\+ 网关接管"\)\}/);
  assert.doesNotMatch(takenOver, /directProfiles\.map|onSwitchProfile|onEdit|removeRelayProfile/);
  assert.match(functionSource("SortableRelayProfileCard"), /\{gatewayManaged \|\| !poolLink \? null : <PoolLinkControl link=\{poolLink\} profileId=\{profile\.id\} \/>\}/);
});

test("Codex 被网关接管时隐藏旧版供应商功能；总开关关闭时保留恢复入口（#2801）", () => {
  const screen = functionSource("RelayScreen");
  assert.doesNotMatch(screen, /actions=\{takenOverByGateway \? null : \(/);
  // 总开关收进页面底部「高级」；关闭时页面顶部一条提示带「打开配置切换」。
  // 被网关接管时整个「高级」不显示；只有总开关被关掉时才显示，好把它重新打开。
  assert.match(screen, /\{takenOverByGateway && normalized\.relayProfilesEnabled \? null : \(\r?\n\s*<CxSection>[\s\S]*?className="switch-row compact relay-master-switch/);
  assert.match(screen, /onClick=\{\(\) => void saveRelaySettings\(\{ \.\.\.normalized, relayProfilesEnabled: true \}\)\}/);
  assert.match(screen, /\{takenOverByGateway \? null : \(\r?\n\s+<button className="cx-primary" onClick=\{\(\) => \{ setNewProfileDraft\(null\); setDetailProfileId\(null\); setProviderCatalogOpen\(true\); \}\}/);
  assert.match(screen, /重新开启只恢复配置写入能力，不会切换直连供应商或断开当前网关/);
  // 导入只写旧版供应商列表、不动 Codex 配置，接管期间也要能从 cc-switch 导入。
  const header = screen.slice(screen.indexOf("<PageHeader"), screen.indexOf("{connection}"));
  assert.doesNotMatch(header, /takenOverByGateway \? null : \(?\s*<div className="third-party-import"/);
  assert.match(header, /<div className="third-party-import" ref=\{thirdPartyImportTriggerRef\}>/);
  assert.doesNotMatch(screen, /disabled=\{takenOverByGateway\}/);
  // #2801：列表藏起来后给出去处：点「直连」断开后恢复，迁移到供应商池走「添加供应商 → 导入」。
  assert.match(screen, /点上方「直连」断开网关后恢复；要把它们加进供应商池，去「供应商池」→「添加供应商」→「导入」/);
});

test("多开页可以直接编辑、删除供应商：删除先查占用再二次确认，网关托管项不给入口（#2801）", () => {
  const screen = functionSource("CodexMultiLaunchScreen");
  // 编辑就地打开同一个供应商详情页，返回 / 保存后回到多开列表。
  assert.match(screen, /<RelayProfileDetail\r?\n\s+profile=\{detailProfile\}/);
  assert.match(screen, /onBack=\{\(\) => setDetailProfileId\(null\)\}/);
  assert.match(screen, /onClick=\{\(\) => setDetailProfileId\(profile\.id\)\}/);
  // 删除：先用旧版列表同一套占用检查，再提醒运行中的多开实例，最后危险确认。
  const remove = screen.slice(screen.indexOf("const deleteProfile"), screen.indexOf("if (detailProfile)"));
  const issue = remove.indexOf("relayProfileDeleteIssue(normalized, profile.id)");
  const confirm = remove.indexOf("appConfirm(message");
  const save = remove.indexOf("removeRelayProfile(normalized, profile.id)");
  assert.ok(issue > 0 && confirm > issue && save > confirm, "删除必须先查占用、再确认、最后保存");
  assert.match(remove, /runningInstancesForProfile\(actions\.multiLaunchInstances, profile\.id\)/);
  assert.match(remove, /danger: true/);
  assert.match(screen, /const gatewayManaged = profile\.id === CODEX_GATEWAY_RELAY_ID;/);
  // 编辑 / 删除收进行尾三点菜单；网关托管项没有菜单，整行也不可点。
  assert.match(screen, /\{gatewayManaged \? null : \(\r?\n\s+<Menu label=/);
  assert.match(screen, /onClick=\{gatewayManaged \? undefined : \(\) => setDetailProfileId\(profile\.id\)\}/);
  assert.match(screen, /disabled=\{normalized\.relayProfiles\.length <= 1\}/);
  // 使用中的供应商给出标记；导入入口指到旧版供应商页。
  assert.match(screen, /\{active \? <CxBadge>\{t\("使用中"\)\}<\/CxBadge> : null\}/);
  assert.match(screen, /onClick=\{onOpenRelay\}/);
  assert.match(functionSource("App"), /<CodexMultiLaunchScreen\r?\n\s+form=\{settingsForm\}\r?\n\s+relayFiles=\{relayFiles\}\r?\n\s+onOpenRelay=\{\(\) => void navigate\("relay"\)\}/);
  // 网关接管期间从多开进详情页，不能「设为当前」绕过断开并还原。
  assert.match(functionSource("RelayProfileDetail"), /isNew \|\| form\.activeRelayId === CODEX_GATEWAY_RELAY_ID \? null : \(/);
});

test("用量图表有鼠标交互：整列感应、高亮当天、浮出明细卡", () => {
  const usage = readSource("./GatewayUsage.tsx");
  assert.match(usage, /className="gw-bar-hit"/);
  assert.match(usage, /className="gw-chart-tip"/);
  const css = readSource("./GatewayPage.css");
  assert.match(css, /\.gw-chart\[data-hovering="true"\] \.gw-bar:not\(\[data-active="true"\]\)/);
  assert.match(css, /\.gw-usage-row:hover/);
});

test("Codex 概览与 Claude / Grok 同一套仪表板：被网关接管时状态栏只给管理接入、断开并还原，不给切换下拉框", () => {
  const screen = functionSource("OverviewScreen");
  assert.match(screen, /<AgentDashboard\n\s*actions=\{codexActions\}\n\s*agent="codex"/);
  const gatewayBranch = screen.slice(screen.indexOf("const codexActions = gatewayManaged ? ("), screen.indexOf(") : activeProfile ? ("));
  assert.match(gatewayBranch, /t\("管理接入"\)/);
  assert.doesNotMatch(gatewayBranch, /AppSelect|switchFromOverview/);
  // 健康检查放在 Codex 仪表板下面
  assert.ok(screen.indexOf("overview-health-header") > screen.indexOf("<AgentDashboard"));
});

test("从 Agent 概览点「全部」「请求日志」：会话页停在这个 Agent 的页签，请求日志按它过滤", () => {
  const app = functionSource("App");
  assert.match(app, /if \(kind === "sessions"\) setSessionTab\(agent\);\n\s*else setRequestLogAgentFilter\(agent\);/);
  assert.match(app, /<RequestLogScreen\s+initialAgent=\{requestLogAgentFilter\}/);
  const dashboard = readSource("./AgentDashboard.tsx");
  assert.match(dashboard, /onOpenList\("sessions", agent\)/);
  assert.match(dashboard, /onOpenList\("requestLog", agent\)/);
});

test("「关于」并进「设置」：侧栏底部只留齿轮，设置模式一个分类一页，旧的关于入口换到版本与更新", () => {
  const sidebar = functionSource("AppSidebar");
  assert.match(sidebar, /onClick=\{\(\) => openSettings\("settings", \["app-settings-general"\]\)\}/);
  const app = functionSource("App");
  // 只显示侧栏选中分类的卡片；深链到某张卡片时换到它所在的分类。
  assert.match(app, /settingsSections\.map\(\(id\) => `:not\(#\$\{id\}\)`\)/);
  assert.match(app, /if \(settingsFocus\) setSettingsSections\(settingsSectionsFor\(settingsFocus\)\);/);
  const entries = text.slice(text.indexOf("function settingsNavEntries"), text.indexOf("const settingsModeRoutes"));
  for (const id of ["app-settings-general", "app-settings-config", "app-settings-update", "app-settings-logs", "app-settings-diagnostics", "app-settings-about"]) {
    assert.ok(entries.includes(`"${id}"`), `${id} 要归到某个设置分类`);
  }
  assert.match(app, /if \(target === "about"\) setSettingsFocus\("app-settings-update"\);/);
  assert.ok(app.indexOf("<GlobalSettingsScreen") < app.indexOf("<AboutScreen"));
  const ids = [...text.slice(text.indexOf("function appSettingsSections"), text.indexOf("function AppSettingsFocus")).matchAll(/id: "([^"]+)"/g)].map((match) => match[1]);
  assert.equal(ids.length, 6);
  for (const id of ids) assert.match(text, new RegExp(`id="${id}"`));
});

test("Computer Use 工作原理图与网关总览同一套三列拓扑", () => {
  const page = readSource("./ComputerUsePage.tsx");
  assert.match(page, /<HubTopology links=\{links\}>/);
  assert.match(page, /data-node="cu-core"/);
  assert.match(page, /data-node=\{`cu-client:\$\{id\}`\}/);
  const hub = readSource("./HubOverview.tsx");
  assert.match(hub, /export function HubTopology\(/);
});

test("首次入门清单与可选导览分开；模式确认后显示清单，旧用户可从设置重开", () => {
  const app = functionSource("App");
  // 使用模式问卷先弹，答完才进引导。
  assert.match(app, /\{agentFocusOpen \? <AgentFocusDialog initial=\{agentFocus\} onDone=\{closeAgentFocus\} onCancel=\{cancelAgentFocus\} \/> : null\}/);
  assert.match(app, /\{tourOpen && !agentFocusOpen \? <OnboardingTour focus=\{agentFocus\} onClose=\{closeTour\} \/> : null\}/);
  assert.match(app, /setupOpen && !agentFocusOpen && !agentFocusChecking \? \(\s*<OnboardingSetup/);
  assert.match(app, /window\.localStorage\.setItem\(TOUR_STORAGE_KEY, String\(TOUR_VERSION\)\)/);
  assert.match(functionSource("initialSetupOpen"), /has\("previewRoute"\)/);
  assert.match(functionSource("initialSetupOpen"), /shouldShowSetup\(/);
  assert.match(app, /if \(agentFocusOpen \|\| tourOpen\) return;\s*setPaletteOpen/);
  assert.match(app, /updatePrompt && !agentFocusOpen && !tourOpen/);
  assert.match(functionSource("GlobalSettingsScreen"), /onClick=\{onOpenTour\}/);
  const model = readSource("./onboarding-model.ts");
  const sidebar = functionSource("AppSidebar");
  for (const match of model.matchAll(/target: "([^"]+)"/g)) {
    const target = match[1];
    const anchored = target.startsWith("nav-") ? sidebar.includes(`tour: "${target}"`) : sidebar.includes(`data-tour="${target}"`) || sidebar.includes(`tour: "${target}"`);
    assert.ok(anchored, `引导目标 ${target} 在侧栏里没有锚点`);
  }
});

test("设置页「版本与更新」：已是最新时不显示旧版本的安装包和说明；有新版本时说明收进可展开区", () => {
  const about = functionSource("AboutScreen");
  // 只有有新版本时才显示安装包名；已在后台下好时主按钮是「安装」。
  assert.match(about, /latest && update\?\.assetName \?/);
  assert.match(about, /ready \? t\("安装"\) : t\("下载并安装"\)/);
  assert.match(about, /title=\{t\("有可用更新时自动下载"\)\}/);
  assert.match(about, /\{update\?\.updateAvailable && update\.releaseSummary \? \(\n\s*<details className="app-update-notes[^"]*">/);
  assert.doesNotMatch(about, /<Metric label=\{t\("状态"\)\}/);
});

test("Computer Use 的页签与会话页等同一种下划线页签（tp-tabs），不再改写 mg-tabs", () => {
  const page = readSource("./ComputerUsePage.tsx");
  assert.match(page, /<div className="tp-tabs cu-tabs" role="tablist"/);
  const css = readSource("./ComputerUsePage.css");
  assert.doesNotMatch(css, /\.mg-tabs\.cu-tabs/);
});

test("供应商头像在任何列表里都不参与伸缩：挑选弹层、导入弹窗都出过被拉成横条的问题", () => {
  const css = readSource("./GatewayPage.css");
  assert.match(css, /span\.gw-avatar\[data-size\] \{\s*flex: none;/);
  assert.doesNotMatch(css, /\.gw-select-item span \{/);
});

test("Claude Code / Grok 的概览是仪表板：接入状态、今日数据、最近会话与请求、配置摘要", () => {
  assert.match(functionSource("OverviewScreen"), /<AgentDashboard agent=\{activeTool\}/);
  const dashboard = readSource("./AgentDashboard.tsx");
  for (const command of ["load_gateway_usage", "load_request_logs", "list_agent_sessions", "load_agent_mcp", "list_installed_skills", "open_agent_terminal"]) {
    assert.ok(dashboard.includes(`"${command}"`), command);
  }
  const lib = readSource("../src-tauri/src/lib.rs");
  assert.match(lib, /agent_launch_commands::open_agent_terminal,/);
});

test("网关页的 Agent 图标：Claude Code / Grok 用遮罩按文字颜色绘制，不再把黑色单色图当 <img>", () => {
  for (const name of ["GatewayAgents.tsx", "TrayPanel.tsx", "GatewayPage.tsx"]) {
    const source = readSource(`./${name}`);
    assert.doesNotMatch(source, /src=\{agentIcon\(/, name);
  }
  const css = readSource("./GatewayPage.css");
  assert.match(css, /\.gw-agent-mono \{[^}]*mask: var\(--gw-agent-icon\)/);
});

test("内层滚动的页面（网关各页、Computer Use）滚动区延伸到窗口边，滚动条不压内容", () => {
  const css = readSource("./styles.css");
  assert.match(css, /\.shell :is\(\.gw-content, \.cu-content\) \{[^}]*width: calc\(100% \+ 20px\);[^}]*margin-right: -20px;[^}]*padding-right: 20px;/);
});

test("会话页可以删除会话，不再有「在终端继续」；Claude Code / Grok 走 delete_agent_session，Codex 走 delete_local_session", () => {
  const sessions = readSource("./GatewaySessions.tsx");
  assert.doesNotMatch(sessions, /在终端继续|resume_agent_session/);
  assert.match(sessions, /"delete_agent_session"/);
  assert.match(sessions, /"delete_local_session"/);
  assert.match(sessions, /appConfirm\(/);
  const lib = readSource("../src-tauri/src/lib.rs");
  assert.match(lib, /gateway_commands::delete_agent_session,/);
});

test("会话页列出 Claude Desktop 的会话：标出 Desktop / 已归档，恢复命令用 cliSessionId，删除前提示先退出 Desktop", () => {
  const sessions = readSource("./GatewaySessions.tsx");
  // Desktop / 已归档是行标题旁的小标签（CxRow 的 badges）。
  assert.match(sessions, /session\.surface === "desktop" \? <CxBadge title=\{t\("Claude Desktop 里的会话"\)\}>Desktop<\/CxBadge>/);
  assert.match(sessions, /session\.archived \? <CxBadge>\{t\("已归档"\)\}<\/CxBadge>/);
  assert.match(sessions, /const id = session\.resumeId \?\? session\.id;/);
  assert.match(sessions, /需要先退出 Claude Desktop/);
  const commands = readSource("../src-tauri/src/gateway_commands.rs");
  assert.match(commands, /sessions::list_claude_sessions_with_desktop\(/);
  assert.match(commands, /"claude" if desktop => \{\n\s*if codex_plus_core::computer_use::claude_desktop_running\(\)/);
});

test("添加供应商是弹出框，不再铺在供应商池页面里；池为空时显示引导", () => {
  const providers = readSource("./GatewayProviders.tsx");
  assert.match(providers, /className="gw-modal gw-catalog-modal"/);
  assert.match(providers, /\{catalogOpen \? \(\n\s*<ProviderCatalog/);
  assert.doesNotMatch(providers, /showCatalog/);
  assert.match(providers, /className="gw-card gw-pool-empty"/);
});

test("侧栏一级分组标题比子项醒目，快速跳转的快捷键按平台显示且弱化（#2778）", () => {
  assert.match(functionSource("AppSidebar"), /<kbd>\{paletteShortcut\}<\/kbd>/);
  assert.doesNotMatch(functionSource("AppSidebar"), /<kbd>⌘K<\/kbd>/);
  const label = styles.match(/\.side-group-label > button:first-child \{[^}]*\}/)?.[0] ?? "";
  const item = styles.match(/\n\.side-item \{[^}]*\}/)?.[0] ?? "";
  const size = (rule: string) => Number(rule.match(/font-size: ([\d.]+)px/)?.[1] ?? 0);
  assert.ok(size(item) > 0 && size(label) >= size(item), "分组标题字号不能比子项小");
  assert.match(label, /font-weight: 700/);
  assert.match(styles, /\.side-search kbd \{[^}]*font-size: 9\.5px/);
});

test("Codex 在它自己的窗口里退出后，侧栏 / 顶栏的运行状态能跟上（#2778）", () => {
  const entry = functionSource("App");
  assert.match(entry, /invoke<boolean \| null>\("codex_app_running"\)/);
  assert.match(entry, /if \(document\.visibilityState !== "visible"\) return;/);
  assert.match(entry, /running !== codexAppRunning\) void refreshOverview\(true\)/);
  assert.match(text, /const codexRunningPollMs = 5_000;/);
});

test("外部启动、没挂载增强脚本的 Codex：健康检查提示并给「重启并挂载」，侧栏停止按钮染色", () => {
  const health = functionSource("healthItems");
  assert.match(health, /overview\?\.runtime_health\?\.codex_unattached\n    \? \{/);
  assert.match(health, /status: "unattached",\n      ok: false,/);
  assert.match(health, /action: "restart" as const,/);
  const overview = functionSource("OverviewScreen");
  assert.match(overview, /item\.action === "restart" \? \(\n\s+<button className="cx-secondary" onClick=\{\(\) => void actions\.restart\(\)\}/);
  assert.match(functionSource("statusLabel"), /unattached: t\("未挂载"\)/);
  assert.match(functionSource("App"), /unattached: Boolean\(overview\?\.runtime_health\?\.codex_unattached\)/);
  assert.match(text, /const unattached = codexRun\.running && codexRun\.unattached;/);
  assert.match(text, /onClick=\{unattached \? codexRun\.onRestart : codexRun\.running \? codexRun\.onStop : codexRun\.onStart\}/);
  assert.match(text, /unattached \? <RotateCcw aria-hidden="true" \/>/);
  assert.match(readSource("./cursor-ui.css"), /\.cx-side-run\[data-unattached="true"\]:not\(:disabled\) \{\n  color: hsl\(var\(--status-warning\)\);/);
});

test("拓展是侧栏一级入口，不再藏在 Codex 页签里；Grok 不在侧栏 Agent 列表", () => {
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const routeTool"));
  assert.doesNotMatch(sections, /"userScripts"/, "拓展不再出现在 Agent 页签里");
  assert.match(text, /const globalNavigationRoutes: Route\[\] = \[[^\]]*"userScripts"/);
  assert.match(text, /const hiddenSidebarTools = new Set<string>\(\["grok"\]\);/);
  assert.match(text, /setToolEntries\(\(result\.tools \?\? \[\]\)\.filter\(\(tool\) => !hiddenSidebarTools\.has\(tool\.id\)\)\);/);
  // 上次停在被隐藏的工具上时回到 Codex。
  assert.match(text, /setActiveTool\(visibleActiveTool\(result\.activeTool\)\);/);
  assert.match(text, /setActiveTool\(visibleActiveTool\(normalized\.activeTool\)\);/);
});

test("供应商详情页不再 absolute 盖住吸顶标题：打开时 .screen 改纵向 flex 不滚动（#2888）", () => {
  const css = readSource("./styles.css");
  // .screen 让出滚动：详情页在文档流里占满剩余高度，内部由 .relay-detail-body 滚。
  assert.match(css, /\.screen:has\(\.relay-detail-page\) \{\n  display: flex;\n  flex-direction: column;\n  overflow: hidden;/);
  // 详情页本身不再是覆盖 .screen 的 absolute 层。
  const rule = css.match(/\.screen \.relay-detail-page \{[^}]*\}/)?.[0] ?? "";
  assert.match(rule, /position: relative;/);
  assert.match(rule, /inset: auto;/);
  assert.match(rule, /flex: 1 1 auto;/);
});
