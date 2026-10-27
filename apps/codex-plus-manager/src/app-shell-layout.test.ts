/** 外壳布局：一条侧栏（网关 / Agent / 工具），⌘K 面板，右下角提示卡。直接扫源码，防止结构被无意改回。 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const text = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
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

test("侧栏顶部是网关状态卡，点它进入总览；Agent 点名字回到上次看的页面，没有就到各自的落脚页", () => {
  const sidebar = functionSource("AppSidebar");
  assert.match(sidebar, /className=\{`side-hub[^`]*`\} data-tour="hub" onClick=\{\(\) => onNavigate\("hub"\)\}/);
  assert.match(sidebar, /useGatewayPayload\(/);
  assert.match(text, /const agentHomeRoute: Record<string, Route> = \{ codex: "overview", claude: "claude", grok: "overview" \}/);
  assert.match(text, /const agentEntryRoute = \(toolId: ToolId\) => agentTarget\(agentRoutes\(toolId\), lastAgentRoute\.get\(toolId\), agentHomeRoute\[toolId\] \?\? "overview"\)/);
  assert.match(functionSource("App"), /onOpenAgent=\{\(toolId, destination\) => void switchTool\(toolId, destination \?\? agentEntryRoute\(toolId\)\)\}/);
  assert.match(functionSource("App"), /if \(!globalNavigationRoutes\.includes\(route\)\) lastAgentRoute\.set\(activeTool, route\);/);
});

test("Agent 一行一个始终可见（与设计图一致）；点开 Agent 后它的全部页面都在内容区顶部的页签里", () => {
  const sidebar = functionSource("AppSidebar");
  // 在网关、全局管理、工具这些通用页面时 Agent 行不显示选中，侧栏同一时间只有一个选中项。
  assert.match(sidebar, /className=\{`side-item side-agent \$\{current && !onGlobalPage \? "current active" : ""\}`\}/);
  // 状态点表示接入方式而不是运行状态（运行状态在顶栏和 Codex 行的播放 / 停止键上）。
  assert.match(sidebar, /tone: "direct"/);
  const access = sidebar.slice(sidebar.indexOf("const access ="), sidebar.indexOf("return (", sidebar.indexOf("const access =")));
  assert.doesNotMatch(access, /running/);
  // 只有 Codex 行有播放 / 停止：命令行 Agent 没有常驻进程可启停。
  assert.match(sidebar, /\{tool\.id === "codex" && tool\.switchable \? \(\n\s*<button\n\s*aria-label=\{codexRun\.running \? t\("停止 Codex"\) : t\("启动 Codex"\)\}/);
  assert.match(sidebar, /onClick=\{codexRun\.running \? codexRun\.onStop : codexRun\.onStart\}/);
  // 主动停止写 user_stopped，崩溃检测不把它当成意外停止。
  assert.match(text, /const stoppedByUser = result\.latest_launch\?\.phase === "user_stopped";/);
  const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.match(lib, /commands::stop_codex_plus,/);
  // 侧栏不再列当前 Agent 的页面。
  assert.doesNotMatch(sidebar, /agent-pages|navigationSections/);
  // 页签：概览 / 供应商 / 增强 / 设置 分组用细线隔开，仍按页面的 tool 归属过滤；通用页面不显示。
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  for (const id of ["overview", "providers", "enhance", "settings"]) assert.match(sections, new RegExp(`id: "${id}"`));
  const tabs = functionSource("AgentPageTabs");
  assert.match(tabs, /if \(globalNavigationRoutes\.includes\(route\)\) return null;/);
  assert.match(tabs, /agentTabGroups\(navigationSections, routeTool, activeTool\)/);
  assert.match(tabs, /className="agent-page-tabs-sep"/);
  assert.match(functionSource("App"), /<AgentPageTabs activeTool=\{activeTool\}/);
});

test("⌘K 打开命令面板，总览页渲染在 hub 路由", () => {
  const entry = functionSource("App");
  assert.match(entry, /event\.key\.toLowerCase\(\) === "k"/);
  assert.match(entry, /<CommandPalette commands=\{paletteCommands\}/);
  assert.match(entry, /route === "hub" \? \(\s*<HubOverview/);
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
  const management = readFileSync(new URL("./ManagementPages.tsx", import.meta.url), "utf8");
  const extra = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
  const cache = readFileSync(new URL("./agent-cache.tsx", import.meta.url), "utf8");
  assert.match(management, /export function PageHeader\([^)]*\) \{\n  return <TopbarActions>\{actions\}<\/TopbarActions>;/);
  assert.match(extra, /function Header\([^)]*\) \{ return <TopbarActions>\{actions\}<\/TopbarActions>; \}/);
  for (const source of [management, extra]) assert.doesNotMatch(source, /className="mg-header"/);
  assert.match(cache, /<TopbarActions>/);
  assert.doesNotMatch(cache, /CardHeader|className="panel agent-cache-panel"/);
  // 顶栏里给页面操作留了单独的挂载点，排在 Codex 状态与启动按钮左边。
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
  assert.match(text, /const managementNavigationRoutes: Route\[\] = \["requestLog", "sessions", "context", "skills"\]/);
  for (const id of ["requestLog", "sessions", "context", "skills"]) {
    const line = text.split("\n").find((row) => row.includes(`{ id: "${id}",`));
    assert.ok(line, id);
    assert.doesNotMatch(line, /tool: "codex"/, `${id} 不再只属于 Codex`);
  }
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  assert.doesNotMatch(sections, /"requestLog"|"sessions"|"context"|"skills"/);
  assert.match(functionSource("AppSidebar"), /managementNavigationRoutes\.map/);
  // 网关的会话页并进「会话」：Codex 页签是完整的会话管理，其余页签是多 Agent 会话列表。
  assert.doesNotMatch(text, /"gatewaySessions"/);
  const entry = functionSource("App");
  assert.match(entry, /route === "sessions" && sessionTab !== "codex" \? \(/);
  assert.match(entry, /<SessionsView agent=\{sessionTab\}/);
  assert.match(entry, /route === "sessions" && sessionTab === "codex" \? \(\s*<ProjectSessionPage/);
});

test("请求日志按 Agent 筛选；网关记录带 agent 字段，没有的算 Codex", () => {
  const screen = functionSource("RequestLogScreen");
  assert.match(screen, /agentFilter !== "all" && requestLogAgent\(entry\) !== agentFilter/);
  assert.match(screen, /\[entries, filter, query, sessionFilter, agentFilter\]/);
  assert.match(functionSource("requestLogAgent"), /return entry\.agent \|\| "codex";/);
});

test("旧版 Codex 供应商配置页顶部：没接入时建议改用新版网关，接入后显示状态并能断开还原", () => {
  const app = functionSource("App");
  assert.match(app, /<LegacyRelayGatewayHint[\s\S]*onOpenGateway=\{\(\) => void navigate\("gatewayProviders"\)\}/);
  assert.match(app, /invoke<\{ status: string; message: string \}>\("disable_gateway_binding", \{ agent: "codex" \}\)/);
  const hint = functionSource("LegacyRelayGatewayHint");
  assert.match(hint, /t\("建议改用新版网关"\)/);
  assert.match(hint, /form\.activeRelayId === CODEX_GATEWAY_RELAY_ID/);
  assert.match(hint, /t\("Codex 当前经 Codex\+\+ 网关（新版）"\)/);
  assert.match(hint, /t\("Codex 已从网关切回直连"\)/);
  // 列表里托管的网关供应商单独标出，只能「使用」，不能编辑、删除。
  const card = functionSource("SortableRelayProfileCard");
  assert.match(card, /const gatewayManaged = profile\.id === CODEX_GATEWAY_RELAY_ID;/);
  assert.match(card, /data-tone="gateway">\{t\("新版网关"\)\}/);
  assert.match(card, /\{gatewayManaged \? null : \(\s*<span className="relay-card-extra">/);
  // 被网关接管时旧版列表整块隐藏，要先断开并还原；页签名带「旧版」。
  const relay = functionSource("RelayScreen");
  assert.match(relay, /const takenOverByGateway = normalized\.activeRelayId === CODEX_GATEWAY_RELAY_ID;/);
  assert.match(relay, /\{normalized\.relayProfilesEnabled && !takenOverByGateway \? \(/);
  assert.match(text, /id: "relay", get label\(\) \{ return t\("旧版供应商配置"\); \}/);
});

test("Skills 与 MCP 共用一张多 Agent 表：每个 Agent 一列勾选，不再有页面自己的配色和按钮尺寸", () => {
  const extra = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
  const css = readFileSync(new URL("./ManagementPages.css", import.meta.url), "utf8");
  assert.match(extra, /className="mg-matrix mg-agent-matrix mg-skill-matrix"/);
  assert.match(extra, /MCP_AGENTS\.map\(\(agent\) => <AgentColumn /);
  assert.match(extra, /MCP_AGENTS\.map\(\(agent\) => <MatrixCell /);
  assert.doesNotMatch(extra, /mg-agent-toggle|mg-skill-group-select/);
  assert.doesNotMatch(css, /\.dark \.mg-skills-page|\.mg-skills-page \.mg-primary|\.mg-skills-page :is\(\.mg-button/);
});

test("Agent 页签条不能是滚动容器：否则在 .screen 的 grid 里会被长页面挤成 1px，点进去就看不到页签", () => {
  const css = readFileSync(new URL("./ToolPages.css", import.meta.url), "utf8");
  const rules = [...css.matchAll(/\.agent-page-tabs\s*\{([^}]*)\}/g)].map((match) => match[1]);
  assert.ok(rules.length > 0);
  for (const body of rules) assert.doesNotMatch(body, /overflow/);
  assert.match(rules.join("\n"), /flex-wrap:\s*wrap/);
});

test("安装维护并进设置页：设置分组只有一个页面，设置页下方渲染维护内容，旧路由跳到设置", () => {
  const sections = text.slice(text.indexOf("const navigationSections"), text.indexOf("const globalNavigationRoutes"));
  assert.match(sections, /\{ id: "settings", routes: \["agentSettings"\] \}/);
  const app = functionSource("App");
  assert.match(app, /const next: Route = target === "maintenance" \? "agentSettings" : target === "about" \? "settings" : target;/);
  assert.match(app, /\{route === "agentSettings" \|\| route === "maintenance" \? \(/);
  // 合并后的设置页用左侧分类目录，目录里的每一项都对应一张卡片。
  assert.match(app, /<PageSectionNav label=\{t\("设置分类"\)\} sections=\{codexSettingsSections\(\)\} \/>/);
  const ids = [...text.slice(text.indexOf("function codexSettingsSections"), text.indexOf("const EXPERIMENTAL_TITLE")).matchAll(/id: "([^"]+)"/g)].map((match) => match[1]);
  assert.ok(ids.length >= 7);
  for (const id of ids) assert.match(text, new RegExp(`<Panel id="${id}">`));
  assert.ok(app.indexOf("<AgentSettingsScreen") < app.indexOf("<MaintenanceScreen"), "设置在上、安装维护在下");
});

test("推荐内容只显示赞助商推荐", () => {
  const screen = functionSource("RecommendationsScreen");
  assert.doesNotMatch(screen, /t\("普通推荐"\)|ad\.type === "normal"/);
  assert.match(screen, /t\("赞助商推荐"\)/);
});

test("设置类页面共用左侧分类目录：增强、设置、微信连接；开关列表限宽", () => {
  for (const page of ["settings-page enhance-page sectioned-page", "settings-page codex-settings-page sectioned-page", "weixin-connect-page sectioned-page"]) {
    assert.ok(text.includes(`className="${page}"`), page);
  }
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  assert.match(css, /\.shell \.sectioned-page \{[^}]*grid-template-columns:\s*172px minmax\(0, 880px\)/);
  // 「（实验）」和「需重启」从文字里拿出来做成标题旁的小标签。
  const toggle = functionSource("FeatureToggle");
  assert.match(toggle, /EXPERIMENTAL_TITLE\.test\(title\)/);
  assert.match(toggle, /RESTART_HINT\.test\(detail\)/);
});

test("插件市场紧凑布局的规则带 .shell 前缀：正式版 styles.css 后加载，同权重的 .panel { padding: 0 } 会清掉卡片内边距", () => {
  const css = readFileSync(new URL("./PluginMarketScreen.css", import.meta.url), "utf8");
  const block = css.slice(css.lastIndexOf("/*", css.indexOf("紧凑布局"))).replace(/\/\*[\s\S]*?\*\//g, "");
  const selectors = [...block.matchAll(/(?:^|\})\s*([^{}]+?)\s*\{/g)].map((match) => match[1].trim()).filter(Boolean);
  assert.ok(selectors.length > 10);
  for (const group of selectors) for (const selector of group.split(",")) assert.match(selector.trim(), /^\.shell /, selector);
  assert.match(block, /\.shell \.plugin-market-card \{[^}]*padding:\s*14px 16px/);
});

test("导入供应商弹窗：只有文字那块占满宽度，头像（也是 span）保持固定尺寸", () => {
  const css = readFileSync(new URL("./GatewayPage.css", import.meta.url), "utf8");
  assert.doesNotMatch(css, /\.gw-import-item > span \{/);
  assert.match(css, /\.gw-import-item > span:not\(\.gw-avatar\) \{[^}]*flex: 1/);
  assert.match(css, /\.gw-import-item > \.gw-avatar \{[^}]*flex: none/);
});

test("代理设置按归属拆开：本地服务与请求避震移到网关设置（新旧共用），旧版页只留旧版专用的两项", () => {
  assert.match(text, /id: "relayEnvironment", get label\(\) \{ return t\("旧版代理设置"\); \}/);
  const shared = functionSource("SharedServiceSettings");
  assert.match(shared, /t\("本地服务（新旧共用）"\)/);
  assert.match(shared, /className="gw-setting-row"/);
  assert.match(shared, /protocolProxyAutoRetry/);
  assert.match(shared, /helperMaxRequestBodyMib/);
  const legacy = functionSource("ProxySettingsScreen");
  assert.doesNotMatch(legacy, /helperMaxRequestBodyMib|protocolProxyAutoRetryMaxAttempts/);
  assert.match(legacy, /const takenOverByGateway = form\.activeRelayId === CODEX_GATEWAY_RELAY_ID;/);
  assert.match(legacy, /t\("目录外模型映射（旧版）"\)/);
  assert.match(functionSource("App"), /<SharedServiceSettings[\s\S]*?form=\{settingsForm\}/);
});

test("置顶推荐位只放在网关总览一处，各 Agent 的概览不再重复", () => {
  assert.match(functionSource("App"), /\{route === "hub" \? <SponsorBoard ads=\{ads\} actions=\{actions\} \/> : null\}/);
  assert.doesNotMatch(functionSource("OverviewScreen"), /SponsorBoard/);
  assert.equal((text.match(/<SponsorBoard /g) ?? []).length, 1);
});

test("Codex 被网关接管时，旧版供应商页的配置切换、从第三方导入、添加供应商整组隐藏（不是置灰）", () => {
  const screen = functionSource("RelayScreen");
  assert.match(screen, /actions=\{takenOverByGateway \? null : \(/);
  assert.doesNotMatch(screen, /disabled=\{takenOverByGateway\}/);
});

test("用量图表有鼠标交互：整列感应、高亮当天、浮出明细卡", () => {
  const usage = readFileSync(new URL("./GatewayUsage.tsx", import.meta.url), "utf8");
  assert.match(usage, /className="gw-bar-hit"/);
  assert.match(usage, /className="gw-chart-tip"/);
  const css = readFileSync(new URL("./GatewayPage.css", import.meta.url), "utf8");
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
  assert.match(app, /<RequestLogScreen initialAgent=\{requestLogAgentFilter\}/);
  const dashboard = readFileSync(new URL("./AgentDashboard.tsx", import.meta.url), "utf8");
  assert.match(dashboard, /onOpenList\("sessions", agent\)/);
  assert.match(dashboard, /onOpenList\("requestLog", agent\)/);
});

test("「关于」并进「设置」：侧栏底部只留齿轮，合并页用左侧目录，旧的关于入口滚到版本与更新", () => {
  const sidebar = functionSource("AppSidebar");
  assert.match(sidebar, /\(\["settings"\] as Route\[\]\)\.map/);
  const app = functionSource("App");
  assert.match(app, /<PageSectionNav label=\{t\("设置分类"\)\} sections=\{appSettingsSections\(\)\} \/>/);
  assert.match(app, /if \(target === "about"\) setSettingsFocus\("app-settings-update"\);/);
  assert.ok(app.indexOf("<GlobalSettingsScreen") < app.indexOf("<AboutScreen"));
  const ids = [...text.slice(text.indexOf("function appSettingsSections"), text.indexOf("function AppSettingsFocus")).matchAll(/id: "([^"]+)"/g)].map((match) => match[1]);
  assert.equal(ids.length, 6);
  for (const id of ids) assert.match(text, new RegExp(`id="${id}"`));
});

test("Computer Use 工作原理图与网关总览同一套三列拓扑", () => {
  const page = readFileSync(new URL("./ComputerUsePage.tsx", import.meta.url), "utf8");
  assert.match(page, /<HubTopology links=\{links\}>/);
  assert.match(page, /data-node="cu-core"/);
  assert.match(page, /data-node=\{`cu-client:\$\{id\}`\}/);
  const hub = readFileSync(new URL("./HubOverview.tsx", import.meta.url), "utf8");
  assert.match(hub, /export function HubTopology\(/);
});

test("首次使用引导：首次启动自动打开（预览模式除外），看完或跳过都记下版本；设置页可重新打开；每一步的目标在侧栏都有锚点", () => {
  const app = functionSource("App");
  assert.match(app, /\{tourOpen \? <OnboardingTour onClose=\{closeTour\} \/> : null\}/);
  assert.match(app, /window\.localStorage\.setItem\(TOUR_STORAGE_KEY, String\(TOUR_VERSION\)\)/);
  assert.match(functionSource("initialTourOpen"), /has\("previewRoute"\)/);
  assert.match(functionSource("GlobalSettingsScreen"), /onClick=\{onOpenTour\}/);
  const model = readFileSync(new URL("./onboarding-model.ts", import.meta.url), "utf8");
  const sidebar = functionSource("AppSidebar");
  for (const match of model.matchAll(/target: "([^"]+)"/g)) {
    const target = match[1];
    const anchored = target.startsWith("nav-") ? sidebar.includes("data-tour={`nav-${item.id}`}") : sidebar.includes(`data-tour="${target}"`) || (target === "settings" && sidebar.includes("data-tour={routeId}"));
    assert.ok(anchored, `引导目标 ${target} 在侧栏里没有锚点`);
  }
});

test("设置页「版本与更新」：已是最新时不显示旧版本的安装包和说明；有新版本时说明收进可展开区", () => {
  const about = functionSource("AboutScreen");
  assert.match(about, /update\?\.updateAvailable && update\.assetName \?/);
  assert.match(about, /\{update\?\.updateAvailable && update\.releaseSummary \? \(\n\s*<details className="app-update-notes">/);
  assert.doesNotMatch(about, /<Metric label=\{t\("状态"\)\}/);
});

test("Computer Use 的页签与会话页等同一种下划线页签（tp-tabs），不再改写 mg-tabs", () => {
  const page = readFileSync(new URL("./ComputerUsePage.tsx", import.meta.url), "utf8");
  assert.match(page, /<div className="tp-tabs cu-tabs" role="tablist"/);
  const css = readFileSync(new URL("./ComputerUsePage.css", import.meta.url), "utf8");
  assert.doesNotMatch(css, /\.mg-tabs\.cu-tabs/);
});

test("供应商头像在任何列表里都不参与伸缩：挑选弹层、导入弹窗都出过被拉成横条的问题", () => {
  const css = readFileSync(new URL("./GatewayPage.css", import.meta.url), "utf8");
  assert.match(css, /span\.gw-avatar\[data-size\] \{\s*flex: none;/);
  assert.doesNotMatch(css, /\.gw-select-item span \{/);
});

test("Claude Code / Grok 的概览是仪表板：接入状态、今日数据、最近会话与请求、配置摘要", () => {
  assert.match(functionSource("OverviewScreen"), /<AgentDashboard agent=\{activeTool\}/);
  const dashboard = readFileSync(new URL("./AgentDashboard.tsx", import.meta.url), "utf8");
  for (const command of ["load_gateway_usage", "load_request_logs", "list_agent_sessions", "load_agent_mcp", "list_installed_skills", "open_agent_terminal"]) {
    assert.ok(dashboard.includes(`"${command}"`), command);
  }
  const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.match(lib, /agent_launch_commands::open_agent_terminal,/);
});

test("网关页的 Agent 图标：Claude Code / Grok 用遮罩按文字颜色绘制，不再把黑色单色图当 <img>", () => {
  for (const name of ["GatewayAgents.tsx", "TrayPanel.tsx", "GatewayPage.tsx"]) {
    const source = readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
    assert.doesNotMatch(source, /src=\{agentIcon\(/, name);
  }
  const css = readFileSync(new URL("./GatewayPage.css", import.meta.url), "utf8");
  assert.match(css, /\.gw-agent-mono \{[^}]*mask: var\(--gw-agent-icon\)/);
});

test("内层滚动的页面（网关各页、Computer Use）滚动区延伸到窗口边，滚动条不压内容", () => {
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  assert.match(css, /\.shell :is\(\.gw-content, \.cu-content\) \{[^}]*width: calc\(100% \+ 20px\);[^}]*margin-right: -20px;[^}]*padding-right: 20px;/);
});

test("会话页可以删除会话，不再有「在终端继续」；Claude Code / Grok 走 delete_agent_session，Codex 走 delete_local_session", () => {
  const sessions = readFileSync(new URL("./GatewaySessions.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(sessions, /在终端继续|resume_agent_session/);
  assert.match(sessions, /"delete_agent_session"/);
  assert.match(sessions, /"delete_local_session"/);
  assert.match(sessions, /appConfirm\(/);
  const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.match(lib, /gateway_commands::delete_agent_session,/);
});
