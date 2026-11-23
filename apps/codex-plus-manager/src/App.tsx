import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { open, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ArrowRight, Languages, Moon, Rocket, Sparkles, Sun, AppWindow, X } from "lucide-react";
import { setSponsorAds } from "./sponsor-store";
import { managerExtensionPages, managerPageOverrides, type ManagerExtensionPage } from "./extension-manager-pages";
import { Suspense, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Badge as UiBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { mergeRotatedAuthContents, rebaseSettingsEdits } from "./settings-rebase";
import { contextEntriesByKind, dedupeContextEntryList, type CodexContextEntries, type CodexContextEntry, type ContextKind } from "./context-config-text";
import { MAIN_WINDOW_READY_EVENT, parseTrayRequest, TRAY_REQUEST_EVENT, TRAY_RESULT_EVENT, type TrayRequest, type TrayResult } from "./tray-panel-model";
import { AppDialogHost, appConfirm } from "./app-dialog";
import { publishGatewayPayload } from "./gateway-store";
import { CommandPalette, type PaletteCommand } from "./CommandPalette";
import { SessionsView, sessionDeleteSucceeded } from "./AgentSessions";
import { draftFromBinding, bindingFromDraft, validateDraft, type GatewayPayload, type GatewayResult } from "./gateway-model";
import { SessionAgentTabs, type SessionTab } from "./SessionAgentTabs";
import { UsageTabs, onOpenRequestLog, type RequestLogFocus } from "./UsageTabs";
import { LocalServiceTabs } from "./LocalServiceTabs";
import { agentTabGroups, pageVisibleForTool } from "./page-groups";
import { keepIfSame, usePolling } from "./use-polling";
import { useStableCallbacks } from "./use-stable-callbacks";
import { isRouteAlias, resolveRouteAlias } from "./route-aliases";
import { CodexLaunchButton, type CodexLaunchState } from "./CodexLaunchButton";
import { OnboardingSetup } from "./OnboardingSetup";
import { hasCodexEnhancements } from "./onboarding-setup-model";
import { markSetupSeen, resolveSetupAutoOpen, saveSetupState, setupAutoOpenPossible, setupCardNeeded } from "./onboarding-setup-store";
import { AgentDisconnectDialog } from "./AgentDisconnectDialog";
import { TOUR_STORAGE_KEY, TOUR_VERSION } from "./onboarding-model";
import "./ToolPages.css";
import { BACKEND_SETTINGS_CHANGED_EVENT } from "./settings-events";
import type { ExtensionHost } from "./extension-host-model";
import { UPDATE_CHECK_INTERVAL_MS, UpdateNotifications, updatePreferencesFrom } from "./update-notification";
import { McpPage } from "./ManagementPages";
import { ProjectSessionPage } from "./ProjectSessionPage";
import type { LocalSessionFilter } from "./management-model";
import { managerNavigationDestination, type ManagerNavigationIntent } from "./enhancement-navigation";
import { shouldBackfillRelayProfileBeforeSwitch } from "./relay-live-files";
import {
  providerSyncStreamPercent,
  resolveProviderSyncCompletion,
  type ProviderSyncStreamProgress,
} from "./provider-sync-flow";
import { preferredProviderSyncTarget } from "./provider-sync-target";
import { resolveLaunchStatus, launchCompletionNotice } from "./launch-status";
import type { DreamSkinCommunityResult, DreamSkinRuntimeResult } from "./dream-skin";
import { subscribeLanguage, getLanguage, t, tf, toggleLanguage } from "@/i18n";
import { relayProfileSwitchValidation, isAggregateRelayProfile, activeRelayProfile, configureRelayProfileEditing, syncLegacyRelayFields } from "./relay-profile-editing";
import {
  applyResolvedTheme,
  loadThemePreference,
  resolveTheme,
  storeThemePreference,
  systemPrefersDark,
  toggledThemePreference,
  watchSystemTheme,
  type ThemePreference,
} from "./theme-mode";
import {
  multiLaunchInstanceFailure,
  multiLaunchInstanceIsMain,
  multiLaunchInstanceLabel as formatMultiLaunchInstanceLabel,
  multiLaunchInstanceRunning,
  multiLaunchInstanceVisible,
  type MultiLaunchInstance,
} from "./multi-launch-model";
import type {
  Actions,
  AdsResult,
  AutoSaveOutcome,
  AutoTakeoverResult,
  BackendSettings,
  CcsProvidersResult,
  ClaudeLocalStatus,
  CommandResult,
  DiagnosticsResult,
  EnvConflict,
  EnvConflictsResult,
  ExtractRelayCommonConfigResult,
  LiveConflictResolution,
  LiveContextEntriesResult,
  LocalSession,
  LocalSessionsResult,
  LogsResult,
  McpImportPreviewResult,
  MultiLaunchInstancesResult,
  NoticeAction,
  NoticeState,
  OverviewResult,
  ProviderDoctorResult,
  ProviderImportRequest,
  ProviderSyncPayload,
  ProviderSyncProgress,
  ProviderSyncTargetsResult,
  RelayEnvironmentResult,
  RelayFilesResult,
  RelayProfile,
  ScriptMarketResult,
  SessionIndexCleanupCandidate,
  SessionIndexRepairReport,
  SettingsPayload,
  SettingsResult,
  Status,
  TaskProgress,
  Theme,
  ToolEntry,
  UpdateDownloadState,
  UpdateResult,
  UserScriptInventory,
  WatcherResult,
  WeixinConnectStatusResult,
  WeixinQrResult,
} from "./app-types";
import {
  claudeLocalizationMemo,
  claudeLocalMemo,
  contextEntriesFromSettings,
  contextSelectionForAllEntries,
  defaultSettings,
  isSuccessStatus,
  normalizeSettings,
  SettingsAutoSave,
  statusClass,
  statusLabel,
  stringifyError,
} from "./app-shared";
import {
  agentEntryRoute,
  agentHomeRoute,
  agentRoutes,
  customizeRoutes,
  gatewayNavigationRoutes,
  gatewayRouteByTab,
  gatewayTabByRoute,
  globalNavigationRoutes,
  hiddenSidebarTools,
  isKnownRoute,
  lastAgentRoute,
  managementNavigationRoutes,
  navAliasRoute,
  navigationSections,
  type Route,
  routes,
  routeSubtitle,
  type RouteTarget,
  routeTitle,
  routeTool,
  settingsModeRoutes,
  isLocalServiceRoute,
  type ToolId,
  visibleActiveTool,
} from "./routes";
import { CodexMultiLaunchScreen } from "./MultiLaunchScreen";
import { SessionRepairPanel } from "./SessionRepairPanel";
import { HomeScreen } from "./HomeScreen";
import { OverviewScreen } from "./OverviewScreen";
import {
  DreamSkinCommunityLinkDialog,
  LiveConflictDialog,
  NoticeDialog,
  PendingProviderImportDialog,
  SessionIndexCleanupDialog,
  UpdateDialog,
} from "./AppDialogs";
import { LegacyRelayGatewayHint, ProxySettingsScreen, RelayScreen } from "./RelayScreen";
import { activeSettingsEntry, AppSidebar, CustomizeTabs, settingsNavEntries } from "./AppSidebar";
import { lazyNamed, PageFallback } from "./lazy-page";

// 不常用的页面按需加载（首页、侧栏、Codex 连接 / 多开 / 会话 / MCP 等常用页仍随主界面一起加载）。
const GatewayPage = lazyNamed(() => import("./GatewayPage"), "GatewayPage");
const ClaudeProviders = lazyNamed(() => import("./ClaudeProviders"), "ClaudeProviders");
const HubOverview = lazyNamed(() => import("./HubOverview"), "HubOverview");
const SkillsPage = lazyNamed(() => import("./SkillsPage"), "SkillsPage");
const ComputerUsePage = lazyNamed(() => import("./ComputerUsePage"), "ComputerUsePage");
const AgentCollaborationPage = lazyNamed(() => import("./AgentCollaborationPage"), "AgentCollaborationPage");
const PluginMarketScreen = lazyNamed(() => import("./PluginMarketScreen"), "PluginMarketScreen");
const RemoteControlPage = lazyNamed(() => import("./RemoteControlPage"), "RemoteControlPage");
const ClaudeLocalizationScreen = lazyNamed(() => import("./ClaudeDesktopPage"), "ClaudeLocalizationScreen");
const WeixinConnectScreen = lazyNamed(() => import("./WeixinPage"), "WeixinConnectScreen");
const UserScriptsScreen = lazyNamed(() => import("./ExtensionsPage"), "UserScriptsScreen");
const EnhanceScreen = lazyNamed(() => import("./ExtensionsPage"), "EnhanceScreen");
const ExtensionManagerPanel = lazyNamed(() => import("./ExtensionManagerPanel"), "ExtensionManagerPanel");
const GlobalSettingsScreen = lazyNamed(() => import("./SettingsScreens"), "GlobalSettingsScreen");
const AboutScreen = lazyNamed(() => import("./SettingsScreens"), "AboutScreen");
const AppSettingsFocus = lazyNamed(() => import("./SettingsScreens"), "AppSettingsFocus");
const MaintenanceScreen = lazyNamed(() => import("./SettingsScreens"), "MaintenanceScreen");
const AgentSettingsScreen = lazyNamed(() => import("./SettingsScreens"), "AgentSettingsScreen");
const SharedServiceSettings = lazyNamed(() => import("./SettingsScreens"), "SharedServiceSettings");
const AgentCachePanel = lazyNamed(() => import("./agent-cache"), "AgentCachePanel");
const OnboardingTour = lazyNamed(() => import("./OnboardingTour"), "OnboardingTour");
const RequestLogScreen = lazyNamed(() => import("./RequestLogScreen"), "RequestLogScreen");

/// 多开启动后轮询「就绪/失败」结论的节奏：worker 最长等 90 秒就绪，这里给到 2 分钟。
const MULTI_LAUNCH_POLL_INTERVAL_MS = 3000;
const MULTI_LAUNCH_POLL_TIMEOUT_MS = 120_000;

type PendingDreamSkinCommunityResult = CommandResult<{ versionId: string }>;

/// activate_installed_dream_skin_theme（dream_skin_bridge.rs）的结果：需要重启 Codex 才完整切换时 savedForNextLaunch 为 true。
type DreamSkinInstalledThemeActivationResult = CommandResult<{ savedForNextLaunch?: boolean }>;

type LaunchCommandResult = CommandResult<{
  launchStartedAtMs?: number;
  nativeBrowserRestoreFailed?: boolean;
}>;

type MultiLaunchResult = CommandResult<{
  /** 实例已在运行、本次没有重新写入供应商配置（需要「重启」才生效）。 */
  alreadyRunning?: boolean;
  instanceId?: string;
  profileId?: string;
  pid?: number;
  debugPort?: number;
  helperPort?: number;
  codexHome?: string;
  sqliteHome?: string;
  userDataDir?: string;
  appPath?: string;
}>;

type ToolsResult = {
  status: string;
  message: string;
  tools: ToolEntry[];
  activeTool: string;
};

/** apply_context_entry：saved 是保存后的设置，liveEntries 是同步后 live config.toml 里的条目（失败的那一步之后为 null）。 */
type AppliedContextEntryResult = CommandResult<{
  saved: SettingsPayload | null;
  liveEntries: CodexContextEntries | null;
}>;

type DesktopCodexCliResult = CommandResult<{
  path: string | null;
}>;

type RelayResult = CommandResult<{
  authenticated: boolean;
  authSource: string;
  accountLabel: string | null;
  configPath: string;
  configured: boolean;
  requiresOpenaiAuth: boolean;
  hasBearerToken: boolean;
  backupPath: string | null;
}>;

type RelayPayload = Omit<RelayResult, "status" | "message">;

type DeleteLocalSessionsResult = CommandResult<{
  results: Array<{ sessionId: string; title: string; deleted: boolean; message: string }>;
  succeeded: number;
  failed: number;
  undoToken: string | null;
}>;

type DeleteLocalSessionResult = CommandResult<{
  status: string;
  session_id: string;
  message: string;
  undo_token: string | null;
  backup_path: string | null;
}>;

type ContextEntriesResult = CommandResult<{
  settings: BackendSettings;
  entries: CodexContextEntries;
}>;

type RelaySwitchResult = CommandResult<{
  settings: BackendSettings;
  settingsPath: string;
  user_scripts: unknown;
  relay: RelayPayload;
  /** 实时配置 endpoint 被外部改过而停下时才有（#2666）。 */
  liveConflict?: { profileId: string; profileName: string };
}>;

type SettingsBackfillResult = CommandResult<{
  settings: BackendSettings;
  liveConflict?: boolean;
}>;

type RelayProfileTestResult = CommandResult<{
  httpStatus: number;
  endpoint: string;
  responsePreview: string;
  protocolValid: boolean;
  completionStatus: "completed" | "incomplete" | "failed";
}>;

type RelayProfileModelsResult = CommandResult<{
  models: string[];
  endpoint: string;
}>;

type PendingProviderImportResult = CommandResult<{
  pending: ProviderImportRequest | null;
  /** pending 文件内容的指纹，确认 / 取消时带回。 */
  fingerprint?: string | null;
}>;

type PendingProviderImport = { request: ProviderImportRequest; fingerprint: string };

function pendingProviderImportFromResult(result: { pending: ProviderImportRequest | null; fingerprint?: string | null }): PendingProviderImport | null {
  return result.pending ? { request: result.pending, fingerprint: result.fingerprint ?? "" } : null;
}

type RemoveEnvConflictsResult = CommandResult<{
  removed: Array<{
    name: string;
    removedProcess: boolean;
    removedUser: boolean;
  }>;
  backupPath: string | null;
  remaining: EnvConflict[];
}>;

type SessionIndexCleanupPreviewPayload = {
  snapshotSha256: string;
  candidates: SessionIndexCleanupCandidate[];
};

type SessionIndexCleanupApplyPayload = {
  prunedEntries?: number;
  backupDir?: string | null;
};

type UpdateStage = "downloading" | "writing" | "launching";

type UpdateProgressEvent = {
  stage: UpdateStage;
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number | null;
  elapsedMs: number;
};

type InstallResult = CommandResult<{
  silent_shortcut: { installed: boolean; path: string | null };
  management_shortcut: { installed: boolean; path: string | null };
}>;

function providerSyncProgressMessage(result: CommandResult<ProviderSyncPayload>): string {
  const changed = result.changedSessionFiles ?? 0;
  const rows = result.sqliteRowsUpdated ?? 0;
  const insertedCatalogRows = result.sqliteCatalogRowsInserted ?? 0;
  const removedCatalogRows = result.sqliteCatalogRowsRemoved ?? 0;
  const pruned = result.prunedSessionIndexEntries ?? 0;
  const target = result.targetProvider || t("当前 provider");
  const skipped = result.skippedLockedRolloutFiles?.length ?? 0;
  const prunedText = pruned ? tf("，清理 {0} 条失效任务索引", [pruned]) : "";
  const skippedText = skipped ? tf("，跳过 {0} 个占用文件", [skipped]) : "";
  const catalogText = insertedCatalogRows ? tf("，补齐 {0} 条侧边栏索引", [insertedCatalogRows]) : "";
  const catalogCleanupText = removedCatalogRows
    ? tf("，清理 {0} 条误列的子任务侧边栏索引", [removedCatalogRows])
    : "";
  return tf("已同步到 {0}：修复 {1} 个会话文件，更新 {2} 行数据库索引{3}{4}{5}{6}。", [
    target,
    changed,
    rows,
    catalogText,
    catalogCleanupText,
    prunedText,
    skippedText,
  ]);
}

function syncMarketInstalledState(current: ScriptMarketResult | null, userScripts: UserScriptInventory): ScriptMarketResult | null {
  if (!current) return current;
  const installed = new Map(
    (userScripts.scripts ?? [])
      .filter((script) => script.market_id)
      .map((script) => [script.market_id || "", script.version || ""]),
  );
  return {
    ...current,
    user_scripts: userScripts,
    market: {
      ...current.market,
      scripts: current.market.scripts.map((script) => {
        const installedVersion = installed.get(script.id) || "";
        return {
          ...script,
          installed: Boolean(installedVersion),
          installedVersion,
          updateAvailable: Boolean(installedVersion) && installedVersion !== script.version,
        };
      }),
    },
  };
}

type StartupResult = CommandResult<{
  showUpdate: boolean;
}>;

const MANAGER_NAVIGATION_EVENT = "manager-navigation-requested";
/** 后端存下网页导入 / 一键换肤待确认项后发出（`src-tauri/src/lib.rs`）。 */
const PENDING_IMPORT_CHANGED_EVENT = "manager-pending-import-changed";
/** 窗口聚焦触发的概览刷新最短间隔。 */
const FOCUS_REFRESH_MIN_INTERVAL_MS = 5_000;
/** 轻量检查 Codex 进程是否还在的间隔（只在管理器窗口可见时查）。 */
const codexRunningPollMs = 5_000;

export function App() {
  useSyncExternalStore(subscribeLanguage, getLanguage, getLanguage);
  const [themePreference, setThemePreference] = useState<ThemePreference>(() => loadThemePreference());
  const [systemDark, setSystemDark] = useState(() => systemPrefersDark());
  const theme: Theme = resolveTheme(themePreference, systemDark);
  const [route, setRoute] = useState<Route>(() => loadInitialRoute());
  // 轮询、事件回调里读当前页用（闭包里的 route 可能是旧的）。
  const routeRef = useRef(route);
  routeRef.current = route;
  /** 合并后的「设置」页里要滚到的分组（原来跳「关于」的地方滚到版本与更新、日志）。 */
  const [settingsFocus, setSettingsFocus] = useState<string | null>(() => initialSettingsFocus());
  /** 设置模式里整体设置页当前显示的卡片（一个分类一页）；深链到某张卡片时换到它所在的分类。 */
  const [settingsSections, setSettingsSections] = useState<string[]>(() => settingsSectionsFor(initialSettingsFocus() ?? "app-settings-general"));
  /** 从旧的「旧版代理设置」入口进来：「连接与供应商」页直接展开「高级」。 */
  const [relayAdvancedOpen, setRelayAdvancedOpen] = useState(() => initialPreviewRoute() === "relayEnvironment");
  /** 进设置模式前停的页面：侧栏「返回」回到这里。 */
  const lastMainRoute = useRef<Route>("home");
  const [notice, setNotice] = useState<NoticeState | null>(null);
  const [liveConflictDialog, setLiveConflictDialog] = useState<{
    profileName: string;
    resolve: (choice: LiveConflictResolution | null) => void;
  } | null>(null);
  const [sessionIndexCleanupDialog, setSessionIndexCleanupDialog] = useState<{
    candidates: SessionIndexCleanupCandidate[];
    resolve: (selectedIds: string[] | null) => void;
  } | null>(null);
  const [overview, setOverview] = useState<OverviewResult | null>(null);
  const [settings, setSettings] = useState<SettingsResult | null>(null);
  const [legacyDisconnectOpen, setLegacyDisconnectOpen] = useState(false);
  /** 断开本地服务成功后要切到的直连供应商：经本地服务时在列表里点了某个直连供应商的「使用」。 */
  const legacyDisconnectThen = useRef<string | null>(null);
  const [addDirectProviderRequested, setAddDirectProviderRequested] = useState(false);
  // 外部改设置时要拿「上次读到的设置」与当前编辑比对，监听回调里读这个 ref 拿最新值。
  const settingsRef = useRef<SettingsResult | null>(null);
  settingsRef.current = settings;
  const [weixinStatus, setWeixinStatus] = useState<WeixinConnectStatusResult | null>(null);
  const [weixinQr, setWeixinQr] = useState<WeixinQrResult | null>(null);
  const [relayFiles, setRelayFiles] = useState<RelayFilesResult | null>(null);
  const [envConflicts, setEnvConflicts] = useState<EnvConflictsResult | null>(null);
  const [relayEnvironment, setRelayEnvironment] = useState<RelayEnvironmentResult | null>(null);
  const [ccsProviders, setCcsProviders] = useState<CcsProvidersResult | null>(null);
  const [pendingProviderImport, setPendingProviderImport] = useState<PendingProviderImport | null>(null);
  const [localSessions, setLocalSessions] = useState<LocalSessionsResult | null>(null);
  const localSessionsRef = useRef(localSessions);
  localSessionsRef.current = localSessions;
  const [liveContextEntries, setLiveContextEntries] = useState<CodexContextEntries | null>(null);
  const [logs, setLogs] = useState<LogsResult | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsResult | null>(null);
  const [watcher, setWatcher] = useState<WatcherResult | null>(null);
  // 皮肤界面已移到 Codex 里；管理器只留网页一键换肤确认和「恢复默认皮肤」急救入口。
  const [pendingDreamSkinCommunity, setPendingDreamSkinCommunity] = useState("");
  const [dreamSkinCommunityInstalling, setDreamSkinCommunityInstalling] = useState(false);
  const dreamSkinCommunityInstallingRef = useRef(false);
  const [dreamSkinRestartPending, setDreamSkinRestartPending] = useState(false);
  const [update, setUpdate] = useState<UpdateResult | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  /** 正在看的拓展页面（route === "extension" 时）。 */
  const [extensionPageKey, setExtensionPageKey] = useState<string | null>(null);
  /** 被拓展顶替、但用户点了「查看内置页面」的路由。 */
  const [builtinPagesShown, setBuiltinPagesShown] = useState<Set<string>>(() => new Set());
  const [tourOpen, setTourOpen] = useState(false);
  // 启动引导：setupOpen 表示还没做完也没关掉（侧栏有进度卡），wizardOpen 表示两步向导正开着。
  const [setupOpen, setSetupOpen] = useState(setupCardNeeded);
  const [wizardOpen, setWizardOpen] = useState(false);
  // 从设置里「重新打开引导」时换一个 key，让向导从第一步重新开始。
  const [setupRun, setSetupRun] = useState(0);
  // 侧栏底部给进度卡留的位置（设置模式的侧栏没有）。
  const [setupSlot, setSetupSlot] = useState<HTMLDivElement | null>(null);
  // 向导只在它从没出现过时自动打开，而且等设置文件读回来再决定（#2902 / #2936）：页面本地存储会丢，
  // 设置文件里可能记着它出现过。判定期间进度卡也先不出，免得它先闪一下再被向导盖住。
  // 由启动流程在设置读回来后判定一次（见下方启动 effect）。
  const [setupChecking, setSetupChecking] = useState(setupAutoOpenPossible);
  const closeSetup = (completed: boolean) => {
    saveSetupState(completed ? "completed" : "dismissed");
    setSetupOpen(false);
    setWizardOpen(false);
  };
  const closeTour = useCallback(() => {
    // 看完或跳过都记下来，下次不再自动弹出；设置页里可以重新打开。
    try { window.localStorage.setItem(TOUR_STORAGE_KEY, String(TOUR_VERSION)); } catch { /* 存不了就只影响本次 */ }
    setTourOpen(false);
  }, []);
  // 「会话」页默认看所有 Agent；切到 Codex 页签才是带删除 / 归档 / 修复的完整会话管理。
  // 会话页默认看 Codex 的完整会话管理（删除、归档、移动、撤销都在这里）。
  const [sessionTab, setSessionTab] = useState<SessionTab>("codex");
  const [recoveryRefreshRequest, setRecoveryRefreshRequest] = useState(0);
  /** 请求日志打开时的 Agent 过滤（从 Agent 概览跳过去时设置，用一次就清掉）。 */
  const [requestLogAgentFilter, setRequestLogAgentFilter] = useState<"all" | "codex" | "claude" | "grok">("all");
  const [requestLogFocus, setRequestLogFocus] = useState<RequestLogFocus>({});
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "k") {
        event.preventDefault();
        if (wizardOpen || tourOpen) return;
        setPaletteOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [wizardOpen, tourOpen]);
  useEffect(() => {
    if (wizardOpen || tourOpen) setPaletteOpen(false);
  }, [wizardOpen, tourOpen]);
  const [updatePrompt, setUpdatePrompt] = useState<UpdateResult | null>(null);
  const updateNotifications = useRef(new UpdateNotifications());
  const updateCheckPending = useRef<{ manual: boolean } | null>(null);
  const updateInstallPending = useRef(false);
  // 更新提醒偏好以已保存的设置为准；草稿里的开关等自动保存落盘后才生效。
  useEffect(() => {
    if (settings) updateNotifications.current.setPreferences(updatePreferencesFrom(settings.settings));
  }, [settings]);
  const [updateDownload, setUpdateDownload] = useState<UpdateDownloadState | null>(null);
  const updateDownloadPending = useRef<string | null>(null);
  /** 正在进行的后台下载：点「安装」时先等它下完，不再并行下第二份（两份同时写同一个文件会坏）。 */
  const updateDownloadPromise = useRef<Promise<void> | null>(null);
  const [updateInstallProgress, setUpdateInstallProgress] = useState<TaskProgress>({
    active: false,
    percent: 0,
    message: t("尚未运行安装包更新。"),
  });
  const [ads, setAds] = useState<AdsResult | null>(null);
  // 共享供应商右下角「正在找合适的供应商？」的悬浮提示：点 × 关掉，本次运行内不再出现。
  const [sponsorHintClosed, setSponsorHintClosed] = useState(false);
  const [scriptMarket, setScriptMarket] = useState<ScriptMarketResult | null>(null);
  const [launchForm, setLaunchForm] = useState({
    appPath: "",
    debugPort: "9229",
    helperPort: "57321",
  });
  const prevLaunchStatusRef = useRef<string | null>(null);
  const overviewRequestSequence = useRef(0);
  const launchPendingRef = useRef(false);
  const [launchPending, setLaunchPending] = useState(false);
  /** 侧栏 Claude 行的启动键：本地模式状态读一次、动作完成后再读一次（读状态要查进程和钥匙串，不轮询）。 */
  const [claudeLocalRun, setClaudeLocalRun] = useState<CommandResult<ClaudeLocalStatus> | null>(claudeLocalMemo.status);
  const [claudeLocalRunBusy, setClaudeLocalRunBusy] = useState(false);
  const [multiLaunchPending, setMultiLaunchPending] = useState<string | null>(null);
  const [multiLaunchInstances, setMultiLaunchInstances] = useState<MultiLaunchInstance[]>([]);
  const [multiLaunchInstancesLoading, setMultiLaunchInstancesLoading] = useState(false);
  const [multiLaunchActionPending, setMultiLaunchActionPending] = useState<string | null>(null);
  const multiLaunchInstancesRequestRef = useRef(false);
  const [settingsForm, setSettingsForm] = useState<BackendSettings>({ ...defaultSettings });
  // 左侧 Agent 切换栏的数据源。后端是唯一事实来源，不落 localStorage —— 多窗口
  // 同时开着时才不会各说各话。
  const [toolEntries, setToolEntries] = useState<ToolEntry[]>([]);
  const [activeTool, setActiveTool] = useState<ToolId>("codex");
  /** 万物皆拓展页里选的宿主（Codex / Claude）。和侧栏当前的 Agent 互不联动：切宿主不换侧栏的 Agent，点侧栏的 Agent 也不改这里。 */
  const [extensionHost, setExtensionHost] = useState<ExtensionHost>("codex");
  useEffect(() => {
    if (!globalNavigationRoutes.includes(route)) lastAgentRoute.set(activeTool, route);
  }, [route, activeTool]);
  const [providerSyncProgress, setProviderSyncProgress] = useState<ProviderSyncProgress>({
    active: false,
    percent: 0,
    message: t("尚未运行历史会话修复。"),
    result: null,
  });
  const [providerSyncTargets, setProviderSyncTargets] = useState<ProviderSyncTargetsResult | null>(null);
  const [sessionIndexRepairActive, setSessionIndexRepairActive] = useState(false);
  const sessionIndexRepairRunning = useRef(false);
  const sessionIndexReportLoading = useRef(false);
  const [sessionIndexRepairReport, setSessionIndexRepairReport] = useState<SessionIndexRepairReport | null>(null);
  const [sessionIndexRepairReportError, setSessionIndexRepairReportError] = useState<string | null>(null);
  const [selectedProviderSyncTarget, setSelectedProviderSyncTarget] = useState("");
  const [autoTakeover, setAutoTakeoverState] = useState<AutoTakeoverResult | null>(null);
  const [relaySwitching, setRelaySwitching] = useState(false);
  const settingsDirty = useMemo(
    () => Boolean(settings && !backendSettingsEqual(settingsForm, settings.settings)),
    [settings, settingsForm],
  );

  /// 保存撞上 settings_revision_conflict（后台 token 刷新等抢先写盘）时自动恢复：
  /// 拉最新设置，把本次提交的编辑 rebase 上去、合并外部轮换的登录 token 后重试一次；
  /// 再次冲突或读取失败返回 null，由调用方按原逻辑抛错、保留草稿。
  const retrySaveSettingsAfterConflict = async <T,>(
    args: Record<string, unknown> | undefined,
    snapshot: BackendSettings | undefined,
  ): Promise<T | null> => {
    const previous = settingsRef.current?.settings;
    if (!snapshot || !previous) return null;
    let latest: SettingsResult;
    try {
      latest = await invoke<SettingsResult>("load_settings");
    } catch {
      return null;
    }
    if (!latest || !isSuccessStatus(latest.status)) return null;
    const base = normalizeSettings(previous);
    const fresh = normalizeSettings(latest.settings);
    const merged = mergeRotatedAuthContents(
      rebaseSettingsEdits(normalizeSettings(snapshot), base, fresh),
      base,
      fresh,
    );
    try {
      const retried = await invoke<T>("save_settings", {
        ...args,
        settings: merged,
        expectedRevision: merged.revision || "",
      });
      const response = retried as { message?: string };
      if (response?.message?.includes("settings_revision_conflict")) return null;
      return retried;
    } catch {
      return null;
    }
  };

  const call = <T,>(command: string, args?: Record<string, unknown>) => {
    const snapshot = args?.settings as BackendSettings | undefined;
    return invoke<T>(command, command === "save_settings"
      ? { ...args, expectedRevision: snapshot?.revision || "" }
      : args).then(async (result) => {
        const response = result as { message?: string };
        if (command === "save_settings" && response?.message?.includes("settings_revision_conflict")) {
          const recovered = await retrySaveSettingsAfterConflict<T>(args, snapshot);
          if (recovered !== null) return recovered;
          // 所有保存调用都保留当前草稿，不能把失败响应当成新表单回填。
          throw new Error(response.message);
        }
        return result;
      });
  };

  const logDiagnostic = (event: string, detail: Record<string, unknown> = {}) => {
    void invoke("write_diagnostic_event", { event, detail }).catch(() => {});
  };

  const run = async <T,>(task: () => Promise<T>): Promise<T | null> => {
    try {
      return await task();
    } catch (error) {
      showNotice(t("调用失败"), stringifyError(error), "failed");
      return null;
    }
  };

  const refreshTools = async (silent = true) => {
    const result = await run(() => call<ToolsResult>("list_tools"));
    if (result) {
      setToolEntries((result.tools ?? []).filter((tool) => !hiddenSidebarTools.has(tool.id)));
      setActiveTool(visibleActiveTool(result.activeTool));
      if (!silent) showResultNotice(t("工具列表"), result, { silentSuccess: true });
    }
    return result;
  };

  /// 切换侧栏聚焦的工具。纯 UI 状态：写回 settings.json 的 `activeTool`，
  /// 不触发任何供应商配置写入 —— 切工具 ≠ 切供应商。
  /// 传了 `destination` 就在切换成功后直接落到那一页（侧栏点 Agent 或它的子页面时用）。
  const switchTool = async (toolId: ToolId, destination?: Route) => {
    if (toolId === activeTool) {
      if (destination) await navigate(destination);
      // 通用页面占据整个右侧，点击当前 Agent 也要能回到它的工作区。
      else if (globalNavigationRoutes.includes(route)) await navigate("overview");
      return;
    }
    const target = toolEntries.find((tool) => tool.id === toolId);
    if (target && !target.switchable) {
      showNotice(t("该工具暂不可切换"), tf("{0} 的供应商配置还没接入，切过去只会显示空列表。", [target.name]), "failed");
      return;
    }
    setActiveTool(toolId);
    const next = { ...settingsForm, activeTool: toolId };
    setSettingsForm(next);
    // 供应商页是跟着工具走的，切工具后如果当前页不属于新工具就跳到它自己的页。
    // 设置页也只属于 Codex，同样按这条规则跳走。
    const currentRoute = routes.find((candidate) => candidate.id === route);
    if (!pageVisibleForTool(currentRoute, toolId)) {
      setRoute(agentHomeRoute[toolId] ?? "overview");
    }
    const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      // 先保存 Agent 选择，再离开通用页；失败时保留原页面与布局。
      if (destination) await navigate(destination);
      else if (globalNavigationRoutes.includes(route)) await navigate("overview");
    } else {
      // 写盘失败就回滚 UI，别让侧栏显示一个没保存的状态。
      setActiveTool(activeTool);
      void refreshSettings(true);
    }
  };

  const refreshOverview = async (silent = false) => {
    const requestSequence = ++overviewRequestSequence.current;
    const result = await run(() => call<OverviewResult>("load_overview"));
    if (result && requestSequence === overviewRequestSequence.current) {
      // 崩溃检测：进程从运行状态变为停止/失败 → 弹出通知
      const prev = prevLaunchStatusRef.current;
      const current = result.latest_launch?.status;
      // 用户在侧栏点了「停止」不算意外停止。
      const stoppedByUser = result.latest_launch?.phase === "user_stopped";
      if (prev && prev === "running" && current && !stoppedByUser && (current === "stopped" || current === "failed" || current === "crashed")) {
        showNotice(t("Codex 意外停止"), tf("进程状态：{0}。是否要重新启动？", [current]), "failed");
      }
      prevLaunchStatusRef.current = current ?? null;
      setOverview(result);
      if (!silent) showResultNotice(t("概览已检查"), result, { silentSuccess: true });
    }
  };

  // Computer Use 和 Claude 增强页的开关、「启动引导出现过」的记号由后端单独写进 settings.json：只合并这些键和版本号，
  // 别处没保存的草稿保持不动；否则表单里的旧版本号会让之后的保存全部报冲突（#2937）。
  useEffect(() => {
    const sync = async () => {
      const result = await run(() => call<SettingsResult>("load_settings"));
      if (!result) return;
      const fresh = normalizeSettings(result.settings) as BackendSettings & Record<string, unknown>;
      setSettings(result);
      setSettingsForm((current) => {
        const merged: Record<string, unknown> = { ...current, revision: fresh.revision };
        for (const key of [
          "computerUseMode", "computerUseApproval", "computerUsePip",
          "claudeAppVoiceInput", "claudeAppLocalization", "claudeAppExtensions", "claudeAppComputerUse", "claudeAppBashTimeout", "claudeDesktopLocal",
          // Claude 自动接管的开关也是后端单独写的（set_claude_auto_takeover）。
          "claudeAppAutoTakeover",
          // 「启动引导出现过」的记号也要带上：表单留着旧值，下一次整份保存会把刚记下的写回去。
          "managerAgentFocus",
          "nativeAuthSelection",
        ]) merged[key] = fresh[key];
        return merged as BackendSettings;
      });
    };
    const handler = () => void sync();
    window.addEventListener(BACKEND_SETTINGS_CHANGED_EVENT, handler);
    return () => window.removeEventListener(BACKEND_SETTINGS_CHANGED_EVENT, handler);
    // 只在挂载时注册；run / call 只依赖稳定的 setter。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const refreshSettings = async (silent = false) => {
    const result = await run(() => call<SettingsResult>("load_settings"));
    if (result) {
      setSettings(result);
      const normalized = normalizeSettings(result.settings);
      setSettingsForm(normalized);
      // 侧栏聚焦的工具以后端存的为准，避免刷新后跳回 codex。
      setActiveTool(visibleActiveTool(normalized.activeTool));
      setLaunchForm((current) => ({
        ...current,
        appPath: current.appPath || result.settings.codexAppPath || "",
      }));
      if (!silent) showResultNotice(t("设置已加载"), result, { silentSuccess: true });
      return normalized;
    }
    return null;
  };

  const refreshWeixinStatus = async (silent = false) => {
    const result = await run(() => call<WeixinConnectStatusResult>("weixin_connect_status"));
    if (result) {
      setWeixinStatus(result);
      if (!silent) showResultNotice(t("微信连接"), result, { silentSuccess: true });
    }
    return result;
  };

  const dreamSkinRequest = () => ({
    request: {
      debugPort: overview?.latest_launch?.debug_port ?? parsePort(launchForm.debugPort, 9229),
      helperPort: overview?.latest_launch?.helper_port ?? parsePort(launchForm.helperPort, 57321),
    },
  });

  /// force 只在「刷新列表」时为 true：后端跳过清单缓存重新下载；其余时候用缓存。
  const refreshScriptMarket = async (silent = false, preserveRuntime = false, force = false) => {
    const result = await run(() => call<ScriptMarketResult>("refresh_script_market", { force }));
    if (result) {
      setScriptMarket(result);
      setSettings((current) => (current ? {
        ...current,
        user_scripts: preserveRuntime ? current.user_scripts : result.user_scripts,
      } : current));
      if (!silent || !isSuccessStatus(result.status)) showResultNotice(t("拓展"), result, { silentSuccess: true });
    }
  };

  const refreshUserScriptInventory = async () => {
    const result = await run(() => call<SettingsResult>("refresh_user_script_inventory"));
    if (result) {
      setSettings(result);
      setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
    }
    return result;
  };

  const reloadUserScripts = async (silent = false) => {
    const result = await run(() => call<SettingsResult>("reload_user_scripts"));
    if (result) {
      setSettings(result);
      setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
      showResultNotice(t("本地拓展"), result, { silentSuccess: silent });
    }
    return result?.status;
  };

  const installMarketScript = async (id: string) => {
    const result = await run(() => call<ScriptMarketResult>("install_market_script", { id }));
    if (result) {
      setScriptMarket(result);
      setSettings((current) => (current ? { ...current, user_scripts: result.user_scripts } : current));
      showResultNotice(t("拓展"), result);
    }
  };

  const setUserScriptsEnabled = async (enabled: boolean, host: ExtensionHost = "codex") => {
    const result = await run(() => call<SettingsResult>("set_user_scripts_enabled", { enabled, ...(host === "claude" ? { host } : {}) }));
    if (!result) return;
    setSettings(result);
    setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
    if (host === "claude" && isSuccessStatus(result.status)) {
      setSettingsForm((current) => ({
        ...current,
        revision: result.settings.revision,
        claudeAppExtensions: result.settings.claudeAppExtensions,
      }));
      if (claudeLocalizationMemo.status) {
        claudeLocalizationMemo.status = { ...claudeLocalizationMemo.status, extensions: result.user_scripts?.claude_enabled === true };
      }
    }
    showResultNotice(t("本地拓展"), result);
    if (isSuccessStatus(result.status) && host === "codex") await reloadUserScripts();
  };

  const setUserScriptsSkipNextStart = async (skip: boolean) => {
    const result = await run(() => call<SettingsResult>("set_user_scripts_skip_next_start", { skip }));
    if (!result) return;
    setSettings(result);
    setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
    showResultNotice(t("本地拓展"), result);
  };

  /// 保存拓展配置后热重载，页面侧的 ctx.config 才会读到新值。
  const setExtensionConfig = async (key: string, values: Record<string, unknown>, options: { silent?: boolean } = {}) => {
    const result = await run(() => call<SettingsResult>("set_extension_config", { key, values }));
    if (!result) return false;
    setSettings(result);
    // 自动保存（Agent 桌宠页）成功时不弹提示，失败照常提示。
    showResultNotice(t("本地拓展"), result, { silentSuccess: options.silent === true });
    if (!isSuccessStatus(result.status)) return false;
    // 拓展用 ctx.onConfigUpdate 接住新配置时原地生效，不必整体热重载。
    const live = await call<string>("update_extension_config_live", { key }).catch(() => "reload");
    if (live !== "applied") await reloadUserScripts(options.silent === true);
    return true;
  };

  /// 拓展管理器面板里的 codexPlusManager.toast()：用拓展名作标题的普通提示。
  const showExtensionNotice = (title: string, message: string) => showNotice(title, message, "ok");

  /// 启停单个拓展后热重载，Codex 页面才会真的装上 / 卸下它（与整体开关、保存配置一致）。
  const setUserScriptEnabled = async (key: string, enabled: boolean, host: ExtensionHost = "codex") => {
    const result = await run(() => call<SettingsResult>("set_user_script_enabled", { key, enabled, ...(host === "claude" ? { host } : {}) }));
    if (result) {
      setSettings(result);
      setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
      if (isSuccessStatus(result.status) && host === "codex") {
        const reloadStatus = await reloadUserScripts();
        if (reloadStatus !== "warning") await refreshUserScriptInventory();
      } else {
        showResultNotice(t("本地拓展"), result);
        await refreshUserScriptInventory();
      }
    }
  };

  const deleteUserScript = async (key: string) => {
    const script = settings?.user_scripts?.scripts?.find((item) => item.key === key);
    const name = script?.name || key;
    const isPackage = script?.kind === "package";
    const question = isPackage
      ? tf("卸载拓展“{0}”？整个拓展文件夹会移进废纸篓，需要时可以从废纸篓找回。", [name])
      : tf("删除脚本“{0}”？此操作会移除本地脚本文件。", [name]);
    if (!(await appConfirm(question, { title: isPackage ? t("卸载拓展") : t("删除脚本"), confirmText: isPackage ? t("卸载") : t("删除"), danger: true }))) return;
    const result = await run(() => call<SettingsResult>("delete_user_script", { key }));
    if (result) {
      setSettings(result);
      setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
      showResultNotice(t("本地拓展"), result);
      await refreshUserScriptInventory();
    }
  };

  const refreshRelayFiles = async (silent = false) => {
    const result = await run(() => call<RelayFilesResult>("read_relay_files"));
    if (result) {
      setRelayFiles(result);
      if (!silent) showResultNotice(t("配置文件"), result, { silentSuccess: true });
    }
    return result;
  };

  const refreshEnvConflicts = async (silent = false) => {
    const result = await run(() => call<EnvConflictsResult>("check_env_conflicts"));
    if (result) {
      setEnvConflicts(result);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice(t("环境变量检测"), result, { silentSuccess: true });
    }
    return result;
  };

  const refreshRelayEnvironment = async (silent = false) => {
    const result = await run(() => call<RelayEnvironmentResult>("check_relay_environment"));
    if (result) {
      setRelayEnvironment(result);
      if (!silent) showResultNotice(t("中转站环境配置检测"), result, { silentSuccess: true });
    }
    return result;
  };

  const removeEnvConflicts = async (names: string[]) => {
    const uniqueNames = Array.from(new Set(names.map((name) => name.trim()).filter(Boolean)));
    if (!uniqueNames.length) return;
    if (!(await appConfirm(tf("删除这些环境变量？\n\n{0}\n\n删除前会写入备份。", [uniqueNames.join("\n")]), { title: t("删除环境变量"), confirmText: t("删除"), danger: true }))) return;
    const result = await run(() => call<RemoveEnvConflictsResult>("remove_env_conflicts", { request: { names: uniqueNames } }));
    if (result) {
      setEnvConflicts((current) => ({
        status: result.status,
        message: result.message,
        conflicts: result.remaining,
        sshRemoteLoopback: current?.sshRemoteLoopback ?? null,
      }));
      showNotice(t("环境变量清理"), result.message, result.status);
    }
  };

  const refreshCcsProviders = async (silent = false) => {
    const result = await run(() => call<CcsProvidersResult>("load_ccs_providers"));
    if (result) {
      setCcsProviders(result);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice(t("cc-switch 导入"), result, { silentSuccess: true });
    }
    return result;
  };

  const importCcsProviders = async (bindings: Record<string, string> = {}, only?: string[]) => {
    const result = await run(() => call<SettingsResult>("import_ccs_providers", { bindings, only }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showResultNotice(t("cc-switch 导入"), result);
      await refreshCcsProviders(true);
    }
  };

  /** 把 Magpie 里勾选的供应商导入成直连供应商（「从第三方导入」对话框）。 */
  const importMagpieProviders = async (ids: string[]) => {
    const result = await run(() => call<SettingsResult>("import_magpie_providers", { ids }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showResultNotice(t("Magpie 导入"), result);
    }
  };

  const refreshPendingProviderImport = async (silent = true) => {
    const result = await run(() => call<PendingProviderImportResult>("load_pending_provider_import"));
    if (result) {
      setPendingProviderImport(pendingProviderImportFromResult(result));
      if (!silent && !isSuccessStatus(result.status)) showResultNotice(t("Codex++ 导入"), result, { silentSuccess: true });
    }
    return result;
  };

  // 确认 / 取消都带上弹窗展示时的指纹：期间被新链接覆盖时后端拒绝写入，弹窗换成新的那条重新确认。
  const confirmPendingProviderImport = async () => {
    const fingerprint = pendingProviderImport?.fingerprint ?? "";
    const result = await run(() => call<SettingsResult>("confirm_pending_provider_import", { fingerprint }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showResultNotice(t("Codex++ 导入"), result);
      if (isSuccessStatus(result.status)) setPendingProviderImport(null);
      else await refreshPendingProviderImport(true);
      await refreshCcsProviders(true);
    }
  };

  const dismissPendingProviderImport = async () => {
    const fingerprint = pendingProviderImport?.fingerprint ?? null;
    const result = await run(() => call<PendingProviderImportResult>("dismiss_pending_provider_import", { fingerprint }));
    if (result) {
      setPendingProviderImport(isSuccessStatus(result.status) ? pendingProviderImportFromResult(result) : null);
      showResultNotice(t("Codex++ 导入"), result, { silentSuccess: true });
    }
  };

  /** 会话页的搜索词和归档筛选；导航、删除后的刷新沿用最近一次的条件。 */
  const localSessionFilter = useRef<LocalSessionFilter>({ query: "", archived: false });
  const refreshLocalSessions = async (silent = false, offset = 0, filter?: LocalSessionFilter): Promise<LocalSessionsResult | null> => {
    if (filter) localSessionFilter.current = filter;
    const { query, archived } = localSessionFilter.current;
    const result = await run(() =>
      call<LocalSessionsResult>("list_local_sessions", {
        request: { offset, limit: 50, query, archived },
      }),
    );
    if (result) {
      // 期间又换了筛选条件：这次结果已经过时，交给后发的请求。
      if (localSessionFilter.current.query !== query || localSessionFilter.current.archived !== archived) return result;
      if (!result.sessions.length && result.offset > 0) {
        return refreshLocalSessions(silent, Math.max(0, result.offset - result.limit));
      }
      setLocalSessions(result);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice(t("会话管理"), result, { silentSuccess: true });
    }
    return result;
  };

  const listProjectSessions = async (cwd: string): Promise<LocalSession[]> => {
    const result = await run(() => call<LocalSessionsResult>("list_project_sessions", { request: { cwd } }));
    if (!result || !isSuccessStatus(result.status)) {
      if (result) showResultNotice(t("会话管理"), result);
      return [];
    }
    showNotice(t("会话管理"), result.message, "ok");
    return result.sessions;
  };


  const requestDeleteLocalSession = (session: LocalSession) =>
    call<DeleteLocalSessionResult>("delete_local_session", {
      request: { sessionId: session.id, title: session.title, dbPath: session.dbPath },
    });

  /** 删除后提示里的「撤销」：按删除结果里的 token 从备份恢复。 */
  const undoDeleteLocalSessions = async (token: string) => {
    const result = await run(() => call<DeleteLocalSessionResult>("undo_delete_local_sessions", { token }));
    if (!result) return;
    showNotice(t("撤销删除"), result.message, result.status === "undone" ? "ok" : "failed");
    await refreshLocalSessions(true, localSessions?.offset ?? 0);
  };
  const undoDeleteAction = (token: string | null | undefined): NoticeAction | undefined =>
    token ? { label: t("撤销"), run: () => void undoDeleteLocalSessions(token) } : undefined;

  const refreshPendingDreamSkinCommunity = async () => {
    const result = await run(() => call<PendingDreamSkinCommunityResult>("load_pending_dream_skin_community"));
    if (result) setPendingDreamSkinCommunity(result.versionId);
    return result;
  };

  /// 网页一键换肤（dreamskin:// 链接由系统交给管理器）：确认后下载安装，再按安装出的主题 id 应用。
  /// 主题库和编辑界面已移到 Codex 里，这里只走安装 + 应用两条命令。
  const confirmPendingDreamSkinCommunity = async () => {
    if (dreamSkinCommunityInstallingRef.current) return;
    dreamSkinCommunityInstallingRef.current = true;
    setDreamSkinCommunityInstalling(true);
    try {
      const result = await run(() => call<DreamSkinCommunityResult>("confirm_pending_dream_skin_community"));
      if (!result) return;
      showResultNotice(t("DreamSkin 社区"), result);
      if (!isSuccessStatus(result.status)) return;
      setPendingDreamSkinCommunity("");
      setRoute("enhance");
      const themeId = result.installedThemeId?.trim();
      if (themeId) {
        const activation = await run(() => call<DreamSkinInstalledThemeActivationResult>(
          "activate_installed_dream_skin_theme",
          { id: themeId },
        ));
        if (activation) {
          showResultNotice(t("主题库"), activation);
          if (isSuccessStatus(activation.status) && activation.savedForNextLaunch) setDreamSkinRestartPending(true);
        }
      }
      await refreshSettings(true);
    } finally {
      dreamSkinCommunityInstallingRef.current = false;
      setDreamSkinCommunityInstalling(false);
    }
  };

  const dismissPendingDreamSkinCommunity = async () => {
    if (dreamSkinCommunityInstallingRef.current) return;
    const result = await run(() => call<PendingDreamSkinCommunityResult>("dismiss_pending_dream_skin_community"));
    if (!result) return;
    if (isSuccessStatus(result.status)) setPendingDreamSkinCommunity("");
    else showResultNotice(t("DreamSkin 社区"), result);
  };

  const selectSessionIndexCleanupCandidates = (candidates: SessionIndexCleanupCandidate[]) =>
    new Promise<string[] | null>((resolve) => {
      setSessionIndexCleanupDialog({
        candidates,
        resolve,
      });
    });

  const deleteLocalSession = async (session: LocalSession) => {
    const title = session.title || session.id;
    const confirmed = await appConfirm(tf("删除会话“{0}”？此操作会删除本地数据库记录和 rollout 文件，并创建备份。", [title]), {
      title: t("删除会话"),
      confirmText: t("删除"),
      danger: true,
    });
    if (!confirmed) return false;
    const result = await run(() => requestDeleteLocalSession(session));
    const deleted = !!result && sessionDeleteSucceeded(result.status);
    if (result) {
      showNotice(t("会话删除"), result.message, deleted ? "ok" : "failed", deleted ? undoDeleteAction(result.undo_token) : undefined);
      await refreshLocalSessions(true, localSessions?.offset ?? 0);
    }
    return deleted;
  };

  const deleteLocalSessions = async (sessions: LocalSession[]) => {
    const uniqueSessions = Array.from(new Map(sessions.map((session) => [session.id, session])).values());
    if (!uniqueSessions.length) {
      showNotice(t("批量删除会话"), t("请先选择要删除的会话。"), "failed");
      return false;
    }
    const preview = uniqueSessions
      .slice(0, 6)
      .map((session) => `- ${truncateSessionDeletePreview(session.title || session.id)}`)
      .join("\n");
    const extraCount = uniqueSessions.length > 6 ? tf("\n...以及另外 {0} 个会话", [uniqueSessions.length - 6]) : "";
    const confirmed = await appConfirm(
      tf("删除选中的 {0} 个会话？此操作会删除本地数据库记录和 rollout 文件，并为每个会话创建备份。\n\n{1}{2}", [uniqueSessions.length, preview, extraCount]),
      { title: t("批量删除会话"), confirmText: t("删除"), danger: true },
    );
    if (!confirmed) return false;
    // 一次调用在后台逐个删除，不会每删一个就往返一次、卡住窗口；所有备份合成一个撤销 token。
    const result = await run(() => call<DeleteLocalSessionsResult>("delete_local_sessions", {
      request: { sessions: uniqueSessions.map((session) => ({ sessionId: session.id, title: session.title, dbPath: session.dbPath })) },
    }));
    if (result) showNotice(t("批量删除会话"), result.message, result.status, result.succeeded ? undoDeleteAction(result.undoToken) : undefined);
    await refreshLocalSessions(true, localSessions?.offset ?? 0);
    return true;
  };

  const archiveLocalSessions = async (sessions: LocalSession[], archived = true) => {
    const uniqueSessions = Array.from(new Map(sessions.map((session) => [session.id, session])).values());
    if (!uniqueSessions.length) return false;
    const confirmed = await appConfirm(
      archived
        ? tf("归档选中的 {0} 个会话？只改本地归档状态，不删除对话文件，之后可以取消归档。", [uniqueSessions.length])
        : tf("取消归档选中的 {0} 个会话？它们会回到未归档列表。", [uniqueSessions.length]),
      { title: archived ? t("归档会话") : t("取消归档会话"), confirmText: archived ? t("归档") : t("取消归档") },
    );
    if (!confirmed) return false;
    const result = await run(() => call<CommandResult<{ succeeded: number; failed: number }>>("archive_local_sessions", {
      request: { sessions: uniqueSessions, archived },
    }));
    if (result) showNotice(archived ? t("归档会话") : t("取消归档会话"), result.message, result.status);
    await refreshLocalSessions(true, localSessions?.offset ?? 0);
    return true;
  };

  const refreshLiveContextEntries = async (silent = false) => {
    const result = await run(() => call<LiveContextEntriesResult>("read_live_context_entries"));
    if (result) {
      setLiveContextEntries(result.entries);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice(t("MCP&插件"), result, { silentSuccess: true });
    }
    return result;
  };

  const syncLiveContextEntries = async (next: BackendSettings, silent = false) => {
    const result = await run(() => call<LiveContextEntriesResult>("sync_live_context_entries", { request: { settings: next } }));
    if (result) {
      setLiveContextEntries(result.entries);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice(t("MCP&插件"), result, { silentSuccess: true });
    }
    return result;
  };

  const refreshLogs = async (silent = false) => {
    const result = await run(() => call<LogsResult>("read_latest_logs", { request: { lines: 240 } }));
    if (result) {
      setLogs(result);
      if (!silent) showResultNotice(t("日志已刷新"), result, { silentSuccess: true });
    }
  };

  const clearLogs = async () => {
    const result = await run(() => call<LogsResult>("clear_logs"));
    if (result) {
      setLogs(result);
      showResultNotice(t("日志清理"), result, { silentSuccess: false });
    }
  };

  const refreshDiagnostics = async (silent = false) => {
    const result = await run(() => call<DiagnosticsResult>("copy_diagnostics"));
    if (result) {
      setDiagnostics(result);
      if (!silent) showResultNotice(t("诊断已生成"), result, { silentSuccess: true });
    }
  };

  const refreshWatcher = async (silent = false) => {
    const result = await run(() => call<WatcherResult>("load_watcher_state"));
    if (result) {
      setWatcher(result);
      if (!silent) showResultNotice(t("Watcher 状态"), result, { silentSuccess: true });
    }
  };

  /**
   * 读取自动接管状态。总开关关闭时不探测：守护线程本来就不工作，
   * 读到的只会是「未运行」，徒增一次 IPC。
   * `enabled` 由刚保存开关的调用方传入：那时闭包里的 settings 还是旧值（关），
   * 照旧值判断会把刚打开的状态清空，状态行一直显示「未运行」。
   */
  const refreshAutoTakeover = async (silent = false, enabled = settings?.settings.codexAppAutoTakeover) => {
    if (enabled === false) {
      setAutoTakeoverState(null);
      return;
    }
    const result = await run(() => call<AutoTakeoverResult>("load_auto_takeover_state"));
    if (result) {
      setAutoTakeoverState(result);
      if (!silent) showResultNotice(t("自动接管状态"), result, { silentSuccess: true });
    }
  };

  /** 开关必须走既有设置保存链路，才能在设置文件里持久化并被守护线程读到。 */
  const setAutoTakeover = async (enabled: boolean) => {
    if (!settings) return;
    const saved = await saveSettingsValue({ ...settings.settings, codexAppAutoTakeover: enabled }, false);
    if (!saved) return;
    setSettings((current) => (current ? { ...current, settings: saved } : current));
    if (enabled) {
      await refreshAutoTakeover(true, true);
    } else {
      setAutoTakeoverState(null);
    }
  };

  const runAutoTakeoverCommand = async (command: string) => {
    const result = await run(() => call<AutoTakeoverResult>(command));
    if (result) {
      setAutoTakeoverState(result);
      showResultNotice(t("自动接管"), result, { silentSuccess: false });
    }
  };

  // 首页和设置页都显示接管状态：开着时轮询，接管、跳过、出错都能马上看到（原先只在进页面时读一次）。
  // 首页的「Codex++ 没开机自启」提示要用到 watcher，进首页时顺带读一次。
  const autoTakeoverOn = Boolean(settings?.settings.codexAppAutoTakeover);
  const autoTakeoverVisible = route === "home" || route === "agentSettings";
  const autoTakeoverPolling = autoTakeoverVisible && autoTakeoverOn;
  // 关掉开关或离开页面后，还在路上的那一轮结果不再写回（关开关时状态已清空）。
  const autoTakeoverPollingRef = useRef(autoTakeoverPolling);
  autoTakeoverPollingRef.current = autoTakeoverPolling;
  // 状态没变时沿用旧对象，免得每 3 秒整个 App 重渲染；窗口不可见时暂停。
  usePolling(async () => {
    const result = await invoke<AutoTakeoverResult>("load_auto_takeover_state");
    if (autoTakeoverPollingRef.current) setAutoTakeoverState((previous) => keepIfSame(previous, result));
  }, 3000, { enabled: autoTakeoverPolling });
  useEffect(() => {
    if (route !== "home" || !autoTakeoverOn) return;
    let cancelled = false;
    void invoke<WatcherResult>("load_watcher_state").then((result) => { if (!cancelled) setWatcher(result); }).catch(() => {});
    return () => { cancelled = true; };
  }, [autoTakeoverOn, route]);

  const navigate = async (target: RouteTarget) => {
    // 旧 id 在这里换成现在的页面：关于 → 设置并滚到版本与更新；旧版代理设置 → 「连接与供应商」并展开「高级」。
    if (target === "about") setSettingsFocus("app-settings-update");
    setRelayAdvancedOpen(target === "relayEnvironment");
    const next = resolveRouteAlias<Route>(target);
    setRoute(next);
    await loadRouteData(next);
  };

  /** 进页面时读它要的数据：互不依赖的一起发出，页面用不到的不读。 */
  const loadRouteData = async (next: Route) => {
    const loads: Promise<unknown>[] = [];
    const needsSettings: Route[] = ["relay", "multiLaunch", "sessions", "context", "weixin", "settings", "agentSettings", "enhance", "userScripts", "pluginMarket"];
    if (needsSettings.includes(next)) loads.push(refreshSettings(true));
    // 回到会话页时停在原来那一页。
    const sessionsOffset = localSessionsRef.current?.offset ?? 0;
    switch (next) {
      case "overview":
      case "gatewaySettings":
      case "settings":
        loads.push(refreshOverview(true));
        break;
      case "relay":
        // 「连接与供应商」页底部的「高级」里有环境检测。
        loads.push(refreshRelayFiles(true), refreshEnvConflicts(true), refreshCcsProviders(true), refreshRelayEnvironment(true));
        break;
      case "multiLaunch":
        loads.push(refreshRelayFiles(true), refreshMultiLaunchInstances(true));
        break;
      case "sessions":
        loads.push(refreshLocalSessions(true, sessionsOffset), refreshProviderSyncTargets(true), refreshSessionIndexRepairReport());
        break;
      case "context":
        loads.push(refreshRelayFiles(true), refreshLiveContextEntries(true));
        break;
      case "pluginMarket":
        // 「本机配置」页签列 config.toml 里的插件条目。
        loads.push(refreshLiveContextEntries(true));
        break;
      case "weixin":
        // 微信页从最近的会话里挑，读第一页。
        loads.push(refreshWeixinStatus(true), refreshLocalSessions(true));
        break;
      case "userScripts":
        loads.push((async () => {
          // 市场和本地清单一起读；清单是最新的本地状态，两个都回来后以它为准。
          const [, inventory] = await Promise.all([refreshScriptMarket(true), refreshUserScriptInventory()]);
          if (inventory) {
            setSettings(inventory);
            setScriptMarket((current) => syncMarketInstalledState(current, inventory.user_scripts));
          }
        })());
        break;
      case "home":
        loads.push(refreshAds(true));
        break;
      case "agentSettings":
        loads.push(refreshOverview(true), refreshWatcher(true), refreshAutoTakeover(true));
        break;
      default:
        break;
    }
    await Promise.all(loads);
  };
  // 用量统计里点一行：切到「请求」页签并按它（Agent / 供应商 / 模型）筛选。
  const navigateRef = useRef(navigate);
  navigateRef.current = navigate;
  useEffect(() => onOpenRequestLog((focus) => {
    setRequestLogAgentFilter(focus.agent ?? "all");
    setRequestLogFocus(focus);
    void navigateRef.current("requestLog");
  }), []);

  const consumePendingManagerNavigation = async (): Promise<boolean> => {
    try {
      const navigation = await invoke<ManagerNavigationIntent | null>("consume_pending_manager_navigation");
      if (!navigation) return false;
      const destination = managerNavigationDestination(navigation);
      if (destination.route === "enhance" || destination.route === "pluginMarket" || destination.route === "weixin" || destination.route === "sessions") {
        setActiveTool("codex");
      }
      if (destination.route === "sessions") {
        setSessionTab("codex");
        setRecoveryRefreshRequest(current => current + 1);
      }
      // 和侧栏点进去走同一条路：会话页的列表等数据也会读。
      await navigate(destination.route);
      return true;
    } catch (error) {
      logDiagnostic("manager.navigation_failed", { error: stringifyError(error) });
    }
    return false;
  };

  const consumeNavigationRef = useRef(consumePendingManagerNavigation);
  consumeNavigationRef.current = consumePendingManagerNavigation;

  const launchMultiInstance = async (profile: RelayProfile) => {
    if (multiLaunchPending) return;
    setMultiLaunchPending(profile.id);
    try {
      const result = await run(() => call<MultiLaunchResult>("launch_codex_multi_instance", {
        request: { profileId: profile.id },
      }));
      if (!result) return;
      if (result.alreadyRunning) {
        showNotice(t("Codex多开"), result.message, result.status);
        await refreshMultiLaunchInstances(true);
        return;
      }
      // 父进程只知道「worker 已拉起」，真正起来没有要等它就绪（最长 90 秒），
      // 所以这里不直接报成功，改为轮询到有结论再给提示。
      await refreshMultiLaunchInstances(true);
      const outcome = await pollMultiLaunchOutcome(profile.id);
      if (outcome === "ready") {
        showNotice(t("Codex多开"), t("Codex 多开实例已就绪。"), "ok");
      } else if (outcome === "failed") {
        showNotice(t("Codex多开"), t("Codex 实例启动失败，请查看实例列表中的原因。"), "failed");
      } else {
        showNotice(t("Codex多开"), result.message, result.status);
      }
      await refreshMultiLaunchInstances(true);
    } finally {
      setMultiLaunchPending(null);
    }
  };

  /**
   * 等待某个多开实例给出结论（就绪 / 失败），最长约 2 分钟。
   *
   * worker 是独立进程且 stdout 被丢弃，父进程只能靠 `launch-result.json` 判断成败；
   * 没有这个轮询的话，Windows 上「点了没窗口」会一直没有任何反馈。
   */
  const pollMultiLaunchOutcome = async (instanceId: string): Promise<"ready" | "failed" | "pending"> => {
    const deadline = Date.now() + MULTI_LAUNCH_POLL_TIMEOUT_MS;
    while (Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, MULTI_LAUNCH_POLL_INTERVAL_MS));
      const result = await refreshMultiLaunchInstances(true);
      const instance = result?.instances?.find((item) => item.instanceId === instanceId);
      if (!instance) continue;
      if (multiLaunchInstanceRunning(instance)) return "ready";
      if (multiLaunchInstanceFailure(instance)) return "failed";
    }
    return "pending";
  };

  const refreshMultiLaunchInstances = async (silent = true) => {
    if (multiLaunchInstancesRequestRef.current) return null;
    multiLaunchInstancesRequestRef.current = true;
    if (!silent) setMultiLaunchInstancesLoading(true);
    try {
      const result = await run(() => call<MultiLaunchInstancesResult>("list_codex_multi_instances"));
      if (result) {
        // 失败实例必须留在列表里：多开在 Windows 上起不来时，
        // 界面要给出「启动失败 + 原因」，只留运行中的会让它彻底消失。
        const instances = Array.isArray(result.instances)
          ? result.instances.filter(multiLaunchInstanceVisible)
          : [];
        // 每 3 秒轮询一次：列表没变时沿用旧数组，免得整个 App 重渲染。
        setMultiLaunchInstances((previous) => keepIfSame(previous, instances));
        if (!silent && !isSuccessStatus(result.status)) showNotice(t("当前运行实例"), result.message, result.status);
      }
      return result;
    } finally {
      multiLaunchInstancesRequestRef.current = false;
      if (!silent) setMultiLaunchInstancesLoading(false);
    }
  };

  const restartMultiLaunchInstance = async (instance: MultiLaunchInstance) => {
    // #3025: the main process is not a multi-instance. The backend rejects instanceId "main".
    if (multiLaunchInstanceIsMain(instance)) {
      showNotice(t("重启 Codex 实例"), "主 Codex 请使用顶部的「重启 Codex」。", "failed");
      return;
    }
    if (multiLaunchActionPending) return;
    setMultiLaunchActionPending(instance.instanceId);
    try {
      const result = await run(() => call<MultiLaunchResult>("restart_codex_multi_instance", {
        request: { instanceId: instance.instanceId },
      }));
      if (result) showNotice(t("重启 Codex 实例"), result.message, result.status);
      await refreshMultiLaunchInstances(true);
    } finally {
      setMultiLaunchActionPending(null);
    }
  };

  const stopMultiLaunchInstance = async (instance: MultiLaunchInstance) => {
    if (multiLaunchInstanceIsMain(instance)) {
      showNotice(t("结束 Codex 实例"), "主 Codex 请使用侧栏的「停止 Codex」。", "failed");
      return;
    }
    if (multiLaunchActionPending) return;
    const name = formatMultiLaunchInstanceLabel(instance, { main: t("主 Codex"), fallback: t("Codex 实例") });
    if (!(await appConfirm(tf("确定结束“{0}”吗？", [name]), { title: t("结束实例"), confirmText: t("结束"), danger: true }))) return;
    setMultiLaunchActionPending(instance.instanceId);
    try {
      const result = await run(() => call<CommandResult<{ instanceId?: string }>>("stop_codex_multi_instance", {
        request: { instanceId: instance.instanceId },
      }));
      if (result) showNotice(t("结束 Codex 实例"), result.message, result.status);
      await refreshMultiLaunchInstances(true);
    } finally {
      setMultiLaunchActionPending(null);
    }
  };

  /** 启动 Codex；返回是否确认启动成功（托盘面板据此决定要不要把主窗口叫出来看提示）。 */
  const launch = async (): Promise<boolean> => {
    if (launchPendingRef.current) return false;
    launchPendingRef.current = true;
    setLaunchPending(true);
    try {
      const result = await launchCommand("launch_codex_plus");
      if (!result) return false;
      if (!isSuccessStatus(result.status)) {
        showNotice(t("启动任务"), result.message, result.status);
        return false;
      }
      showNotice(t("启动任务"), t("正在等待 Codex 启动结果…"), "accepted");
      const completion = await waitForLaunchCompletion(result.launchStartedAtMs);
      showLaunchCompletionNotice(t("启动任务"), completion, result.launchStartedAtMs);
      return Boolean(
        completion
        && resolveLaunchStatus(completion.latest_launch, result.launchStartedAtMs ?? 0) === "success",
      );
    } finally {
      launchPendingRef.current = false;
      setLaunchPending(false);
    }
  };

  const restart = async (syncActiveRelay = false) => {
    if (launchPendingRef.current) return false;
    launchPendingRef.current = true;
    setLaunchPending(true);
    try {
      const result = await launchCommand("restart_codex_plus", syncActiveRelay);
      if (!result) return false;
      if (!isSuccessStatus(result.status)) {
        showNotice(t("重启 Codex"), result.message, result.status);
        return false;
      }
      showNotice(
        t("重启 Codex"),
        result.nativeBrowserRestoreFailed
          ? t("原生浏览器文件恢复失败，仍会继续启动。")
          : t("正在等待 Codex 重新启动…"),
        result.nativeBrowserRestoreFailed ? "failed" : "accepted",
      );
      const completion = await waitForLaunchCompletion(result.launchStartedAtMs);
      showLaunchCompletionNotice(t("重启 Codex"), completion, result.launchStartedAtMs);
      const succeeded = Boolean(
        completion
        && resolveLaunchStatus(completion.latest_launch, result.launchStartedAtMs ?? 0) === "success",
      );
      if (succeeded) setDreamSkinRestartPending(false);
      return succeeded;
    } finally {
      launchPendingRef.current = false;
      setLaunchPending(false);
    }
  };

  /** 侧栏 Codex 行的「停止」：结束主 Codex，和重启共用停止流程。 */
  const stopCodex = async (): Promise<boolean> => {
    if (launchPendingRef.current) return false;
    launchPendingRef.current = true;
    setLaunchPending(true);
    try {
      const result = await launchCommand("stop_codex_plus");
      if (result) showNotice(t("停止 Codex"), result.message, result.status);
      await refreshOverview(true);
      return Boolean(result && isSuccessStatus(result.status));
    } finally {
      launchPendingRef.current = false;
      setLaunchPending(false);
    }
  };

  const launchCommand = async (command: "launch_codex_plus" | "restart_codex_plus" | "stop_codex_plus", syncActiveRelay = false) => {
    const result = await run(() =>
      call<LaunchCommandResult>(command, {
        request: {
          appPath: launchForm.appPath,
          debugPort: numberOrDefault(launchForm.debugPort, 9229),
          helperPort: numberOrDefault(launchForm.helperPort, 57321),
          syncActiveRelay,
        },
      }),
    );
    return result;
  };

  const waitForLaunchCompletion = async (requestStartedAtMs?: number) => {
    if (!requestStartedAtMs) {
      await refreshOverview(true);
      return null;
    }
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
      const result = await run(() => call<OverviewResult>("load_overview"));
      if (result) {
        setOverview(result);
        const resolution = resolveLaunchStatus(result.latest_launch, requestStartedAtMs);
        if (resolution === "success" || resolution === "failed") return result;
      }
      await new Promise((resolve) => window.setTimeout(resolve, 400));
    }
    await refreshOverview(true);
    return null;
  };

  const showLaunchCompletionNotice = (title: string, result: OverviewResult | null, requestedAt?: number) => {
    const notice = launchCompletionNotice(result?.latest_launch ?? null, requestedAt ?? 0);
    showNotice(title, t(notice.message), notice.status);
  };

  const repairShortcuts = async () => {
    const result = await run(() => call<InstallResult>("repair_shortcuts"));
    if (result) {
      showNotice(t("快捷方式修复"), result.message, result.status);
      await refreshOverview(true);
    }
  };

  const setWatcherEnabled = async (enabled: boolean) => {
    const result = await run(() => call<WatcherResult>("set_watcher_enabled", { enabled }));
    if (result) {
      setWatcher(result);
      showNotice(t("开机打开 Codex++"), result.message, result.status);
    }
  };

  const setWatcherStartMinimized = async (minimized: boolean) => {
    const result = await run(() => call<WatcherResult>("set_watcher_start_minimized", { minimized }));
    if (result) {
      setWatcher(result);
      showNotice(t("开机启动自动最小化"), result.message, result.status);
    }
  };

  const setWatcherStartCodex = async (startCodex: boolean) => {
    const result = await run(() => call<WatcherResult>("set_watcher_start_codex", { startCodex }));
    if (result) {
      setWatcher(result);
      showNotice(t("登录时自动启动 Codex"), result.message, result.status);
    }
  };

  const setWatcherStartCodexOnOpen = async (enabled: boolean) => {
    const result = await run(() => call<WatcherResult>("set_watcher_start_codex_on_open", { enabled }));
    if (result) {
      setWatcher(result);
      showNotice(t("打开 Codex++ 时自动启动 Codex"), result.message, result.status);
    }
  };

  // 启动时的后台检查早于设置加载：先单独读一次更新偏好，避免漏掉「跳过此版本」和「关闭自动检查」。
  const ensureUpdatePreferences = async () => {
    try {
      const result = await call<SettingsResult>("load_settings");
      if (!updateNotifications.current.preferencesLoaded) updateNotifications.current.setPreferences(updatePreferencesFrom(result.settings));
    } catch {
      // 读不到设置时按默认（自动检查、不跳过）继续，不打扰用户。
    }
  };

  const checkUpdate = async (silent = false) => {
    if (silent) {
      if (!updateNotifications.current.preferencesLoaded) await ensureUpdatePreferences();
      // 关闭「启动时自动检查更新」后，启动与定时的后台检查都不再联网；手动检查不受影响。
      if (!updateNotifications.current.autoCheckEnabled()) return;
    }
    if (updateInstallPending.current) return;
    if (updateCheckPending.current) {
      // 手动检查复用正在进行的后台请求，但仍按用户主动检查显示结果。
      if (!silent) updateCheckPending.current.manual = true;
      return;
    }
    const request = { manual: !silent };
    updateCheckPending.current = request;
    try {
      const result = await call<UpdateResult>("check_update");
      // 后台检查失败时保留上次可用的安装包，手动检查仍显示错误。
      if (request.manual || result.status !== "failed") setUpdate(result);
      if (result.status !== "failed") void downloadUpdateInBackground(result);
      // 后台检查到新版本不弹窗：安装包在后台下载，侧栏底部卡片提示「已下载 · 点击安装」，点了再弹窗。
      // 只有用户主动点「检查更新」才直接弹窗。
      if (!request.manual) return;
      if (updateNotifications.current.shouldPrompt(result, true)) {
        setUpdatePrompt(result);
      } else {
        showNotice(t("GitHub Release 检查"), result.message, result.status);
      }
    } catch (error) {
      if (request.manual) showNotice(t("GitHub Release 检查"), stringifyError(error), "failed");
    } finally {
      updateCheckPending.current = null;
    }
  };

  const closeUpdatePrompt = () => {
    if (updatePrompt?.latestVersion) updateNotifications.current.dismiss(updatePrompt.latestVersion);
    setUpdatePrompt(null);
  };

  // 「跳过此版本」：本次会话立即生效，并只把这一个键写进 settings.json；
  // 写完只合并该键和版本号，别处没保存的草稿保持不动。
  const skipUpdateVersion = async (version: string) => {
    updateNotifications.current.skipVersion(version);
    setUpdatePrompt(null);
    try {
      const result = await call<SettingsResult>("skip_update_version", { version });
      if (!isSuccessStatus(result.status)) {
        showNotice(t("跳过此版本"), result.message, "failed");
        return;
      }
      const fresh = normalizeSettings(result.settings);
      setSettings(result);
      setSettingsForm((current) => ({ ...current, updateSkippedVersion: fresh.updateSkippedVersion, revision: fresh.revision }));
    } catch (error) {
      showNotice(t("跳过此版本"), stringifyError(error), "failed");
    }
  };

  const releaseOf = (selected: UpdateResult | null | undefined) =>
    selected?.latestVersion && selected.assetName && selected.assetUrl
      ? {
          version: selected.latestVersion,
          url: "",
          body: selected.releaseSummary ?? "",
          asset_name: selected.assetName,
          asset_url: selected.assetUrl,
        }
      : null;

  /**
   * 「有可用更新时自动下载」：检查到新版本就在后台把安装包下好，只下载、不安装；
   * 用户点「安装」时 perform_update 直接用下好的安装包。失败不打扰用户，点「安装」时会重新下载。
   */
  const downloadUpdateInBackground = async (result: UpdateResult) => {
    const release = releaseOf(result);
    if (!result.updateAvailable || !release) return;
    if (result.downloaded) {
      setUpdateDownload({ version: release.version, state: "ready", percent: 100 });
      return;
    }
    if (!updateNotifications.current.preferencesLoaded) await ensureUpdatePreferences();
    if (!updateNotifications.current.autoDownloadEnabled()) return;
    if (updateDownloadPending.current === release.version || updateInstallPending.current) return;
    updateDownloadPending.current = release.version;
    setUpdateDownload({ version: release.version, state: "downloading", percent: null });
    let finish = () => {};
    updateDownloadPromise.current = new Promise<void>((resolve) => { finish = resolve; });
    let unlisten: (() => void) | null = null;
    try {
      unlisten = await listen<UpdateProgressEvent>("update-download-progress", (event) => {
        const progress = event.payload;
        setUpdateDownload({ version: release.version, state: "downloading", percent: progress.stage === "downloading" ? progress.percent ?? null : 100 });
      });
      const downloaded = await call<UpdateResult>("download_update", { release });
      if (isSuccessStatus(downloaded.status)) {
        setUpdateDownload({ version: release.version, state: "ready", percent: 100 });
        setUpdate((current) => (current?.latestVersion === release.version ? { ...current, downloaded: true } : current));
      } else {
        setUpdateDownload({ version: release.version, state: "failed", percent: null, message: downloaded.message });
      }
    } catch (error) {
      setUpdateDownload({ version: release.version, state: "failed", percent: null, message: stringifyError(error) });
    } finally {
      updateDownloadPending.current = null;
      updateDownloadPromise.current = null;
      finish();
      unlisten?.();
    }
  };

  const performUpdate = async (selectedUpdate = update) => {
    if (updateInstallPending.current) return;
    const release = releaseOf(selectedUpdate);
    if (!release) {
      showNotice(t("更新安装"), t("请先检查更新并选择可下载的 Release asset。"), "failed");
      return;
    }
    updateInstallPending.current = true;
    setUpdateInstallProgress({
      active: true,
      percent: 0,
      indeterminate: true,
      message: updateDownloadPromise.current ? t("正在等后台下载完成…") : t("正在准备下载安装包…"),
    });
    // 后台还在下这个版本：等它下完，perform_update 会直接用下好的安装包。
    if (updateDownloadPromise.current) await updateDownloadPromise.current;
    const fallbackTimer = window.setTimeout(() => {
      setUpdateInstallProgress((current) =>
        current.active && current.indeterminate
          ? { ...current, message: t("更新包仍在处理中，请保持窗口打开；完成后会自动更新状态。") }
          : current,
      );
    }, 15_000);
    let unlisten: (() => void) | null = null;
    try {
      unlisten = await listen<UpdateProgressEvent>("update-progress", (event) => {
        const progress = event.payload;
        const message =
          progress.stage === "downloading"
            ? t("正在下载安装包…")
            : progress.stage === "writing"
              ? t("正在写入安装包并准备安装…")
              : t("正在启动安装器…");
        setUpdateInstallProgress({
          active: true,
          percent: progress.stage === "downloading" ? (progress.percent ?? 0) : 0,
          indeterminate: progress.stage !== "downloading" || progress.percent == null,
          message,
        });
      });
      const result = await run(() => call<UpdateResult>("perform_update", { release }));
      if (result) {
        setUpdate({ ...selectedUpdate!, ...result });
        setUpdateInstallProgress({
          active: false,
          percent: result.progress ?? 100,
          indeterminate: false,
          message: result.message,
          failed: result.status === "failed",
        });
        showNotice(t("更新安装"), result.message, result.status);
      } else {
        setUpdateInstallProgress({
          active: false,
          percent: 100,
          indeterminate: false,
          message: t("安装包更新失败，请查看错误提示后重试。"),
          failed: true,
        });
      }
    } catch (error) {
      const message = stringifyError(error);
      setUpdateInstallProgress({ active: false, percent: 0, indeterminate: false, message, failed: true });
      showNotice(t("更新安装"), message, "failed");
    } finally {
      updateInstallPending.current = false;
      window.clearTimeout(fallbackTimer);
      unlisten?.();
    }
  };

  const saveSettings = async () => {
    const next = normalizeSettings(settingsForm);
    const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
    if (result) {
      if (isSuccessStatus(result.status)) {
        setSettings(result);
        setSettingsForm(normalizeSettings(result.settings));
      }
      showNotice(t("设置保存"), route === "weixin" && result.status === "ok"
        ? t("已保存，下一条微信消息生效。")
        : result.message, result.status);
    }
  };

  const saveSettingsValue = async (next: BackendSettings, silent = true) => {
    const normalized = normalizeSettings(next);
    const result = await run(() => call<SettingsResult>("save_settings", { settings: normalized }));
    if (result && isSuccessStatus(result.status)) {
      const saved = normalizeSettings(result.settings);
      setSettings(result);
      setSettingsForm(saved);
      if (!silent) showNotice(t("设置保存"), result.message, result.status);
      return saved;
    }
    if (result) showNotice(t("设置保存"), result.message, result.status);
    // 保留草稿供用户重载后合并，不能拿最新版本号自动重试旧快照。
    return null;
  };

  /// 自动保存：不弹成功提示；校验不过就不保存，把原因交给页面显示，避免输入到一半就弹窗。
  ///
  /// 保存期间用户又改了表单时，保留新草稿、只同步版本号（否则下一次保存会撞版本冲突），
  /// 草稿仍是「有修改」，由页面下一轮再存。
  const autoSaveSettings = async (snapshot: BackendSettings): Promise<AutoSaveOutcome> => {
    const normalized = normalizeSettings(snapshot);
    try {
      const result = await call<SettingsResult>("save_settings", { settings: normalized });
      if (!isSuccessStatus(result.status)) return { status: "failed", message: result.message };
      const saved = normalizeSettings(result.settings);
      setSettings(result);
      setSettingsForm((current) => (backendSettingsEqual(current, snapshot) ? saved : { ...current, revision: saved.revision }));
      return { status: "saved" };
    } catch (error) {
      return { status: "failed", message: stringifyError(error) };
    }
  };

  const beginWeixinQrLogin = async () => {
    const result = await run(() => call<WeixinQrResult>("weixin_connect_qr_start", {
      baseUrl: settingsForm.weixinConnectBaseUrl,
      routeTag: settingsForm.weixinConnectRouteTag,
    }));
    if (!result) return;
    setWeixinQr(result);
    showResultNotice(t("微信扫码登录"), result, { silentSuccess: true });
  };

  const startWeixinConnect = async () => {
    const saved = await saveSettingsValue(settingsForm, true);
    if (!saved) return;
    const result = await run(() => call<WeixinConnectStatusResult>("weixin_connect_start"));
    if (!result) return;
    setWeixinStatus(result);
    showResultNotice(t("微信连接"), result);
    await refreshSettings(true);
  };

  const stopWeixinConnect = async () => {
    const result = await run(() => call<WeixinConnectStatusResult>("weixin_connect_stop"));
    if (!result) return;
    setWeixinStatus(result);
    showResultNotice(t("微信连接"), result);
    await refreshSettings(true);
  };

  const chooseWeixinPath = async (kind: "workDir" | "codexPath") => {
    try {
      const selected = await open({
        directory: kind === "workDir",
        multiple: false,
        title: kind === "workDir" ? t("选择微信连接工作目录") : t("选择 Codex CLI"),
      });
      if (typeof selected !== "string" || !selected.trim()) return;
      if (kind === "codexPath") {
        await saveSettingsValue({ ...settingsForm, weixinConnectCodexPath: selected.trim() }, false);
      } else {
        setSettingsForm((current) => ({ ...current, weixinConnectWorkDir: selected.trim() }));
      }
    } catch (error) {
      showNotice(t("微信连接"), stringifyError(error), "failed");
    }
  };

  const useDesktopCodexCli = async () => {
    const result = await run(() => call<DesktopCodexCliResult>("find_desktop_codex_cli"));
    if (!result) return;
    const path = result.path?.trim();
    if (isSuccessStatus(result.status) && path) {
      const saved = await saveSettingsValue({ ...settingsForm, weixinConnectCodexPath: path }, false);
      if (!saved) return;
    }
    showResultNotice(t("Codex CLI 路径"), result);
  };

  const resetSettings = async () => {
    const result = await run(() => call<SettingsResult>("reset_settings"));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showNotice(t("设置重置"), result.message, result.status);
    }
  };

  const exportSettings = async () => {
    try {
      const selected = await saveDialog({
        title: t("导出 Codex++ 配置"),
        defaultPath: "codex-plus-plus-settings.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!selected) return;
      const result = await run(() => call<CommandResult<{ path: string }>>("export_settings", {
        path: selected,
        settings: normalizeSettings(settingsForm),
      }));
      if (result) showNotice(t("导出 Codex++ 配置"), result.message, result.status);
    } catch (error) {
      showNotice(t("导出 Codex++ 配置"), stringifyError(error), "failed");
    }
  };

  const importSettings = async () => {
    try {
      const selected = await open({
        title: t("导入 Codex++ 配置"),
        multiple: false,
        directory: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (typeof selected !== "string" || !selected.trim()) return;
      const result = await run(() => call<SettingsResult>("preview_import_settings", { path: selected }));
      if (!result || !isSuccessStatus(result.status)) {
        if (result) showNotice(t("导入 Codex++ 配置"), result.message, result.status);
        return;
      }
      setSettings(settings ?? result);
      setSettingsForm(normalizeSettings(result.settings));
      showNotice(t("导入 Codex++ 配置"), result.message, "ok");
    } catch (error) {
      showNotice(t("导入 Codex++ 配置"), stringifyError(error), "failed");
    }
  };

  const chooseSettingsPath = async () => {
    try {
      const selected = await saveDialog({
        title: t("选择配置文件路径"),
        defaultPath: "settings.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!selected) return;
      const result = await run(() => call<SettingsResult>("set_settings_path", { path: selected }));
      if (result) {
        if (isSuccessStatus(result.status)) {
          setSettings(result);
          setSettingsForm(normalizeSettings(result.settings));
        }
        showNotice(t("配置文件路径"), result.message, result.status);
      }
    } catch (error) {
      showNotice(t("配置文件路径"), stringifyError(error), "failed");
    }
  };

  const resetSettingsPath = async () => {
    const result = await run(() => call<SettingsResult>("reset_settings_path"));
    if (result) {
      if (isSuccessStatus(result.status)) {
        setSettings(result);
        setSettingsForm(normalizeSettings(result.settings));
      }
      showNotice(t("配置文件路径"), result.message, result.status);
    }
  };

  const refreshAds = async (silent = false) => {
    const result = await run(() => call<AdsResult>("load_ads"));
    if (result) {
      setAds(result);
      setSponsorAds(result);
      if (!silent) showResultNotice(t("推荐内容"), result, { silentSuccess: true });
    }
  };

  const refreshProviderSyncTargets = async (silent = false) => {
    const result = await run(() => call<ProviderSyncTargetsResult>("load_provider_sync_targets"));
    if (result) {
      setProviderSyncTargets(result);
      const targets = result.targets ?? [];
      const saved = settingsForm.providerSyncLastSelectedProvider;
      const preferred = preferredProviderSyncTarget(targets, result.currentProvider, saved);
      setSelectedProviderSyncTarget(preferred);
      if (!silent && !isSuccessStatus(result.status)) showNotice(t("Provider 同步目标"), result.message, result.status);
    }
    return result;
  };

  const refreshSessionIndexRepairReport = async (isCurrent = () => true) => {
    if (sessionIndexReportLoading.current || sessionIndexRepairRunning.current) return;
    sessionIndexReportLoading.current = true;
    try {
      // 持久报告读取失败时保留现有结果，后台刷新不触发全局通知或忙碌状态。
      const result = await call<CommandResult<{ report: SessionIndexRepairReport | null }>>(
        "load_session_index_repair_report",
      );
      if (isCurrent() && !sessionIndexRepairRunning.current && isSuccessStatus(result.status)) {
        setSessionIndexRepairReportError(null);
        setSessionIndexRepairReport((previous) => {
          if ((previous?.checkedAtMs ?? 0) > (result.report?.checkedAtMs ?? 0)) return previous;
          return keepIfSame(previous, result.report);
        });
      } else if (isCurrent() && !isSuccessStatus(result.status)) {
        setSessionIndexRepairReportError(result.message || t("读取会话索引修复报告失败"));
      }
    } catch (error) {
      if (isCurrent()) {
        setSessionIndexRepairReportError(
          tf("读取会话索引修复报告失败：{0}", [stringifyError(error)]),
        );
      }
    } finally {
      sessionIndexReportLoading.current = false;
    }
  };

  const repairSessionIndex = async () => {
    if (sessionIndexRepairRunning.current || providerSyncProgress.active) return;
    sessionIndexRepairRunning.current = true;
    setSessionIndexRepairActive(true);
    try {
      const result = await run(() => call<CommandResult<SessionIndexRepairReport>>("repair_session_index"));
      if (result) {
        if (isSuccessStatus(result.status)) {
          setSessionIndexRepairReport(result);
          await refreshLocalSessions(true);
        }
        showNotice(t("修复会话索引"), result.message, result.status);
      }
    } finally {
      sessionIndexRepairRunning.current = false;
      setSessionIndexRepairActive(false);
    }
  };

  const recoverProviderSync = async () => {
    const backupDir = providerSyncProgress.result?.backupDir;
    if (providerSyncProgress.active || !backupDir) return;
    if (!(await appConfirm(t("恢复这次未完成的修复？期间被其他操作修改的数据会保留，并报告冲突。"), { title: t("恢复修复"), confirmText: t("恢复") }))) return;
    setProviderSyncProgress((current) => ({ ...current, active: true, message: t("正在恢复…") }));
    try {
      const result = await run(() => call<CommandResult<ProviderSyncPayload>>("recover_provider_sync", { backupDir }));
      if (result) {
        setProviderSyncProgress({ active: false, percent: 100, message: result.message, result });
        showNotice(t("恢复历史会话修复"), result.message, result.status);
        await refreshLocalSessions(true);
      }
    } finally {
      setProviderSyncProgress((current) => ({ ...current, active: false }));
    }
  };

  const syncProvidersNow = async () => {
    if (sessionIndexRepairRunning.current) return;
    if (providerSyncProgress.active) return;
    setProviderSyncProgress({
      active: true,
      percent: 0,
      message: t("正在扫描历史会话与索引…"),
      result: null,
    });
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<ProviderSyncStreamProgress>("provider-sync-progress", (event) => {
        const progress = event.payload;
        const message = (() => {
          switch (progress.phase) {
            case "scanning":
              return t("正在扫描历史会话与索引…");
            case "planning":
              return t("正在检查会话 provider 标记…");
            case "backing_up":
              return t("正在创建修复备份…");
            case "rewriting":
              return t("正在写入会话修复…");
            case "updating_indexes":
              return t("正在更新会话索引…");
            case "rolling_back":
              return t("正在回滚已写入的会话…");
            case "complete":
              return t("正在完成历史会话修复…");
          }
        })();
        setProviderSyncProgress((current) => {
          if (!current.active) return current;
          return {
            ...current,
            percent: Math.max(current.percent, providerSyncStreamPercent(progress)),
            message,
          };
        });
      });
      const targetProvider = selectedProviderSyncTarget || undefined;
      const result = await run(() =>
        call<CommandResult<ProviderSyncPayload>>("sync_providers_now", { targetProvider }),
      );
      if (result) {
        const syncSucceeded = isSuccessStatus(result.status) && result.syncStatus === "synced";
        let finalResult =
          isSuccessStatus(result.status) && !syncSucceeded
            ? {
                ...result,
                status: "failed",
                message: result.syncMessage || t("历史会话修复失败，请查看错误提示后重试。"),
              }
            : result;
        let cleanupFailure: { status: Status; message: string } | null = null;
        if (syncSucceeded) {
          const preview = await run(() =>
            call<CommandResult<SessionIndexCleanupPreviewPayload>>("preview_session_index_cleanup"),
          );
          if (!preview) {
            cleanupFailure = {
              status: "failed",
              message: t("幽灵任务索引处理失败，请查看错误提示后重试。"),
            };
          } else if (isSuccessStatus(preview.status) && preview.candidates.length > 0) {
            const selectedIds = await selectSessionIndexCleanupCandidates(preview.candidates);
            if (selectedIds?.length) {
              const cleanup = await run(() =>
                call<CommandResult<SessionIndexCleanupApplyPayload>>("apply_session_index_cleanup", {
                  snapshotSha256: preview.snapshotSha256,
                  threadIds: selectedIds,
                }),
              );
              if (cleanup && isSuccessStatus(cleanup.status)) {
                finalResult = {
                  ...result,
                  prunedSessionIndexEntries: cleanup.prunedEntries ?? 0,
                };
              } else {
                cleanupFailure = cleanup ?? {
                  status: "failed",
                  message: t("幽灵任务索引处理失败，请查看错误提示后重试。"),
                };
              }
            }
          } else if (!isSuccessStatus(preview.status)) {
            cleanupFailure = preview;
          }
        }
        const completion = resolveProviderSyncCompletion(finalResult, cleanupFailure);
        setProviderSyncProgress({
          active: false,
          percent: 100,
          message:
            completion.progressMessage ??
            (isSuccessStatus(completion.result.status)
              ? providerSyncProgressMessage(completion.result)
              : completion.result.message),
          result: completion.result,
        });
        if (targetProvider && syncSucceeded) {
          const next = {
            ...settingsForm,
            providerSyncLastSelectedProvider: targetProvider,
            providerSyncSavedProviders: Array.from(
              new Set([...(settingsForm.providerSyncSavedProviders ?? []), targetProvider]),
            ).sort(),
          };
          setSettingsForm(next);
        }
        await refreshProviderSyncTargets(true);
        const noticeTitle =
          completion.noticeKind === "cleanup" ? t("清理幽灵任务索引") : t("历史会话修复");
        showNotice(
          noticeTitle,
          completion.result.message,
          completion.result.status,
        );
      } else {
        setProviderSyncProgress({
          active: false,
          percent: 100,
          message: t("历史会话修复失败，请查看错误提示后重试。"),
          result: null,
        });
      }
    } catch (error) {
      const message = stringifyError(error);
      setProviderSyncProgress({
        active: false,
        percent: 100,
        message,
        result: null,
      });
      showNotice(t("历史会话修复"), message, "failed");
    } finally {
      unlisten?.();
    }
  };

  const applyRelayInjection = async (silent = false) => {
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (settingsResult) {
      setSettings(settingsResult);
      setSettingsForm(normalizeSettings(settingsResult.settings));
      if (!isSuccessStatus(settingsResult.status)) {
        showNotice(t("设置保存"), settingsResult.message, settingsResult.status);
        return false;
      }
    } else {
      return false;
    }
    const result = await run(() => call<RelayResult>("apply_relay_injection"));
    if (result) {
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice(t("官方混入 API Key"), result.message, result.status);
    }
    return !!result && isSuccessStatus(result.status) && result.configured;
  };

  const applyPureApiInjection = async (silent = false) => {
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (settingsResult) {
      setSettings(settingsResult);
      setSettingsForm(normalizeSettings(settingsResult.settings));
      if (!isSuccessStatus(settingsResult.status)) {
        showNotice(t("设置保存"), settingsResult.message, settingsResult.status);
        return false;
      }
    } else {
      return false;
    }
    const result = await run(() => call<RelayResult>("apply_pure_api_injection"));
    if (result) {
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice(t("纯 API 模式"), result.message, result.status);
    }
    return !!result && isSuccessStatus(result.status) && result.configured;
  };

  const clearRelayInjection = async (silent = false) => {
    const result = await run(() => call<RelayResult>("clear_relay_injection"));
    if (result) {
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice(t("官方登录模式"), result.message, result.status);
    }
    return !!result && isSuccessStatus(result.status) && !result.configured;
  };

  const saveRelayFile = async (kind: "config" | "auth", contents: string, silent = false) => {
    const result = await run(() => call<RelayFilesResult>("save_relay_file", { request: { kind, contents } }));
    if (result) {
      setRelayFiles(result);
      if (!silent || !isSuccessStatus(result.status)) {
        showNotice(kind === "config" ? "config.toml" : "auth.json", result.message, result.status);
      }
    }
  };

  /**
   * MCP 页的开关 / 编辑 / 删除：一次后端调用完成改公共配置、保存设置、同步 live config.toml
   * （原来是三次串行往返）。tomlBody 为 null 表示删除。返回是否全部成功。
   */
  const applyContextEntry = async (next: BackendSettings, kind: ContextKind, id: string, tomlBody: string | null) => {
    const result = await run(() => call<AppliedContextEntryResult>("apply_context_entry", { request: { settings: next, kind, id, tomlBody } }));
    if (!result) return false;
    if (result.saved) {
      setSettings({ status: "ok", message: result.message, ...result.saved });
      setSettingsForm(normalizeSettings(result.saved.settings));
    }
    if (result.liveEntries) setLiveContextEntries(result.liveEntries);
    if (!isSuccessStatus(result.status)) showResultNotice(t("MCP&插件"), result);
    return isSuccessStatus(result.status);
  };

  const previewMcpServersJson = async (json: string) => {
    const result = await run(() => call<McpImportPreviewResult>("preview_mcp_servers_json", { json }));
    if (result && !isSuccessStatus(result.status)) showResultNotice(t("MCP 导入"), result);
    return result;
  };

  const importMcpServersJson = async (next: BackendSettings, json: string) => {
    const result = await run(() =>
      call<ContextEntriesResult>("import_mcp_servers_json", { request: { settings: next, json } }),
    );
    if (!result) return null;
    let normalized = normalizeSettings(result.settings);
    const saveResult = await run(() => call<SettingsResult>("save_settings", { settings: normalized }));
    if (saveResult) {
      setSettings(saveResult);
      normalized = normalizeSettings(saveResult.settings);
    }
    setSettingsForm(normalized);
    showResultNotice(t("MCP 导入"), result);
    return isSuccessStatus(result.status) ? normalized : null;
  };

  const extractRelayCommonConfig = async (configContents: string) => {
    const result = await run(() =>
      call<ExtractRelayCommonConfigResult>("extract_relay_common_config", {
        request: { configContents },
      }),
    );
    if (result) showResultNotice(t("通用配置文件"), result);
    return result && isSuccessStatus(result.status) ? result : null;
  };

  const testRelayProfile = async (profile: RelayProfile) => {
    const result = await run(() => call<RelayProfileTestResult>("test_relay_profile", { profile }));
    if (result) showNotice(t("供应商测试"), result.message, result.status);
  };

  const diagnoseRelayProfile = async (profile: RelayProfile) => {
    const result = await run(() => call<ProviderDoctorResult>("diagnose_relay_profile", { profile }));
    // 结果在诊断弹窗里逐项展示，不再另弹一条提示。
    return result ?? null;
  };

  const fetchRelayProfileModels = async (profile: RelayProfile) => {
    const result = await run(() => call<RelayProfileModelsResult>("fetch_relay_profile_models", { profile }));
    if (result) showNotice(t("模型列表"), result.message, result.status);
    return result && isSuccessStatus(result.status) ? result.models : null;
  };

  const switchOfficialMode = async () => {
    const switched = await clearRelayInjection(true);
    if (!switched) return;
    showNotice(t("官方登录模式"), t("已切回官方登录。"), "ok");
  };

  const switchPureApiMode = async () => {
    const switched = await applyPureApiInjection(true);
    if (!switched) return;
    showNotice(t("纯 API 模式"), t("已切换到纯 API。"), "ok");
  };

  // 同步占住切换：切换前的校验和快照回填是异步的，state 要到下一次渲染才更新，连点两下会发起两次切换。
  const relaySwitchingRef = useRef(false);
  const switchRelayProfile = async (
    next: BackendSettings,
    previousActiveRelayId = settingsForm.activeRelayId,
    liveConflictResolution?: LiveConflictResolution,
  ): Promise<boolean> => {
    if (relaySwitchingRef.current) {
      showNotice(t("供应商切换中"), t("上一次切换还没有完成，请稍后再试。"), "failed");
      return false;
    }
    relaySwitchingRef.current = true;
    try {
      return await performRelaySwitch(next, previousActiveRelayId, liveConflictResolution);
    } finally {
      relaySwitchingRef.current = false;
    }
  };

  const performRelaySwitch = async (
    next: BackendSettings,
    previousActiveRelayId: string,
    liveConflictResolution?: LiveConflictResolution,
  ): Promise<boolean> => {
    let switchSettings = normalizeSettings(next);
    if (!switchSettings.relayProfilesEnabled) {
      showNotice(t("供应商配置已关闭"), t("当前不会写入 Codex config.toml / auth.json。打开供应商配置总开关后再切换。"), "failed");
      return false;
    }
    const targetBeforeSnapshot = activeRelayProfile(switchSettings);
    logDiagnostic("switchRelayProfile.start", {
      currentRelayId: settingsForm.activeRelayId,
      targetRelayId: switchSettings.activeRelayId,
      targetRelayName: targetBeforeSnapshot.name,
      targetRelayMode: targetBeforeSnapshot.relayMode,
    });
    const selectedBeforeSave = activeRelayProfile(switchSettings);
    const validationError = relayProfileSwitchValidation(selectedBeforeSave, switchSettings);
    if (validationError) {
      logDiagnostic("switchRelayProfile.validation_failed", {
        targetRelayId: selectedBeforeSave.id,
        targetRelayName: selectedBeforeSave.name,
        error: validationError,
      });
      showNotice(t("供应商配置可能不正确"), validationError, "failed");
      return false;
    }
    // 用户已在冲突弹窗里做了选择时，回填交给后端按选择处理，不再先跑一遍注定失败的回填。
    if (!liveConflictResolution) {
      switchSettings = await snapshotActiveRelayFilesBeforeSwitch(switchSettings, previousActiveRelayId);
    }
    const selectedAfterSave = activeRelayProfile(switchSettings);
    const command = relayProfileSwitchCommand(selectedAfterSave);

    logDiagnostic("switchRelayProfile.apply_start", {
      targetRelayId: selectedAfterSave.id,
      targetRelayName: selectedAfterSave.name,
      previousActiveRelayId,
      command,
      liveConflictResolution: liveConflictResolution ?? null,
    });
    let liveConflict: { profileId: string; profileName: string } | null = null;
    let switched = false;
    setRelaySwitching(true);
    try {
      const result = await run(() =>
        call<RelaySwitchResult>("switch_relay_profile", {
          request: { settings: switchSettings, previousActiveRelayId, liveConflictResolution },
        }),
      );
      if (!result) {
        logDiagnostic("switchRelayProfile.apply_no_result", {
          targetRelayId: selectedAfterSave.id,
        });
        return false;
      }
      const selectedSettings = normalizeSettings(result.settings);
      setSettings({
        revision: selectedSettings.revision || "",
        status: result.status,
        message: result.message,
        settings: selectedSettings,
        settings_path: result.settingsPath,
        user_scripts: result.user_scripts as UserScriptInventory,
      });
      setSettingsForm(selectedSettings);
      await refreshRelayFiles(true);
      if (!isSuccessStatus(result.status)) {
        logDiagnostic("switchRelayProfile.apply_failed", {
          targetRelayId: selectedAfterSave.id,
          status: result.status,
          message: result.message,
          activeRelayId: selectedSettings.activeRelayId,
          liveConflict: !!result.liveConflict,
        });
        // 实时 endpoint 被外部改过（#2666）：不只报错，弹窗让用户选怎么继续。
        // 这里不能 return：弹窗在 finally 之后，return 会让它永远走不到。
        if (result.liveConflict && !liveConflictResolution) {
          liveConflict = result.liveConflict;
        } else {
          showNotice(t("供应商切换"), result.message, result.status);
        }
      } else {
        const currentSelected = activeRelayProfile(selectedSettings);
        logDiagnostic("switchRelayProfile.ok", {
          targetRelayId: currentSelected.id,
          status: result.status,
        });
        if (liveConflictResolution) showNotice(t("供应商切换"), result.message, result.status);
        switched = true;
      }
    } finally {
      setRelaySwitching(false);
    }
    const conflict = liveConflict;
    if (!conflict) return switched;
    // 从托盘面板发起时主窗口可能是隐藏的：弹窗前先把主窗口叫出来，否则弹窗没人看得见。
    void invoke("tray_panel_action", { action: "open" }).catch(() => {});
    const choice = await new Promise<LiveConflictResolution | null>((resolve) => {
      setLiveConflictDialog({ profileName: conflict.profileName, resolve });
    });
    logDiagnostic("switchRelayProfile.live_conflict_choice", {
      profileId: conflict.profileId,
      choice: choice ?? "cancel",
    });
    if (!choice) return false;
    return performRelaySwitch(next, previousActiveRelayId, choice);
  };

  const snapshotActiveRelayFilesBeforeSwitch = async (
    next: BackendSettings,
    previousActiveRelayId: string,
  ): Promise<BackendSettings> => {
    if (!shouldBackfillRelayProfileBeforeSwitch(previousActiveRelayId, next.activeRelayId)) return next;
    const profileId = previousActiveRelayId.trim();
    const result = await run(() =>
      call<SettingsBackfillResult>("backfill_relay_profile_from_live", {
        request: { settings: next, profileId },
      }),
    );
    if (!result) return next;
    // endpoint 被外部改过（#2666）：紧接着的切换会弹窗让用户选择，这里不重复提示。
    if (result.liveConflict) return next;
    const normalized = normalizeSettings(result.settings);
    // degraded = 后端跳过了回填（例如 live config.toml 有语法错误，issue #618）。
    // 切换本身照常继续，只是用原配置走；提示一下原因即可，不能按 failed 处理，
    // 否则会把用户卡在中转态回不去。
    if (isDegradedStatus(result.status)) {
      showNotice(t("供应商切换"), result.message, result.status);
      return normalized;
    }
    if (!isSuccessStatus(result.status)) {
      showNotice(t("供应商切换"), result.message, result.status);
      return next;
    }
    return normalized;
  };

  const copyText = async (text: string, message: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch (error) {
      showNotice(t("复制失败"), stringifyError(error), "failed");
    }
  };

  const openExternalUrl = async (url: string) => {
    const result = await run(() => call<CommandResult<Record<string, unknown>>>("open_external_url", { url }));
    if (result) {
      showResultNotice(t("打开链接"), result, { silentSuccess: true });
    }
  };

  const showNotice = (title: string, message: string, status?: Status, action?: NoticeAction) => {
    setNotice({ title, message: t(message), status, action });
  };

  const exitManagerApp = async () => {
    await call<void>("manager_exit_app");
  };

  const hideManagerToTray = async () => {
    await call<void>("manager_hide_to_tray");
  };

  const showResultNotice = (
    title: string,
    result: Pick<CommandResult<unknown>, "message" | "status">,
    options: { silentSuccess?: boolean } = {},
  ) => {
    if (options.silentSuccess && isSuccessStatus(result.status)) return;
    showNotice(title, result.message, result.status);
  };

  // 正在处理的快捷面板请求数。轻量模式下主窗口可能是为请求在后台重建的，
  // 请求没做完时不能释放窗口，否则结果回不去（#2677）。
  const trayRequestsInFlight = useRef(0);
  // 主窗口仍隐藏时按轻量模式释放（窗口可见、开关关掉时后端不做任何事）。
  const releaseMainWindowWhenHidden = () => {
    if (trayRequestsInFlight.current === 0) void invoke("main_window_release_when_hidden").catch(() => {});
  };

  useEffect(() => {
    // 工具栏只依赖本地配置，不能排在广告等网络请求之后。
    // 切换语言会重载窗口，串行等待会让三个 Agent 图标长时间消失。
    void refreshTools(true);
    // 本地加载互不依赖，一起发出；设置最先发，要不要自动打开启动引导等它读回来再决定。
    const settingsLoad = refreshSettings(true);
    if (setupChecking) {
      // 本地存储说向导没出现过，不等于真没出现过：设置文件里可能留着记号（#2936）。
      void resolveSetupAutoOpen(async () => (await settingsLoad)?.managerAgentFocus).then((autoOpen) => {
        if (autoOpen) {
          // 第一次启动：打开向导，同时记下它出现过，之后只留侧栏的进度卡。
          markSetupSeen();
          setSetupOpen(true);
          setWizardOpen(true);
        }
        setSetupChecking(false);
      });
    }
    // 启动落在首页，首页就是赞助商推荐，所以启动就拉一次；它是网络请求，不挡后面的本地加载。
    void refreshAds(true);
    const localLoads = Promise.all([
      settingsLoad,
      refreshOverview(true),
      refreshEnvConflicts(true),
      refreshProviderSyncTargets(true),
      refreshPendingProviderImport(true),
      refreshPendingDreamSkinCommunity(),
    ]);
    void (async () => {
      const startup = await run(() => call<StartupResult>("startup_options"));
      const handledNavigation = await consumePendingManagerNavigation();
      if (!handledNavigation && startup?.showUpdate) {
        setSettingsFocus("app-settings-update");
        setRoute("settings");
        void checkUpdate(false);
      } else {
        void checkUpdate(true);
      }
      await localLoads;
      // 登录启动或为快捷面板请求在后台重建时窗口是隐藏的：启动流程走完后按轻量模式释放（#2677）。
      releaseMainWindowWhenHidden();
    })();
    // 只在挂载时跑一次。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    // 长期开着管理器也能发现更新；后台检查失败不打扰当前操作。
    const timer = window.setInterval(() => void checkUpdate(true), UPDATE_CHECK_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, []);

  // 主题卡片的「更多操作」是原生 details 弹层：原生不会因为点了别处就收起，
  // 会出现多个菜单同时展开。这里统一做外点收起（details 的开关是 DOM 瞬态，不经 React 状态）。
  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Element) || target.closest("[data-popover-menu]")) return;
      document.querySelectorAll<HTMLDetailsElement>("details[data-popover-menu][open]").forEach((node) => {
        node.open = false;
      });
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, []);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listen(MANAGER_NAVIGATION_EVENT, () => {
      if (disposed) return;
      void consumeNavigationRef.current();
      // 窗口被唤起或获得焦点：顺带读一次待确认的网页导入 / 一键换肤。
      void refreshPendingProviderImport(true);
      void refreshPendingDreamSkinCommunity();
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        stopListening = unlisten;
      }
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, []);

  // 托盘快捷面板发来的请求：打开某页、启动 / 重启 / 停止 Codex。
  // 复用主窗口现有流程（校验、冲突弹窗、等待启动结果），做完给面板回一个结果；
  // 没成功时把主窗口叫出来，失败提示是在主窗口里弹的。
  // 监听只注册一次，处理函数每次渲染更新到 ref 里，读到的总是最新状态。
  const trayRequestHandlerRef = useRef<(request: TrayRequest) => Promise<TrayResult>>(async (request) => ({ id: request.id, ok: false }));
  trayRequestHandlerRef.current = async (request: TrayRequest): Promise<TrayResult> => {
    const revealMainWindow = () => void invoke("tray_panel_action", { action: "open" }).catch(() => {});
    if (request.kind === "navigate") {
      if (request.route === "addDirectProvider") {
        setAddDirectProviderRequested(true);
        await switchTool("codex", "relay");
        return { id: request.id, ok: true };
      }
      if (request.route === "sessions" || request.route === "requestLog") setActiveTool("codex");
      await navigate(request.route);
      return { id: request.id, ok: true };
    }
    const done = request.kind === "launch" ? await launch() : request.kind === "restart" ? await restart() : await stopCodex();
    if (!done) revealMainWindow();
    return { id: request.id, ok: done };
  };

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listen<unknown>(TRAY_REQUEST_EVENT, (event) => {
      const request = parseTrayRequest(event.payload);
      if (disposed || !request) return;
      trayRequestsInFlight.current += 1;
      void trayRequestHandlerRef.current(request)
        .catch((error): TrayResult => ({ id: request.id, ok: false, message: stringifyError(error) }))
        .then((result) => emit(TRAY_RESULT_EVENT, result))
        .catch(() => {})
        .finally(() => {
          trayRequestsInFlight.current -= 1;
          // 做完仍隐藏就再释放；要显示主窗口的情况（失败、打开某页）后端会取消这次释放。
          releaseMainWindowWhenHidden();
        });
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        stopListening = unlisten;
        // 告诉后端监听已就绪：轻量模式下快捷面板会等这个信号再把请求发过来（#2677）。
        void emit(MAIN_WINDOW_READY_EVENT).catch(() => {});
      }
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, []);

  useEffect(() => {
    if (route !== "sessions") return;
    let disposed = false;
    void call<CommandResult<ProviderSyncPayload>>("load_provider_sync_recovery").then((result) => {
      if (disposed || !result.recoveryJournalPath) return;
      setProviderSyncProgress((current) => current.active ? current : ({ active: false, percent: 0, message: result.message, result }));
    }).catch(() => {});
    return () => { disposed = true; };
  }, [route, recoveryRefreshRequest]);

  // 会话页的索引修复报告、多开页的实例列表：只在对应页面、窗口可见时轮询，数据没变不重渲染。
  usePolling(() => refreshSessionIndexRepairReport(() => routeRef.current === "sessions"), 15_000, { enabled: route === "sessions" });
  usePolling(() => refreshMultiLaunchInstances(true), 3_000, { enabled: route === "multiLaunch" });

  useEffect(() => {
    if (getLanguage() === "en") {
      void invoke("update_tray_labels", {
        showLabel: "Show window",
        quitLabel: "Quit",
        windowTitle: "Codex++",
      });
    }
  }, []);

  // 网页导入 / 一键换肤链接存下后后端发事件（macOS「打开 URL」）；Windows 由新进程存好后唤起本窗口，
  // 窗口获得焦点时后端发导航事件（见上面的监听），两种情况都在这里读一次，不再定时轮询磁盘。
  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listen(PENDING_IMPORT_CHANGED_EVENT, () => {
      if (disposed) return;
      void refreshPendingProviderImport(true);
      void refreshPendingDreamSkinCommunity();
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stopListening = unlisten;
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, []);

  useEffect(() => {
    if (!weixinQr || !["", "wait", "scaned"].includes(weixinQr.qrStatus)) return;
    let cancelled = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const result = await call<WeixinQrResult>("weixin_connect_qr_status");
        if (cancelled) return;
        setWeixinQr(result);
        if (result.qrStatus === "confirmed") {
          await refreshSettings(true);
          await refreshWeixinStatus(true);
          showNotice(t("微信扫码登录"), result.message, result.status);
          return;
        }
        if (!isSuccessStatus(result.status) || result.qrStatus === "expired") {
          showResultNotice(t("微信扫码登录"), result);
          return;
        }
        timer = window.setTimeout(poll, 1_000);
      } catch (error) {
        if (!cancelled) showNotice(t("微信扫码登录"), stringifyError(error), "failed");
      }
    };
    void poll();
    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [weixinQr?.qrStatus, weixinQr?.qrContent]);

  // 微信页轮询连接状态：静默读，失败不弹提示（等下一轮），状态没变不重渲染。
  usePolling(async () => {
    const result = await invoke<WeixinConnectStatusResult>("weixin_connect_status");
    setWeixinStatus((previous) => keepIfSame(previous, result));
  }, 2_000, { enabled: route === "weixin", immediate: false });

  useEffect(() => watchSystemTheme(setSystemDark), []);

  useEffect(() => {
    applyResolvedTheme(theme);
  }, [theme]);

  useEffect(() => {
    storeThemePreference(themePreference);
  }, [themePreference]);

  useEffect(() => {
    // 来回切窗口时每次聚焦都会触发：5 秒内只刷新一次概览。
    let lastRefreshAt = 0;
    const refresh = () => {
      const now = Date.now();
      if (now - lastRefreshAt < FOCUS_REFRESH_MIN_INTERVAL_MS) return;
      lastRefreshAt = now;
      void refreshOverview(true);
    };
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, []);

  // 网关改了 Codex 的直连设置（接入 / 断开网关），或在 Codex 设置页改了开关：
  // 重新读取并把尚未保存的编辑叠上去，免得用旧设置整份写回或撞上 revision 冲突。
  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    const syncExternalSettings = async () => {
      let result: SettingsResult;
      try {
        result = await call<SettingsResult>("load_settings");
      } catch {
        return;
      }
      if (disposed || !isSuccessStatus(result.status)) return;
      const previous = settingsRef.current?.settings;
      const latest = normalizeSettings(result.settings);
      setSettings(result);
      setSettingsForm((current) => (previous
        ? rebaseSettingsEdits(normalizeSettings(current), normalizeSettings(previous), latest)
        : latest));
    };
    void listen("codex-plus-settings-changed", () => {
      if (!disposed) void syncExternalSettings();
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stopListening = unlisten;
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listen("runtime-health-changed", () => {
      if (!disposed) void refreshOverview(true);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else {
        stopListening = unlisten;
        // 先订阅再读取，覆盖后台恢复早于监听安装完成的情况。
        void refreshOverview(true);
      }
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, []);

  const saveCodexAppPath = async (appPath: string) => {
    const next = { ...settingsForm, codexAppPath: appPath };
    const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
    if (result) {
      setSettings(result);
      const normalized = normalizeSettings(result.settings);
      setSettingsForm(normalized);
      setLaunchForm((current) => ({ ...current, appPath: normalized.codexAppPath }));
      await refreshOverview(true);
    }
    return result;
  };

  /// 皮肤急救入口：关掉皮肤并恢复 Codex 原始外观；命令只记为待应用，重启 Codex 后完整生效（提示文案来自命令结果）。
  const restoreDreamSkin = async () => {
    const result = await run(() => call<DreamSkinRuntimeResult>("restore_dream_skin", dreamSkinRequest()));
    if (!result) return;
    await refreshSettings(true);
    showResultNotice(t("皮肤管理"), result);
    if (isSuccessStatus(result.status)) setDreamSkinRestartPending(true);
  };

  // 页面拿到的操作：方法引用整个生命周期不变、调用时总读最新 state（以前是依赖列表很长的 useMemo，
  // 漏了 relaySwitching 等值，切换供应商期间按钮仍可点）；会变的数据单独列出，变了才换新对象。
  const actionCallbacks = useStableCallbacks({
      refreshCurrent: () => navigate(route),
      launch,
      launchMultiInstance,
      refreshMultiLaunchInstances,
      restartMultiLaunchInstance,
      stopMultiLaunchInstance,
      restart,
      stopCodex,
      repairShortcuts,
      checkUpdate,
      performUpdate,
      saveSettings,
      saveSettingsValue,
      autoSaveSettings,
      previewSettingsForm: (next: BackendSettings) => setSettingsForm(normalizeSettings(next)),
      refreshSettings,
      resetSettings,
      exportSettings,
      importSettings,
      chooseSettingsPath,
      resetSettingsPath,
      chooseCodexAppPath: async (mode: "folder" | "file") => {
        let selected: unknown;
        try {
          selected = await open(
            mode === "folder"
              ? { directory: true, multiple: false, title: t("选择 Codex 应用目录") }
              : {
                  directory: false,
                  multiple: false,
                  title: t("选择 Codex.exe 或 Codex.app"),
                  filters: [{ name: t("Codex 应用"), extensions: ["exe", "app"] }],
                },
          );
        } catch (error) {
          // Surface plugin failures (e.g. missing capability permission) so the
          // buttons no longer appear unresponsive — see #345.
          const message = error instanceof Error ? error.message : String(error);
          showNotice(t("Codex 应用路径"), tf("打开选择器失败：{0}", [message]), "failed");
          return;
        }
        if (typeof selected === "string" && selected.trim()) {
          const result = await saveCodexAppPath(selected.trim());
          if (result) {
            showNotice(t("Codex 应用路径"), t("应用路径已保存，之后启动会自动复用。"), result.status);
          }
        }
      },
      clearCodexAppPath: async () => {
        const next = { ...settingsForm, codexAppPath: "" };
        const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
        if (result) {
          setSettings(result);
          setSettingsForm(normalizeSettings(result.settings));
          setLaunchForm((current) => ({ ...current, appPath: "" }));
          showNotice(t("Codex 应用路径"), t("已清除保存路径，后续启动会回到自动探测。"), result.status);
          await refreshOverview(true);
        }
      },
      restoreDreamSkin,
      saveManualCodexAppPath: async () => {
        const appPath = launchForm.appPath.trim();
        if (!appPath) {
          showNotice(t("Codex 应用路径"), t("请先填写或选择应用路径。"), "failed");
          return;
        }
        const result = await saveCodexAppPath(appPath);
        if (result) {
          showNotice(t("Codex 应用路径"), t("应用路径已保存，之后启动会自动复用。"), result.status);
        }
      },
      syncProvidersNow,
      recoverProviderSync,
      repairSessionIndex,
      refreshProviderSyncTargets,
      setProviderSyncTarget: (provider: string) => {
        setSelectedProviderSyncTarget(provider);
      },
      refreshRelayFiles,
      refreshEnvConflicts,
      refreshRelayEnvironment,
      removeEnvConflicts,
      refreshCcsProviders,
      importCcsProviders,
      importMagpieProviders,
      refreshLiveContextEntries,
      syncLiveContextEntries,
      refreshAds,
      refreshScriptMarket,
      refreshUserScriptInventory,
      reloadUserScripts: async () => { await reloadUserScripts(); },
      installMarketScript,
      setUserScriptsEnabled,
      setUserScriptsSkipNextStart,
      setUserScriptEnabled,
      setExtensionConfig,
      showExtensionNotice,
      deleteUserScript,
      refreshLocalSessions,
      listProjectSessions,
      deleteLocalSession,
      deleteLocalSessions,
      archiveLocalSessions,
      openExternalUrl,
      applyRelayInjection,
      applyPureApiInjection,
      clearRelayInjection,
      saveRelayFile,
      applyContextEntry,
      previewMcpServersJson,
      importMcpServersJson,
      extractRelayCommonConfig,
      testRelayProfile,
      diagnoseRelayProfile,
      fetchRelayProfileModels,
      switchRelayProfile,
      switchOfficialMode,
      switchPureApiMode,
      refreshLogs,
      clearLogs,
      refreshDiagnostics,
      showMessage: async (title: string, message: string, status?: Status) => showNotice(title, message, status),
      copyLogs: () => copyText(logs?.text ?? "", t("日志已复制。")),
      copyDiagnostics: () => copyText(diagnostics?.report ?? "", t("诊断报告已复制。")),
      goLogs: () => { setSettingsFocus("app-settings-logs"); return navigate("settings"); },
      checkHealth: async () => {
        await Promise.all([refreshOverview(true), refreshWatcher(true)]);
        showNotice(t("检查完成"), t("已刷新 Codex 应用、入口和 Watcher 状态。"), "ok");
      },
      setWatcherEnabled,
      setWatcherStartMinimized,
      setWatcherStartCodex,
      setWatcherStartCodexOnOpen,
      setAutoTakeover,
      refreshAutoTakeover: () => refreshAutoTakeover(false),
      takeOverNow: () => runAutoTakeoverCommand("take_over_codex_now"),
      pauseAutoTakeover: () => runAutoTakeoverCommand("pause_auto_takeover_for_instance"),
      toggleTheme: () => setThemePreference(toggledThemePreference(theme)),
      setThemePreference,
  });
  const actions: Actions = useMemo(
    () => ({
      ...actionCallbacks,
      multiLaunchPending,
      multiLaunchInstances,
      multiLaunchInstancesLoading,
      multiLaunchActionPending,
      relaySwitching,
    }),
    [actionCallbacks, multiLaunchPending, multiLaunchInstances, multiLaunchInstancesLoading, multiLaunchActionPending, relaySwitching],
  );
  // 拓展贡献的管理器页面：侧栏入口与对内置页面的顶替（见 extension-manager-pages.ts）。
  const extensionPages = useMemo(() => managerExtensionPages(settings?.user_scripts), [settings?.user_scripts]);
  const pageOverrides = useMemo(() => managerPageOverrides(extensionPages), [extensionPages]);
  const openExtensionPage = (key: string) => {
    setExtensionPageKey(key);
    void navigate("extension");
  };
  /** 拓展页面里 codexPlusManager.navigate(target)：内置页面的路由 id，或 ext:<拓展 key>。 */
  const navigateFromExtension = (target: string): boolean => {
    if (target.startsWith("ext:")) {
      const key = target.slice(4);
      if (!extensionPages.some((page) => page.key === key)) return false;
      openExtensionPage(key);
      return true;
    }
    // 旧 id（about、maintenance…）照样接受，由 navigate 换成现在的页面。
    const next = isRouteAlias(target) ? target : isKnownRoute(target) ? target : null;
    if (!next || next === "extension") return false;
    void navigate(next);
    return true;
  };
  useEffect(() => {
    if (settingsFocus) setSettingsSections(settingsSectionsFor(settingsFocus));
  }, [settingsFocus]);
  // 「日志与诊断」分类显示时才读日志和诊断报告（诊断在后端还要再读一遍概览）。
  const settingsLogsShown = route === "settings" && settingsSections.includes("app-settings-logs");
  useEffect(() => {
    if (!settingsLogsShown) return;
    void refreshLogs(true);
    void refreshDiagnostics(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsLogsShown]);
  useEffect(() => {
    if (!settingsModeRoutes.has(route)) lastMainRoute.current = route;
  }, [route]);
  const activeExtensionPage = route === "extension" ? extensionPages.find((page) => page.key === extensionPageKey) ?? null : null;
  const overridingPage = route !== "extension" && !builtinPagesShown.has(route) ? pageOverrides[route] ?? null : null;
  const shownExtensionPage = activeExtensionPage ?? overridingPage;
  const renderExtensionPage = (page: ManagerExtensionPage) => {
    const script = settings?.user_scripts?.scripts?.find((item) => item.key === page.key);
    return (
      <ExtensionManagerPanel
        key={page.key}
        scriptKey={page.key}
        config={script?.package?.config ?? {}}
        onSaveConfig={(values) => actions.setExtensionConfig(page.key, values)}
        onToast={(message) => actions.showExtensionNotice(page.title, message)}
        dataSources={page.data}
        onNavigate={navigateFromExtension}
        fullPage
      />
    );
  };
  // 被顶替的页面：上方说明由哪个拓展提供，可以切回内置页面；切回后也能再换回拓展页面。
  const extensionPageView = shownExtensionPage ? (
    <div className="extension-page-host">
      {overridingPage ? (
        <div className="extension-page-banner">
          <span>{tf("此页由拓展「{0}」提供", [overridingPage.title])}</span>
          <Button size="sm" variant="ghost" onClick={() => setBuiltinPagesShown((current) => new Set(current).add(route))}>{t("查看内置页面")}</Button>
        </div>
      ) : null}
      {renderExtensionPage(shownExtensionPage)}
    </div>
  ) : route === "extension" ? (
    <div className="empty">{t("这个拓展页面已经停用或不存在。")}</div>
  ) : null;
  const restorableOverride = route !== "extension" && builtinPagesShown.has(route) ? pageOverrides[route] ?? null : null;
  const isGlobalPage = globalNavigationRoutes.includes(route);
  const isManagementScreen = route === "relay" || route === "multiLaunch" || route === "sessions" || route === "context" || route === "skills";
  const codexAppRunning = overview?.runtime_health?.codex_app?.status === "running";
  // Codex 可能在它自己的窗口里被退出，既不经过 Codex++ 也不触发窗口聚焦（#2778）。
  // 窗口可见时每 5 秒只查一次进程是否还在，和当前显示不一致再完整刷新概览。
  useEffect(() => {
    let disposed = false;
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "visible") return;
      void invoke<boolean | null>("codex_app_running")
        .then((running) => {
          if (!disposed && typeof running === "boolean" && running !== codexAppRunning) void refreshOverview(true);
        })
        .catch(() => undefined);
    }, codexRunningPollMs);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [codexAppRunning]);
  /**
   * 「连接与供应商」页的「经网关」：和「Agent 接入」页打开 Codex 开关同一条命令（save_gateway_binding），
   * 沿用上次的接入设置（默认模型等）；共享供应商是空的或模型不在池里时说明原因并带去对应页面。
   * 切回直连走「断开并还原」（先预览再确认），见 legacyDisconnectOpen。
   */
  const [codexConnecting, setCodexConnecting] = useState(false);
  const connectCodexGateway = async (model?: string) => {
    if (codexConnecting) return;
    setCodexConnecting(true);
    try {
      const loaded = await invoke<GatewayResult>("load_gateway");
      const payload = loaded.gateway && Array.isArray(loaded.pool) ? (loaded as GatewayPayload) : null;
      const previous = payload?.gateway.bindings?.codex;
      // 在列表里点了某个共享供应商的「使用」：默认模型换成它的；其余接入设置沿用上次的。
      const draft = { ...draftFromBinding(previous), ...(model ? { model } : {}) };
      const issue = validateDraft(draft, payload);
      if (issue) {
        await showNotice(t("切换到本地服务"), t(issue), "failed");
        void navigate(issue.includes("共享供应商是空的") ? "gatewayProviders" : "gateway");
        return;
      }
      const result = await invoke<GatewayResult>("save_gateway_binding", { agent: "codex", binding: { ...bindingFromDraft(draft, previous), enabled: true }, reconnect: true });
      publishGatewayPayload(result);
      await refreshSettings(true);
      await showNotice(t("切换到本地服务"), result.message, isSuccessStatus(result.status as Status) ? "ok" : "failed");
    } catch (error) {
      await showNotice(t("切换到本地服务"), stringifyError(error), "failed");
    } finally {
      setCodexConnecting(false);
    }
  };
  /** 断开本地服务后切到列表里点的那个直连供应商（断开时已切回接入前的那个，是它就不再切）。 */
  const switchAfterDisconnect = async () => {
    const target = legacyDisconnectThen.current;
    legacyDisconnectThen.current = null;
    if (!target) return;
    try {
      const fresh = await invoke<SettingsResult>("load_settings");
      if (!isSuccessStatus(fresh.status)) return;
      const current = normalizeSettings(fresh.settings);
      if (current.activeRelayId === target || !current.relayProfiles.some((profile) => profile.id === target)) return;
      await switchRelayProfile(syncLegacyRelayFields({ ...current, activeRelayId: target }), current.activeRelayId);
    } catch (error) {
      await showNotice(t("切换供应商"), stringifyError(error), "failed");
    }
  };
  // 「启动 / 重启 Codex」：Codex 页面的顶栏按钮和侧栏播放键共用一份状态，都走 actions.launch / actions.restart。
  const codexToolEntry = toolEntries.find((entry) => entry.id === "codex");
  const codexLaunch: CodexLaunchState = {
    running: codexAppRunning,
    busy: launchPending,
    unavailableReason: overview?.codex_app?.status === "missing"
      ? t("没有找到 Codex 应用，请先安装 Codex")
      : codexToolEntry && !codexToolEntry.switchable
        ? t("Codex 暂未接入，无法从这里启动")
        : null,
    onLaunch: () => void actions.launch(),
    onRestart: () => void actions.restart(),
  };
  // 侧栏 Codex 行和「Agent 接入」里的 Codex 行共用这一份运行时对象：两边都是启动 / 停止同一个进程。
  const codexRun = {
    running: codexAppRunning,
    unattached: Boolean(overview?.runtime_health?.codex_unattached),
    busy: launchPending,
    unavailableReason: codexLaunch.unavailableReason,
    onStart: codexLaunch.onLaunch,
    onStop: () => void actions.stopCodex(),
    onRestart: () => void actions.restart(),
  };
  /**
   * 侧栏 Claude 行的播放键。读一次 claude_local_status（进程 + 证书探测，比较重，不做轮询），
   * 动作成功后再读一次；失败交给现有的 notice 机制提示。
   */
  const refreshClaudeLocalRun = async () => {
    try {
      const result = await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_status");
      if (result.status !== "ok") return;
      claudeLocalMemo.status = result;
      setClaudeLocalRun(result);
    } catch {
      // 读不到就保持上次状态：按钮照常可点，真正的失败由动作本身报出来。
    }
  };
  const claudeLocalRunAction = async (action: "launch" | "stop") => {
    if (claudeLocalRunBusy) return;
    setClaudeLocalRunBusy(true);
    try {
      const result = await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_action", { action });
      if (result.status === "ok") {
        claudeLocalMemo.status = result;
        setClaudeLocalRun(result);
      } else {
        await actions.showMessage(t("Claude 本地模式"), result.message, "failed");
        await refreshClaudeLocalRun();
      }
    } catch (error) {
      await actions.showMessage(t("Claude 本地模式"), stringifyError(error), "failed");
    } finally {
      setClaudeLocalRunBusy(false);
    }
  };
  useEffect(() => {
    void refreshClaudeLocalRun();
    // 只在 App 挂载时探测一次；Claude 本地模式的状态变化由动作和页面自己维护。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const claudeRunStatus = claudeLocalRun?.status === "ok" ? claudeLocalRun : null;
  /** 代理活着，或者 Claude 还挂在我们的代理端口上：两种情况都算「在跑」，按钮该是停止。 */
  const claudeAttached = Boolean(claudeRunStatus?.running || claudeRunStatus?.localProcess);
  const claudeRun = {
    running: claudeAttached,
    busy: claudeLocalRunBusy,
    unavailableReason: !claudeRunStatus
      ? null
      : !claudeRunStatus.supported
        ? t("未检测到支持的 Claude Desktop")
        : claudeRunStatus.claudeRunning && !claudeAttached
          ? t("Claude 正在运行，请先完全退出，再从 Codex++ 启动")
          : claudeRunStatus.config?.enabled && !claudeRunStatus.nodeAvailable
            ? t("缺少 Node.js，无法启动本地服务")
            : null,
    onStart: () => void claudeLocalRunAction("launch"),
    onStop: () => void claudeLocalRunAction("stop"),
  };
  // Codex 专属页面与 Codex 概览提供启动入口；共享的拓展管理页只在当前 Agent 为 Codex 时提供。
  const onCodexPage = (routeTool(route) === "codex" && (route !== "userScripts" || extensionHost === "codex")) || (route === "overview" && activeTool === "codex");
  const activeToolName = toolEntries.find((entry) => entry.id === activeTool)?.name ?? t("当前 Agent");
  // MCP 页的条目：设置里的条目叠上 live config.toml 的启停状态。只在两者变化时重算，免得每次渲染都给 McpPage 一个新数组。
  const mcpPageEntries = useMemo(() => {
    const entries = contextEntriesWithLiveEntries(settingsForm, liveContextEntries);
    return contextEntriesByKind(entries, "mcp");
  }, [settingsForm, liveContextEntries]);
  // 「Codex 插件」页「本机配置」：config.toml 里的 [plugins.*] 条目（原来在 MCP 页的「管理插件配置」里）。
  const pluginConfigEntries = useMemo(
    () => contextEntriesByKind(contextEntriesWithLiveEntries(settingsForm, liveContextEntries), "plugin"),
    [settingsForm, liveContextEntries],
  );
  // 停在不属于当前 Agent 的页面上：回到当前 Agent 的落脚页。
  useEffect(() => {
    const currentRoute = routes.find((candidate) => candidate.id === route);
    if (currentRoute?.tools && currentRoute.id !== "userScripts" && !pageVisibleForTool(currentRoute, activeTool)) void navigate(agentHomeRoute[activeTool] ?? "overview");
    // navigate 每次渲染都是新函数，只在页面或 Agent 变化时检查。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [route, activeTool]);

  // ⌘K：页面、各 Agent 的页面和常用操作。非当前 Agent 的页面会先切到那个 Agent。
  const paletteCommands: PaletteCommand[] = [];
  const routeCommand = (routeId: Route, group: string, run: () => void, keywords = ""): PaletteCommand | null => {
    const item = routes.find((candidate) => candidate.id === routeId);
    if (!item) return null;
    if (item.tools && routeId !== "userScripts" && !pageVisibleForTool(item, activeTool)) return null;
    const Icon = item.icon;
    return { id: `${group}:${routeId}`, group, label: item.label, keywords: `${routeId} ${keywords}`, icon: <Icon className="h-4 w-4" />, run };
  };
  const pushCommand = (command: PaletteCommand | null) => { if (command) paletteCommands.push(command); };
  pushCommand(routeCommand("home", t("全局管理"), () => void navigate("home"), "home ads 推荐 赞助"));
  gatewayNavigationRoutes.forEach((routeId) => pushCommand(routeCommand(routeId, t("本地服务"), () => void navigate(routeId), "gateway 网关 本地服务")));
  managementNavigationRoutes.forEach((routeId) => pushCommand(routeCommand(routeId, t("全局管理"), () => void navigate(routeId))));
  pushCommand(routeCommand("requestLog", t("全局管理"), () => void navigate("requestLog"), "usage requests"));
  for (const tool of toolEntries.filter((entry) => entry.switchable)) {
    agentRoutes(tool.id).forEach((routeId) => pushCommand(routeCommand(routeId, tool.name, () => void switchTool(tool.id, routeId), tool.id)));
  }
  // 设置的各个分类（含缓存清理、Computer Use 等设置模式里的页面）都能搜到：「更新」「日志」「配置体检」……
  // 网关设置已在「网关」一组里，不重复列。
  const paletteRoutes = new Set(paletteCommands.map((command) => command.id.slice(command.id.indexOf(":") + 1)));
  for (const entry of settingsNavEntries(toolEntries.some((tool) => tool.id === "codex" && tool.switchable))) {
    if (entry.route !== "settings" && paletteRoutes.has(entry.route)) continue;
    const Icon = entry.icon;
    paletteCommands.push({
      id: `settings:${entry.key}`,
      group: t("设置"),
      label: entry.label,
      keywords: `settings ${entry.route} ${entry.key} ${entry.keywords ?? ""}`,
      icon: <Icon className="h-4 w-4" />,
      run: () => {
        if (entry.sections) setSettingsSections(entry.sections);
        void navigate(entry.route);
      },
    });
  }
  extensionPages.filter((page) => page.nav).forEach((page) => paletteCommands.push({
    id: `extension:${page.key}`,
    group: t("拓展"),
    label: page.title,
    keywords: `extension ${page.key}`,
    icon: <AppWindow className="h-4 w-4" />,
    run: () => openExtensionPage(page.key),
  }));
  paletteCommands.push(
    {
      id: "action:codex",
      group: t("操作"),
      label: codexAppRunning ? t("重启 Codex") : t("启动 Codex"),
      keywords: "codex restart launch",
      icon: <Rocket className="h-4 w-4" />,
      run: () => void (codexAppRunning ? actions.restart() : actions.launch()),
    },
    { id: "action:theme", group: t("操作"), label: theme === "dark" ? t("切换到浅色") : t("切换到深色"), keywords: "theme", icon: theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />, run: actions.toggleTheme },
    { id: "action:language", group: t("操作"), label: getLanguage() === "en" ? t("切换到中文") : t("切换到英文"), keywords: "language", icon: <Languages className="h-4 w-4" />, run: () => void toggleLanguage() },
  );

  // 标题：Agent 自己的页面用 Agent 名字（页面在下面的胶囊里），设置模式用分类名，其余用页面名。
  const settingsEntries = settingsNavEntries(true);
  const settingsEntryKey = settingsModeRoutes.has(route) ? activeSettingsEntry(settingsEntries, route, settingsSections) : null;
  const agentPageGroups = agentTabGroups(navigationSections, routeTool, activeTool).flatMap((entry) => entry.routes);
  const onAgentPage = !isGlobalPage && !settingsModeRoutes.has(route) && agentPageGroups.length > 1;
  const pageTitle = route === "extension"
    ? activeExtensionPage?.title ?? t("拓展页面")
    : settingsEntryKey
      ? settingsEntries.find((entry) => entry.key === settingsEntryKey)?.label ?? routeTitle(route)
      : customizeRoutes.includes(route)
        ? t("MCP 与 Skills")
        : onAgentPage
          ? routeTitle(route)
          : isLocalServiceRoute(route)
            ? t("本地服务")
            : routeTitle(navAliasRoute[route] ?? route);

  // 启动引导：两步向导挂在这里（切到设置模式也不卸载），进度卡投到侧栏底部（参照 Cursor 的 Getting Started 卡片）。
  const onboardingSetup = setupOpen && !setupChecking ? (
          <OnboardingSetup
            key={setupRun}
            open={wizardOpen}
            onOpenChange={setWizardOpen}
            cardSlot={setupSlot}
            relaySettings={settings ? normalizeSettings(settings.settings) : null}
            codexInstalled={overview ? overview.codex_app.status === "found" : null}
            codexRunning={codexAppRunning}
            launchPending={launchPending}
            codexEnhancementsEnabled={hasCodexEnhancements(settings)}
            onDismiss={closeSetup}
            onTour={() => { setWizardOpen(false); setTourOpen(true); }}
            onNavigate={(target, agent) => { if (agent) void switchTool(agent, target); else void navigate(target); }}
            onAddProvider={() => { setAddDirectProviderRequested(true); void switchTool("codex", "relay"); }}
            onLaunch={() => codexAppRunning ? restart() : launch()}
          />
        ) : null;

  return (
    <div className={`shell ${theme}`}>
      <AppSidebar
        setupSlotRef={setSetupSlot}
        tools={toolEntries}
        activeTool={activeTool}
        route={route}
        theme={theme}
        version={overview?.current_version ?? update?.currentVersion ?? ""}
        onOpenAgent={(toolId, destination) => void switchTool(toolId, destination ?? agentEntryRoute(toolId))}
        onNavigate={(next) => void navigate(next)}
        extensionPages={extensionPages}
        activeExtensionKey={route === "extension" ? extensionPageKey : null}
        onOpenExtensionPage={openExtensionPage}
        onToggleTheme={actions.toggleTheme}
        onOpenPalette={() => setPaletteOpen(true)}
        pendingUpdateVersion={update?.latestVersion && updateNotifications.current.shouldRemind(update) ? update.latestVersion : null}
        pendingUpdateReady={Boolean(update?.updateAvailable && update.latestVersion && (update.downloaded || (updateDownload?.version === update.latestVersion && updateDownload.state === "ready")))}
        pendingUpdateProgress={update?.latestVersion && updateDownload?.version === update.latestVersion && updateDownload.state === "downloading" ? (updateDownload.percent ?? -1) : null}
        onOpenUpdate={() => { if (update) setUpdatePrompt(update); }}
        settingsSections={settingsSections}
        onOpenSettings={(next, sections) => {
          if (sections) setSettingsSections(sections);
          // 设置页里切分类只换显示的卡片，不再整页重读；各分类的数据在显示时读（见下方 effect）。
          if (next !== route) void navigate(next);
        }}
        onExitSettings={() => void navigate(lastMainRoute.current)}
        codexRun={codexRun}
        claudeRun={claudeRun}
      />
      <CommandPalette commands={paletteCommands} onClose={() => setPaletteOpen(false)} open={paletteOpen} />
      {onboardingSetup}
      {legacyDisconnectOpen ? (
        <AgentDisconnectDialog
          agent={{ agent: "codex", name: "Codex" }}
          busy={false}
          onApply={async (previewToken) => {
            try {
              const result = await invoke<GatewayResult>("disable_gateway_binding", { agent: "codex", previewToken });
              await showNotice(result.status === "ok" ? t("已断开本地服务") : t("断开失败"), result.message, result.status === "ok" ? "ok" : "failed");
              await refreshSettings(true);
              if (result.status === "ok") await switchAfterDisconnect();
              return result.status === "ok";
            } catch (error) {
              await showNotice(t("断开失败"), String(error), "failed");
              return false;
            }
          }}
          // 没法逐项预览时（接入前没留下实时存档）：确认后完整切换到预览里写明的直连供应商。
          onFullSwitch={async (previewToken) => {
            try {
              const result = await invoke<GatewayResult>("disable_gateway_binding_full_switch", { agent: "codex", previewToken });
              publishGatewayPayload(result);
              await showNotice(result.status === "ok" ? t("已断开本地服务") : t("断开失败"), result.message, result.status === "ok" ? "ok" : "failed");
              await refreshSettings(true);
              if (result.status === "ok") await switchAfterDisconnect();
              return result.status === "ok";
            } catch (error) {
              await showNotice(t("断开失败"), String(error), "failed");
              return false;
            }
          }}
          onClose={() => {
            legacyDisconnectThen.current = null;
            setLegacyDisconnectOpen(false);
          }}
          onAddDirectProvider={() => setAddDirectProviderRequested(true)}
        />
      ) : null}
      {tourOpen && !wizardOpen ? <Suspense fallback={null}><OnboardingTour onClose={closeTour} /></Suspense> : null}
      <main className="workspace">
        <section
          className={`screen ${isManagementScreen ? "management-screen" : ""} ${route === "requestLog" ? "request-log-screen" : ""}`.trim()}
          data-settings-mode={settingsModeRoutes.has(route) ? "true" : undefined}
          key={route}
          // 滚动后给吸顶的标题行加一条分隔线（macOS 无标题栏时标题行吸顶）；直接改 DOM 属性，不触发重渲染。
          onScroll={(event) => event.currentTarget.toggleAttribute("data-scrolled", event.currentTarget.scrollTop > 4)}
        >
          {/* macOS 没有系统标题栏时，这一行就是标题栏：空白处可拖动窗口、双击缩放（按钮不受影响）。 */}
          <header className="cx-page-head" data-tauri-drag-region="deep">
            <div className="cx-page-title">
              <h1 title={route === "extension" ? undefined : routeSubtitle(navAliasRoute[route] ?? route)}>{pageTitle}</h1>
            </div>
            <div className="topbar-actions">
              {/* data-topbar-actions：页面级操作的挂载点（TopbarActions / PageHeader），Claude 页把原版启动按钮 portal 进来。 */}
              <div className="topbar-page-actions" data-topbar-actions />
              {/* 「启动 / 重启 Codex」：只在 Codex 的页面里放在标题右侧（#2744 / #2745）；
                  其它页面靠侧栏 Codex 行常驻的播放键启动，停止也在那里。 */}
              {onCodexPage ? <CodexLaunchButton className="topbar-codex-launch" size="sm" state={codexLaunch} /> : null}
            </div>
          </header>
          {/* Agent 的页面在侧栏里它那一行下面。 */}
          <CustomizeTabs activeTool={activeTool} onNavigate={(next) => void navigate(next)} route={route} />
          {restorableOverride ? (
            <div className="extension-page-banner">
              <span>{tf("拓展「{0}」提供了这一页的替代页面", [restorableOverride.title])}</span>
              <Button size="sm" variant="ghost" onClick={() => setBuiltinPagesShown((current) => { const next = new Set(current); next.delete(route); return next; })}>{t("使用拓展页面")}</Button>
            </div>
          ) : null}
          {/* 按需加载的页面在这里等代码到位；不常用的页面才走这里，常用页不受影响。 */}
          <Suspense fallback={<PageFallback />}>
          {extensionPageView ?? (<>
          {route === "overview" ? (
            <OverviewScreen
              overview={overview}
              activeTool={activeTool}
              toolEntries={toolEntries}
              form={settingsForm}
              onNavigate={(next) => void navigate(next)}
              onOpenList={(kind, agent) => {
                // 从 Agent 概览跳过去：会话页停在这个 Agent 的页签，请求日志按它过滤。
                if (kind === "sessions") setSessionTab(agent);
                else setRequestLogAgentFilter(agent);
                void navigate(kind);
              }}
              actions={actions}
            />
          ) : null}
          {/* 「Codex 连接」一页：连接方式（经网关 / 直连）+ 直连供应商（可加入共享供应商）+ 高级（原「旧版代理设置」）。
              旧入口 relayEnvironment 在 navigate 里换成这一页并展开「高级」。 */}
          {route === "relay" ? (
            <RelayScreen
              addDirectProviderRequested={addDirectProviderRequested}
              onRequestedProviderOpened={() => setAddDirectProviderRequested(false)}
              settings={settings}
              relayFiles={relayFiles}
              envConflicts={envConflicts}
              ccsProviders={ccsProviders}
              form={settingsForm}
              actions={actions}
              onOpenGateway={() => void navigate("gatewayProviders")}
              onUseShared={(model) => void connectCodexGateway(model)}
              sharedConnecting={codexConnecting}
              onDisconnectGateway={(then) => {
                legacyDisconnectThen.current = then ?? null;
                setLegacyDisconnectOpen(true);
              }}
              advancedOpen={relayAdvancedOpen}
              connection={(
                <LegacyRelayGatewayHint
                  form={settingsForm}
                  connecting={codexConnecting}
                  onConnect={() => void connectCodexGateway()}
                  onDisconnect={() => setLegacyDisconnectOpen(true)}
                  onManageAgents={() => void navigate("gateway")}
                  onOpenGateway={() => void navigate("gatewayProviders")}
                />
              )}
              advanced={(
                <ProxySettingsScreen
                  dirty={settingsDirty}
                  form={settingsForm}
                  relayEnvironment={relayEnvironment}
                  onFormChange={setSettingsForm}
                  actions={actions}
                />
              )}
            />
          ) : null}
          {route === "multiLaunch" ? (
            <CodexMultiLaunchScreen
              form={settingsForm}
              relayFiles={relayFiles}
              onOpenRelay={() => void navigate("relay")}
              actions={actions}
            />
          ) : null}
          {route === "gatewayUsage" || route === "requestLog" ? <UsageTabs onChange={(next) => void navigate(next)} value={route} /> : null}
          {isLocalServiceRoute(route) ? <LocalServiceTabs onChange={(next) => void navigate(next)} value={route} /> : null}
          {route === "requestLog" ? (
            <RequestLogScreen
              initialAgent={requestLogAgentFilter}
              initialFocus={requestLogFocus}
              onConsumeInitialAgent={() => { setRequestLogAgentFilter("all"); setRequestLogFocus({}); }}
            />
          ) : null}
          {route === "claude" ? (
            <ClaudeLocalizationScreen
              notify={actions.showMessage}
              onNavigate={(next) => void navigate(next)}
              onLocalStatus={(status) => {
                // 页面里启动 / 停止后，侧栏 Claude 行的播放键立刻跟着变，不用轮询。
                claudeLocalMemo.status = status;
                setClaudeLocalRun((current) => (sameClaudeRunState(current, status) ? current : status));
              }}
            />
          ) : null}
          {route === "claudePatch" ? <ClaudeLocalizationScreen mode="patch" onNavigate={(next) => void navigate(next)} /> : null}
          {/* Claude 的供应商：从「Desktop 增强」的页签里移出来，和 Codex 的「连接与供应商」一样单独一页。 */}
          {route === "claudeProviders" ? (
            <div className="claude-localization-page cx-page">
              <ClaudeProviders notify={actions.showMessage} onManage={() => void navigate("gatewayProviders")} />
            </div>
          ) : null}
          {/* Codex 页签是完整的会话管理，其余页签是各 Agent 的会话列表。 */}
          {route === "sessions" ? <SessionAgentTabs onChange={setSessionTab} value={sessionTab} /> : null}
          {route === "sessions" && sessionTab !== "codex" ? (
            <div className="mg-page gateway-page">
              <div className="mg-content gw-content">
                <SessionsView agent={sessionTab} notify={actions.showMessage} />
              </div>
            </div>
          ) : null}
          {route === "sessions" && sessionTab === "codex" ? (
            <ProjectSessionPage
              sessions={localSessions}
              onRefresh={(offset, filter) => actions.refreshLocalSessions(false, offset ?? 0, filter)}
              onDelete={actions.deleteLocalSession}
              onDeleteMany={actions.deleteLocalSessions}
              onArchiveMany={actions.archiveLocalSessions}
              onListProjectSessions={actions.listProjectSessions}
              onPickFolder={async (title) => {
                try {
                  const selected = await open({ directory: true, multiple: false, title });
                  return typeof selected === "string" && selected.trim() ? selected : null;
                } catch (error) {
                  showNotice(t("移动会话"), stringifyError(error), "failed");
                  return null;
                }
              }}
              onRelocated={async (message, ok) => {
                showNotice(t("移动会话"), message, ok ? "ok" : "failed");
                await actions.refreshLocalSessions(true, localSessions?.offset ?? 0);
              }}
              repairPanel={<SessionRepairPanel dirty={settingsDirty} form={settingsForm} providerSyncProgress={providerSyncProgress}
                sessionIndexRepairActive={sessionIndexRepairActive} sessionIndexRepairReport={sessionIndexRepairReport}
                sessionIndexRepairReportError={sessionIndexRepairReportError} providerSyncTargets={providerSyncTargets}
                selectedProviderSyncTarget={selectedProviderSyncTarget} onFormChange={setSettingsForm} actions={actions} />}
            />
          ) : null}
          {route === "context" ? (
            <McpPage
              entries={mcpPageEntries}
              onSave={(kind, id, body) => actions.applyContextEntry(settingsForm, kind, id, body)}
              onDelete={(entry) => actions.applyContextEntry(settingsForm, entry.kind, entry.id, null)}
              onRefresh={async () => { await Promise.all([actions.refreshLiveContextEntries(), actions.refreshRelayFiles()]); }}
              onImport={async (json) => {
                const next = await actions.importMcpServersJson(settingsForm, json);
                if (!next) return false;
                const synced = await actions.syncLiveContextEntries(next, true);
                return Boolean(synced && isSuccessStatus(synced.status));
              }}
              notify={actions.showMessage}
            />
          ) : null}
          {route === "skills" ? <SkillsPage notify={actions.showMessage} /> : null}
          {route === "weixin" ? (
            <WeixinConnectScreen
              form={settingsForm}
              status={weixinStatus}
              qr={weixinQr}
              sessions={localSessions?.sessions ?? []}
              onFormChange={setSettingsForm}
              autoSave={<SettingsAutoSave dirty={settingsDirty} form={settingsForm} actions={actions} />}
              onQrLogin={() => void beginWeixinQrLogin()}
              onStart={() => void startWeixinConnect()}
              onStop={() => void stopWeixinConnect()}
              onChooseWorkDir={() => void chooseWeixinPath("workDir")}
              onChooseCodexPath={() => void chooseWeixinPath("codexPath")}
              onUseDesktopCodexCli={() => void useDesktopCodexCli()}
              onOpenQr={(url) => void openExternalUrl(url)}
              onCopyQr={(url) => void copyText(url, t("微信登录链接已复制。"))}
            />
          ) : null}
          {/* 旧的 dreamSkin 路由（皮肤急救）并进了「启动急救」页；navigate 会把它改写成 enhance，这里兜底。 */}
          {route === "enhance" || route === "dreamSkin" ? (
            <EnhanceScreen
              dirty={settingsDirty}
              form={settingsForm}
              onFormChange={setSettingsForm}
              settings={settings}
              launchPending={launchPending}
              skinRestartPending={dreamSkinRestartPending}
              actions={actions}
            />
          ) : null}
          {route === "userScripts" ? <UserScriptsScreen host={extensionHost} onHostChange={setExtensionHost} settings={settings} market={scriptMarket} onOpenRescue={() => void navigate("enhance")} onOpenPluginMarket={() => void switchTool("codex", "pluginMarket")} onOpenClaude={() => void switchTool("claude", "claude")} actions={actions} /> : null}
          {route === "pluginMarket" ? (
            <PluginMarketScreen
              configEntries={pluginConfigEntries}
              onInstalledChanged={() => void actions.refreshLiveContextEntries()}
              onOpenExtensions={() => void navigate("userScripts")}
              onSaveConfig={(id, body) => actions.applyContextEntry(settingsForm, "plugin", id, body)}
            />
          ) : null}
          {route === "home" ? (
            <HomeScreen
              ads={ads}
              actions={actions}
              tools={toolEntries}
              codexRunning={codexAppRunning}
              launchPending={launchPending}
              onLaunchCodex={() => void (codexAppRunning ? restart() : launch())}
              onNavigate={(next) => void navigate(next)}
              onOpenAgent={(agent, destination) => void switchTool(agent, destination ?? agentEntryRoute(agent))}
              autoTakeover={autoTakeover}
              autoTakeoverOn={settings ? autoTakeoverOn : null}
              watcherEnabled={watcher ? Boolean(watcher.enabled) : null}
            />
          ) : null}
          {route === "agentCache" ? <AgentCachePanel /> : null}
          {route === "agentCollaboration" ? <AgentCollaborationPage /> : null}
          {route === "computerUse" ? <ComputerUsePage /> : null}
          {route === "remoteControl" ? <RemoteControlPage /> : null}
          {route === "hub" ? (
            <HubOverview
              codex={{ running: codexAppRunning, providerName: toolEntries.find((entry) => entry.id === "codex")?.activeRelayName ?? null }}
              notify={actions.showMessage}
              onNavigate={(next) => void navigate(next)}
              onOpenAgent={(agent) => void switchTool(agent, agentHomeRoute[agent] ?? "overview")}
            />
          ) : null}
          {gatewayTabByRoute[route] ? (
            <GatewayPage
              extraSettings={
                // 这块和设置页同一个按需加载的模块：网关页已经显示出来了，等它时不再把整页换成占位。
                <Suspense fallback={null}>
                  <SharedServiceSettings
                    autoSave={<SettingsAutoSave dirty={settingsDirty} form={settingsForm} actions={actions} />}
                    form={settingsForm}
                    onFormChange={setSettingsForm}
                    overview={overview}
                  />
                </Suspense>
              }
              codexRun={codexRun}
              notify={actions.showMessage}
              onOpenCodexProviderSettings={(addDirectProvider) => {
                if (addDirectProvider) setAddDirectProviderRequested(true);
                void switchTool("codex", "relay");
              }}
              onTabChange={(tab) => void navigate(gatewayRouteByTab[tab])}
              tab={gatewayTabByRoute[route]}
            />
          ) : null}
          {/* 共享供应商右下角的悬浮提示：在找供应商时顺手带去首页的赞助商推荐；首页没有赞助商时不显示。 */}
          {route === "gatewayProviders" && !sponsorHintClosed && (ads?.ads?.length ?? 0) + (ads?.topAds?.length ?? 0) > 0 ? (
            <div className="cx-float-hint">
              <button className="cx-float-hint-main" onClick={() => void navigate("home")} type="button">
                <Sparkles aria-hidden="true" />
                <span className="cx-float-hint-copy">
                  <strong>{t("正在找合适的供应商？")}</strong>
                  <small>{t("去首页看看赞助商推荐")}</small>
                </span>
                <ArrowRight aria-hidden="true" />
              </button>
              <button aria-label={t("关闭")} className="cx-float-hint-close" onClick={() => setSponsorHintClosed(true)} title={t("关闭")} type="button">
                <X aria-hidden="true" />
              </button>
            </div>
          ) : null}
          {route === "agentSettings" ? (
            // 设置与安装维护合在一页：左侧分类目录，右侧限宽列表。
            <div className="settings-page codex-settings-page">
              <div className="sectioned-body">
                {/* 常用的在上：应用与入口、开机启动、自动接管；额外启动参数和手动启动收进最后的「高级」。 */}
                <MaintenanceScreen
                  overview={overview}
                  launchPending={launchPending}
                  watcher={watcher}
                  autoTakeover={autoTakeover}
                  settings={settings}
                  launchForm={launchForm}
                  onLaunchFormChange={setLaunchForm}
                  actions={actions}
                  advanced={(
                    <AgentSettingsScreen
                      dirty={settingsDirty}
                      form={settingsForm}
                      onFormChange={setSettingsForm}
                      actions={actions}
                    />
                  )}
                />
              </div>
            </div>
          ) : null}
          {route === "settings" ? (
            // 设置与关于合在一页：左侧分类目录，右侧限宽。
            <div className="settings-page app-settings-page">
              {/* 一个分类一页：只渲染侧栏选中分类的卡片，日志、诊断、配置体检也只在显示时才读。 */}
              <div className="sectioned-body">
                <AppSettingsFocus focus={settingsFocus} onDone={() => setSettingsFocus(null)} />
                <GlobalSettingsScreen
                  sections={settingsSections}
                  onOpenSetup={() => {
                    // 重新走一遍启动引导：换 key 让向导从第一步开始；进度卡在主侧栏里，先回到进设置前的页面。
                    setSetupRun((run) => run + 1);
                    setSetupOpen(true);
                    setWizardOpen(true);
                    void navigate(lastMainRoute.current);
                  }}
                  onOpenTour={() => {
                    // 导览高亮的是主侧栏里的入口，设置模式的侧栏里没有：先回到进设置前的页面再开始，点了马上能看到。
                    void navigate(lastMainRoute.current);
                    setTourOpen(true);
                  }}
                  dirty={settingsDirty}
                  form={settingsForm}
                  settings={settings}
                  theme={theme}
                  themePreference={themePreference}
                  actions={actions}
                  onOpenAgentSettings={() => void (async () => {
                    if (activeTool !== "codex") await switchTool("codex");
                    await navigate("agentSettings");
                  })()}
                />
                <AboutScreen
                  sections={settingsSections}
                  form={settingsForm}
                  onFormChange={setSettingsForm}
                  overview={overview}
                  update={update}
                  updateDownload={updateDownload}
                  updateInstallProgress={updateInstallProgress}
                  logs={logs}
                  diagnostics={diagnostics}
                  actions={actions}
                />
              </div>
            </div>
          ) : null}
          </>)}
          </Suspense>
        </section>
      </main>
      {updatePrompt && !wizardOpen && !tourOpen ? (
        <UpdateDialog
          update={updatePrompt}
          downloaded={Boolean(updatePrompt.downloaded || (updateDownload?.version === updatePrompt.latestVersion && updateDownload?.state === "ready"))}
          progress={updateInstallProgress}
          onClose={closeUpdatePrompt}
          onSkipVersion={() => {
            if (updatePrompt.latestVersion) void skipUpdateVersion(updatePrompt.latestVersion);
          }}
          onMuteToday={() => {
            updateNotifications.current.muteToday();
            setUpdatePrompt(null);
          }}
          onUpdate={() => void performUpdate(updatePrompt)}
          onOpenUrl={(url) => void openExternalUrl(url)}
        />
      ) : null}
      {notice ? (
        <NoticeDialog
          key={`${notice.title}-${notice.message}-${notice.status ?? ""}`}
          notice={notice}
          onClose={() => setNotice(null)}
        />
      ) : null}
      <AppDialogHost />
      {liveConflictDialog ? (
        <LiveConflictDialog
          profileName={liveConflictDialog.profileName}
          onChoose={(choice) => {
            liveConflictDialog.resolve(choice);
            setLiveConflictDialog(null);
          }}
        />
      ) : null}
      {sessionIndexCleanupDialog ? (
        <SessionIndexCleanupDialog
          request={sessionIndexCleanupDialog}
          onCancel={() => {
            sessionIndexCleanupDialog.resolve(null);
            setSessionIndexCleanupDialog(null);
          }}
          onConfirm={(selectedIds) => {
            sessionIndexCleanupDialog.resolve(selectedIds);
            setSessionIndexCleanupDialog(null);
          }}
        />
      ) : null}
      {pendingProviderImport ? (
        <PendingProviderImportDialog
          request={pendingProviderImport.request}
          onConfirm={() => void confirmPendingProviderImport()}
          onDismiss={() => void dismissPendingProviderImport()}
        />
      ) : null}
      {pendingDreamSkinCommunity ? (
        <DreamSkinCommunityLinkDialog
          installPending={dreamSkinCommunityInstalling}
          versionId={pendingDreamSkinCommunity}
          onConfirm={() => void confirmPendingDreamSkinCommunity()}
          onDismiss={() => void dismissPendingDreamSkinCommunity()}
        />
      ) : null}
    </div>
  );
}

/// 清单时间是 ISO 字符串（UTC），按本地时区显示成「2026-10-17 17:46」；解析失败原样返回。
function formatMarketUpdatedAt(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/**
 * 侧栏 Claude 行的播放键只看这几位；页面每 3 秒会刷新一次状态，其余字段变了不必让整个 App 重渲染。
 */
function sameClaudeRunState(
  current: CommandResult<ClaudeLocalStatus> | null,
  next: CommandResult<ClaudeLocalStatus>,
): boolean {
  if (!current || current.status !== next.status) return false;
  return current.running === next.running
    && current.claudeRunning === next.claudeRunning
    && current.supported === next.supported
    && current.nodeAvailable === next.nodeAvailable
    && current.config?.enabled === next.config?.enabled;
}
/** 某张设置卡片所在的分类（深链「滚到版本与更新」之类）。 */
function settingsSectionsFor(sectionId: string): string[] {
  const entry = settingsNavEntries(true).find((candidate) => candidate.sections?.includes(sectionId));
  return entry?.sections ?? ["app-settings-general"];
}

function Badge({ status }: { status: string }) {
  return <UiBadge className={statusClass(status)} variant="secondary">{statusLabel(status)}</UiBadge>;
}

function contextEntriesWithLiveEntries(settings: BackendSettings, liveEntries: CodexContextEntries | null): CodexContextEntries {
  const commonEntries = contextEntriesFromSettings(settings);
  if (!liveEntries) return commonEntries;
  const liveByKind: Record<ContextKind, Map<string, CodexContextEntry>> = {
    mcp: new Map(liveEntries.mcpServers.map((entry) => [entry.id, entry])),
    skill: new Map(liveEntries.skills.map((entry) => [entry.id, entry])),
    plugin: new Map(liveEntries.plugins.map((entry) => [entry.id, entry])),
  };
  return {
    mcpServers: mergeLiveContextEntries(commonEntries.mcpServers, liveByKind.mcp),
    skills: mergeLiveContextEntries(commonEntries.skills, liveByKind.skill),
    plugins: mergeLiveContextEntries(commonEntries.plugins, liveByKind.plugin),
  };
}

function mergeLiveContextEntries(entries: CodexContextEntry[], liveEntries: Map<string, CodexContextEntry>): CodexContextEntry[] {
  const uniqueEntries = dedupeContextEntryList(entries);
  const merged = uniqueEntries.map((entry) => {
    const live = liveEntries.get(entry.id);
    return withLiveEntryState(entry, live);
  });
  const knownIds = new Set(uniqueEntries.map((entry) => entry.id));
  for (const liveEntry of liveEntries.values()) {
    if (!knownIds.has(liveEntry.id)) merged.push(liveEntry);
  }
  return merged;
}

function withLiveEntryState(entry: CodexContextEntry, live?: CodexContextEntry): CodexContextEntry {
  return live ? { ...entry, enabled: live.enabled } : entry;
}

/// 后端「降级但流程可继续」的状态。目前用于回填跳过（live config.toml 语法
/// 错误，issue #618）：要在提示用户的同时继续执行，不能当成失败中断。
function isDegradedStatus(status?: Status) {
  return status === "degraded";
}

function truncateSessionDeletePreview(value: string) {
  const normalized = value.trim();
  return normalized.length > 20 ? `${normalized.slice(0, 20)}...` : normalized;
}

function backendSettingsEqual(left: BackendSettings, right: BackendSettings): boolean {
  const { revision: _leftRevision, ...leftValues } = normalizeSettings(left);
  const { revision: _rightRevision, ...rightValues } = normalizeSettings(right);
  return JSON.stringify(leftValues) === JSON.stringify(rightValues);
}

function parsePort(value: string, fallback: number): number {
  const parsed = Number.parseInt(value, 10);
  return Number.isInteger(parsed) && parsed >= 1 && parsed <= 65535 ? parsed : fallback;
}

function relayProfileSwitchCommand(profile: RelayProfile): "clear_relay_injection" | "apply_relay_injection" | "apply_pure_api_injection" {
  if (isAggregateRelayProfile(profile)) return "apply_relay_injection";
  if (profile.relayMode === "pureApi") return "apply_pure_api_injection";
  if (profile.relayMode === "official" && !profile.officialMixApiKey) return "clear_relay_injection";
  if (profile.configContents.trim()) return "apply_relay_injection";
  return profile.officialMixApiKey ? "apply_relay_injection" : "clear_relay_injection";
}

function numberOrDefault(value: string, fallback: number) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

/** 截图 / 预览用的 `?previewRoute=`（可以是旧 id）。 */
function initialPreviewRoute(): string | null {
  if (typeof window === "undefined") return null;
  return new URLSearchParams(window.location.search).get("previewRoute");
}

/** 启动落在首页；`?previewRoute=` 指定页面（旧 id 换成现在的页面），要看更新时落在设置页。 */
function loadInitialRoute(): Route {
  if (typeof window === "undefined") return "home";
  const params = new URLSearchParams(window.location.search);
  const previewRoute = initialPreviewRoute();
  const previewTarget = previewRoute ? resolveRouteAlias<string>(previewRoute) : null;
  if (previewTarget && isKnownRoute(previewTarget)) {
    return previewTarget;
  }
  if (params.get("showUpdate") === "1" || window.location.hash === "#about") {
    return "settings";
  }
  return "home";
}

/** 「关于」已经并进「设置」：启动时要看更新，就滚到版本与更新。 */
function initialSettingsFocus(): string | null {
  if (typeof window === "undefined") return null;
  const params = new URLSearchParams(window.location.search);
  return params.get("showUpdate") === "1" || window.location.hash === "#about" || initialPreviewRoute() === "about" ? "app-settings-update" : null;
}

// 供应商编辑纯函数依赖的默认设置与上下文全选（见 relay-profile-editing.ts）。
configureRelayProfileEditing({
  defaultRelayBaseUrl: () => defaultSettings.relayBaseUrl,
  defaultRelayProfiles: () => defaultSettings.relayProfiles,
  contextSelectionForAllEntries,
});
