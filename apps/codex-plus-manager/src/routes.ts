// 管理器的路由表：页面 id、侧栏分组、按模式过滤、页面标题与说明。
import {
  Cable,
  BookOpen,
  GitBranch,
  LifeBuoy,
  Layers3,
  Languages,
  House,
  List,
  Palette,
  PackageOpen,
  FileCode2,
  MousePointerClick,
  Smartphone,
  Network,
  ScanLine,
  Settings,
  Store,
  Trash2,
  AppWindow,
  Server,
  Waypoints,
  Plug,
  Split,
  BarChart3,
  MessagesSquare,
  SlidersHorizontal,
  type LucideIcon,
} from "lucide-react";
import type { GatewayTab } from "./GatewayPage";
import { agentTabGroups, agentTarget } from "./page-groups";
import type { RouteAlias } from "./route-aliases";
import { t } from "@/i18n";

/** 左侧 Agent 切换栏的工具标识。后端 `list_tools` 返回同名字符串。 */
export type ToolId = string;

export type Route = "home" | "relay" | "claudeProviders" | "claudeDesktopProviders" | "multiLaunch" | "claude" | "claudePatch" | "requestLog" | "sessions" | "context" | "skills" | "weixin" | "enhance" | "dreamSkin" | "userScripts" | "claudeExtensions" | "pluginMarket" | "agentCache" | "agentCollaboration" | "computerUse" | "remoteControl" | "hub" | "gateway" | "gatewayProviders" | "gatewayRouting" | "gatewayUsage" | "gatewaySettings" | "settings" | "agentSettings" | "extension";
/** 进来的路由可以是旧 id（见 route-aliases.ts），由 `navigate` 换成现在的页面。 */
export type RouteTarget = Route | RouteAlias;
export const isKnownRoute = (target: string): target is Route => routes.some((item) => item.id === target);

/**
 * 导航项归属。
 *
 * - `"codex"` / `"grok"`：这一页只属于某个工具，切到别的工具时隐藏。
 *   绝大部分功能（会话、MCP、皮肤、脚本市场、安装维护…）都是 Codex 专属的。
 * - `tools`：共享页面的可见 Agent；保留 `tool` 供原有顶栏行为使用。
 * - 不写 `tool` / `tools`：与工具无关的应用级页面（设置、关于），任何工具下都显示。
 *
 * 新增页面时**必须**想清楚归属：默认可见会让 Codex 专属功能在 Grok 下露出来。
 */
export const routes: Array<{ id: Route; label: string; icon: LucideIcon; tool?: string; tools?: readonly string[] }> = [
  // 首页：整个程序的落脚页，专门放赞助商推荐（原「推荐内容」页并进来）。
  { id: "home", get label() { return t("首页"); }, icon: House },
  // 「Codex 连接」：连接方式（经网关 / 直连）+ 直连供应商 + 高级；原「旧版供应商配置」「旧版代理设置」合成这一页。
  { id: "relay", get label() { return t("连接与供应商"); }, icon: Cable, tool: "codex" },
  // Claude Code（命令行）与 Claude Desktop 拆成两个独立 Agent：各自管自己的供应商开关、默认模型与连接方式。
  { id: "claudeProviders", get label() { return t("供应商"); }, icon: Cable, tool: "claude" },
  { id: "claudeDesktopProviders", get label() { return t("供应商"); }, icon: Cable, tool: "claude-desktop" },
  { id: "multiLaunch", get label() { return t("多开"); }, icon: Layers3, tool: "codex" },
  // 增强针对的是 Claude Desktop（汉化、Cowork 时限等），归属 Claude Desktop。
  { id: "claude", get label() { return t("Desktop 增强"); }, icon: Languages, tool: "claude-desktop" },
  // 补丁 Desktop：专门给「改过代码的 Desktop 副本」用（制作/更新/检查/启动增强版副本），
  // 完整功能模式留在「Desktop 增强」页，两页同一个组件按 mode 分渲染。
  { id: "claudePatch", get label() { return t("补丁 Desktop"); }, icon: PackageOpen, tool: "claude-desktop" },
  // Claude Desktop 的「万物皆拓展」：往 Claude 页面加的页面扩展和 Code 会话里的按钮、工具（拓展包里的 claudeDesktop 部分）。
  { id: "claudeExtensions", get label() { return t("万物皆拓展"); }, icon: FileCode2, tool: "claude-desktop" },
  // 请求日志、会话、MCP、Skills 管理所有 Agent，不属于某一个 Agent。
  { id: "sessions", get label() { return t("会话"); }, icon: MessagesSquare },
  { id: "context", label: "MCP", icon: Network },
  { id: "skills", label: "Skills", icon: BookOpen },
  { id: "weixin", get label() { return t("微信连接"); }, icon: ScanLine, tool: "codex" },
  // 启动急救：增强总开关、独立模式补丁、本地拓展总开关、下次启动跳过拓展、恢复默认皮肤都在这一页。
  { id: "enhance", get label() { return t("启动急救"); }, icon: LifeBuoy, tool: "codex" },
  // 旧入口：皮肤急救并进了「启动急救」页（dreamskin:// 确认与深链仍可能用到这条路由）。
  { id: "dreamSkin", get label() { return t("皮肤管理"); }, icon: Palette, tool: "codex" },
  // Codex 的「万物皆拓展」：Codex++ 拓展。不再是侧栏里单独一项，按 Agent 拆开放进各自的页签（Claude Desktop 的是 claudeExtensions）。
  { id: "userScripts", get label() { return t("万物皆拓展"); }, icon: FileCode2, tool: "codex" },
  // Codex 官方插件目录（和 Codex++ 自己的「万物皆拓展」是两回事）。
  { id: "pluginMarket", get label() { return t("Codex 插件"); }, icon: Store, tool: "codex" },
  // 缓存清理覆盖多个 AI 应用，属于应用级页面，在各工具下均可访问。
  { id: "agentCache", get label() { return t("AI Agent 缓存清理"); }, icon: Trash2 },
  // 跨客户端协作属于应用级能力，各工具下共用同一组任务。
  { id: "agentCollaboration", get label() { return t("Agent 协作"); }, icon: GitBranch },
  { id: "computerUse", get label() { return t("Computer Use"); }, icon: MousePointerClick },
  // 远程控制：在浏览器或手机上操控 Codex 的会话。和「微信连接」同是「在手机上用 Codex」，挂在侧栏 Codex 下面、紧跟微信连接。
  { id: "remoteControl", get label() { return t("远程控制"); }, icon: Smartphone, tool: "codex" },
  // 本地服务（原「聚合网关」）：Codex++ 的后台能力，侧栏只有一个「本地服务」入口，
  // 页内两个页签（服务与接入、路由，见 localServiceRoutes）；供应商统一在各 Agent 自己的页面管理，服务设置已并入「服务与接入」。
  // `gatewayProviders` / `gatewaySettings` / `hub` 路由 id 仍保留在表里以兼容拓展与旧深链。
  { id: "hub", get label() { return t("本地服务总览"); }, icon: Waypoints },
  { id: "gatewayProviders", get label() { return t("共享供应商"); }, icon: Server },
  { id: "gateway", get label() { return t("服务与接入"); }, icon: Plug },
  { id: "gatewayRouting", get label() { return t("路由"); }, icon: Split },
  { id: "gatewayUsage", get label() { return t("用量"); }, icon: BarChart3 },
  { id: "gatewaySettings", get label() { return t("设置"); }, icon: SlidersHorizontal },
  // 和侧栏底部齿轮同名；页面标题按选中的分类显示（通用、配置文件…）。
  { id: "settings", get label() { return t("设置"); }, icon: Settings },
  // Agent 专属设置：目前只有 Codex 有专属选项（启动参数等）。
  // Claude / Grok 没有独立设置项，原来那页只有一个跳回各自主页的按钮，所以不再给它们显示。
  { id: "agentSettings", get label() { return t("Codex 设置"); }, icon: Settings, tool: "codex" },
  { id: "requestLog", get label() { return t("请求日志"); }, icon: List },
  // 拓展贡献的管理器页面（codexpp.json 的 manager.nav）：侧栏入口按拓展列出，这里只是它们共用的路由。
  { id: "extension", get label() { return t("拓展页面"); }, icon: AppWindow },
];

/**
 * Agent 的页面分组：侧栏只列 Agent，点开后它的全部页面都在内容区顶部的页签里，分组之间用细线隔开。
 * 每个页面仍按 `routes` 里的 `tool` 归属过滤；只有一个页面的 Agent 不显示页签。
 */
export const navigationSections: Array<{ id: string; routes: Route[] }> = [
  // Agent 不再有概览页：「连接与供应商」排第一，直连模式的老用户每天在这里切供应商，想改用网关也从这里开始。
  // Codex 的启动 / 重启并挂载在侧栏 Codex 行的播放键上，用量在「用量」页，调试连接状态在设置里。
  { id: "providers", routes: ["relay", "claudeProviders", "claudeDesktopProviders"] },
  { id: "enhance", routes: ["claude", "claudePatch", "claudeExtensions", "enhance", "pluginMarket", "userScripts", "weixin", "remoteControl", "multiLaunch"] },
  // Codex 设置（含安装维护）在齿轮的设置模式里，不再占 Agent 的页签。
];
export const routeTool = (routeId: Route) => routes.find((candidate) => candidate.id === routeId)?.tool;
/** 某个 Agent 能看到的全部页面，按页签顺序。 */
export const agentRoutes = (toolId: ToolId) => agentTabGroups(navigationSections, routeTool, toolId).flatMap((entry) => entry.routes);
/** 每个 Agent 上次打开的页面：在侧栏点 Agent 时回到那里。 */
export const lastAgentRoute = new Map<string, Route>();

/**
 * 与具体 Agent 无关的应用级页面：网关、工具、关于、整体设置。打开它们时侧栏里没有 Agent 处于选中态。
 * Computer Use 同时接入 Claude Code / Claude Desktop / DSH 等多个客户端，不属于任何一个 Agent，
 * 所以和 Agent 协作一样放在「工具」里，而不是重复出现在每个 Agent 的「扩展」分组里。
 */
export const globalNavigationRoutes: Route[] = ["home", "extension", "hub", "gatewayProviders", "gateway", "gatewayRouting", "gatewayUsage", "gatewaySettings", "requestLog", "sessions", "context", "skills", "computerUse", "agentCollaboration", "agentCache", "settings"];
/** 本地服务的几页：⌘K 里的顺序（供应商在各 Agent 页管理，服务设置已并入「服务与接入」）。 */
// 「用量」并进全局管理：统计（gatewayUsage）与请求日志（requestLog）是同一页的两个页签。
export const gatewayNavigationRoutes: Route[] = ["gateway", "gatewayRouting"];
/** ⌘K「全局管理」分组：用量、会话、MCP、Skills。万物皆拓展按 Agent 拆开，在各 Agent 的页面里。 */
export const managementNavigationRoutes: Route[] = ["gatewayUsage", "sessions", "context", "skills"];
/** 侧栏里同一个入口的别名路由：请求日志是「用量」页的第二个页签。 */
export const navAliasRoute: Partial<Record<Route, Route>> = { requestLog: "gatewayUsage" };
/** 网关的各个页签由侧栏直接选中，页内不再显示标签条。 */
export const gatewayTabByRoute: Partial<Record<Route, GatewayTab>> = {
  gateway: "agents",
  gatewayProviders: "providers",
  gatewayRouting: "routing",
  gatewayUsage: "usage",
  gatewaySettings: "gateway",
};
export const gatewayRouteByTab = Object.fromEntries(Object.entries(gatewayTabByRoute).map(([route, tab]) => [tab, route])) as Record<GatewayTab, Route>;
/**
 * 每个 Agent 在侧栏里的落脚页：点 Agent 名字先到这里。
 * Claude Code / Grok 的供应商不再在 Codex++ 里单独配置：要么经网关接入共享供应商（「Agent 接入」），
 * 要么不接入、照它自己的配置文件直连。
 */
export const agentHomeRoute: Record<string, Route> = {
  codex: "relay",
  claude: "claudeProviders",
  "claude-desktop": "claudeDesktopProviders",
  grok: "gateway",
};
/**
 * 侧栏 Agent 列表展开：后端 `list_tools` 把 Claude Code 与 Claude Desktop 合在 `claude` 一项下，
 * 侧栏与导航把它们拆成 `claude`（Claude Code）与 `claude-desktop`（Claude Desktop）两个独立 Agent，
 * 对应 `gateway.json` 里本来就分开的两份绑定。
 */
export function expandSidebarTools<T extends { id: string; name: string }>(tools: readonly T[]): T[] {
  const hasDesktop = tools.some((tool) => tool.id === "claude-desktop");
  const result: T[] = [];
  for (const tool of tools) {
    if (tool.id === "claude") {
      result.push({ ...tool, name: "Claude Code" });
      if (!hasDesktop) {
        result.push({ ...tool, id: "claude-desktop", name: "Claude Desktop" });
      }
    } else {
      result.push(tool);
    }
  }
  return result;
}
/**
 * 侧栏 Agent 列表里不显示的工具。Grok 在 Codex++ 里没有专属的增强页面，只经网关接入，
 * 在「Agent 接入」里管理；请求日志、MCP 等跨 Agent 的页面照常支持它。
 */
export const hiddenSidebarTools = new Set<string>(["grok"]);
/** 上次停在被隐藏的工具上时回到 Codex，免得侧栏没有任何 Agent 处于选中态。 */
export const visibleActiveTool = (toolId: string | null | undefined) =>
  toolId && !hiddenSidebarTools.has(toolId) ? toolId : "codex";
export const agentEntryRoute = (toolId: ToolId) => agentTarget(agentRoutes(toolId), lastAgentRoute.get(toolId), agentHomeRoute[toolId] ?? "relay");

/** 进入设置模式的路由：侧栏切成设置目录。 */
export const settingsModeRoutes = new Set<Route>(["settings", "agentSettings", "agentCache", "computerUse", "agentCollaboration"]);
/** 「本地服务」一页的两个页签（服务与接入、路由），侧栏里只有一个「本地服务」入口；旧的总览与设置路由也算这一页。 */
export const localServiceRoutes: Route[] = ["gateway", "gatewayRouting"];
export const isLocalServiceRoute = (route: Route) => route === "hub" || route === "gatewaySettings" || localServiceRoutes.includes(route);
/** 「MCP 与 Skills」一页：两个胶囊切换。万物皆拓展在各 Agent 的页签里，不放在这里。 */
export const customizeRoutes: Route[] = ["context", "skills"];

export function routeTitle(route: Route) {
  return routes.find((item) => item.id === route)?.label ?? t("首页");
}

/** 页面副标题。 */
export function routeSubtitle(route: Route) {
  const subtitles: Record<Route, string> = {
    home: t("赞助商推荐：支持 Codex++ 的服务"),
    relay: t("管理 API 供应商、协议、Key 与配置文件"),
    claudeProviders: t("Claude Code 的连接方式（聚合 / 直连）、供应商开关与默认模型"),
    claudeDesktopProviders: t("Claude Desktop 经本地服务使用的供应商、模型组与默认模型"),
    multiLaunch: t("为不同供应商启动相互隔离的 Codex 实例"),
    claude: t("启动原版 Claude Desktop，选择 3p 或完整功能模式"),
    claudePatch: t("制作、更新与启动改过代码的 Desktop 副本；原件与 app.asar 不动"),
    requestLog: t("所有 Agent 经 Codex++ 的请求：状态、模型、重试、token 用量与缓存命中"),
    sessions: t("Codex、Claude Code、Grok 的本地会话放在一起：在终端继续，Codex 会话还能删除、归档与修复"),
    context: t("同时管理 Codex、Claude Code、Grok 的 MCP 服务器：每列一个 Agent，打开即写入它的配置"),
    skills: t("从 GitHub 仓库安装 Skill，按 Agent 启用到 Codex、Claude Code 或 Grok"),
    weixin: t("通过个人微信连接本机 Codex 会话"),
    enhance: t("Codex 打不开或显示异常时用：增强总开关、独立模式补丁、本地拓展与恢复默认皮肤"),
    dreamSkin: t("Codex 打不开或显示异常时用：增强总开关、独立模式补丁、本地拓展与恢复默认皮肤"),
    userScripts: t("Codex++ 拓展：内置、精选、社区和自己做的拓展"),
    claudeExtensions: t("Claude Desktop 的拓展：往 Claude 页面加的页面扩展，以及 Code 会话里的按钮和工具"),
    pluginMarket: t("Codex 官方插件目录：检索插件，按需下载并安装到 Codex"),
    agentCache: t("扫描 Codex、Claude 与 Codex++ 的已知缓存目录，由你选择清理项目。"),
    agentCollaboration: t("连接 Codex、DeepSeek Harness、Claude Code 与 Google Antigravity，派发任务并查看结果。"),
    computerUse: t("复用 ChatGPT.app 的 Computer Use 与 Chrome 控制，通过 MCP 接给 Claude Code、Claude Desktop、DeepSeek Harness、ZCode、WorkBuddy 与 Devin。"),
    remoteControl: t("在浏览器里查看和操控 Codex 桌面的会话：发消息、停止、审批命令、回答提问"),
    hub: t("任意 Agent 经 Codex++ 接入任意供应商：谁在用、走哪家、是否健康，一屏看全"),
    gateway: t("选择哪些 Agent 经 Codex++ 本地服务接入，以及它们默认用哪个模型"),
    gatewayProviders: t("接入本地服务的 Agent 共用的供应商：多 Key 轮换、出错换下一家、协议自动转换"),
    gatewayRouting: t("多家都有的模型怎么分、你建的组怎么走：路由方式、会话固定、出错换谁"),
    gatewayUsage: t("调用统计与每条请求：按 Agent、供应商与模型汇总 token、缓存与费用"),
    gatewaySettings: t("本地服务的监听地址、Key 轮换方式与退出时的恢复策略"),
    settings: t("主题、配置文件、版本更新与日志诊断，不随 Agent 变化"),
    agentSettings: t("当前 Agent 的专属设置"),
    extension: t("由拓展提供的页面"),
  };
  return subtitles[route];
}
