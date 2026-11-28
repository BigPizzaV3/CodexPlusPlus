// 主界面侧栏与设置模式目录、Agent 页签。
import {
  ArrowLeft,
  BookOpen,
  Download,
  Info,
  ScrollText,
  GitBranch,
  Layers3,
  Languages,
  House,
  List,
  Loader2,
  Palette,
  Play,
  FileCode2,
  Moon,
  MousePointerClick,
  PawPrint,
  Network,
  RotateCcw,
  Search,
  Settings,
  Square,
  Store,
  Stethoscope,
  Sun,
  Trash2,
  Wrench,
  ArrowUp,
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
import { managerNavPages, type ManagerExtensionPage, type ManagerNavGroup } from "./extension-manager-pages";
import { Fragment, useEffect, useRef, useState } from "react";
import { AgentGlyph, useGatewayPayload } from "./hub-shared";
import { paletteShortcut } from "./command-palette-model";
import { agentName } from "./gateway-model";
import { agentTabGroups, pageVisibleForTool } from "./page-groups";
import { getLanguage, t, tf, toggleLanguage } from "@/i18n";
import type { Theme, ToolEntry } from "./app-types";
import { APP_LOGO_URL } from "./app-shared";
import { useClaudeEnhance } from "./claude-enhance";
import { useSideIndicator } from "./side-indicator";
import {
  customizeRoutes,
  globalNavigationRoutes,
  navigationSections,
  type Route,
  routes,
  routeTool,
  settingsModeRoutes,
  localServiceRoutes,
  type ToolId,
} from "./routes";

/**
 * 设置模式（参照 Cursor）：侧栏换成「返回 + 搜索设置 + 平铺分类」，每个分类单独成页。
 * 应用级设置（整体设置页）按分组拆成几个分类，只显示选中的那几张卡片；其余分类各是一条路由。
 */
type SettingsNavEntry = {
  key: string;
  label: string;
  icon: LucideIcon;
  route: Route;
  /** route 为 settings 时只显示这些卡片（卡片 id）。 */
  sections?: string[];
  group: "app" | "agent" | "tools" | "beta";
  keywords?: string;
};

export function settingsNavEntries(codexAvailable: boolean): SettingsNavEntry[] {
  const entries: SettingsNavEntry[] = [
    { key: "general", label: t("通用"), icon: Settings, route: "settings", sections: ["app-settings-general"], group: "app", keywords: "theme mode tour 主题 模式 引导 托盘" },
    { key: "config", label: t("配置文件"), icon: FileCode2, route: "settings", sections: ["app-settings-config"], group: "app", keywords: "backup import export 备份 导入 导出 路径" },
    { key: "doctor", label: t("配置体检"), icon: Stethoscope, route: "settings", sections: ["app-settings-doctor"], group: "app", keywords: "doctor ai config check health 体检 检查 诊断 配置" },
    { key: "update", label: t("版本与更新"), icon: Download, route: "settings", sections: ["app-settings-update"], group: "app", keywords: "update version 更新 版本" },
    { key: "logs", label: t("日志与诊断"), icon: ScrollText, route: "settings", sections: ["app-settings-logs", "app-settings-diagnostics"], group: "app", keywords: "logs diagnostics 日志 诊断" },
    { key: "about", label: t("关于"), icon: Info, route: "settings", sections: ["app-settings-about"], group: "app", keywords: "about github 关于" },
    { key: "codex", label: "Codex", icon: AppWindow, route: "agentSettings", group: "agent", keywords: "codex launch args takeover 启动参数 接管 维护 修复" },
    { key: "cache", label: t("缓存清理"), icon: Trash2, route: "agentCache", group: "tools", keywords: "cache clean 缓存 清理" },
    { key: "computerUse", label: "Computer Use", icon: MousePointerClick, route: "computerUse", group: "beta", keywords: "computer use mcp" },
    { key: "agentCollaboration", label: t("Agent 协作"), icon: GitBranch, route: "agentCollaboration", group: "beta", keywords: "collaboration 协作" },
  ];
  return entries.filter((entry) => entry.key !== "codex" || codexAvailable);
}
/** 设置分类当前是哪一个：整体设置页按显示的卡片判断，其余按路由。 */
export function activeSettingsEntry(entries: SettingsNavEntry[], route: Route, settingsSections: string[]) {
  if (route === "settings") return entries.find((entry) => entry.route === "settings" && entry.sections?.[0] === settingsSections[0])?.key ?? "general";
  return entries.find((entry) => entry.route === route)?.key ?? null;
}

/** 「MCP 与 Skills」页顶部的胶囊。 */
export function CustomizeTabs({ route, activeTool, onNavigate }: { route: Route; activeTool: ToolId; onNavigate: (route: Route) => void }) {
  if (!customizeRoutes.includes(route)) return null;
  const items = customizeRoutes
    .map((routeId) => routes.find((candidate) => candidate.id === routeId))
    .filter((item): item is (typeof routes)[number] => Boolean(item) && (!item!.tools || pageVisibleForTool(item!, activeTool)));
  const label = (routeId: Route) => (routeId === "context" ? "MCP" : "Skills");
  return (
    <nav aria-label={t("MCP 与 Skills")} className="tp-tabs cx-pills" role="tablist">
      {items.map((item) => (
        <button aria-selected={route === item.id} data-active={route === item.id} key={item.id} onClick={() => onNavigate(item.id)} role="tab" type="button">
          {label(item.id)}
        </button>
      ))}
    </nav>
  );
}

/**
 * 唯一的侧栏（参照 Cursor 的布局）。
 *
 * 主模式：
 * - 顶部是 ⌘K 搜索；其后几个常用入口：总览、供应商、Agent 接入、用量、自定义（拓展 / MCP / Skills）、会话；
 * - 「Agent」：每个 Agent 一行（状态点表示接入方式），它自己的页面在内容区顶部的胶囊里；
 * - 底部是网关状态卡和「Codex++ 版本 · 齿轮」。
 *
 * 设置模式（停在设置类页面时）：「返回」+ 搜索设置 + 平铺的设置分类，语言与主题在底部。
 */
export function AppSidebar({
  tools,
  activeTool,
  route,
  theme,
  version,
  onOpenAgent,
  onNavigate,
  onToggleTheme,
  onOpenPalette,
  pendingUpdateVersion,
  pendingUpdateReady = false,
  pendingUpdateProgress = null,
  onOpenUpdate,
  codexRun,
  claudeRun,
  extensionPages = [],
  activeExtensionKey = null,
  onOpenExtensionPage,
  settingsSections = ["app-settings-general"],
  onOpenSettings,
  onExitSettings,
  setupSlotRef,
}: {
  /** 启动引导的进度卡（没完成、没关掉时）由引导组件投到这个位置：侧栏底部。 */
  setupSlotRef?: (element: HTMLDivElement | null) => void;
  /** 拓展贡献的管理器页面：声明了 manager.nav 的放进对应分组。 */
  extensionPages?: ManagerExtensionPage[];
  activeExtensionKey?: string | null;
  onOpenExtensionPage?: (key: string) => void;
  /** Codex 行的播放 / 停止：只有桌面应用的 Codex 有，命令行 Agent 没有常驻进程可启停。
      unavailableReason 非空（如没找到 Codex 应用）时播放键置灰，悬停显示原因。 */
  codexRun: { running: boolean; unattached: boolean; busy: boolean; unavailableReason: string | null; onStart: () => void; onStop: () => void; onRestart: () => void };
  /** Claude 行同样有播放 / 停止：Claude Desktop 本地模式可以在这里启动、停止本地服务。
      unavailableReason 非空（没装 Claude、Claude 正开着、缺 Node.js）时置灰，悬停显示原因。 */
  claudeRun: { running: boolean; busy: boolean; unavailableReason: string | null; onStart: () => void; onStop: () => void };
  tools: ToolEntry[];
  activeTool: ToolId;
  route: Route;
  theme: Theme;
  version: string;
  /** 切到某个 Agent 并打开 `destination`（缺省为它的落脚页）。 */
  onOpenAgent: (toolId: ToolId, destination?: Route) => void;
  onNavigate: (route: Route) => void;
  onToggleTheme: () => void;
  onOpenPalette: () => void;
  /** 后台检查到的新版本号；没有新版本时为 null。 */
  pendingUpdateVersion: string | null;
  /** 新版本的安装包已经在后台下好：卡片直接写「点击安装」。 */
  pendingUpdateReady?: boolean;
  /** 后台正在下载安装包：0–100，进度未知时为 -1；没在下载为 null。 */
  pendingUpdateProgress?: number | null;
  onOpenUpdate: () => void;
  /** 整体设置页当前显示的卡片。 */
  settingsSections?: string[];
  /** 打开某个设置分类（整体设置页的分类带卡片 id）。 */
  onOpenSettings?: (route: Route, sections?: string[]) => void;
  /** 设置模式左上角「返回」：回到进设置前的页面。 */
  onExitSettings?: () => void;
}) {
  const gateway = useGatewayPayload(10_000);
  // 增强生效方式决定 Claude 下显示哪个入口、播放键启动哪个：补丁副本时「补丁 Desktop」才出现在侧栏。
  const claudeDelivery = useClaudeEnhance()?.delivery ?? null;
  const [settingsQuery, setSettingsQuery] = useState("");
  // 收起了页面列表的 Agent：再点一次当前 Agent 收起 / 展开它下面的页面（#2913）。切到别的 Agent 时清掉。
  const [collapsedAgent, setCollapsedAgent] = useState<string | null>(null);
  const settingsMode = settingsModeRoutes.has(route);
  const onGlobalPage = globalNavigationRoutes.includes(route);
  const languageLabel = getLanguage() === "en" ? t("切换到中文") : t("切换到英文");
  const themeLabel = theme === "dark" ? t("切换到浅色") : t("切换到深色");
  const connectedAgents = new Set((gateway?.agents ?? []).filter((agent) => agent.enabled).map((agent) => agent.agent as string));
  // 供应商日常在 Codex 里管：没有 Agent 接入网关时，网关一组和接入方式的状态点都不显示。
  // 网关数据要等第一次读回来：在那之前用上次记下的结果，免得网关用户每次启动都看到这一组晚一拍才冒出来。
  const gatewayInUse = gateway ? connectedAgents.size > 0 : loadCachedGatewayInUse();
  useEffect(() => {
    if (gateway) storeCachedGatewayInUse(gatewayInUse);
  }, [gateway, gatewayInUse]);
  const openSettings = (entryRoute: Route, sections?: string[]) => (onOpenSettings ? onOpenSettings(entryRoute, sections) : onNavigate(entryRoute));
  // 选中底色是一块在条目之间滑动的色块：选中项或展开状态变了就滑过去。
  const navRef = useRef<HTMLElement>(null);
  useSideIndicator(navRef, [settingsMode, route, settingsSections.join(","), activeTool, collapsedAgent ?? "", activeExtensionKey ?? ""].join("|"));

  if (settingsMode) {
    const codexAvailable = tools.some((tool) => tool.id === "codex" && tool.switchable);
    const entries = settingsNavEntries(codexAvailable);
    const activeKey = activeSettingsEntry(entries, route, settingsSections);
    const query = settingsQuery.trim().toLowerCase();
    const shown = query ? entries.filter((entry) => `${entry.label} ${entry.keywords ?? ""}`.toLowerCase().includes(query)) : entries;
    const groups: Array<{ id: SettingsNavEntry["group"]; label?: string }> = [{ id: "app" }, { id: "agent" }, { id: "tools" }, { id: "beta", label: "Beta" }];
    return (
      <aside className="side cx-side" data-mode="settings">
        <div aria-hidden="true" className="cx-titlebar-drag" data-tauri-drag-region />
        <button className="cx-side-back" onClick={onExitSettings} type="button">
          <ArrowLeft aria-hidden="true" />
          <span>{t("返回")}</span>
        </button>
        <label className="cx-side-search">
          <Search aria-hidden="true" />
          <input
            aria-label={t("搜索设置")}
            onChange={(event) => setSettingsQuery(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && shown[0]) openSettings(shown[0].route, shown[0].sections);
              if (event.key === "Escape") setSettingsQuery("");
            }}
            placeholder={t("搜索设置")}
            value={settingsQuery}
          />
        </label>
        <nav aria-label={t("设置分类")} className="cx-side-nav" data-tour="settings" ref={navRef}>
          <i aria-hidden="true" className="cx-side-indicator" />
          {groups.map((group) => {
            const items = shown.filter((entry) => entry.group === group.id);
            if (!items.length) return null;
            return (
              <div className="cx-side-group" key={group.id}>
                {group.label ? <div className="cx-side-heading">{group.label}</div> : null}
                {items.map((entry) => {
                  const Icon = entry.icon;
                  const selected = entry.key === activeKey;
                  return (
                    <button
                      aria-current={selected ? "page" : undefined}
                      className={`cx-side-item ${selected ? "active" : ""}`}
                      data-settings-entry={entry.key}
                      key={entry.key}
                      onClick={() => openSettings(entry.route, entry.sections)}
                      type="button"
                    >
                      {entry.key === "codex" ? <AgentGlyph agent="codex" className="cx-side-icon" /> : <Icon aria-hidden="true" className="cx-side-icon" />}
                      <span className="cx-side-label">{entry.label}</span>
                      {/* 有新版本：「版本与更新」后面一个点，进设置也看得到。 */}
                      {entry.key === "update" && pendingUpdateVersion ? <i aria-label={tf("新版本 {0}", [pendingUpdateVersion])} className="side-dot" data-tone="ok" title={tf("新版本 {0}", [pendingUpdateVersion])} /> : null}
                    </button>
                  );
                })}
              </div>
            );
          })}
          {!shown.length ? <p className="cx-side-empty">{t("没有匹配的设置")}</p> : null}
        </nav>
        <footer className="cx-side-foot">
          <span className="cx-side-version">{version ? `Codex++ ${version}` : "Codex++"}</span>
          <button aria-label={languageLabel} className="cx-icon-button" onClick={() => void toggleLanguage()} title={languageLabel} type="button">
            <Languages aria-hidden="true" />
          </button>
          <button aria-label={themeLabel} className="cx-icon-button" onClick={onToggleTheme} title={themeLabel} type="button">
            {theme === "dark" ? <Sun aria-hidden="true" /> : <Moon aria-hidden="true" />}
          </button>
        </footer>
      </aside>
    );
  }

  // 主模式的入口：每项对应一条路由，match 里的路由都算选中（用量的两个页签、MCP 与 Skills 的两个胶囊）。
  // 第一组是首页（赞助商推荐）、MCP 与 Skills、会话、用量、本地服务。万物皆拓展按 Agent 拆开，在 Codex / Claude Desktop 各自的页签里。
  type SideEntry = { key: string; route: Route; label: string; icon: LucideIcon; match: Route[]; tour?: string };
  // 用量也记直连代理的请求，不算网关专属。
  const general: SideEntry[] = [
    { key: "home", route: "home", label: t("首页"), icon: House, match: ["home"] },
    { key: "customize", route: "context", label: t("MCP 与 Skills"), icon: Network, match: customizeRoutes, tour: "management" },
    { key: "sessions", route: "sessions", label: t("会话"), icon: MessagesSquare, match: ["sessions"] },
    { key: "usage", route: "gatewayUsage", label: t("用量"), icon: BarChart3, match: ["gatewayUsage", "requestLog"] },
    // 本地服务（原「聚合网关」一整组）只占一个入口，页内两个页签（服务与接入、路由）。
    { key: "localService", route: "gateway", label: t("本地服务"), icon: Waypoints, match: ["hub", "gatewaySettings", ...localServiceRoutes] },
  ];
  // 本地服务只占第一组里一个入口；声明在 gateway 分组的拓展页面并进其余拓展页面那一节。
  // `gateway` 是已发布的清单分组名，照旧认。
  const extensionItems = (["gateway", "management", "tools"] as ManagerNavGroup[]).flatMap((group) => managerNavPages(extensionPages, group));
  const renderEntry = (item: SideEntry) => {
    const Icon = item.icon;
    const selected = item.match.includes(route);
    return (
      <button
        aria-current={selected ? "page" : undefined}
        className={`cx-side-item ${selected ? "active" : ""}`}
        data-nav={item.key}
        data-tour={item.tour}
        key={item.key}
        onClick={() => onNavigate(item.route)}
        title={item.label}
        type="button"
      >
        <Icon aria-hidden="true" className="cx-side-icon" />
        <span className="cx-side-label">{item.label}</span>
      </button>
    );
  };
  const renderExtensionPage = (page: ManagerExtensionPage) => {
    const Icon = managerPageIcon(page.icon);
    const selected = activeExtensionKey === page.key;
    return (
      <button
        aria-current={selected ? "page" : undefined}
        className={`cx-side-item ${selected ? "active" : ""}`}
        data-extension-page={page.key}
        key={page.key}
        onClick={() => onOpenExtensionPage?.(page.key)}
        title={page.title}
        type="button"
      >
        <Icon aria-hidden="true" className="cx-side-icon" />
        <span className="cx-side-label">{page.title}</span>
      </button>
    );
  };

  return (
    <aside className="side cx-side" data-mode="main">
      <div aria-hidden="true" className="cx-titlebar-drag" data-tauri-drag-region />
      <button className="cx-side-search cx-side-search-button" data-tour="palette" onClick={onOpenPalette} type="button">
        <Search aria-hidden="true" />
        <span>{t("搜索页面和操作")}</span>
        <kbd>{paletteShortcut}</kbd>
      </button>

      <nav aria-label={t("主导航")} className="cx-side-nav" ref={navRef}>
        <i aria-hidden="true" className="cx-side-indicator" />
        <div className="cx-side-group">{general.map(renderEntry)}</div>

        <div className="cx-side-group" data-tour="agents">
          <div className="cx-side-heading">
            <span>{t("Agent")}</span>
          </div>
          {/* 每个 Agent 一行；点一下切到它，回到上次看的页面（没有就是落脚页）。它的页面在内容区顶部的胶囊里。 */}
          {tools.map((tool) => {
            const current = tool.id === activeTool && !onGlobalPage;
            const expanded = current && collapsedAgent !== tool.id;
            // Claude Code（命令行）与 Claude Desktop 各自独立为一行 Agent。
            const displayName = tool.id === "claude" ? "Claude Code" : tool.id === "claude-desktop" ? "Claude Desktop" : tool.name;
            const title = tool.switchable
              ? tf("{0}｜{1}｜{2} 个供应商", [displayName, tool.homeDir || t("未配置目录"), tool.relayCount])
              : tf("{0}｜{1}｜供应商配置尚未接入", [displayName, tool.homeDir || t("未配置目录")]);
            // 状态点表示接入方式：接入网关为绿色；Codex 没接入网关时是直连，由 Codex++ 直接管理供应商配置。
            const access = connectedAgents.has(tool.id)
              ? { tone: "ok", label: t("已经通过 Codex++ 本地服务接入") }
              : tool.id === "codex"
                ? { tone: "direct", label: t("Codex 的供应商配置由 Codex++ 直接管理") }
                : { tone: "off", label: t("未接入本地服务") };
            return (
              <Fragment key={tool.id}>
              <div className={`cx-side-agent ${(tool.id === "codex" && tool.switchable) || tool.id === "claude-desktop" ? "has-run" : ""}`.trim()}>
                <button
                  aria-current={current ? "true" : undefined}
                  aria-expanded={current ? expanded : undefined}
                  aria-label={tool.switchable ? `${displayName} · ${access.label}` : `${displayName} · ${t("待接入")}`}
                  className={`cx-side-item ${current ? "active" : ""} ${expanded && agentTabGroups(navigationSections, routeTool, tool.id).flatMap((entry) => entry.routes).length > 1 ? "has-subnav" : ""}`.trim()}
                  disabled={!tool.switchable}
                  onClick={() => {
                    // 再点当前 Agent：只收起 / 展开它的页面列表，不跳页；点别的 Agent 照常切过去并展开。
                    if (current) {
                      setCollapsedAgent((value) => (value === tool.id ? null : tool.id));
                      return;
                    }
                    setCollapsedAgent(null);
                    onOpenAgent(tool.id);
                  }}
                  title={`${title}｜${access.label}`}
                  type="button"
                >
                  <AgentGlyph agent={tool.id} className="cx-side-icon" />
                  <span className="cx-side-label">{displayName}</span>
                  {!gatewayInUse ? null : <i aria-hidden="true" className="side-dot" data-tone={access.tone} />}
                </button>
                {tool.id === "codex" && tool.switchable ? (() => {
                  // 外部启动、没挂载增强脚本的 Codex：按钮直接变成「重启并挂载」，一步到位。
                  const unattached = codexRun.running && codexRun.unattached;
                  const label = unattached ? t("重启并挂载") : codexRun.running ? t("停止 Codex") : t("启动 Codex");
                  return (
                    <button
                      aria-label={label}
                      className="cx-side-run"
                      data-running={codexRun.running}
                      data-unattached={unattached}
                      data-unavailable={!codexRun.running && Boolean(codexRun.unavailableReason)}
                      disabled={codexRun.busy || (!codexRun.running && Boolean(codexRun.unavailableReason))}
                      onClick={unattached ? codexRun.onRestart : codexRun.running ? codexRun.onStop : codexRun.onStart}
                      title={codexRun.busy ? t("处理中…") : unattached ? t("Codex 不是经 Codex++ 启动的，增强脚本未挂载 · 点此重启并挂载") : codexRun.running ? t("停止 Codex") : codexRun.unavailableReason ?? t("启动 Codex")}
                      type="button"
                    >
                      {codexRun.busy ? <Loader2 aria-hidden="true" className="animate-spin" /> : unattached ? <RotateCcw aria-hidden="true" /> : codexRun.running ? <Square aria-hidden="true" /> : <Play aria-hidden="true" />}
                    </button>
                  );
                })() : null}
                {tool.id === "claude-desktop" && claudeDelivery === "copy" && !claudeRun.running ? (
                  // 启动模式是补丁副本：播放键去 Desktop 增强页启动补丁版（要先检测、可能要制作副本），不再启动原版。
                  <button
                    aria-label={t("启动补丁版")}
                    className="cx-side-run"
                    onClick={() => onOpenAgent("claude-desktop", "claude")}
                    title={t("到 Desktop 增强页启动补丁版")}
                    type="button"
                  >
                    <Play aria-hidden="true" />
                  </button>
                ) : tool.id === "claude-desktop" ? (() => {
                  // Claude Desktop 本地模式：和 Codex 行同一套播放键，启动 / 停止本地服务。
                  const label = claudeRun.running ? t("停止本地服务") : t("启动 Claude Desktop");
                  return (
                    <button
                      aria-label={label}
                      className="cx-side-run"
                      data-running={claudeRun.running}
                      data-unavailable={!claudeRun.running && Boolean(claudeRun.unavailableReason)}
                      disabled={claudeRun.busy || (!claudeRun.running && Boolean(claudeRun.unavailableReason))}
                      onClick={claudeRun.running ? claudeRun.onStop : claudeRun.onStart}
                      title={claudeRun.busy ? t("处理中…") : claudeRun.running ? t("停止本地服务") : claudeRun.unavailableReason ?? t("启动 Claude Desktop")}
                      type="button"
                    >
                      {claudeRun.busy ? <Loader2 aria-hidden="true" className="animate-spin" /> : claudeRun.running ? <Square aria-hidden="true" /> : <Play aria-hidden="true" />}
                    </button>
                  );
                })() : null}
              </div>
              {/* 当前 Agent 的页面：缩进列在它那一行下面（参照 Cursor 侧栏里仓库下面的会话列表），不再占内容区顶部一排胶囊。
                  每个 Agent 的列表都一直挂着，收起的那些高度为 0：切 Agent、收起 / 展开时才有高度过渡，不是瞬间出现 / 消失。 */}
{(() => {
            const pages = agentTabGroups(navigationSections, routeTool, tool.id)
              .flatMap((entry) => entry.routes)
              // 「补丁 Desktop」的内容在启动模式选补丁副本时直接显示在 Desktop 增强页，侧栏不再单列（⌘K 仍能进；停在那页时照常显示）。
              .filter((routeId) => routeId !== "claudePatch" || route === "claudePatch");
            if (pages.length < 2) return null;
            return (
              <div className="cx-side-subnav-wrap" data-open={expanded} inert={!expanded}>
              <div className="cx-side-subnav-clip">
              <div className="cx-side-subnav" role="group" aria-label={t("Agent 页面")}>
                {pages.map((routeId) => {
                  const item = routes.find((candidate) => candidate.id === routeId);
                  if (!item) return null;
                  const selected = route === routeId;
                  return (
                    <button
                      aria-current={selected ? "page" : undefined}
                      className={`cx-side-subitem ${selected ? "active" : ""}`}
                      data-agent-page={routeId}
                      key={routeId}
                      onClick={() => onNavigate(routeId)}
                      type="button"
                    >
                      {item.label}
                    </button>
                  );
                })}
              </div>
              </div>
              </div>
            );
})()}
              </Fragment>
            );
          })}
        </div>

        {extensionItems.length ? (
          <div className="cx-side-group">
            <div className="cx-side-heading"><span>{t("拓展页面")}</span></div>
            {extensionItems.map(renderExtensionPage)}
          </div>
        ) : null}
      </nav>

      {/* 底部状态卡（对应 Cursor 的 Getting Started）：启动引导没完成时是它的进度卡；有新版本时是更新提示。
          网关状态不再单独占一张卡：有 Agent 接入时侧栏已经有「本地服务」一组，它的「总览」就是同一个入口。 */}
      {setupSlotRef ? <div className="cx-side-setup" ref={setupSlotRef} /> : null}
      {pendingUpdateVersion ? (
        <button className="cx-side-card" onClick={onOpenUpdate} type="button">
          <span className="cx-side-card-title"><ArrowUp aria-hidden="true" />{pendingUpdateReady ? tf("新版本 {0} 已下载", [pendingUpdateVersion]) : pendingUpdateProgress !== null ? tf("正在下载新版本 {0}", [pendingUpdateVersion]) : tf("新版本 {0}", [pendingUpdateVersion])}</span>
          <span className="cx-side-card-action">
            {pendingUpdateReady ? t("点击安装") : pendingUpdateProgress !== null ? (pendingUpdateProgress >= 0 ? `${Math.round(pendingUpdateProgress)}%` : t("下载中…")) : t("查看更新")}
          </span>
        </button>
      ) : null}
      <footer className="cx-side-foot">
        <img alt="" className="cx-side-logo" draggable={false} src={APP_LOGO_URL} />
        <span className="cx-side-version">{version ? `Codex++ ${version}` : "Codex++"}</span>
        <button
          aria-label={t("设置")}
          className="cx-icon-button"
          data-tour="settings"
          onClick={() => openSettings("settings", ["app-settings-general"])}
          title={t("设置")}
          type="button"
        >
          <Settings aria-hidden="true" />
        </button>
      </footer>
    </aside>
  );
}

/** 上次读到的「有没有 Agent 经网关接入」，侧栏在网关数据读回来之前先按它显示。 */
const GATEWAY_IN_USE_STORAGE_KEY = "codex-plus-gateway-in-use";
function loadCachedGatewayInUse(): boolean {
  try {
    return window.localStorage.getItem(GATEWAY_IN_USE_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}
function storeCachedGatewayInUse(inUse: boolean) {
  try {
    if (loadCachedGatewayInUse() !== inUse) window.localStorage.setItem(GATEWAY_IN_USE_STORAGE_KEY, inUse ? "1" : "0");
  } catch {
    // 存不了只影响下次启动时那一拍。
  }
}

/** 拓展页面的侧栏图标（清单 manager.icon）：只认这几个名字，其余用默认图标。 */
const MANAGER_PAGE_ICONS: Record<string, LucideIcon> = {
  "app-window": AppWindow, "bar-chart": BarChart3, list: List, server: Server, store: Store, palette: Palette,
  network: Network, book: BookOpen, waypoints: Waypoints, plug: Plug, split: Split, messages: MessagesSquare,
  paw: PawPrint, "git-branch": GitBranch, wrench: Wrench, settings: Settings, code: FileCode2, layers: Layers3,
};
function managerPageIcon(name: string | null): LucideIcon {
  return (name && MANAGER_PAGE_ICONS[name]) || AppWindow;
}
