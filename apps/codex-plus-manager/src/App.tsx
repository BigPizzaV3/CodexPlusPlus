import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { open, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  ArrowRight,
  Blocks,
  Cable,
  BookOpen,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Copy,
  Download,
  GripVertical,
  Info,
  ScrollText,
  Github,
  GitBranch,
  ExternalLink,
  KeyRound,
  Layers3,
  Languages,
  House,
  List,
  Loader2,
  Palette,
  Play,
  MessageCircle,
  PackageOpen,
  FileCode2,
  FolderOpen,
  Moon,
  MousePointerClick,
  PawPrint,
  Network,
  PowerOff,
  Plus,
  RefreshCw,
  RotateCcw,
  Rocket,
  ScanLine,
  Search,
  Settings,
  Sparkles,
  ShieldCheck,
  ShieldAlert,
  Square,
  Store,
  Stethoscope,
  Sun,
  Trash2,
  Wrench,
  ArrowUp,
  AppWindow,
  Server,
  Tag,
  Waypoints,
  Plug,
  Split,
  BarChart3,
  MessagesSquare,
  SlidersHorizontal,
  TriangleAlert,
  CircleX,
  X,
  Upload,
  type LucideIcon,
} from "lucide-react";
import { createPresetPatch } from "@/components/ProviderPresetSelector";
import { PRESETS, type ProviderPreset } from "./presets";
import { setSponsorAds, useSponsorPresets } from "./sponsor-store";
import { createPortal } from "react-dom";
import { managerExtensionPages, managerNavPages, managerPageOverrides, type ManagerExtensionPage, type ManagerNavGroup } from "./extension-manager-pages";
import {
  createContext,
  Fragment,
  memo,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type ReactNode,
} from "react";
import { Badge as UiBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { isGitHubRepositoryHomepage } from "./github-repository";
import { AgentCachePanel } from "./agent-cache";
import { mergeRotatedAuthContents, rebaseSettingsEdits } from "./settings-rebase";
import { contextEntriesByKind, dedupeContextEntryList, type CodexContextEntries, type CodexContextEntry, type ContextKind } from "./context-config-text";
import { summarizeRecentRequests, type RecentRequestSummary } from "./overview-model";
import { AgentCollaborationPage } from "./AgentCollaborationPage";
import { DshPluginsPanel } from "./DshPluginsPanel";
import { ConfigDoctorPanel } from "./ConfigDoctorPanel";
import { CommunityExtensionsPanel, type CommunitySort } from "./CommunityExtensionsPanel";
import { ExtensionCard, githubAvatar } from "./ExtensionCard";
import { CuratedPackagesSection } from "./CuratedPackages";
import { ExtensionPublishDialog } from "./ExtensionPublishDialog";
import { MyPublishedPanel } from "./MyPublishedPanel";
import { AgentIcon, Switch } from "./gateway-ui";
import { MAIN_WINDOW_READY_EVENT, parseTrayRequest, TRAY_REQUEST_EVENT, TRAY_RESULT_EVENT, type TrayRequest, type TrayResult } from "./tray-panel-model";
import { AppSelect, type AppSelectOption } from "./AppSelect";
import { DisplaySettings } from "./DisplaySettings";
import { ExtensionConfigForm } from "./extension-config-form";
import { ExtensionManagerPanel } from "./ExtensionManagerPanel";
import { AppDialogHost, appConfirm } from "./app-dialog";
import { GatewayPage } from "./GatewayPage";
import { AgentGlyph, HubOverview, useGatewayPayload, useTodayUsage } from "./HubOverview";
import { publishGatewayPayload } from "./gateway-store";
import { diagnosticFacts, filterLogEntries, formatLogTime, parseLogText } from "./log-view-model";
import { CommandPalette, type PaletteCommand } from "./CommandPalette";
import { paletteShortcut } from "./command-palette-model";
import { SessionsView, sessionDeleteSucceeded } from "./AgentSessions";
import {
  agentName,
  groupDisplayName,
  formatQueueWait,
  formatRequestCost,
  formatTokens,
  maskAccountName,
  queueWaitMs,
  QUEUE_REASON_PREFIX,
  foreignCompactionDropped,
  FOREIGN_COMPACTION_DROPPED_REASON,
  compactionNote,
  parseCompactionNote,
  type CompactionNote,
  usageRangeStart,
  relayFilterFromFocus,
  relayFilterArgs,
  type RelayFilter,
  importCandidates,
  draftFromBinding,
  bindingFromDraft,
  validateDraft,
  type GatewayPayload,
  type ImportCandidate,
  type GatewayResult,
  type UsageRange,
} from "./gateway-model";
import { SessionAgentTabs, type SessionTab } from "./SessionAgentTabs";
import { UsageTabs, onOpenRequestLog, type RequestLogFocus } from "./UsageTabs";
import { noticeDuration, noticeTone } from "./notice-model";
import { agentTabGroups, pageVisibleForTool } from "./page-groups";
import { TopbarActions } from "./TopbarActions";
import { keepIfSame, usePolling } from "./use-polling";
import { useStableCallbacks } from "./use-stable-callbacks";
import { isRouteAlias, resolveRouteAlias } from "./route-aliases";
import { CodexLaunchButton, type CodexLaunchState } from "./CodexLaunchButton";
import { OnboardingTour } from "./OnboardingTour";
import { OnboardingSetup } from "./OnboardingSetup";
import { hasCodexEnhancements, SETUP_STORAGE_KEY, serializeSetupState, shouldShowSetup } from "./onboarding-setup-model";
import { AgentFocusDialog } from "./AgentFocusDialog";
import { AgentDisconnectDialog } from "./AgentDisconnectDialog";
import { agentVisible, commitAgentFocusSelection, providersHidden, singleAgent, type AgentFocus, type FocusMode } from "./agent-focus-model";
import { backupAgentFocus, needAgentFocusPrompt, resolveAgentFocusPrompt, restoreAgentFocusFromSettings, saveAgentFocus, useAgentFocus } from "./agent-focus-store";
import { AgentDashboard, type DashboardAgent, type DashboardRequestLogs, type DashboardStat, type DashboardStatus } from "./AgentDashboard";
import { TOUR_STORAGE_KEY, TOUR_VERSION } from "./onboarding-model";
import "./ToolPages.css";
import { BACKEND_SETTINGS_CHANGED_EVENT, ComputerUsePage } from "./ComputerUsePage";
import { RemoteControlPage, remoteControlRequest } from "./RemoteControlPage";
import { announceBackendSettingsChanged } from "./settings-events";
import { PluginMarketScreen } from "./PluginMarketScreen";
import { defaultMarketSort, parseMarketTime, relativeAge, sortMarketScripts, type LocalScriptState, type ScriptMarketSort } from "./script-market-model";
import { extensionEnabledForHost, extensionsForHost, extensionStateForHost, type ExtensionHost } from "./extension-host-model";
import { ReleaseNotes } from "./ReleaseNotes";
import { UPDATE_CHECK_INTERVAL_MS, UpdateNotifications, updatePreferencesFrom } from "./update-notification";
import { McpPage, Menu, MenuItem, PageHeader } from "./ManagementPages";
import { SkillsPage } from "./SkillsPage";
import { ProjectSessionPage } from "./ProjectSessionPage";
import type { LocalSessionFilter } from "./management-model";
import { managerNavigationDestination, type ManagerNavigationIntent } from "./enhancement-navigation";
import {
  builtinSettingValue,
  enhancementItem,
  isEnhancementItemMasterGated,
  managerBuiltinCards,
  managerStandaloneGroup,
  type EnhancementSettingItem,
} from "./enhancement-catalog";
import { aggregateMemberCandidates } from "./aggregate-members";
import { shouldBackfillRelayProfileBeforeSwitch } from "./relay-live-files";
import { NativeAuthControl } from "./NativeAuthControls";
import { relayBillingCacheKey, relayBillingProbeEligible, type RelayBillingProbe } from "./relay-billing";
import { RelayBillingLine, RelayBillingRetry, useRelayBillingProbe } from "./RelayBillingLine";
import { errorText } from "@/lib/utils";
import { mergeRequestLogDelta, reuseRequestLogEntries, type RequestLogCursor } from "./request-log-merge";
import { historyRecoveryStatusLabel } from "./history-recovery-report";
import { usePopoverLayer } from "./use-popover-layer";
import { resolveAnchoredMenuLayout, type AnchoredMenuLayout } from "./popover-layer";
import {
  providerSyncStreamPercent,
  resolveProviderSyncCompletion,
  type ProviderSyncStreamProgress,
} from "./provider-sync-flow";
import { isProviderSyncTargetSelectable, preferredProviderSyncTarget } from "./provider-sync-target";
import { resolveLaunchStatus, launchCompletionNotice } from "./launch-status";
import type { DreamSkinCommunityResult, DreamSkinRuntimeResult } from "./dream-skin";
import { subscribeLanguage, getLanguage, t, tf, toggleLanguage } from "@/i18n";
import {
  applyRelayProfilePatchToFiles,
  relayProfileSwitchValidation,
  syncLegacyRelayFields,
  createRelayProfile,
  createAggregateRelayProfile,
  duplicateRelayProfile,
  reorderRelayProfiles,
  isAggregateRelayProfile,
  relayProviderView,
  aggregateProfileForMode,
  normalizeAggregateConfig,
  activeRelayProfile,
  configureRelayProfileEditing,
  type ProviderViewKind,
} from "./relay-profile-editing";
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
  multiLaunchInstanceCanRestart,
  multiLaunchInstanceCanStop,
  multiLaunchInstanceFailure,
  multiLaunchInstanceIsMain,
  multiLaunchInstanceLabel as formatMultiLaunchInstanceLabel,
  multiLaunchInstanceRunning,
  multiLaunchInstanceStatus,
  multiLaunchInstanceVisible,
  runningInstancesForProfile,
  type MultiLaunchInstance,
} from "./multi-launch-model";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxFilters, CxRow, CxSection, CxStatusIcon, CxToolbar } from "./cx-layout";
import { Puzzle } from "lucide-react";
import * as AgentCx from "./cx-layout"; // cx-pages:agents：Agent 页面在函数内解构，避免和别的分组的具名导入重名
import type {
  Actions,
  AdItem,
  AdsResult,
  AggregateMode,
  AutoSaveOutcome,
  AutoTakeoverResult,
  BackendSettings,
  BuiltinExtension,
  CcsProvidersResult,
  ClaudeBashTimeoutSettings,
  ClaudeLocalConfig,
  ClaudeLocalizationStatus,
  ClaudeLocalStatus,
  ClaudeTemplateVersion,
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
  ProviderSyncTargetOption,
  ProviderSyncTargetSource,
  ProviderSyncTargetsResult,
  RelayEnvironmentResult,
  RelayFilesResult,
  RelayProfile,
  RequestLogEntry,
  ScriptMarketItem,
  ScriptMarketResult,
  SessionIndexCleanupCandidate,
  SessionIndexRepairReport,
  SettingsPayload,
  SettingsResult,
  SshRemoteLoopbackNoticeData,
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
  APP_LOGO_URL,
  AutoSaveStatus,
  autoTakeoverView,
  clampNumber,
  claudeLocalizationMemo,
  claudeLocalMemo,
  CODEX_GATEWAY_RELAY_ID,
  codexGatewayModelCount,
  confirmRemoveRelayProfile,
  contextEntriesFromSettings,
  contextSelectionForAllEntries,
  defaultSettings,
  enhancementPlatform,
  formatProgressPercent,
  formatTime,
  healthItems,
  isSuccessStatus,
  Metric,
  multiLaunchSupported,
  normalizeRetryStatusCodes,
  normalizeSettings,
  providerInitial,
  relayProfileConfigBrief,
  relayProfileModeLabel,
  relayProfileTagTone,
  relayProtocolLabel,
  relayProtocolShortLabel,
  requestLogAgent,
  SettingRow,
  SettingsAutoSave,
  StatusBadge,
  statusClass,
  statusLabel,
  stringifyError,
  TaskProgressBox,
  ToggleVisual,
  Toolbar,
  useEnhanceAutoSave,
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
  routeVisible,
  settingsModeRoutes,
  type ToolId,
  visibleActiveTool,
  visibleRoutes,
} from "./routes";
import { RelayProfileDetail } from "./RelayEditor";

/// 会话维护页的两条开关与 Codex 设置页共用清单里的文案（#2846 拆开后分别显示）。
const providerSyncItem = enhancementItem("providerSyncEnabled");
const sessionIndexRepairItem = enhancementItem("sessionIndexRepairEnabled");

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
const SCRIPT_MARKET_REPOSITORY_URL = "https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket";

type RequestLogsResult = {
  status: Status;
  message: string;
  entries: RequestLogEntry[];
  /** 符合条件的不止返回的这些。 */
  more?: boolean;
  /** 日志文件里最早一条的开始时间（日志按大小滚动）。 */
  oldestMs?: number | null;
  /** 文件里最后读到的位置：下一轮带上它只读新写入的记录。 */
  cursor?: RequestLogCursor | null;
  /** 带游标读到的增量：entries 只是新完成的 + 进行中的，要并进已有列表。 */
  incremental?: boolean;
};

/** apply_context_entry：saved 是保存后的设置，liveEntries 是同步后 live config.toml 里的条目（失败的那一步之后为 null）。 */
type AppliedContextEntryResult = CommandResult<{
  saved: SettingsPayload | null;
  liveEntries: CodexContextEntries | null;
}>;

type WeixinConnectCatalog = {
  status: string;
  message?: string;
  model?: string;
  default_model?: string;
  models: string[];
  modelMetadata?: Record<string, {
    displayName?: string;
    supportedReasoningEfforts?: { reasoningEffort: string; description?: string }[];
  }>;
};

type WeixinConnectProjects = {
  projects: { id: string; name: string; roots?: { path: string }[] }[];
};

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

const providerSyncSourceLabels: Record<ProviderSyncTargetSource, string> = {
  get config() { return t("配置"); },
  get rollout() { return t("会话"); },
  get sqlite() { return t("索引"); },
  get manual() { return t("手动"); },
};

function providerSyncTargetLabel(target: ProviderSyncTargetOption): string {
  const labels = target.sources.map((source) => providerSyncSourceLabels[source]).filter(Boolean);
  const current = target.isCurrentProvider ? [t("当前")] : [];
  const unavailable = isProviderSyncTargetSelectable(target) ? [] : [t("供应商切换不可用")];
  return [...labels, ...current, ...unavailable].join(" / ") || t("发现");
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
/** 设置页里「使用模式」一行的说明。 */
function agentFocusDescription(focus: AgentFocus): string {
  if (focus === "codex") return t("只管理 Codex：隐藏其他 Agent 的界面，已有网关连接保持不变。");
  if (focus === "enhance") return t("只用增强功能：隐藏供应商、网关和用量，关闭旧版供应商配置切换。已有网关连接仍会运行并管理对应配置。");
  return t("全部功能：同时管理 Codex、Claude Code、Grok 等多个 Agent。只用 Codex 或只用增强功能的话，可以切换模式隐藏用不到的内容。");
}
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
  const [setupOpen, setSetupOpen] = useState(initialSetupOpen);
  // 使用模式（只管理 Codex / 全部功能）：没问过就先问，问完再进首次引导（引导按模式换说法）。
  const agentFocus = useAgentFocus();
  // 问卷等设置文件读回来再决定弹不弹（#2936）：本地存储为空不等于没选过，设置文件里可能记着。
  const [agentFocusOpen, setAgentFocusOpen] = useState(false);
  // 判定期间入门清单也先不出，免得它先闪一下再被问卷盖住。
  // 由启动流程在设置读回来后判定一次（见下方启动 effect）。
  const [agentFocusChecking, setAgentFocusChecking] = useState(needAgentFocusPrompt);
  const cancelAgentFocus = useCallback(() => {
    setAgentFocusOpen(false);
    if (!agentFocus) {
      // 第一次问时点「稍后设置」：按全部功能记下（和没选时的显示一样），之后启动不再追问，设置里随时可改（#2936）。
      saveAgentFocus("all");
      setSetupOpen(false);
    }
  }, [agentFocus]);
  // 本地存储丢了使用模式、设置文件里还记着时补回来，问卷不再每次启动都弹（#2902）。
  const settingsLoaded = Boolean(settings);
  const savedAgentFocus = settings?.settings.managerAgentFocus;
  useEffect(() => {
    if (!settingsLoaded) return;
    if (restoreAgentFocusFromSettings(savedAgentFocus)) {
      setAgentFocusOpen(false);
    } else if (!savedAgentFocus && agentFocus) {
      // 升级前就选过模式的老用户：设置文件里还没有备份，补存一次。
      backupAgentFocus(agentFocus);
    }
  }, [settingsLoaded, savedAgentFocus, agentFocus]);
  const closeSetup = (completed: boolean) => {
    try { window.localStorage.setItem(SETUP_STORAGE_KEY, serializeSetupState(completed ? "completed" : "dismissed")); } catch { /* 本次窗口内仍可关闭。 */ }
    setSetupOpen(false);
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
        if (agentFocusOpen || tourOpen) return;
        setPaletteOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [agentFocusOpen, tourOpen]);
  useEffect(() => {
    if (agentFocusOpen || tourOpen) setPaletteOpen(false);
  }, [agentFocusOpen, tourOpen]);
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
  // 供应商池右下角「正在找合适的供应商？」的悬浮提示：点 × 关掉，本次运行内不再出现。
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

  // Computer Use 和 Claude 增强页的开关、使用模式由后端单独写进 settings.json：只合并这些键和版本号，
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
          // 使用模式也要带上：表单留着旧值，下一次整份保存会把刚记下的模式写回去。
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

  const importCcsProviders = async (bindings: Record<string, string> = {}) => {
    const result = await run(() => call<SettingsResult>("import_ccs_providers", { bindings }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showResultNotice(t("cc-switch 导入"), result);
      await refreshCcsProviders(true);
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
    const needsSettings: Route[] = ["relay", "multiLaunch", "sessions", "context", "weixin", "settings", "agentSettings", "enhance", "userScripts"];
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

  const closeAgentFocus = (mode: FocusMode) => commitAgentFocusSelection(mode, {
    disableRelaySwitching: async () => {
      // 用最新版本关闭旧配置切换，失败时保留原模式和问卷，不提前隐藏管理入口。
      const latest = await run(() => call<SettingsResult>("load_settings"));
      if (!latest || !isSuccessStatus(latest.status)) {
        if (latest) showNotice(t("设置读取失败"), latest.message, latest.status);
        return false;
      }
      const next = normalizeSettings({ ...latest.settings, relayProfilesEnabled: false });
      const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
      if (!result || !isSuccessStatus(result.status)) {
        if (result) showNotice(t("设置保存"), result.message, result.status);
        return false;
      }
      const saved = normalizeSettings(result.settings);
      const previous = normalizeSettings(settingsRef.current?.settings ?? latest.settings);
      setSettings(result);
      // 模式选择只修改这一个开关；其他页面尚未保存的编辑继续保留。
      setSettingsForm((current) => ({
        ...rebaseSettingsEdits(normalizeSettings(current), previous, saved),
        relayProfilesEnabled: false,
      }));
      return !saved.relayProfilesEnabled;
    },
    commit: (next) => {
      const firstChoice = agentFocus === null;
      saveAgentFocus(next);
      setAgentFocusOpen(false);
      if (firstChoice) setSetupOpen(true);
    },
  });

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
    // 本地加载互不依赖，一起发出；设置最先发，使用模式问卷等它读回来再决定弹不弹。
    const settingsLoad = refreshSettings(true);
    if (agentFocusChecking) {
      // 本地存储为空不等于没选过，设置文件里可能记着（#2936）。
      void resolveAgentFocusPrompt(async () => (await settingsLoad)?.managerAgentFocus).then((ask) => {
        if (ask) setAgentFocusOpen(true);
        setAgentFocusChecking(false);
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
   * 沿用上次的接入设置（默认模型等）；供应商池是空的或模型不在池里时说明原因并带去对应页面。
   * 切回直连走「断开并还原」（先预览再确认），见 legacyDisconnectOpen。
   */
  const [codexConnecting, setCodexConnecting] = useState(false);
  const connectCodexGateway = async () => {
    if (codexConnecting) return;
    setCodexConnecting(true);
    try {
      const loaded = await invoke<GatewayResult>("load_gateway");
      const payload = loaded.gateway && Array.isArray(loaded.pool) ? (loaded as GatewayPayload) : null;
      const previous = payload?.gateway.bindings?.codex;
      const draft = draftFromBinding(previous);
      const issue = validateDraft(draft, payload);
      if (issue) {
        await showNotice(t("切换到网关"), t(issue), "failed");
        void navigate(issue.includes("供应商池是空的") ? "gatewayProviders" : "gateway");
        return;
      }
      const result = await invoke<GatewayResult>("save_gateway_binding", { agent: "codex", binding: { ...bindingFromDraft(draft, previous), enabled: true }, reconnect: true });
      publishGatewayPayload(result);
      await refreshSettings(true);
      await showNotice(t("切换到网关"), result.message, isSuccessStatus(result.status as Status) ? "ok" : "failed");
    } catch (error) {
      await showNotice(t("切换到网关"), stringifyError(error), "failed");
    } finally {
      setCodexConnecting(false);
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
  // 没勾选的 Agent 不进侧栏和命令面板；只影响显示，它的配置和网关接入照旧。
  const shownTools = toolEntries.filter((entry) => agentVisible(agentFocus, entry.id));
  const soloAgent = singleAgent(agentFocus);
  // MCP 页的条目：设置里的条目叠上 live config.toml 的启停状态。只在两者变化时重算，免得每次渲染都给 McpPage 一个新数组。
  const mcpPageEntries = useMemo(() => {
    const entries = contextEntriesWithLiveEntries(settingsForm, liveContextEntries);
    return [...contextEntriesByKind(entries, "mcp"), ...contextEntriesByKind(entries, "plugin")];
  }, [settingsForm, liveContextEntries]);
  // 选中的页签对应的 Agent 被隐藏了（换了使用模式）时退回 Codex。
  const shownSessionTab: SessionTab = soloAgent === "codex" || (sessionTab !== "all" && !agentVisible(agentFocus, sessionTab)) ? "codex" : sessionTab;
  // 侧栏聚焦的 Agent 被隐藏了：换到第一个还显示的 Agent，免得停在一个看不见的 Agent 的页面上。
  const fallbackTool = shownTools.find((entry) => entry.switchable)?.id;
  useEffect(() => {
    if (agentFocus && fallbackTool && !agentVisible(agentFocus, activeTool)) void switchTool(fallbackTool);
    // switchTool 每次渲染都是新函数，只在选择或聚焦变化时检查。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentFocus, activeTool, fallbackTool]);
  // 停在当前模式下隐藏的页面上（启动默认的网关总览、托盘跳过来的请求日志等）：回到当前 Agent 的落脚页。
  useEffect(() => {
    if (!routeVisible(agentFocus, route)) void navigate(agentHomeRoute[activeTool] ?? "overview");
    const currentRoute = routes.find((candidate) => candidate.id === route);
    if (currentRoute?.tools && currentRoute.id !== "userScripts" && !pageVisibleForTool(currentRoute, activeTool)) void navigate(agentHomeRoute[activeTool] ?? "overview");
    // navigate 每次渲染都是新函数，只在模式或页面变化时检查。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentFocus, route, activeTool]);
  useEffect(() => {
    if (!agentVisible(agentFocus, extensionHost)) setExtensionHost("codex");
  }, [agentFocus, extensionHost]);
  // 旧版曾在模式切换后才保存配置；遇到遗留的未保存状态，重新确认并允许用户重试。
  const focusRepairRequested = useRef(false);
  useEffect(() => {
    if (agentFocus === "enhance" && settings?.settings.relayProfilesEnabled && !focusRepairRequested.current) {
      focusRepairRequested.current = true;
      setAgentFocusOpen(true);
    }
  }, [agentFocus, settings]);

  // ⌘K：页面、各 Agent 的页面和常用操作。非当前 Agent 的页面会先切到那个 Agent。
  const paletteCommands: PaletteCommand[] = [];
  const routeCommand = (routeId: Route, group: string, run: () => void, keywords = ""): PaletteCommand | null => {
    const item = routes.find((candidate) => candidate.id === routeId);
    if (!item || !routeVisible(agentFocus, routeId)) return null;
    if (item.tools && routeId !== "userScripts" && !pageVisibleForTool(item, activeTool)) return null;
    const Icon = item.icon;
    return { id: `${group}:${routeId}`, group, label: item.label, keywords: `${routeId} ${keywords}`, icon: <Icon className="h-4 w-4" />, run };
  };
  const pushCommand = (command: PaletteCommand | null) => { if (command) paletteCommands.push(command); };
  pushCommand(routeCommand("home", t("全局管理"), () => void navigate("home"), "home ads 推荐 赞助"));
  gatewayNavigationRoutes.forEach((routeId) => pushCommand(routeCommand(routeId, t("网关"), () => void navigate(routeId), "gateway")));
  managementNavigationRoutes.forEach((routeId) => pushCommand(routeCommand(routeId, t("全局管理"), () => void navigate(routeId))));
  pushCommand(routeCommand("requestLog", t("全局管理"), () => void navigate("requestLog"), "usage requests"));
  for (const tool of shownTools.filter((entry) => entry.switchable)) {
    agentRoutes(tool.id).forEach((routeId) => pushCommand(routeCommand(routeId, tool.name, () => void switchTool(tool.id, routeId), tool.id)));
  }
  // 设置的各个分类（含缓存清理、Computer Use 等设置模式里的页面）都能搜到：「更新」「日志」「配置体检」……
  // 网关设置已在「网关」一组里，不重复列。
  const paletteRoutes = new Set(paletteCommands.map((command) => command.id.slice(command.id.indexOf(":") + 1)));
  for (const entry of settingsNavEntries(agentFocus, shownTools.some((tool) => tool.id === "codex" && tool.switchable))) {
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
  const settingsEntries = settingsNavEntries(agentFocus, true);
  const settingsEntryKey = settingsModeRoutes.has(route) ? activeSettingsEntry(settingsEntries, route, settingsSections) : null;
  const agentPageGroups = agentTabGroups(navigationSections, routeTool, activeTool).flatMap((entry) => visibleRoutes(agentFocus, entry.routes));
  const onAgentPage = !isGlobalPage && !settingsModeRoutes.has(route) && agentPageGroups.length > 1;
  const pageTitle = route === "extension"
    ? activeExtensionPage?.title ?? t("拓展页面")
    : settingsEntryKey
      ? settingsEntries.find((entry) => entry.key === settingsEntryKey)?.label ?? routeTitle(route)
      : customizeRoutes.includes(route)
        ? t("MCP 与 Skills")
        : onAgentPage
          ? routeTitle(route)
          : soloAgent && route === "gateway"
            ? tf("{0} 接入", [agentName(soloAgent)])
            : routeTitle(navAliasRoute[route] ?? route);

  // 入门清单放在侧栏底部（参照 Cursor 的 Getting Started 卡片）：平时收成一张进度卡，点开在上方展开清单。
  const setupCard = setupOpen && !agentFocusOpen && !agentFocusChecking ? (
          <OnboardingSetup
            focus={agentFocus}
            relaySettings={settings ? normalizeSettings(settings.settings) : null}
            codexInstalled={overview ? overview.codex_app.status === "found" : null}
            codexRunning={codexAppRunning}
            launchPending={launchPending}
            codexEnhancementsEnabled={hasCodexEnhancements(settings)}
            notify={actions.showMessage}
            onChangeMode={() => setAgentFocusOpen(true)}
            onDismiss={closeSetup}
            onTour={() => setTourOpen(true)}
            onNavigate={(target, agent) => { if (agent) void switchTool(agent, target); else void navigate(target); }}
            onAddProvider={() => { setAddDirectProviderRequested(true); void switchTool("codex", "relay"); }}
            onCheckCodex={async () => {
              const currentApp = await run(() => call<OverviewResult>("load_overview"));
              const currentSettings = await run(() => call<SettingsResult>("load_settings"));
              if (!currentApp || !currentSettings || !isSuccessStatus(currentApp.status) || !isSuccessStatus(currentSettings.status)) return false;
              setOverview(currentApp);
              const fresh = normalizeSettings(currentSettings.settings);
              const previous = normalizeSettings(settingsRef.current?.settings ?? currentSettings.settings);
              setSettings(currentSettings);
              setSettingsForm((current) => rebaseSettingsEdits(normalizeSettings(current), previous, fresh));
              return currentApp.codex_app.status === "found" && hasCodexEnhancements(currentSettings);
            }}
            onLaunch={() => codexAppRunning ? restart() : launch()}
            defaultExpanded={false}
          />
        ) : null;

  return (
    <div className={`shell ${theme}`}>
      <AppSidebar
        setupCard={setupCard}
        tools={shownTools}
        soloAgent={soloAgent}
        focus={agentFocus}
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
      {agentFocusOpen ? <AgentFocusDialog initial={agentFocus} onDone={closeAgentFocus} onCancel={cancelAgentFocus} /> : null}
      {legacyDisconnectOpen ? (
        <AgentDisconnectDialog
          agent={{ agent: "codex", name: "Codex" }}
          busy={false}
          onApply={async (previewToken) => {
            try {
              const result = await invoke<GatewayResult>("disable_gateway_binding", { agent: "codex", previewToken });
              await showNotice(result.status === "ok" ? t("已断开网关") : t("断开失败"), result.message, result.status === "ok" ? "ok" : "failed");
              await refreshSettings(true);
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
              await showNotice(result.status === "ok" ? t("已断开网关") : t("断开失败"), result.message, result.status === "ok" ? "ok" : "failed");
              await refreshSettings(true);
              return result.status === "ok";
            } catch (error) {
              await showNotice(t("断开失败"), String(error), "failed");
              return false;
            }
          }}
          onClose={() => setLegacyDisconnectOpen(false)}
          onAddDirectProvider={() => setAddDirectProviderRequested(true)}
        />
      ) : null}
      {tourOpen && !agentFocusOpen ? <OnboardingTour focus={agentFocus} onClose={closeTour} /> : null}
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
              <h1 title={route === "extension" ? undefined : routeSubtitle(navAliasRoute[route] ?? route, soloAgent)}>{pageTitle}</h1>
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
          {/* 「Codex 连接」一页：连接方式（经网关 / 直连）+ 直连供应商（可加入供应商池）+ 高级（原「旧版代理设置」）。
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
          {route === "requestLog" ? (
            <RequestLogScreen
              initialAgent={requestLogAgentFilter}
              initialFocus={requestLogFocus}
              onConsumeInitialAgent={() => { setRequestLogAgentFilter("all"); setRequestLogFocus({}); }}
            />
          ) : null}
          {route === "claude" ? (
            <ClaudeLocalizationScreen
              onLocalStatus={(status) => {
                // 页面里启动 / 停止后，侧栏 Claude 行的播放键立刻跟着变，不用轮询。
                claudeLocalMemo.status = status;
                setClaudeLocalRun((current) => (sameClaudeRunState(current, status) ? current : status));
              }}
            />
          ) : null}
          {route === "claudePatch" ? <ClaudeLocalizationScreen mode="patch" /> : null}
          {/* 只管理 Codex 时没有 Agent 页签，直接是 Codex 的完整会话管理。 */}
          {route === "sessions" && !soloAgent ? <SessionAgentTabs onChange={setSessionTab} value={sessionTab} /> : null}
          {route === "sessions" && shownSessionTab !== "codex" ? (
            <div className="mg-page gateway-page">
              <div className="mg-content gw-content">
                <SessionsView agent={shownSessionTab} notify={actions.showMessage} />
              </div>
            </div>
          ) : null}
          {route === "sessions" && shownSessionTab === "codex" ? (
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
          {route === "userScripts" ? <UserScriptsScreen host={extensionHost} focus={agentFocus} onHostChange={setExtensionHost} settings={settings} market={scriptMarket} onOpenRescue={() => void navigate("enhance")} onOpenPluginMarket={() => void switchTool("codex", "pluginMarket")} onOpenClaude={() => void switchTool("claude", "claude")} actions={actions} /> : null}
          {route === "pluginMarket" ? <PluginMarketScreen onOpenExtensions={() => void navigate("userScripts")} /> : null}
          {route === "home" ? (
            <HomeScreen
              ads={ads}
              actions={actions}
              tools={shownTools}
              focus={agentFocus}
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
                <SharedServiceSettings
                  autoSave={<SettingsAutoSave dirty={settingsDirty} form={settingsForm} actions={actions} />}
                  form={settingsForm}
                  onFormChange={setSettingsForm}
                  overview={overview}
                />
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
          {/* 供应商池右下角的悬浮提示：在找供应商时顺手带去首页的赞助商推荐；首页没有赞助商时不显示。 */}
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
                  onOpenTour={() => {
                    // 入门清单和导览指的都是主侧栏里的入口，设置模式的侧栏里没有：先回到进设置前的页面，
                    // 再打开入门清单并开始导览，点了马上能看到。
                    setSetupOpen(true);
                    void navigate(lastMainRoute.current);
                    setTourOpen(true);
                  }}
                  agentFocus={agentFocus}
                  onOpenAgentFocus={() => setAgentFocusOpen(true)}
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
        </section>
      </main>
      {updatePrompt && !agentFocusOpen && !tourOpen ? (
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

function SearchablePathPicker({
  value,
  options,
  placeholder,
  onChange,
}: {
  value: string;
  options: string[];
  placeholder: string;
  onChange: (value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const filteredOptions = useMemo(() => {
    const query = value.trim().toLowerCase();
    return options.filter((option) => !query || option.toLowerCase().includes(query)).slice(0, 30);
  }, [options, value]);
  // 外点关闭/滚动跟随/多开约束统一交给 usePopoverLayer；菜单 portal 出去，不被滚动区裁掉。
  const { rootRef, menuRef, menuStyle } = usePopoverLayer({ open, onOpenChange: setOpen, maxHeight: 280 });

  return (
    <div className="weixin-search-picker" ref={rootRef}>
      <div className="weixin-search-input-wrap">
        <Search className="weixin-search-input-icon h-4 w-4" />
        <Input
          aria-expanded={open}
          aria-label={placeholder}
          className="h-10"
          onChange={(event) => {
            onChange(event.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setOpen(false);
          }}
          placeholder={placeholder}
          value={value}
        />
        <ChevronDown className={`weixin-search-input-chevron h-4 w-4${open ? " is-open" : ""}`} />
      </div>
      {open ? createPortal(
        <div className="weixin-search-menu weixin-search-menu-portal" ref={menuRef} role="listbox" style={menuStyle}>
          {filteredOptions.length ? filteredOptions.map((option) => (
            <button
              className="weixin-search-option"
              key={option}
              onClick={() => {
                onChange(option);
                setOpen(false);
              }}
              type="button"
            >
              <span>{option}</span>
            </button>
          )) : (
            <div className="weixin-search-empty">{t("没有匹配的已有目录，可继续直接输入。")}</div>
          )}
        </div>,
        document.body,
      ) : null}
    </div>
  );
}

function SearchableSessionPicker({
  sessions,
  selectedId,
  onSelect,
}: {
  sessions: LocalSession[];
  selectedId: string;
  onSelect: (session: LocalSession | null) => void;
}) {
  const selected = sessions.find((session) => session.id === selectedId) ?? null;
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const filteredSessions = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    return sessions
      .filter((session) => !normalized || [session.title, session.cwd, session.id, session.modelProvider].some((value) => value.toLowerCase().includes(normalized)))
      .slice(0, 30);
  }, [query, sessions]);
  const { rootRef, menuRef, menuStyle } = usePopoverLayer({ open, onOpenChange: setOpen, maxHeight: 280 });

  useEffect(() => {
    if (selected) setQuery(selected.title || selected.id);
  }, [selected]);

  return (
    <div className="weixin-search-picker" ref={rootRef}>
      <div className="weixin-search-input-wrap">
        <Search className="weixin-search-input-icon h-4 w-4" />
        <Input
          aria-expanded={open}
          aria-label={t("已有会话")}
          className="h-10"
          onChange={(event) => {
            setQuery(event.target.value);
            if (selectedId) onSelect(null);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setOpen(false);
          }}
          placeholder={sessions.length ? t("搜索已有会话") : t("暂无可用的本地会话")}
          value={query}
        />
        <ChevronDown className={`weixin-search-input-chevron h-4 w-4${open ? " is-open" : ""}`} />
      </div>
      {open ? createPortal(
        <div className="weixin-search-menu weixin-search-menu-portal weixin-session-menu" ref={menuRef} role="listbox" style={menuStyle}>
          {filteredSessions.length ? filteredSessions.map((session) => (
            <button
              aria-selected={session.id === selectedId}
              className="weixin-search-option weixin-session-option"
              key={session.id}
              onClick={() => {
                onSelect(session);
                setQuery(session.title || session.id);
                setOpen(false);
              }}
              type="button"
            >
              <strong>{session.title || t("未命名会话")}</strong>
              <span>{session.cwd || t("未记录项目路径")}</span>
              <small>{formatTime(session.updatedAtMs ?? 0)} · {session.modelProvider || t("provider 未记录")}</small>
            </button>
          )) : (
            <div className="weixin-search-empty">{t("没有匹配的本地会话。")}</div>
          )}
        </div>,
        document.body,
      ) : null}
    </div>
  );
}

function WeixinConnectScreen({
  form,
  status,
  qr,
  sessions,
  onFormChange,
  autoSave,
  onQrLogin,
  onStart,
  onStop,
  onChooseWorkDir,
  onChooseCodexPath,
  onUseDesktopCodexCli,
  onOpenQr,
  onCopyQr,
}: {
  form: BackendSettings;
  status: WeixinConnectStatusResult | null;
  qr: WeixinQrResult | null;
  sessions: LocalSession[];
  onFormChange: (value: BackendSettings) => void;
  autoSave: ReactNode;
  onQrLogin: () => void;
  onStart: () => void;
  onStop: () => void;
  onChooseWorkDir: () => void;
  onChooseCodexPath: () => void;
  onUseDesktopCodexCli: () => void;
  onOpenQr: (url: string) => void;
  onCopyQr: (url: string) => void;
}) {
  const [selectedSessionId, setSelectedSessionId] = useState("");
  const gatewayPayload = useGatewayPayload(10_000);
  const [catalog, setCatalog] = useState<WeixinConnectCatalog | null>(null);
  const [catalogError, setCatalogError] = useState("");
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [catalogRevision, setCatalogRevision] = useState(0);
  const [projects, setProjects] = useState<WeixinConnectProjects | null>(null);
  const [projectsError, setProjectsError] = useState("");
  const [projectsLoading, setProjectsLoading] = useState(false);
  const [projectsRevision, setProjectsRevision] = useState(0);
  const [resetPeerId, setResetPeerId] = useState("");
  const [resetPending, setResetPending] = useState(false);
  const [resetMessage, setResetMessage] = useState("");
  // 加加跑在远程控制里：没开时微信里的加加不可用，这里提示一下（null = 还没读到）。
  const [remoteControlRunning, setRemoteControlRunning] = useState<boolean | null>(null);

  useEffect(() => {
    let disposed = false;
    remoteControlRequest("status")
      .then((value) => { if (!disposed) setRemoteControlRunning(Boolean(value.running)); })
      .catch(() => { if (!disposed) setRemoteControlRunning(null); });
    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let inFlight = false;
    const refresh = async () => {
      if (inFlight) return;
      inFlight = true;
      setCatalogLoading(true);
      try {
        const result = await invoke<WeixinConnectCatalog>("weixin_connect_catalog");
        if (disposed) return;
        if (result.status === "failed" || !Array.isArray(result.models)) {
          throw new Error(result.message || t("读取模型目录失败。"));
        }
        setCatalog(result);
        setCatalogError("");
      } catch (error) {
        if (!disposed) setCatalogError(String(error));
      } finally {
        inFlight = false;
        if (!disposed) setCatalogLoading(false);
      }
    };
    void refresh();
    return () => {
      disposed = true;
    };
  }, [catalogRevision]);

  const projectCodexPath = form.weixinConnectCodexPath.trim();
  const projectWorkDir = form.weixinConnectWorkDir.trim();
  useEffect(() => {
    let disposed = false;
    let inFlight = false;
    setProjects(null);
    setProjectsError("");
    if (!projectsRevision && !form.weixinConnectProjectId) {
      setProjectsLoading(false);
      return;
    }
    const refresh = async () => {
      if (inFlight) return;
      inFlight = true;
      setProjectsLoading(true);
      try {
        const result = await invoke<WeixinConnectProjects>("weixin_connect_projects", {
          codexPath: projectCodexPath,
          workDir: projectWorkDir,
        });
        if (disposed) return;
        if (!Array.isArray(result.projects)) throw new Error(t("读取项目列表失败。"));
        setProjects(result);
        setProjectsError("");
      } catch (error) {
        if (!disposed) setProjectsError(String(error));
      } finally {
        inFlight = false;
        if (!disposed) setProjectsLoading(false);
      }
    };
    // 输入目录或 CLI 路径时稍等，避免每个按键都启动一次项目查询。
    const initialTimer = window.setTimeout(() => void refresh(), 350);
    return () => {
      disposed = true;
      window.clearTimeout(initialTimer);
    };
  }, [projectCodexPath, projectWorkDir, projectsRevision]);

  const models = catalog?.models ?? [];
  const inheritedModel = catalog?.model || catalog?.default_model || "";
  const effectiveModel = form.weixinConnectModel || inheritedModel;
  const reasoningEfforts = Array.from(new Set(
    (catalog?.modelMetadata?.[effectiveModel]?.supportedReasoningEfforts ?? [])
      .map((entry) => entry.reasoningEffort)
      .filter(Boolean),
  ));
  const modelUnavailable = Boolean(catalog && !catalogError && form.weixinConnectModel
    && !models.includes(form.weixinConnectModel));
  const reasoningUnavailable = Boolean(catalog && !catalogError && form.weixinConnectReasoningEffort
    && !reasoningEfforts.includes(form.weixinConnectReasoningEffort));
  const projectOptions = projects?.projects ?? [];
  const projectUnavailable = Boolean(projects && !projectsError && form.weixinConnectProjectId
    && !projectOptions.some((project) => project.id === form.weixinConnectProjectId));
  const workDirOptions = useMemo(
    () => Array.from(new Set(sessions.map((session) => session.cwd.trim()).filter(Boolean))).sort(),
    [sessions],
  );
  const runtimeState = status?.state ?? "stopped";
  const running = ["starting", "running", "retrying"].includes(runtimeState);
  const stopping = runtimeState === "stopping";
  const runtimeLabel = {
    starting: t("正在启动"),
    running: t("运行中"),
    retrying: t("正在重试"),
    stopping: t("正在停止"),
    error: t("异常"),
    stopped: t("已停止"),
  }[runtimeState] ?? statusLabel(runtimeState);

  const { CxBadge, CxCard, CxRow, CxSection, CxStatusIcon } = AgentCx;
  const statusTone = runtimeState === "running" ? "ok" : runtimeState === "error" ? "danger" : running ? "warn" : "off";
  const accountLabel = status?.accountId || form.weixinConnectAccountId || t("未登录");
  const loggedIn = Boolean(form.weixinConnectToken);
  const outboundBlocked = !form.weixinConnectAllowOutboundAttachments && (
    !form.weixinConnectAllowFrom.split(",").some((peer) => peer.trim())
    || form.weixinConnectAllowFrom.split(",").some((peer) => peer.trim() === "*")
    || !form.weixinConnectWorkDir.trim()
  );
  const resetSession = async () => {
    setResetPending(true);
    setResetMessage("");
    try {
      await invoke("weixin_connect_reset_session", { peerId: resetPeerId.trim(), accountId: form.weixinConnectAccountId.trim() });
      setResetMessage(t("已保存重置请求，下一条消息开启新会话，原历史保留。"));
    } catch (error) {
      setResetMessage(String(error));
    } finally {
      setResetPending(false);
    }
  };

  return (
    <div className="weixin-connect-page sectioned-page cx-page">
      {/* 状态一行：没登录时唯一的主操作是扫码登录；登录后是启动 / 停止，重新登录收进三点菜单。 */}
      <CxCard className={`weixin-status-panel is-${runtimeState}`}>
        <div id="weixin-section-status">
          <CxRow
            badges={<CxBadge tone={statusTone === "ok" ? "ok" : statusTone === "danger" ? "danger" : "neutral"}>{runtimeLabel}</CxBadge>}
            detail={(
              <span className="weixin-status-detail">
                <span title={status?.message ?? t("微信连接未启动。")}>{status?.message ?? t("微信连接未启动。")}</span>
                <span title={accountLabel}>{t("账号")} <code>{accountLabel}</code></span>
                <span>{t("已处理消息")} <b>{status?.processedMessages ?? 0}</b></span>
                <span title={status?.lastPeerId || t("暂无")}>{t("最近联系人")} <code>{status?.lastPeerId || t("暂无")}</code></span>
              </span>
            )}
            icon={<CxStatusIcon tone={statusTone}><MessageCircle /></CxStatusIcon>}
            title={t("个人微信连接")}
          >
            <span className="weixin-autosave">{autoSave}</span>
            {!loggedIn ? (
              <button className="cx-primary" onClick={onQrLogin} type="button">
                <ScanLine />
                {t("扫码登录")}
              </button>
            ) : running || stopping ? (
              <button className="cx-secondary" disabled={stopping} onClick={onStop} type="button">
                <PowerOff />
                {stopping ? t("正在停止") : t("停止")}
              </button>
            ) : (
              <button className="cx-primary" onClick={onStart} type="button">
                <Play />
                {t("启动")}
              </button>
            )}
            {loggedIn ? (
              <Menu>
                <MenuItem onClick={onQrLogin}>{t("重新登录")}</MenuItem>
              </Menu>
            ) : null}
          </CxRow>
        </div>
      </CxCard>

      {qr?.qrContent ? (
        <CxSection title={qr.qrStatus === "scaned" ? t("已扫码，请在手机上确认") : t("微信扫码登录")}>
          <CxCard className="weixin-qr-card">
            {qr.qrSvg ? (
              <div className="weixin-qr-image" dangerouslySetInnerHTML={{ __html: qr.qrSvg }} />
            ) : null}
            <div className="weixin-qr-side">
              <p>{t("在手机微信中打开登录链接，或复制到可生成二维码的设备完成确认。")}</p>
              <div className="weixin-qr-content">{qr.qrContent}</div>
              <div className="weixin-qr-actions">
                <button className="cx-primary" onClick={() => onOpenQr(qr.qrContent)} type="button">
                  <ExternalLink />
                  {t("打开登录链接")}
                </button>
                <button className="cx-secondary" onClick={() => onCopyQr(qr.qrContent)} type="button">
                  <Copy />
                  {t("复制链接")}
                </button>
              </div>
            </div>
          </CxCard>
        </CxSection>
      ) : null}

      {/* 设置只留每天会碰的两节：对话（谁能用、在哪个目录、用哪个模型）和权限；改动即自动保存。
          扫码后自动填好的凭据、偶尔才用的项目归属与重置会话、连接参数都收进「高级」。 */}
      <CxSection id="weixin-section-session" title={t("对话")}>
        <CxCard className="weixin-settings-card">
          <SettingRow description={t("每个微信联系人会映射到独立的 Codex 会话。")} title={t("允许的微信用户 ID")}>
            <Input
              aria-label={t("允许的微信用户 ID")}
              onChange={(event) => onFormChange({ ...form, weixinConnectAllowFrom: event.target.value })}
              placeholder="user@im.wechat"
              value={form.weixinConnectAllowFrom}
            />
          </SettingRow>
          <SettingRow description={t("新会话在这个目录里工作；也可以从已有会话带入它的目录。")} title={t("工作目录")}>
            <div className="weixin-path-row">
              <SearchablePathPicker
                onChange={(value) => {
                  setSelectedSessionId("");
                  onFormChange({ ...form, weixinConnectWorkDir: value });
                }}
                options={workDirOptions}
                placeholder={t("搜索或输入工作目录")}
                value={form.weixinConnectWorkDir}
              />
              <button aria-label={t("选择工作目录")} className="cx-icon-button" onClick={onChooseWorkDir} title={t("选择工作目录")} type="button">
                <FolderOpen />
              </button>
            </div>
          </SettingRow>
          <SettingRow title={t("从已有会话带入")}>
            <SearchableSessionPicker
              onSelect={(session) => {
                setSelectedSessionId(session?.id ?? "");
                if (session?.cwd) onFormChange({ ...form, weixinConnectWorkDir: session.cwd });
              }}
              selectedId={selectedSessionId}
              sessions={sessions}
            />
          </SettingRow>
          {/* 模型和推理强度并成一行：两个下拉并排，推理强度跟着模型走。 */}
          <SettingRow
            description={catalogError
              || (modelUnavailable ? t("当前模型已不在可用模型目录中，请重新选择。") : undefined)
              || (reasoningUnavailable ? t("当前模型不支持已选推理强度，请重新选择或继承。") : undefined)
              || (catalog && !models.length ? t("当前没有可用模型，请检查供应商配置。") : undefined)}
            title={t("模型")}
          >
            <div className="weixin-path-row weixin-model-row">
              <div aria-invalid={modelUnavailable || Boolean(catalogError)} className="weixin-select">
                <AppSelect
                  ariaLabel={t("模型")}
                  onChange={(next) => onFormChange({ ...form, weixinConnectModel: next })}
                  options={[
                    { value: "", label: inheritedModel ? tf("继承 Codex 默认模型（{0}）", [groupDisplayName(gatewayPayload, inheritedModel) ?? catalog?.modelMetadata?.[inheritedModel]?.displayName ?? inheritedModel]) : t("继承 Codex 默认模型") },
                    ...(form.weixinConnectModel && !models.includes(form.weixinConnectModel) ? [{
                      value: form.weixinConnectModel,
                      label: `${groupDisplayName(gatewayPayload, form.weixinConnectModel) ?? form.weixinConnectModel} (${modelUnavailable ? t("不可用") : t("当前值")})`,
                    }] : []),
                    ...models.map((model) => ({ value: model, label: groupDisplayName(gatewayPayload, model) ?? (catalog?.modelMetadata?.[model]?.displayName || model) })),
                  ]}
                  value={form.weixinConnectModel}
                />
              </div>
              <div aria-invalid={reasoningUnavailable} className="weixin-select weixin-effort-select">
                <AppSelect
                  ariaLabel={t("推理强度")}
                  onChange={(next) => onFormChange({ ...form, weixinConnectReasoningEffort: next })}
                  options={[
                    { value: "", label: t("继承推理强度") },
                    ...(form.weixinConnectReasoningEffort && !reasoningEfforts.includes(form.weixinConnectReasoningEffort) ? [{
                      value: form.weixinConnectReasoningEffort,
                      label: `${form.weixinConnectReasoningEffort} (${reasoningUnavailable ? t("不可用") : t("当前值")})`,
                    }] : []),
                    ...reasoningEfforts.map((effort) => ({ value: effort, label: effort })),
                  ]}
                  value={form.weixinConnectReasoningEffort}
                />
              </div>
              <button aria-label={t("刷新模型目录")} className="cx-icon-button" disabled={catalogLoading} onClick={() => setCatalogRevision((value) => value + 1)} title={t("刷新模型目录")} type="button">
                <RefreshCw className={catalogLoading ? "animate-spin" : ""} />
              </button>
            </div>
          </SettingRow>
        </CxCard>
      </CxSection>

      {/* 加加：没自己选过模式的联系人默认交给加加（远程控制没开时自动退回直连）；推送默认关。 */}
      <CxSection
        aside={remoteControlRunning === null ? null : (
          <CxBadge tone={remoteControlRunning ? "ok" : "neutral"}>{remoteControlRunning ? t("远程控制已开") : t("远程控制未开")}</CxBadge>
        )}
        id="weixin-section-jiajia"
        title={t("加加")}
      >
        <CxCard className="weixin-settings-card">
          <SettingRow
            description={remoteControlRunning === false
              ? t("加加需要先打开远程控制；没开时消息先按直连处理")
              : t("没单独选过的联系人用这个模式；在微信里发 /加加 或 /直连 可单独切换")}
            title={t("微信默认模式")}
          >
            <AppSelect
              ariaLabel={t("微信默认模式")}
              onChange={(next) => onFormChange({ ...form, weixinConnectDefaultMode: next })}
              options={[
                { value: "jiajia", label: t("加加") },
                { value: "direct", label: t("直连") },
              ]}
              value={form.weixinConnectDefaultMode}
            />
          </SettingRow>
          <SettingRow description={t("没在对话时，加加的审批卡片和提醒也发到微信；回复数字即可审批")} title={t("推送到微信")}>
            <Switch
              label={t("推送到微信")}
              on={form.weixinConnectPushToWeixin}
              onToggle={() => onFormChange({ ...form, weixinConnectPushToWeixin: !form.weixinConnectPushToWeixin })}
            />
          </SettingRow>
        </CxCard>
      </CxSection>

      <CxSection id="weixin-section-account" title={t("权限")}>
        <CxCard className="weixin-settings-card">
          <SettingRow description={t("Codex 在工作目录里能做什么")} title={t("沙箱权限")}>
            <AppSelect
              ariaLabel={t("沙箱权限")}
              onChange={(next) => onFormChange({ ...form, weixinConnectSandbox: next })}
              options={[
                { value: "read-only", label: t("只读") },
                { value: "workspace-write", label: t("允许修改工作目录") },
                { value: "danger-full-access", label: t("完全访问") },
              ]}
              value={form.weixinConnectSandbox}
            />
          </SettingRow>
          <SettingRow
            description={outboundBlocked ? t("先填允许的微信用户 ID（不能是 *）并选好工作目录，才能打开") : t("允许向指定微信用户回传工作区文件")}
            title={t("工作区附件回传")}
          >
            <Switch
              label={t("允许向指定微信用户回传工作区文件")}
              on={form.weixinConnectAllowOutboundAttachments}
              disabled={outboundBlocked}
              onToggle={() => onFormChange({ ...form, weixinConnectAllowOutboundAttachments: !form.weixinConnectAllowOutboundAttachments })}
            />
          </SettingRow>
        </CxCard>
      </CxSection>

      <details className="weixin-advanced" id="weixin-section-cli">
        <summary>
          <span>{t("高级")}</span>
          <small>{t("新会话项目")} · {t("重置联系人会话")} · {t("登录凭据")} · {t("连接参数")}</small>
        </summary>
        <CxCard className="weixin-settings-card">
          <SettingRow
            description={projectsError || (projectUnavailable ? t("当前项目已不在项目列表中，请重新选择。") : t("新会话归到 Codex 里的哪个项目"))}
            title={t("新会话项目")}
          >
            <div aria-invalid={projectUnavailable || Boolean(projectsError)} className="weixin-path-row">
              <AppSelect
                ariaLabel={t("新会话项目")}
                onChange={(id) => {
                  const selected = projectOptions.find((project) => project.id === id);
                  onFormChange({
                    ...form,
                    weixinConnectProjectId: id,
                    weixinConnectWorkDir: selected?.roots?.[0]?.path || form.weixinConnectWorkDir,
                  });
                }}
                options={[
                  { value: "", label: t("不归属项目") },
                  ...(form.weixinConnectProjectId && !projectOptions.some((project) => project.id === form.weixinConnectProjectId) ? [{
                    value: form.weixinConnectProjectId,
                    label: `${form.weixinConnectProjectId} (${projectUnavailable ? t("不可用") : t("当前值")})`,
                  }] : []),
                  ...projectOptions.map((project) => ({ value: project.id, label: project.name || project.id })),
                ]}
                value={form.weixinConnectProjectId}
              />
              <button aria-label={t("刷新项目")} className="cx-icon-button" disabled={projectsLoading} onClick={() => setProjectsRevision((value) => value + 1)} title={t("刷新项目")} type="button">
                <RefreshCw className={projectsLoading ? "animate-spin" : ""} />
              </button>
            </div>
          </SettingRow>
          <SettingRow description={resetMessage || t("重置该联系人会话，下一条消息生效并保留原历史")} title={t("重置联系人会话")}>
            <div className="weixin-path-row">
              <Input aria-label={t("需要重置的联系人 ID")} placeholder={status?.lastPeerId || t("联系人 ID")} value={resetPeerId} onChange={(event) => setResetPeerId(event.target.value)} />
              <button aria-label={t("重置该联系人会话")} className="cx-secondary" disabled={resetPending || !resetPeerId.trim()} onClick={() => void resetSession()} title={t("重置该联系人会话，下一条消息生效并保留原历史")} type="button">
                <RefreshCw className={resetPending ? "animate-spin" : ""} />
                {t("重置")}
              </button>
            </div>
          </SettingRow>
          <SettingRow description={t("扫码后自动保存，也可粘贴已有 Bearer token")} title={t("登录凭据")}>
            <Input
              aria-label={t("登录凭据")}
              autoComplete="off"
              onChange={(event) => onFormChange({ ...form, weixinConnectToken: event.target.value })}
              placeholder={t("扫码后自动保存，也可粘贴已有 Bearer token")}
              type="password"
              value={form.weixinConnectToken}
            />
          </SettingRow>
          <SettingRow title={t("iLink API 地址")}>
            <Input
              aria-label={t("iLink API 地址")}
              onChange={(event) => onFormChange({ ...form, weixinConnectBaseUrl: event.target.value })}
              value={form.weixinConnectBaseUrl}
            />
          </SettingRow>
          <SettingRow description={t("扫码后自动填写")} title={t("账号标识")}>
            <Input
              aria-label={t("账号标识")}
              onChange={(event) => onFormChange({ ...form, weixinConnectAccountId: event.target.value })}
              placeholder={t("扫码后自动填写")}
              value={form.weixinConnectAccountId}
            />
          </SettingRow>
          <SettingRow description={t("仅在网关要求时填写")} title={t("SKRouteTag")}>
            <Input
              aria-label={t("SKRouteTag")}
              onChange={(event) => onFormChange({ ...form, weixinConnectRouteTag: event.target.value })}
              placeholder={t("仅在网关要求时填写")}
              value={form.weixinConnectRouteTag}
            />
          </SettingRow>
          <SettingRow description={t("留空时从 PATH 查找 codex")} title={t("Codex CLI 路径")}>
            <div className="weixin-path-row weixin-cli-path-row">
              <Input
                aria-label={t("Codex CLI 路径")}
                onChange={(event) => onFormChange({ ...form, weixinConnectCodexPath: event.target.value })}
                placeholder={t("留空时从 PATH 查找 codex")}
                value={form.weixinConnectCodexPath}
              />
              <button aria-label={t("选择 Codex CLI")} className="cx-icon-button" onClick={onChooseCodexPath} title={t("选择 Codex CLI")} type="button">
                <FolderOpen />
              </button>
              <Menu>
                <MenuItem onClick={onUseDesktopCodexCli}>{t("使用桌面版内置 CLI")}</MenuItem>
              </Menu>
            </div>
          </SettingRow>
        </CxCard>
      </details>
    </div>
  );
}

/// 置顶赞助位：只读广告源的 `top_ad/topAds`（`top_ad` 可以是单个对象或数组），过期的不显示。
function featuredAds(ads: AdsResult | null): AdItem[] {
  const sourceAds = ads?.topAds?.length ? ads.topAds : ads?.topAd ? [ads.topAd] : [];
  return sourceAds.filter((ad) => !isExpiredAd(ad));
}

function OverviewScreen({
  overview,
  activeTool,
  toolEntries,
  form,
  onNavigate,
  onOpenList,
  actions,
}: {
  overview: OverviewResult | null;
  activeTool: ToolId;
  toolEntries: ToolEntry[];
  form: BackendSettings;
  onNavigate: (route: Route) => void;
  /** 打开会话页 / 请求日志并选中这个 Agent。 */
  onOpenList: (kind: "sessions" | "requestLog", agent: DashboardAgent) => void;
  actions: Actions;
}) {
  const gatewayPayload = useGatewayPayload(10_000);
  const health = healthItems(overview);
  const tool = toolEntries.find((entry) => entry.id === activeTool);
  const normalized = normalizeSettings(form);
  const activeProfile = normalized.relayProfiles.find((profile) => profile.id === normalized.activeRelayId);
  // 最近请求只在 Codex 概览里用，数据由仪表板读请求日志时一并交过来；失败时静默显示空状态，不打断概览。
  const [recent, setRecent] = useState<RecentRequestSummary | null>(null);
  const summarizeCodexRequests = (result: DashboardRequestLogs | null) => {
    // 网关记录里也有 Claude Code / Grok 的请求，Codex 概览只统计 Codex 自己的。
    const entries = result?.status === "ok" ? ((result.entries ?? []) as RequestLogEntry[]) : [];
    setRecent(summarizeRecentRequests(entries.filter((entry) => requestLogAgent(entry) === "codex"), Date.now()));
  };
  // 「刷新状态」连仪表板的数字、最近会话和请求一起重读。
  const [dashboardRefresh, setDashboardRefresh] = useState(0);
  const switchFromOverview = (profileId: string) => {
    if (!profileId || profileId === normalized.activeRelayId) return;
    void actions.switchRelayProfile(syncLegacyRelayFields({ ...normalized, activeRelayId: profileId }), normalized.activeRelayId);
  };
  const gatewayManaged = activeProfile?.id === CODEX_GATEWAY_RELAY_ID;
  // Codex 的状态栏：经网关 / 直连哪家供应商，操作是网关管理或切换供应商。
  const codexStatus: DashboardStatus = gatewayManaged
    ? { tone: "gateway", text: codexGatewayModelCount(activeProfile) ? tf("经 Codex++ 网关 · {0} 个模型", [codexGatewayModelCount(activeProfile)]) : t("经 Codex++ 网关") }
    : activeProfile
      ? { tone: "relay", text: tf("直连 · {0}", [[activeProfile.name || t("未命名供应商"), relayProtocolLabel(activeProfile.protocol), activeProfile.model].filter(Boolean).join(" · ")]) }
      : { tone: "missing", text: t("还没有配置供应商。") };
  const codexActions = gatewayManaged ? (
    <>
      <button className="cx-secondary" onClick={() => onNavigate("gateway")} type="button">{t("管理接入")}</button>
    </>
  ) : activeProfile ? (
    <>
      <AppSelect
        ariaLabel={t("切换供应商")}
        className="overview-current-select agent-dash-select"
        disabled={actions.relaySwitching || !normalized.relayProfilesEnabled || normalized.relayProfiles.length < 2}
        onChange={(next) => switchFromOverview(next)}
        options={normalized.relayProfiles.map((profile) => ({ value: profile.id, label: profile.name || t("未命名供应商") }))}
        size="sm"
        title={!normalized.relayProfilesEnabled ? t("供应商配置切换已关闭，在供应商配置页打开后才能切换") : t("切换供应商")}
        value={normalized.activeRelayId}
      />
      <button className="cx-secondary" onClick={() => onNavigate("relay")} type="button">{t("管理供应商")}</button>
    </>
  ) : (
    <button className="cx-primary" onClick={() => onNavigate("relay")} type="button"><Plus aria-hidden="true" />{t("添加供应商")}</button>
  );
  const codexStats: DashboardStat[] = [
    { label: t("24 小时请求"), value: recent ? recent.total : "—" },
    { label: t("成功率"), value: recent?.successRate == null ? "—" : `${Math.round(recent.successRate * 100)}%`, tone: recent?.failed ? "warn" : "ok" },
    { label: t("平均耗时"), value: recent?.avgDurationMs == null ? "—" : `${(recent.avgDurationMs / 1000).toFixed(1)}s` },
    { label: t("常用模型"), value: groupDisplayName(gatewayPayload, recent?.topModel ?? "") ?? recent?.topModel ?? "—", mono: false },
  ];
  const { CxBadge, CxCard, CxRow, CxSection, CxStatusIcon } = AgentCx;
  // 健康检查：一张卡一行一项。有问题的项排前面、带橙色 / 红色标签；正常的收成一行「其余 N 项正常」，点开再看。
  const healthRows = [
    {
      title: t("Codex 版本"),
      status: overview?.codex_version ? "ok" : "not_checked",
      ok: Boolean(overview?.codex_version),
      detail: overview?.codex_version ?? t("未检测到 Codex 应用版本。"),
      icon: Tag,
    },
    ...health,
  ];
  const healthProblems = healthRows.filter((item) => !item.ok);
  const healthFine = healthRows.filter((item) => item.ok);
  const [healthExpanded, setHealthExpanded] = useState(false);
  const healthRow = (item: (typeof healthRows)[number]) => (
    <CxRow
      detail={<span title={item.detail}>{item.detail}</span>}
      icon={<CxStatusIcon tone={item.ok ? (statusClass(item.status) === "idle" ? "off" : "ok") : statusClass(item.status) === "bad" ? "danger" : "warn"}><item.icon /></CxStatusIcon>}
      key={item.title}
      muted={item.ok && healthProblems.length > 0}
      title={item.title}
    >
      <CxBadge tone={item.ok ? (statusClass(item.status) === "good" ? "ok" : "neutral") : statusClass(item.status) === "bad" ? "danger" : "warn"}>{statusLabel(item.status)}</CxBadge>
      {"action" in item && item.action === "restart" ? (
        <button className="cx-secondary" onClick={() => void actions.restart()} type="button">
          <RotateCcw aria-hidden="true" />{t("重启并挂载")}
        </button>
      ) : null}
    </CxRow>
  );
  return (
    <>
      {activeTool === "codex" ? (
        // 三个 Agent 的概览同一套仪表板；Codex 另在下面放健康检查。
        <AgentDashboard
          actions={codexActions}
          agent="codex"
          name="Codex"
          notify={actions.showMessage}
          onNavigate={onNavigate}
          onOpenList={onOpenList}
          onRequestLogs={summarizeCodexRequests}
          refreshKey={dashboardRefresh}
          stats={codexStats}
          status={codexStatus}
          version={overview?.codex_version}
        >
          <CxSection
            aside={
              <span className="overview-health-header-actions">
                {healthProblems.length ? <span title={t("概览只展示关键问题，具体配置在对应页面处理")}>{tf("{0} 项需处理", [healthProblems.length])}</span> : null}
                <button aria-label={t("刷新状态")} className="cx-icon-button" onClick={() => { setDashboardRefresh((value) => value + 1); void actions.refreshCurrent(); }} title={t("刷新状态")} type="button">
                  <RefreshCw aria-hidden="true" />
                </button>
              </span>
            }
            className="overview-health"
            title={t("健康检查")}
          >
            <CxCard className="overview-health-list">
              {healthProblems.map(healthRow)}
              {healthExpanded ? healthFine.map(healthRow) : (
                <CxRow
                  className="overview-health-more"
                  detail={healthFine.map((item) => item.title).join(" · ")}
                  icon={<CxStatusIcon tone="ok"><CheckCircle2 /></CxStatusIcon>}
                  onClick={() => setHealthExpanded(true)}
                  title={healthProblems.length ? tf("其余 {0} 项正常", [healthFine.length]) : tf("{0} 项检查全部正常", [healthFine.length])}
                >
                  <ChevronDown aria-hidden="true" className="cx-row-chevron" />
                </CxRow>
              )}
            </CxCard>
          </CxSection>
        </AgentDashboard>
      ) : activeTool === "claude" || activeTool === "grok" ? (
        <AgentDashboard agent={activeTool} name={tool?.name ?? activeTool} notify={actions.showMessage} onNavigate={onNavigate} onOpenList={onOpenList} />
      ) : null}
    </>
  );
}

/**
 * 新旧两套共用的本地服务设置：服务状态、请求体上限、请求避震。放在「网关 → 网关设置」，
 * 按网关设置的行样式排版；改的是 settings.json（随主窗口的设置一起自动保存）。
 */
function SharedServiceSettings({
  overview,
  form,
  onFormChange,
  autoSave,
}: {
  overview: OverviewResult | null;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  /** 自动保存状态，放在第一个分组标题的右侧。 */
  autoSave?: ReactNode;
}) {
  const [retryStatusInput, setRetryStatusInput] = useState("");
  const setRetryStatusCodes = (codes: number[]) =>
    onFormChange({ ...form, protocolProxyAutoRetryStatusCodes: normalizeRetryStatusCodes(codes) });
  const addRetryStatusCodes = () => {
    const next = retryStatusInput.split(/[,，\s]+/).map(Number);
    if (!normalizeRetryStatusCodes(next).length) return;
    setRetryStatusCodes([...form.protocolProxyAutoRetryStatusCodes, ...next]);
    setRetryStatusInput("");
  };
  // 本地服务的地址与运行状态在上面「监听地址」已经有了，这里只列协议转换与调试连接。
  const health = healthItems(overview).filter((item) => item.title === t("协议转换") || item.title === t("调试连接状态"));
  const tone = (item: (typeof health)[number]) => (["running", "connected", "ok"].includes(item.status) ? "ok" : item.ok ? undefined : "warning");
  const number = (value: number, min: number, max: number, unit: string, apply: (next: number) => void, step = 1) => (
    <label className="gw-unit-input">
      <input
        max={max}
        min={min}
        onChange={(event) => apply(clampNumber(Number(event.currentTarget.value), min, max))}
        step={step}
        type="number"
        value={value}
      />
      <span>{unit}</span>
    </label>
  );
  const retry = form.protocolProxyAutoRetry;
  const retryModeLabel = form.protocolProxyAutoRetryMode === "buffered" ? t("整段缓冲") : form.protocolProxyAutoRetryMode === "streaming-legacy" ? t("流式（旧行为）") : t("流式（推荐）");
  const sharedHint = t("网关和旧版本地协议代理共用这些设置");
  // 本地服务一节、请求避震一节。避震的五个参数默认值够用，收进折叠区，摘要里显示当前值。
  return (
    <div className="gw-shared-settings">
      {autoSave ? <TopbarActions><span className="cx-set-autosave">{autoSave}</span></TopbarActions> : null}
      <CxSection title={<span title={sharedHint}>{t("本地服务")}</span>}>
        <CxCard>
          {health.map((item) => (
            <CxRow detail={item.detail} key={item.title} title={item.title} tooltip={item.detail}>
              <CxBadge tone={tone(item) === "ok" ? "ok" : tone(item) === "warning" ? "warn" : "neutral"}>{statusLabel(item.status)}</CxBadge>
            </CxRow>
          ))}
          <CxRow
            detail={t("长上下文会话超过上限会报 413；调大会增加本地服务的内存占用。")}
            title={t("请求体上限")}
            tooltip={t("默认 128 MiB，可填 32~512。长上下文会话（1M 窗口 + 截图历史）的请求体会超过旧的 32 MiB 硬上限并触发 413；调大后本地代理处理单次请求的内存占用也会增加。保存后立即对本地代理生效；环境变量 CODEX_PLUS_MAX_HTTP_BODY_BYTES（字节）优先于本设置。")}
          >
            {number(form.helperMaxRequestBodyMib, 32, 512, "MiB", (next) => onFormChange({ ...form, helperMaxRequestBodyMib: next }), 16)}
          </CxRow>
        </CxCard>
      </CxSection>

      <CxSection title={<span title={sharedHint}>{t("请求避震")}</span>}>
        <CxCard>
          <CxRow
            detail={retry
              ? t("在本地服务内消化上游过载与限流，尽量不把错误交给 Agent。作用于经过网关和旧版本地协议代理的请求。")
              : t("已关闭：上游过载或限流时，错误直接交给 Agent。")}
            title={t("启用请求避震")}
          >
            <Switch label={t("启用请求避震")} on={retry} onToggle={() => onFormChange({ ...form, protocolProxyAutoRetry: !retry })} />
          </CxRow>
          {retry ? (
            <details className="cx-set-disclosure">
              <summary>
                {t("重试参数")}
                <span className="cx-set-summary-hint">
                  {[tf("最多 {0} 次", [form.protocolProxyAutoRetryMaxAttempts]), tf("间隔 {0} 秒", [form.protocolProxyAutoRetryIntervalSeconds]), retryModeLabel].join(" · ")}
                </span>
              </summary>
              <CxRow detail={t("0–20 次；次数用尽后把错误交给 Agent。")} title={t("最多自动重试次数")}>
                {number(form.protocolProxyAutoRetryMaxAttempts, 0, 20, t("次"), (next) => onFormChange({ ...form, protocolProxyAutoRetryMaxAttempts: next }))}
              </CxRow>
              <CxRow detail={t("之后按 2 倍退避，单次最长 30 秒；有 Retry-After 时优先采用。")} title={t("首次重试间隔")}>
                {number(form.protocolProxyAutoRetryIntervalSeconds, 0, 30, t("秒"), (next) => onFormChange({ ...form, protocolProxyAutoRetryIntervalSeconds: next }))}
              </CxRow>
              <CxRow
                detail={form.protocolProxyAutoRetryMode === "buffered"
                  ? t("始终由代理重试，但要等整段生成完才显示。")
                  : form.protocolProxyAutoRetryMode === "streaming-legacy"
                    ? t("旧版流式行为，仅在新版有问题时使用。")
                    : t("保留实时输出；正文出现前遇到过载由代理重试。")}
                title={t("避震模式")}
                tooltip={t("流式：保留实时输出，正文提交前遇到过载由代理重试，之后交给 Codex；「首字提交延迟」为出现正文后再缓冲的时长，0 表示立即提交。整段缓冲：始终由代理重试，但要等整段生成完才显示。")}
              >
                <AppSelect
                  className="cx-set-select"
                  label={t("避震模式")}
                  onChange={(next) => onFormChange({ ...form, protocolProxyAutoRetryMode: next })}
                  options={[
                    { value: "streaming", label: t("流式（推荐）"), hint: t("保留实时输出；正文出现前遇到过载由代理重试。") },
                    { value: "streaming-legacy", label: t("流式（旧行为）"), hint: t("旧版流式行为，仅在新版有问题时使用。") },
                    { value: "buffered", label: t("整段缓冲"), hint: t("始终由代理重试，但要等整段生成完才显示。") },
                  ]}
                  size="sm"
                  value={form.protocolProxyAutoRetryMode}
                  menuWidth={300}
                />
              </CxRow>
              {form.protocolProxyAutoRetryMode === "streaming" ? (
                <CxRow detail={t("出现正文后再缓冲的时长，0 表示立即提交。")} title={t("首字提交延迟")}>
                  {number(form.protocolProxyAutoRetryCommitDelayMs, 0, 10000, "ms", (next) => onFormChange({ ...form, protocolProxyAutoRetryCommitDelayMs: next }), 100)}
                </CxRow>
              ) : null}
              <CxRow detail={t("命中时退避重发。默认不选；常用 429（限流）和 5xx（服务端故障）。")} title={t("重试的 HTTP 状态码")}>
                <button
                  className="cx-secondary"
                  onClick={() => setRetryStatusCodes([...form.protocolProxyAutoRetryStatusCodes, ...COMMON_RETRY_STATUS_CODES])}
                  type="button"
                >
                  {t("填入常用")}
                </button>
              </CxRow>
              {/* 状态码编辑不能放进 <label>：WebKit 会把点击转发给第一个按钮，一次删掉两个状态码。 */}
              <div className="cx-set-block">
                <div className="gw-retry-codes" role="group" aria-label={t("重试的 HTTP 状态码")}>
                  {form.protocolProxyAutoRetryStatusCodes.map((status) => (
                    <button
                      className="gw-chip gw-retry-code"
                      key={status}
                      onClick={() => setRetryStatusCodes(form.protocolProxyAutoRetryStatusCodes.filter((item) => item !== status))}
                      title={t("点击移除")}
                      type="button"
                    >
                      {status} ×
                    </button>
                  ))}
                  <input
                    aria-label={t("输入状态码后回车")}
                    className="gw-retry-input"
                    inputMode="numeric"
                    onChange={(event) => setRetryStatusInput(event.currentTarget.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        addRetryStatusCodes();
                      }
                    }}
                    placeholder={t("加状态码，回车")}
                    value={retryStatusInput}
                  />
                </div>
              </div>
            </details>
          ) : null}
        </CxCard>
      </CxSection>
    </div>
  );
}

/** 「Codex 连接」页把「打开某个供应商的编辑页并跳到某项设置」交给嵌在页底「高级」里的列表用。 */
const RelayProfileLinkContext = createContext<((profileId: string, section: "proxy") => void) | null>(null);

function ProxySettingsScreen({
  dirty,
  form,
  relayEnvironment,
  onFormChange,
  actions,
}: {
  dirty: boolean;
  form: BackendSettings;
  relayEnvironment: RelayEnvironmentResult | null;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  // Codex 被新版网关接管时，旧版专用的设置不起作用，隐藏。
  const takenOverByGateway = form.activeRelayId === CODEX_GATEWAY_RELAY_ID;
  const proxyProfiles = useMemo(
    () => (form.relayProfiles ?? []).filter((profile) => profile.relayMode !== "aggregate"),
    [form.relayProfiles],
  );
  const proxyProfileKey = proxyProfiles.map((profile) => profile.id).join("|");
  // 每个供应商的独立代理只在这里列出来，一次读完；改代理去供应商编辑页（随「保存供应商」一起写），这里不再单独编辑。
  const [proxies, setProxies] = useState<Record<string, { proxyUrl?: string; configured?: boolean }>>({});
  const openRelayProfile = useContext(RelayProfileLinkContext);
  useEffect(() => {
    let cancelled = false;
    const ids = proxyProfileKey ? proxyProfileKey.split("|") : [];
    if (!ids.length) {
      setProxies({});
      return;
    }
    void invoke<{ status: string; proxies?: Record<string, { proxyUrl?: string; configured?: boolean }> }>("load_relay_proxies", { profileIds: ids })
      .then((result) => {
        if (!cancelled && result.status === "ok") setProxies(result.proxies ?? {});
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [proxyProfileKey]);

  // 嵌在「Codex 连接」页底部的「高级」折叠区里：每项一行，说明一行，长说明放悬停。
  return (
    <div className="settings-page proxy-settings-page cx-codex-advanced-body">
      <SettingsAutoSave dirty={dirty} form={form} actions={actions} />
      {takenOverByGateway ? (
        <CxRow
          detail={t("Codex 正由 Codex++ 网关接管，旧版代理设置不起作用，已隐藏。断开并还原后恢复。")}
          muted
          title={t("目录外模型映射（旧版）")}
        />
      ) : (
      <>
      <CxRow
        detail={form.protocolProxyMapUnlistedModels
          ? t("注意：手填但没写进模型列表的模型也会被改写，请先把要用的模型都加进列表。")
          : t("后台任务的固定模型名改写为当前供应商的默认模型；接入网关的 Agent 不用这一项。")}
        title={t("目录外模型映射（旧版）")}
        tooltip={t("Codex 的自动审查（codex-auto-review）与标题/摘要等后台任务固定使用 gpt-* 等内部模型名，不经选模；第三方上游没有这些名字，原样转发只会报「模型不存在」并可能反复消耗额度。开启后，不在当前供应商模型列表里的请求模型会改写为该供应商的默认模型再转发（#1183 / #1446 / #1457）；官方登录与聚合供应商不受影响，已配置「自动审批模型」时 codex-auto-review 仍按该配置改写。默认关闭：开启后手填但未写进模型列表的模型也会被改写，请先把要用的模型都加进列表。")}
      >
        <Switch
          label={t("改写目录外模型名")}
          on={form.protocolProxyMapUnlistedModels}
          onToggle={() => onFormChange({ ...form, protocolProxyMapUnlistedModels: !form.protocolProxyMapUnlistedModels })}
        />
      </CxRow>

      <div className="cx-codex-proxy-head" title={t("为旧版的单个供应商单独指定 HTTP 代理；留空表示沿用系统代理。网关供应商的代理在供应商池里编辑供应商时设置。")}>
        <strong>{t("供应商独立代理（旧版供应商）")}</strong>
        <small>{t("点「编辑」到供应商编辑页里改，随「保存供应商」一起保存；当前供应商会重新应用，重启 Codex 后作用于会话请求。")}</small>
      </div>
      {proxyProfiles.length ? proxyProfiles.map((profile) => {
        const proxy = proxies[profile.id];
        const url = (proxy?.proxyUrl ?? "").trim();
        return (
          <CxRow
            detail={url || (proxy?.configured ? t("已配置独立代理（来自环境变量）") : t("未配置独立代理，沿用系统代理"))}
            key={profile.id}
            title={profile.name || t("未命名供应商")}
          >
            {openRelayProfile ? (
              <button
                aria-label={tf("编辑 {0} 的独立代理", [profile.name || t("未命名供应商")])}
                className="cx-secondary"
                onClick={() => openRelayProfile(profile.id, "proxy")}
                type="button"
              >
                {t("编辑")}
              </button>
            ) : null}
          </CxRow>
        );
            }) : (
        <CxRow detail={t("还没有可配置独立代理的供应商。")} muted title={t("供应商独立代理（旧版供应商）")} />
      )}
      </>
      )}

      <RelayEnvironmentScreen result={relayEnvironment} actions={actions} />
    </div>
  );
}

function formatRequestLogTime(timestampMs: number) {
  if (!Number.isFinite(timestampMs) || timestampMs <= 0) return "-";
  const date = new Date(timestampMs);
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear()
    && date.getMonth() === now.getMonth()
    && date.getDate() === now.getDate();
  const time = date.toLocaleTimeString(getLanguage() === "en" ? "en-US" : "zh-CN", { hour12: false });
  return sameDay ? time : `${date.getMonth() + 1}/${date.getDate()} ${time}`;
}

function formatRequestLogTokens(value?: number | null) {
  if (value == null) return "-";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

/** 缓存命中率展示：≥99% 保留一位小数，只有完全命中才显示 100%。 */
function formatRequestLogCacheHitRatio(
  inputTokens?: number | null,
  cachedTokens?: number | null,
  ratio?: number | null,
) {
  const input = inputTokens ?? 0;
  const cached = cachedTokens ?? 0;
  if (input <= 0 || cached <= 0) return "-";
  if (cached >= input) return "100%";
  const percent = (ratio ?? cached / input) * 100;
  if (percent >= 99) return `${percent.toFixed(1)}%`;
  return `${Math.round(percent)}%`;
}

/** 会话 id 展示：UUIDv7 前缀按时间递增、容易重名，这里取末 8 位区分。 */
function formatRequestLogSession(threadId?: string | null): string {
  const value = (threadId ?? "").trim();
  if (!value) return "-";
  return value.length > 12 ? `…${value.slice(-8)}` : value;
}

function formatRequestLogDuration(durationMs: number) {
  if (!Number.isFinite(durationMs) || durationMs < 0) return "-";
  if (durationMs < 1000) return `${durationMs}ms`;
  if (durationMs < 60_000) return `${(durationMs / 1000).toFixed(1)}s`;
  return `${Math.floor(durationMs / 60_000)}m${Math.round((durationMs % 60_000) / 1000)}s`;
}

function requestLogRetryLabel(entry: RequestLogEntry) {
  const base = entry.autoRetries > 0
    ? tf("重试 ×{0}", [String(entry.autoRetries)])
    : entry.attempts > 1
      ? tf("切换 ×{0}", [String(entry.attempts)])
      : "";
  // 网关因 Codex++ 本地限速排过队：标出排了多久。
  const queued = queueWaitMs(entry.retryReasons);
  const queue = queued === null ? "" : tf("限速排队 {0}", [formatQueueWait(queued)]);
  // 换上游后去掉了别家签发的压缩上下文：较早的对话内容上游已看不到，必须让用户看得见。
  const compaction = foreignCompactionDropped(entry.retryReasons) ? t("已去掉别家压缩上下文") : "";
  // 压缩请求改发到了压缩模型，或回退到了原模型（#3142）。
  const note = compactionNote(entry.retryReasons);
  const compactModel = note ? (note.kind === "model" ? t("压缩模型") : t("压缩回退原模型")) : "";
  return [base, queue, compaction, compactModel].filter(Boolean).join(" · ") || "-";
}

/** 压缩模型说明的展示文字（#3142）。 */
function compactionNoteText(note: CompactionNote): string {
  if (note.kind === "model") return tf("压缩改用 {0}", [note.model]);
  const reason = note.reason === "unavailable" ? t("压缩模型不可用")
    : note.reason === "incompatible" ? t("压缩形态与压缩模型的上游不兼容")
    : note.reason === "window" ? t("压缩模型的窗口装不下待压缩内容")
    : note.reason === "failed" ? t("压缩模型的上游拒绝或出错")
    : note.reason;
  return tf("压缩模型 {0} 未使用（{1}），已用原模型", [note.model, reason]);
}

/** 重试原因的展示：网关记的限速排队换成「限速排队 Xs」，去掉别家压缩上下文换成说明，其它原样。 */
function requestLogRetryReasons(entry: RequestLogEntry): string {
  return (entry.retryReasons ?? [])
    .map((reason) => {
      if (reason === FOREIGN_COMPACTION_DROPPED_REASON) {
        return t("已去掉别家签发的压缩上下文，较早的对话内容不可用");
      }
      const note = parseCompactionNote(reason);
      if (note) return compactionNoteText(note);
      if (!reason.startsWith(QUEUE_REASON_PREFIX)) return reason;
      const queued = queueWaitMs([reason]);
      return queued === null ? reason : tf("限速排队 {0}", [formatQueueWait(queued)]);
    })
    .join("、");
}

/** 生图请求（网关记成 imageGenerations / imageEdits）：列表里标「生图」，运行中显示「生成中」而不是「流式中」。 */
function isImageRequest(entry: { wireApi?: string }): boolean {
  return (entry.wireApi ?? "").startsWith("image");
}

const REQUEST_LOG_LIMIT = 300;
const REQUEST_LOG_POLL_MS = 2000;

/** 失败请求在列表里直接显示的错误摘要：上游返回 JSON 时取里面的 message，否则取第一行。 */
function requestLogErrorSummary(error: string): string {
  const text = error.trim();
  try {
    const value = JSON.parse(text) as { error?: { message?: unknown } | string; message?: unknown };
    const message = typeof value.error === "string" ? value.error : value.error?.message ?? value.message;
    if (typeof message === "string" && message.trim()) return message.trim();
  } catch {
    // 不是 JSON，按原文处理。
  }
  return text.split("\n")[0] ?? text;
}

type RequestLogRowProps = {
  entry: RequestLogEntry;
  /** 模型名、上游模型名的展示名（分组别名），父组件按 entries 缓存好传进来。 */
  model: string;
  upstream: string;
  expanded: boolean;
  /** 只有进行中的行用得到（算已用时）；其它行传 0，免得每秒跟着重渲染。 */
  nowMs: number;
  onToggle: (id: string) => void;
};

/** 请求日志的一行（含展开的明细）。按 props 记忆：自动刷新时只有变了的行（进行中、新来的）才重渲染。 */
const RequestLogRow = memo(function RequestLogRow({ entry, model, upstream, expanded, nowMs, onToggle }: RequestLogRowProps) {
  const running = entry.state === "running";
  const detailId = `request-log-detail-${entry.id}`;
  return (
    <>
      <div
        className={`request-log-row request-log-entry ${running ? "running" : entry.success ? "ok" : "bad"}`}
        data-expanded={expanded || undefined}
        onClick={() => onToggle(entry.id)}
        role="row"
      >
        <span role="cell">
          {/* 展开 / 收起靠第一格的按钮（能用键盘聚焦、带 aria-expanded）；点整行也一样。 */}
          <button
            aria-controls={expanded ? detailId : undefined}
            aria-expanded={expanded}
            className="request-log-toggle"
            onClick={(event) => {
              event.stopPropagation();
              onToggle(entry.id);
            }}
            type="button"
          >
            {formatRequestLogTime(entry.startedAtMs || entry.timestampMs)}
          </button>
        </span>
        {running ? (
          <span className="request-log-status running" role="cell">
            {t("运行中")}
          </span>
        ) : (
          <span
            className={`request-log-status ${entry.success ? "ok" : "bad"}`}
            role="cell"
            title={entry.error ?? ""}
          >
            {entry.statusCode || (entry.success ? "OK" : "ERR")}
            {!entry.success ? (
              <small className="request-log-stream-failed">
                {entry.stream ? t("流失败") : t("失败")}
              </small>
            ) : null}
          </span>
        )}
        <span className="request-log-ellipsis request-log-source" role="cell" title={`${agentName(requestLogAgent(entry))} · ${maskAccountName(entry.relayName)}`}>
          <AgentGlyph agent={requestLogAgent(entry)} />
          {maskAccountName(entry.relayName) || "-"}
        </span>
        <span className="request-log-ellipsis" role="cell" title={entry.upstreamModel ? `${model} → ${upstream}` : model}>
          {isImageRequest(entry) ? <em className="request-log-kind">{t("生图")}</em> : null}
          {model || "-"}
          {entry.upstreamModel && entry.upstreamModel !== entry.model ? (
            <small> → {upstream}</small>
          ) : null}
        </span>
        <span
          className={entry.autoRetries > 0 || queueWaitMs(entry.retryReasons) !== null || foreignCompactionDropped(entry.retryReasons) || compactionNote(entry.retryReasons)?.kind === "fallback" ? "request-log-retry" : "request-log-muted"}
          role="cell"
          title={requestLogRetryReasons(entry)}
        >
          {requestLogRetryLabel(entry)}
        </span>
        {!running && !entry.success && entry.error ? (
          // 失败的请求没有 token 和费用：这几列直接显示错误原因。
          <span className="request-log-error-cell" role="cell" title={entry.error}>{requestLogErrorSummary(entry.error)}</span>
        ) : (
          <>
            <span className="request-log-muted" role="cell" title={running ? t("token 用量由上游在请求完成时返回，流式过程中无法准确统计") : ""}>
              {running ? (isImageRequest(entry) ? t("生成中") : t("流式中")) : formatRequestLogTokens(entry.usage?.inputTokens)}
            </span>
            <span
              className="request-log-muted"
              role="cell"
              title={running
                ? t("按流式增量估算，完成后会更新为准确值")
                : ""}
            >
              {running
                ? entry.estimatedOutputTokens
                  ? `~${formatRequestLogTokens(entry.estimatedOutputTokens)}`
                  : (isImageRequest(entry) ? t("生成中") : t("流式中"))
                : formatRequestLogTokens(entry.usage?.outputTokens)}
            </span>
            <span role="cell">
              {running ? (
                <span className="request-log-muted">{(isImageRequest(entry) ? t("生成中") : t("流式中"))}</span>
              ) : entry.usage?.cachedTokens ? (
                <>
                  {formatRequestLogTokens(entry.usage.cachedTokens)}
                  <small>
                    {" · "}
                    {formatRequestLogCacheHitRatio(
                      entry.usage.inputTokens,
                      entry.usage.cachedTokens,
                      entry.cacheHitRatio,
                    )}
                  </small>
                </>
              ) : "-"}
            </span>
            <span className="request-log-muted" role="cell">{entry.cost ? formatRequestCost(entry.cost) : "-"}</span>
          </>
        )}
        <span className="request-log-duration" role="cell">
          {formatRequestLogDuration(
            running
              ? Math.max(0, nowMs - entry.startedAtMs)
              : entry.durationMs,
          )}
          {running ? (
            <small>{t("进行中")}</small>
          ) : entry.firstTokenMs != null ? (
            <small>{tf("首字 {0}", [formatRequestLogDuration(entry.firstTokenMs)])}</small>
          ) : null}
        </span>
      </div>
      {expanded ? (
        <div role="row">
          <div className="request-log-detail" id={detailId} role="cell">
            <dl>
              <div><dt>{t("请求 ID")}</dt><dd><code>{entry.id}</code></dd></div>
              <div>
                <dt>{t("会话 ID")}</dt>
                <dd>
                  {entry.threadTitle ? <strong className="request-log-session-title">{entry.threadTitle}</strong> : null}
                  <code>{entry.threadId || "-"}</code>
                </dd>
              </div>
              <div><dt>{t("路径")}</dt><dd><code>{entry.method} {entry.path}</code></dd></div>
              <div><dt>{t("上游协议")}</dt><dd>{entry.wireApi || "-"}</dd></div>
              <div><dt>{t("上游模型")}</dt><dd>{upstream || "-"}</dd></div>
              <div><dt>{t("首字耗时")}</dt><dd>{entry.firstTokenMs == null ? (running ? t("进行中") : "-") : formatRequestLogDuration(entry.firstTokenMs)}</dd></div>
              <div><dt>{t("尝试次数")}</dt><dd>{entry.attempts || "-"}</dd></div>
              <div><dt>{t("自动重试")}</dt><dd>{entry.autoRetries}</dd></div>
              <div><dt>{t("重试原因")}</dt><dd>{requestLogRetryReasons(entry) || "-"}</dd></div>
              {entry.attemptDetails?.length ? (
                <div className="request-log-attempts">
                  <dt>{t("尝试明细")}</dt>
                  <dd>
                    <ul>
                      {entry.attemptDetails.map((detail) => (
                        <li key={`${detail.attempt}-${detail.code ?? ""}`}>
                          <strong>{tf("第 {0} 次", [detail.attempt])}</strong>
                          <code>{detail.code || "-"}</code>
                          {detail.delayMs != null ? (
                            <small>{tf("退避 {0}", [formatRequestLogDuration(detail.delayMs)])}</small>
                          ) : null}
                          {detail.message ? <small title={detail.message}>{detail.message}</small> : null}
                        </li>
                      ))}
                    </ul>
                  </dd>
                </div>
              ) : null}
              <div><dt>{t("流式")}</dt><dd>{entry.stream ? "SSE" : "JSON"}</dd></div>
              <div><dt>{t("流结果")}</dt><dd>{entry.streamOutcome || "-"}</dd></div>
              <div><dt>{t("推理 tokens")}</dt><dd>{formatRequestLogTokens(entry.usage?.reasoningTokens)}</dd></div>
              <div><dt>{t("错误")}</dt><dd>{entry.error || "-"}</dd></div>
            </dl>
          </div>
        </div>
      ) : null}
    </>
  );
});

function RequestLogScreen({ initialAgent = "all", initialFocus = {}, onConsumeInitialAgent }: { initialAgent?: "all" | "codex" | "claude" | "grok"; initialFocus?: RequestLogFocus; onConsumeInitialAgent?: () => void }) {
  const payload = useGatewayPayload(10_000);
  const [entries, setEntries] = useState<RequestLogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  // 时间段、供应商、模型在后端筛：日志只读最近 REQUEST_LOG_LIMIT 条，先截再筛会漏。
  const [range, setRange] = useState<UsageRange>(initialFocus.range ?? "all");
  const [relayFilter, setRelayFilter] = useState<RelayFilter | null>(() => relayFilterFromFocus(initialFocus));
  const [modelFilter, setModelFilter] = useState(initialFocus.model ?? "");
  const [more, setMore] = useState(false);
  const [oldestMs, setOldestMs] = useState<number | null>(null);
  const [filter, setFilter] = useState<"all" | "running" | "success" | "failed" | "retried">("all");
  const [sessionFilter, setSessionFilter] = useState("all");
  const [agentFilter, setAgentFilter] = useState<"all" | "codex" | "claude" | "grok">(initialAgent);
  // 只管理 Codex 时固定看 Codex 的请求，不出现 Agent 筛选。
  const codexOnly = singleAgent(useAgentFocus()) === "codex";
  const agentScope = codexOnly ? "codex" : agentFilter;
  // 初始过滤只用一次：之后从侧栏进来仍是「全部」。
  useEffect(() => { if (initialAgent !== "all" || Object.keys(initialFocus).length) onConsumeInitialAgent?.(); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [clearing, setClearing] = useState(false);
  const [nowMs, setNowMs] = useState(() => Date.now());

  const hasRunning = entries.some((entry) => entry.state === "running");
  useEffect(() => {
    if (!hasRunning) return;
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [hasRunning]);

  const sinceMs = usageRangeStart(range);
  const filterArgs = {
    limit: REQUEST_LOG_LIMIT,
    sinceMs: sinceMs ?? null,
    agent: agentScope === "all" ? null : agentScope,
    ...relayFilterArgs(relayFilter),
    model: modelFilter || null,
  };
  // 自动刷新的定时器拿最新的筛选条件；序号保证只认最后一次读取。
  const filterRef = useRef(filterArgs);
  filterRef.current = filterArgs;
  const latestLoad = useRef(0);
  // 自动刷新只读上次之后新写入的记录（后端按游标增量读），不再每轮把整个日志读一遍、整表重渲染。
  const cursorRef = useRef<RequestLogCursor | null>(null);
  const entriesRef = useRef(entries);
  entriesRef.current = entries;
  const load = async (silent = false) => {
    const id = ++latestLoad.current;
    // 手动刷新、换筛选条件时整页重读：旧游标对应的是旧条件下的列表。
    if (!silent) {
      setLoading(true);
      cursorRef.current = null;
    }
    const args = silent && cursorRef.current ? { ...filterRef.current, cursor: cursorRef.current } : filterRef.current;
    try {
      const result = await invoke<RequestLogsResult>("load_request_logs", args);
      if (id !== latestLoad.current) return;
      if (result.status === "ok") {
        cursorRef.current = result.cursor ?? null;
        if (result.incremental) {
          const merged = mergeRequestLogDelta(entriesRef.current, result.entries ?? [], REQUEST_LOG_LIMIT, filterRef.current.sinceMs);
          if (merged.entries !== entriesRef.current) setEntries(merged.entries);
          if (merged.truncated) setMore(true);
        } else {
          setEntries((prev) => reuseRequestLogEntries(prev, result.entries ?? []));
          setMore(Boolean(result.more));
          setOldestMs(result.oldestMs ?? null);
        }
        setError("");
      } else {
        setError(result.message ? t(result.message) : t("读取请求日志失败。"));
      }
    } catch (cause) {
      setError(t(errorText(cause)));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [range, agentScope, filterArgs.relayId, filterArgs.relayUnknown, modelFilter]);

  // 每 2 秒增量读一次；窗口不可见时暂停，回到前台立刻补一轮。
  usePolling(() => load(true), REQUEST_LOG_POLL_MS, { enabled: autoRefresh, immediate: false });

  const clearLogs = async () => {
    if (!(await appConfirm(t("清空全部请求日志？此操作不可撤销。"), { title: t("清空请求日志"), confirmText: t("清空"), danger: true }))) return;
    setClearing(true);
    try {
      const result = await invoke<{ status: Status; message: string }>("clear_request_logs");
      if (result.status !== "ok") setError(t(result.message));
      await load(true);
    } catch (cause) {
      setError(t(errorText(cause)));
    } finally {
      setClearing(false);
    }
  };

  const sessions = useMemo(() => {
    const found = new Map<string, { id: string; title: string; count: number }>();
    for (const entry of entries) {
      const id = (entry.threadId ?? "").trim();
      if (!id) continue;
      const title = (entry.threadTitle ?? "").trim();
      const session = found.get(id);
      if (session) {
        session.count += 1;
        if (!session.title && title) session.title = title;
      } else {
        found.set(id, { id, title, count: 1 });
      }
    }
    // entries 本身按时间倒序，Map 的插入顺序即「最近活跃的会话优先」。
    return [...found.values()];
  }, [entries]);

  const hasUnknownSession = useMemo(
    () => entries.some((entry) => !(entry.threadId ?? "").trim()),
    [entries],
  );

  // 日志被清空/切换后，避免筛选项停留在已不存在的会话上。
  useEffect(() => {
    if (sessionFilter === "all" || sessionFilter === "__none__") return;
    if (!sessions.some((session) => session.id === sessionFilter)) {
      setSessionFilter("all");
    }
  }, [sessionFilter, sessions]);

  // 筛的时间段（或点进来的那一项）比日志里最早的一条还早：日志里看到的比统计少。
  const partial = oldestMs !== null && (sinceMs ?? 0) < oldestMs && (range !== "all" || relayFilter !== null || modelFilter !== "");

  // 模型展示名（分组别名）按 entries 和网关数据算一次，行组件和搜索都用它，不再每行每次渲染线性查找。
  const displayNames = useMemo(() => {
    const names = new Map<string, { model: string; upstream: string }>();
    for (const entry of entries) {
      names.set(entry.id, {
        model: groupDisplayName(payload, entry.model) ?? entry.model,
        upstream: groupDisplayName(payload, entry.upstreamModel) ?? entry.upstreamModel,
      });
    }
    return names;
  }, [entries, payload]);
  const toggleExpanded = useCallback((id: string) => setExpanded((current) => (current === id ? null : id)), []);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return entries.filter((entry) => {
      const running = entry.state === "running";
      if (filter === "running" && !running) return false;
      if (filter === "success" && (running || !entry.success)) return false;
      if (filter === "failed" && (running || entry.success)) return false;
      if (filter === "retried" && entry.autoRetries === 0 && entry.attempts <= 1) return false;
      if (agentScope !== "all" && requestLogAgent(entry) !== agentScope) return false;
      const threadId = (entry.threadId ?? "").trim();
      if (sessionFilter === "__none__") {
        if (threadId) return false;
      } else if (sessionFilter !== "all" && threadId !== sessionFilter) {
        return false;
      }
      if (!needle) return true;
      return [
        entry.model,
        entry.upstreamModel,
        displayNames.get(entry.id)?.model,
        displayNames.get(entry.id)?.upstream,
        entry.relayName,
        entry.path,
        entry.threadId ?? "",
        entry.error ?? "",
        entry.wireApi,
        entry.statusCode || "",
      ]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
  }, [entries, filter, query, sessionFilter, agentScope, displayNames]);

  // 工具条：搜索 + 自动刷新开关（关掉时旁边出现手动刷新）+ 三点菜单（清空日志，确认框保留）；
  // 下面一行：状态胶囊在左，Agent / 时间 / 会话三个下拉在右；表格一张卡，占满剩下的高度。
  const statusItems: Array<{ id: typeof filter; label: string }> = [
    { id: "all", label: t("全部") },
    { id: "running", label: t("进行中") },
    { id: "success", label: t("成功") },
    { id: "failed", label: t("失败") },
    { id: "retried", label: t("含重试") },
  ];
  return (
    <div className="request-log-panel request-log-page cx-page">
      <CxToolbar onQuery={setQuery} placeholder={t("搜索模型 / 供应商 / 路径 / 错误 / 状态码")} query={query}>
        <span className="request-log-auto" title={t("每 2 秒读取一次新请求")}>
          <Switch label={t("自动刷新")} on={autoRefresh} onToggle={() => setAutoRefresh(!autoRefresh)} />
          <span>{t("自动刷新")}</span>
        </span>
        {autoRefresh ? null : (
          <button aria-label={t("刷新")} className="cx-secondary request-log-refresh" onClick={() => void load()} title={t("刷新")} type="button">
            <RefreshCw className={loading ? "animate-spin" : undefined} />
          </button>
        )}
        <Menu>
          <MenuItem danger disabled={clearing || entries.length === 0} onClick={() => void clearLogs()}>{t("清空日志")}</MenuItem>
        </Menu>
      </CxToolbar>

      <CxFilters
        items={statusItems}
        label={t("状态筛选")}
        onChange={setFilter}
        trailing={
          <>
            {codexOnly ? null : <AppSelect
              ariaLabel={t("按 Agent 筛选")}
              className="request-log-select"
              menuWidth={160}
              onChange={(next) => setAgentFilter(next)}
              options={[
                { value: "all", label: t("全部 Agent") },
                { value: "codex", label: "Codex" },
                { value: "claude", label: "Claude Code" },
                { value: "grok", label: "Grok" },
              ] as AppSelectOption<typeof agentFilter>[]}
              size="sm"
              value={agentFilter}
            />}
            <AppSelect
              ariaLabel={t("时间段")}
              className="request-log-select"
              menuWidth={140}
              onChange={(next) => setRange(next)}
              options={[
                { value: "all", label: t("全部时间") },
                { value: "today", label: t("今天") },
                { value: "7d", label: t("近 7 天") },
                { value: "30d", label: t("近 30 天") },
              ] as AppSelectOption<UsageRange>[]}
              size="sm"
              value={range}
            />
            <AppSelect
              ariaLabel={t("按会话筛选")}
              className="request-log-select request-log-session-select"
              menuWidth={280}
              onChange={(next) => setSessionFilter(next)}
              options={[
                { value: "all", label: t("全部会话") },
                ...sessions.map((session) => ({
                  value: session.id,
                  label: tf("{0}（{1} 条）", [
                    session.title || tf("会话 {0}", [formatRequestLogSession(session.id)]),
                    String(session.count),
                  ]),
                })),
              ]}
              size="sm"
              title={t("按会话筛选")}
              value={sessionFilter}
            />
          </>
        }
        value={filter}
      />

      {relayFilter || modelFilter ? (
        <div className="request-log-chips">
          {relayFilter ? (
            <button className="request-log-chip" onClick={() => setRelayFilter(null)} title={t("去掉这个筛选")} type="button">
              {tf("供应商：{0}", [relayFilter.unknown ? t("未知") : relayFilter.label])}
              <X aria-hidden="true" className="h-3 w-3" />
            </button>
          ) : null}
          {modelFilter ? (
            <button className="request-log-chip" onClick={() => setModelFilter("")} title={t("去掉这个筛选")} type="button">
              {tf("模型：{0}", [groupDisplayName(payload, modelFilter) ?? modelFilter])}
              <X aria-hidden="true" className="h-3 w-3" />
            </button>
          ) : null}
        </div>
      ) : null}

      {error ? (
        <p className="field-hint request-log-error" role="alert">{error}</p>
      ) : null}

      <CxCard className="request-log-card">
        {loading && entries.length === 0 ? (
          <CxEmpty title={t("正在读取请求日志…")} />
        ) : filtered.length ? (
            <div className="request-log-table" role="table" aria-label={t("请求日志")}>
              <div className="request-log-row request-log-head" role="row">
                <span role="columnheader">{t("时间")}</span>
                <span role="columnheader">{t("状态")}</span>
                <span role="columnheader">{t("来源")}</span>
                <span role="columnheader">{t("模型")}</span>
                <span role="columnheader">{t("重试")}</span>
                <span role="columnheader">{t("输入")}</span>
                <span role="columnheader">{t("输出")}</span>
                <span role="columnheader">{t("缓存")}</span>
                <span role="columnheader">{t("费用")}</span>
                <span role="columnheader">{t("耗时")}</span>
              </div>
              {filtered.map((entry) => {
                const names = displayNames.get(entry.id);
                return (
                  <RequestLogRow
                    entry={entry}
                    expanded={expanded === entry.id}
                    key={entry.id}
                    model={names?.model ?? entry.model}
                    nowMs={entry.state === "running" ? nowMs : 0}
                    onToggle={toggleExpanded}
                    upstream={names?.upstream ?? entry.upstreamModel}
                  />
                );
              })}
            </div>
        ) : entries.length ? (
          <CxEmpty title={t("没有匹配当前筛选条件的请求。")} />
        ) : (
          <CxEmpty title={t("还没有请求日志")} detail={t("在 Codex 或已接入网关的 Agent 里发起一次对话后即可看到。")} />
        )}
      </CxCard>
      {more || partial ? (
        <p className="field-hint request-log-note">
          {more ? tf("只显示最近 {0} 条。", [REQUEST_LOG_LIMIT]) : ""}
          {partial && oldestMs ? tf("请求日志按大小滚动，最早只到 {0}；更早的请求只计入统计。", [new Date(oldestMs).toLocaleString(getLanguage() === "en" ? "en-US" : "zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })]) : ""}
        </p>
      ) : null}
    </div>
  );
}

function RelayEnvironmentScreen({ result, actions }: { result: RelayEnvironmentResult | null; actions: Actions }) {
  const proxyVariables = result?.proxyEnvironment.variables ?? [];
  const proxyVariableLabels = proxyVariables.map((item) => {
    const source = item.source === "user" ? t("用户环境") : item.source === "system" ? t("系统环境") : t("进程环境");
    return tf("{0}（{1}）", [item.name, source]);
  });
  const checks = [
    {
      id: "clash-verge-tun",
      title: t("Clash Verge Rev TUN 模式"),
      passed: result ? !result.clashVergeTun.enabled : false,
      detail: result
        ? result.clashVergeTun.enabled
          ? tf("检测到 TUN 模式已开启，请在 Clash Verge Rev 中关闭。配置：{0}", [result.clashVergeTun.configPath || t("未记录路径")])
          : result.clashVergeTun.configPath
            ? tf("TUN 模式已关闭。配置：{0}", [result.clashVergeTun.configPath])
            : t("未发现 Clash Verge Rev 配置，按未开启处理。")
        : t("等待检测。"),
    },
    {
      id: "proxy-environment",
      title: t("系统代理环境变量"),
      passed: result ? proxyVariables.length === 0 : false,
      detail: result
        ? proxyVariables.length
          ? tf("检测到代理环境变量：{0}。请清理后重新启动 Codex。", [proxyVariableLabels.join(t("、"))])
          : t("未检测到 HTTP_PROXY、HTTPS_PROXY、ALL_PROXY 或 FTP_PROXY。")
        : t("等待检测。"),
    },
    {
      id: "codex-dotenv",
      title: t("Codex .env 文件"),
      passed: result ? !result.codexEnvFile.exists : false,
      detail: result
        ? result.codexEnvFile.exists
          ? tf("检测到可能干扰供应商配置的 .env 文件：{0}", [result.codexEnvFile.path])
          : tf("未发现 .env 文件：{0}", [result.codexEnvFile.path])
        : t("等待检测。"),
    },
  ];
  const failed = result ? checks.filter((check) => !check.passed) : [];

  // 一行：全部正常时只显示结论；有问题时下面逐条列出要处理的项。
  return (
    <>
      <CxRow
        detail={!result ? t("正在读取本机环境") : failed.length ? tf("{0} 项需要处理，处理后重启 Codex", [failed.length]) : t("Clash TUN、代理环境变量、.env 文件都正常")}
        icon={<CxStatusIcon tone={!result ? undefined : failed.length ? "warn" : "ok"}><ShieldCheck aria-hidden="true" /></CxStatusIcon>}
        title={t("中转站环境检测")}
        tooltip={t("直连第三方中转站时，Clash Verge Rev 的 TUN 模式、系统代理环境变量和 ~/.codex/.env 都可能让请求走错地方。")}
      >
        <button className="cx-secondary" onClick={() => void actions.refreshRelayEnvironment()} type="button">
          <RefreshCw aria-hidden="true" />
          {t("重新检测")}
        </button>
      </CxRow>
      {failed.map((check) => (
        <CxRow
          className="cx-env-issue"
          detail={check.detail}
          icon={<CxStatusIcon tone="warn"><ShieldAlert aria-hidden="true" /></CxStatusIcon>}
          key={check.id}
          title={check.title}
        />
      ))}
    </>
  );
}

/** 直连用户点过「不再提示」：不再提醒把直连供应商加入网关供应商池。 */
const DIRECT_POOL_NUDGE_KEY = "codex-plus-plus.direct-pool-nudge-dismissed";

/** 托管的网关供应商带 openai 会话身份：Codex 保留 ChatGPT 官方登录，请求经本地代理进网关。 */
function codexGatewayKeepsLogin(profile: RelayProfile | undefined): boolean {
  return /^\s*model_provider\s*=\s*["']openai["']/m.test(profile?.configContents ?? "");
}

/**
 * 「Codex 连接」页顶部的「连接方式」：Codex 要么经 Codex++ 网关（供应商池），要么直连下面某个供应商，二选一。
 * - Codex 还指向托管供应商、网关里却没有它的接入记录（降级后又升级、gateway.json 换过或坏了）：
 *   给「切回直连」，否则页面一直显示「被网关接管」却断不开；
 * - Codex 正经网关接入：网关一行「使用中」，可去管理或断开并还原；
 * - 断开过但托管供应商还在（Codex 已从网关切回直连）：网关一行给「重新接入」；
 * - 从没接入过：建议改用新版网关（这里的直连配置照旧生效）。
 */
function LegacyRelayGatewayHint({
  form,
  onOpenGateway,
  onManageAgents,
  onDisconnect,
  onConnect,
  connecting = false,
}: {
  form: BackendSettings;
  onOpenGateway: () => void;
  onManageAgents: () => void;
  onDisconnect: () => void;
  /** 一键切到网关（沿用上次的接入设置）；不传时退回打开「Agent 接入」。 */
  onConnect?: () => void;
  connecting?: boolean;
}) {
  const managed = form.relayProfiles.find((profile) => profile.id === CODEX_GATEWAY_RELAY_ID);
  const onGateway = form.activeRelayId === CODEX_GATEWAY_RELAY_ID;
  // 网关里有没有开着的 Codex 接入记录：读共享的网关数据（和本页「供应商池」标记同一份轮询），不再单独请求；
  // 还没读到时当作未知，不误报「还指向网关托管的供应商」。
  const gatewayPayload = useGatewayPayload(15_000);
  const bound = gatewayPayload ? gatewayPayload.gateway?.bindings?.codex?.enabled === true : null;
  const orphan = onGateway && (!managed || bound === false);
  const viaGateway = onGateway && !orphan;
  const direct = form.relayProfiles.find((profile) => profile.id === form.activeRelayId && profile.id !== CODEX_GATEWAY_RELAY_ID);
  const count = codexGatewayModelCount(managed);
  // 状态标题：屏幕阅读器和悬停提示都读这一句。
  const status = orphan
    ? t("Codex 还指向网关托管的供应商")
    : viaGateway
      ? t("Codex 当前经 Codex++ 网关（新版）")
      : managed
        ? t("Codex 已从网关切回直连")
        : t("建议改用新版网关");
  const connect = onConnect ?? onManageAgents;
  // 两段式切换：经网关 ⇄ 直连。切到网关是一步；切回直连先预览要还原哪些文件，再确认。
  const modeSwitch = orphan ? null : (
    <div aria-label={t("Codex 的连接方式")} className="mg-segmented cx-connection-switch" role="radiogroup">
      <button
        aria-checked={viaGateway}
        data-active={viaGateway}
        disabled={connecting || viaGateway || !form.relayProfilesEnabled}
        onClick={connect}
        role="radio"
        title={form.relayProfilesEnabled ? t("Codex 改走 Codex++ 网关，用供应商池里的模型；新开的会话生效") : t("先在页面底部「高级」里打开供应商配置切换")}
        type="button"
      >
        {connecting ? <Loader2 className="animate-spin" /> : <Waypoints />}
        {t("经网关")}
      </button>
      <button
        aria-checked={!onGateway}
        data-active={!onGateway}
        disabled={!onGateway}
        onClick={onDisconnect}
        role="radio"
        title={t("断开网关，还原接入前的直连配置；会先列出要还原的文件，确认后才改")}
        type="button"
      >
        <Cable />
        {t("直连")}
      </button>
    </div>
  );
  // 一行说清：现在怎么连（图标 + 标题 + 一句说明），右侧是「经网关 | 直连」切换和三点菜单。
  // 另一种方式是什么、有什么好处，放在切换按钮的悬停说明里，不再单独占一行。
  const title = orphan
    ? t("Codex 还指向网关托管的供应商")
    : viaGateway
      ? t("经 Codex++ 网关")
      : direct
        ? tf("直连 · {0}", [direct.name || t("未命名供应商")])
        : t("直连");
  const detail = orphan
    ? t("网关里没有 Codex 的接入记录（可能降级后又升级过，或网关状态文件损坏），请求可能连不上。点「切回直连」切回接入前或列表里第一个直连供应商。")
    : viaGateway
      ? count
        ? tf("用供应商池里的 {0} 个模型：多 Key 轮换、按模型路由、出错自动换下一家", [count])
        : t("用供应商池里的模型：多 Key 轮换、按模型路由、出错自动换下一家")
      : t("Codex 直接请求这个供应商，不经网关；切到「经网关」后所有 Agent 共用供应商池");
  return (
    <CxCard className="cx-codex-connection">
      <CxRow
        badges={viaGateway && codexGatewayKeepsLogin(managed) ? <CxBadge>{t("保留 ChatGPT 登录")}</CxBadge> : null}
        detail={detail}
        icon={<CxStatusIcon tone={orphan ? "warn" : "ok"}>{viaGateway || orphan ? <Waypoints /> : <Cable />}</CxStatusIcon>}
        title={title}
        tooltip={status}
      >
        {orphan ? <button className="cx-primary" onClick={onDisconnect} type="button">{t("切回直连")}</button> : modeSwitch}
        <Menu>
          {viaGateway ? <MenuItem onClick={onManageAgents}>{t("管理接入")}</MenuItem> : <MenuItem onClick={onManageAgents}>{t("在「Agent 接入」里选默认模型")}</MenuItem>}
          <MenuItem onClick={onOpenGateway}>{t("打开供应商池")}</MenuItem>
          {viaGateway ? <MenuItem danger onClick={onDisconnect}>{t("断开并还原")}</MenuItem> : null}
        </Menu>
      </CxRow>
    </CxCard>
  );
}

/**
 * 直连供应商与网关供应商池的对应：哪些已经复制进池（池里的供应商带 `codex:<id>` 来源）、哪些还能复制。
 * 复制走网关「导入供应商」同一条命令，只往池里加，不改直连配置，也不切换 Codex 的连接方式。
 */
function useCodexPoolLink(actions: Actions, profileIds: string[]) {
  const payload = useGatewayPayload(15_000);
  const [importing, setImporting] = useState<string | null>(null);
  // 只看当前直连列表里还在的供应商：刚删掉、网关数据还没刷新的不算。
  const candidates = importCandidates(payload).filter((item) => (item.source === "codex" || item.source === "aggregate") && profileIds.includes(item.id));
  const stateOf = (profileId: string): "pooled" | "available" | null => {
    const item = candidates.find((candidate) => candidate.id === profileId);
    return item ? (item.imported ? "pooled" : "available") : null;
  };
  const pending = candidates.filter((item) => !item.imported);
  const importItems = async (items: ImportCandidate[], key: string) => {
    if (!items.length || importing) return;
    setImporting(key);
    try {
      const result = await invoke<GatewayResult>("import_gateway_providers", {
        items: items.map((item) => ({ source: item.source, id: item.id })),
        skipAgents: [],
      });
      publishGatewayPayload(result);
      await actions.showMessage(t("加入供应商池"), result.message, isSuccessStatus(result.status) ? "ok" : "failed");
    } catch (error) {
      await actions.showMessage(t("加入供应商池"), stringifyError(error), "failed");
    } finally {
      setImporting(null);
    }
  };
  return {
    ready: payload !== null,
    stateOf,
    pending,
    importing,
    addOne: (profileId: string) => importItems(candidates.filter((item) => item.id === profileId), profileId),
    addAll: () => importItems(pending, "all"),
  };
}
type CodexPoolLink = ReturnType<typeof useCodexPoolLink>;

/** 行右侧的「已在供应商池 / 加入供应商池」。 */
function PoolLinkControl({ link, profileId }: { link: CodexPoolLink; profileId: string }) {
  const state = link.stateOf(profileId);
  if (state === "pooled") return <CxBadge title={t("网关的供应商池里已经有它，所有接入网关的 Agent 都能用")} tone="ok">{t("已在供应商池")}</CxBadge>;
  if (state !== "available") return null;
  return (
    <button
      className="cx-secondary cx-pool-add"
      disabled={link.importing !== null}
      onClick={(event) => {
        event.stopPropagation();
        void link.addOne(profileId);
      }}
      title={t("复制进网关的供应商池：所有接入网关的 Agent 都能用；直连配置照旧保留")}
      type="button"
    >
      {link.importing === profileId ? <Loader2 className="animate-spin" /> : <Waypoints />}
      {t("加入供应商池")}
    </button>
  );
}

function RelayScreen({
  addDirectProviderRequested,
  onRequestedProviderOpened,
  settings,
  relayFiles,
  envConflicts,
  ccsProviders,
  form,
  actions,
  connection,
  advanced,
  advancedOpen = false,
  onOpenGateway,
}: {
  addDirectProviderRequested: boolean;
  onRequestedProviderOpened: () => void;
  settings: SettingsResult | null;
  relayFiles: RelayFilesResult | null;
  envConflicts: EnvConflictsResult | null;
  ccsProviders: CcsProvidersResult | null;
  form: BackendSettings;
  actions: Actions;
  /** 页面顶部的「连接方式」（经网关 / 直连）。只在列表视图显示，进编辑页不显示。 */
  connection?: ReactNode;
  /** 页面底部「高级」折叠区里的内容：目录外模型映射、供应商独立代理、环境检测。 */
  advanced?: ReactNode;
  /** 从「旧版代理设置」的旧入口进来时直接展开「高级」。 */
  advancedOpen?: boolean;
  /** 打开网关的供应商池。 */
  onOpenGateway?: () => void;
}) {
  // normalizeSettings 要解析共享 TOML、规整每个供应商，不随每次渲染重算。
  const normalized = useMemo(() => normalizeSettings(form), [form]);
  const takenOverByGateway = normalized.activeRelayId === CODEX_GATEWAY_RELAY_ID;
  // 直连供应商和网关供应商池打通：每行标出是否已在池里，没进池的可以一键复制进去。
  const poolLink = useCodexPoolLink(actions, normalized.relayProfiles.map((profile) => profile.id));
  // 直连用户点过「不再提示」就不再提醒加入供应商池。hook 必须在下面编辑页 / 目录页的提前返回之前。
  const [nudgeDismissed, setNudgeDismissed] = useState(() => {
    try { return window.localStorage.getItem(DIRECT_POOL_NUDGE_KEY) === "1"; } catch { return false; }
  });
  const dismissNudge = () => {
    setNudgeDismissed(true);
    try { window.localStorage.setItem(DIRECT_POOL_NUDGE_KEY, "1"); } catch { /* 存不了就只在本次关掉 */ }
  };
  const [detailProfileId, setDetailProfileId] = useState<string | null>(null);
  const [newProfileDraft, setNewProfileDraft] = useState<RelayProfile | null>(null);
  const [providerCatalogOpen, setProviderCatalogOpen] = useState(false);
  useEffect(() => {
    if (!addDirectProviderRequested) return;
    setNewProfileDraft(null);
    setDetailProfileId(null);
    setProviderCatalogOpen(true);
    onRequestedProviderOpened();
  }, [addDirectProviderRequested, onRequestedProviderOpened]);
  // 目录页的搜索词放在这里：从模板进入编辑页再返回时，目录页会重新挂载，搜索不能丢。
  const [providerCatalogQuery, setProviderCatalogQuery] = useState("");
  // 进入时停在当前供应商所在的分区；找不到当前供应商时，没有直连项就展示协议转换分区。
  const initialProviderView = (): ProviderViewKind => {
    const active = normalized.relayProfiles.find((profile) => profile.id === normalized.activeRelayId);
    if (active) return relayProviderView(active);
    const views = normalized.relayProfiles.map(relayProviderView);
    return !views.includes("direct") && views.includes("routing") ? "routing" : "direct";
  };
  const [providerView, setProviderView] = useState<ProviderViewKind>(initialProviderView);
  const [newAggregateMode, setNewAggregateMode] = useState<AggregateMode | null>(null);
  const [thirdPartyImportOpen, setThirdPartyImportOpen] = useState(false);
  const thirdPartyImportTriggerRef = useRef<HTMLDivElement>(null);
  const [thirdPartyImportPosition, setThirdPartyImportPosition] = useState<AnchoredMenuLayout | null>(null);
  const [ccsDbPathDraft, setCcsDbPathDraft] = useState(normalized.ccsDbPath);
  const [ccsPathSaving, setCcsPathSaving] = useState(false);
  const [ccsBindings, setCcsBindings] = useState<Record<string, string>>({});
  // 路径草稿与已保存的不一致时，列表还是旧库的：先「应用」才能导入。
  const ccsPathDirty = ccsDbPathDraft.trim() !== normalized.ccsDbPath;
  useEffect(() => setCcsDbPathDraft(normalized.ccsDbPath), [normalized.ccsDbPath]);
  const saveCcsDbPath = async (path = ccsDbPathDraft) => {
    setCcsPathSaving(true);
    try {
      const saved = await actions.saveSettingsValue({ ...normalized, ccsDbPath: path.trim() }, true);
      if (saved) await actions.refreshCcsProviders();
    } finally {
      setCcsPathSaving(false);
    }
  };
  const selectCcsDatabase = async () => {
    try {
      const selected = await open({
        title: t("选择 cc-switch 数据库"),
        multiple: false,
        directory: false,
        filters: [{ name: "SQLite", extensions: ["db", "sqlite", "sqlite3"] }],
      });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (path) {
        setCcsDbPathDraft(path);
        await saveCcsDbPath(path);
      }
    } catch (error) {
      await actions.showMessage(t("cc-switch 导入"), stringifyError(error), "failed");
    }
  };
  const detailProfile = newProfileDraft || (detailProfileId
    ? normalized.relayProfiles.find((profile) => profile.id === detailProfileId) || null
    : null);
  const isNewProfile = !!newProfileDraft;
  const saveRelaySettings = async (next: BackendSettings) => {
    return actions.saveSettingsValue(next, true);
  };
  // 拖拽排序专用保存：松手时乐观写入表单，卡片立即落位；等 save_settings 回读
  // 会让卡片先弹回原位、保存完成后再整体跳一次。详情页保存与「配置切换」开关
  // 仍走 saveRelaySettings，不参与乐观更新。
  // 连续快速拖拽共用一条回滚基线：只有链尾那次失败才回滚，且基线随链中的成功
  // 保存推进，早先的失败不会盖掉后续已落盘的结果。
  const relayReorderSeqRef = useRef(0);
  const relayReorderRollbackRef = useRef<BackendSettings | null>(null);
  const reorderRelaySettings = async (next: BackendSettings) => {
    const seq = ++relayReorderSeqRef.current;
    if (seq === 1) relayReorderRollbackRef.current = form;
    actions.previewSettingsForm(next);
    const saved = await actions.saveSettingsValue(next, true);
    if (seq === relayReorderSeqRef.current) {
      if (!saved) {
        const rollback = relayReorderRollbackRef.current;
        if (rollback) actions.previewSettingsForm(rollback);
      }
      relayReorderSeqRef.current = 0;
      relayReorderRollbackRef.current = null;
    } else if (saved) {
      relayReorderRollbackRef.current = saved;
    }
    return saved;
  };
  const createNewAggregateProfile = (mode: AggregateMode) => {
    const draft = aggregateProfileForMode(createAggregateRelayProfile(normalized), normalized, mode);
    setDetailProfileId(null);
    setNewProfileDraft(draft);
    setNewAggregateMode(mode);
    if (!normalizeAggregateConfig(draft.aggregate, aggregateMemberCandidates(normalized.relayProfiles, draft.id)).members.length) {
      void actions.showMessage(
        t("创建聚合供应商"),
        t("已打开聚合供应商详情；请先添加或完善至少 1 个 API 供应商，或登录一个官方账号，再勾选为成员。"),
        "failed",
      );
    }
  };
  const visibleProfiles = normalized.relayProfiles.filter((profile) => (
    relayProviderView(profile) === providerView
  ));
  // 分区只在首次进入时自动选择；之后尊重用户点击，避免只有协议转换供应商时
  // 「直连」分页被强制弹回、看起来像切换按钮点不动（#2470/#2481/#2495）。
  const providerViewAutoSelectedRef = useRef(false);
  const selectProviderView = (view: ProviderViewKind) => {
    providerViewAutoSelectedRef.current = true;
    setProviderView(view);
  };
  useEffect(() => {
    // 设置读回之前表单是默认值（自带一个 default 供应商），不能据此定分区。
    if (providerViewAutoSelectedRef.current) return;
    if (!settings || !normalized.relayProfiles.length) return;
    providerViewAutoSelectedRef.current = true;
    setProviderView(initialProviderView());
  }, [settings, normalized.relayProfiles, normalized.activeRelayId]);
  // 分页仅控制渲染。卡片的删除、复制和排序必须基于完整配置，避免隐藏的
  // 当前供应商被 syncLegacyRelayFields 当成不存在，改写成直连列表第一项。
  const switchRelayProfileFromList = (profileId: string) => {
    const next = syncLegacyRelayFields({ ...normalized, activeRelayId: profileId });
    void actions.switchRelayProfile(next, normalized.activeRelayId);
  };
  // 「高级」里的供应商代理列表点进来时直接跳到代理输入框。
  const [detailSection, setDetailSection] = useState<"proxy" | null>(null);
  const openRelayProfileSection = useCallback((profileId: string, section: "proxy") => {
    setNewProfileDraft(null);
    setDetailSection(section);
    setDetailProfileId(profileId);
  }, []);
  const editRelayProfile = async (profileId: string) => {
    setNewProfileDraft(null);
    setDetailSection(null);
    setDetailProfileId(
      normalized.relayProfiles.some((item) => item.id === profileId) ? profileId : null,
    );
  };
  useEffect(() => {
    if (!newProfileDraft && detailProfileId && !normalized.relayProfiles.some((profile) => profile.id === detailProfileId)) {
      setDetailProfileId(null);
    }
  }, [detailProfileId, newProfileDraft, normalized.relayProfiles]);
  useEffect(() => {
    if (!newProfileDraft && detailProfileId === normalized.activeRelayId) {
      void actions.refreshRelayFiles();
    }
  }, [detailProfileId, newProfileDraft, normalized.activeRelayId]);
  const openThirdPartyImport = () => {
    const next = !thirdPartyImportOpen;
    if (next) {
      // 菜单改为 fixed 定位：它原本挂在 .mg-content（overflow:auto）里，
      // 面板较矮时后半截会被裁掉（ccswitch 导入按钮、刷新列表都看不到）。
      // 高度必须按触发按钮的位置收窄，否则 cc-switch 供应商一多（几十行），
      // 菜单底部会超出窗口下沿，最后那排按钮永远点不到（issue #1906）。
      const rect = thirdPartyImportTriggerRef.current?.getBoundingClientRect();
      setThirdPartyImportPosition(
        rect
          ? resolveAnchoredMenuLayout(rect, { width: window.innerWidth, height: window.innerHeight })
          : null,
      );
    }
    setThirdPartyImportOpen(next);
    if (!ccsProviders) void actions.refreshCcsProviders(true);
  };
  useEffect(() => {
    if (!thirdPartyImportOpen) return;
    const close = () => setThirdPartyImportOpen(false);
    // 菜单挂在 .mg-page（overflow:hidden）里，只能自己判定外点，不然点空白处收不起来。
    const handlePointerDown = (event: PointerEvent) => {
      if (thirdPartyImportTriggerRef.current?.contains(event.target as Node)) return;
      close();
    };
    document.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("resize", close);
    // capture 监听内层 .mg-content 的滚动：页面滚了菜单会跟触发按钮错位，得收起。
    // 但供应商列表自己也会滚（滚轮、拖滚动条），scroll 事件同样会冒到 window 的
    // capture 阶段——不排除掉它，列表一动菜单就关，等于列表不能滚（issue #1906）。
    const handleScroll = (event: Event) => {
      if (thirdPartyImportTriggerRef.current?.contains(event.target as Node)) return;
      close();
    };
    window.addEventListener("scroll", handleScroll, { capture: true, passive: true });
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", handleScroll, { capture: true });
    };
  }, [thirdPartyImportOpen]);
  if (detailProfile) {
    return (
      <RelayProfileDetail
        profile={detailProfile}
        relayFiles={!isNewProfile && detailProfile.id === normalized.activeRelayId ? relayFiles : null}
        form={normalized}
        isNew={isNewProfile}
        initialAggregateMode={isNewProfile ? newAggregateMode : null}
        initialSection={isNewProfile ? null : detailSection}
        // 从目录页选模板进来时目录页状态还开着：返回只清掉草稿，回到目录页继续挑。
        onBack={() => {
          setNewProfileDraft(null);
          setNewAggregateMode(null);
          setDetailProfileId(null);
        }}
        onFormChange={saveRelaySettings}
        // 保存后回到供应商列表，目录页一并关掉。
        onSaved={() => {
          setNewProfileDraft(null);
          setNewAggregateMode(null);
          setDetailProfileId(null);
          setProviderCatalogOpen(false);
          setProviderCatalogQuery("");
        }}
        actions={actions}
      />
    );
  }

  if (providerCatalogOpen) {
    return (
      <ProviderCatalogScreen
        form={normalized}
        query={providerCatalogQuery}
        onQueryChange={setProviderCatalogQuery}
        onBack={() => {
          setProviderCatalogOpen(false);
          setProviderCatalogQuery("");
        }}
        // 不关目录页：编辑页优先渲染，从编辑页返回时回到这里而不是供应商列表。
        onSelect={(profile) => {
          setNewProfileDraft(profile);
          setNewAggregateMode(null);
        }}
      />
    );
  }

  const directProfiles = normalized.relayProfiles.filter((profile) => profile.id !== CODEX_GATEWAY_RELAY_ID);
  const openCatalog = () => { setNewProfileDraft(null); setDetailProfileId(null); setProviderCatalogOpen(true); };
  const viewCount = (view: ProviderViewKind) => normalized.relayProfiles.filter((profile) => relayProviderView(profile) === view).length;
  const viewTip: Record<ProviderViewKind, string> = {
    direct: t("直接访问 Responses API 供应商：只写入 Base URL 与 Key，不做协议转换。"),
    modelAggregation: t("Codex 专用的小网关：把几家的模型合并到 Codex 的模型列表，选哪个模型就发到哪家。网关的供应商池默认就是这样分发，而且所有 Agent 都能用。"),
    routing: t("供应商只支持 Chat Completions（或配置了模型路由）：由本地协议代理转换后转发。"),
    failover: t("Codex 专用的小网关：几家组成一个池，出错换下一家、按会话或请求轮换、按权重分。网关的「路由」里用模型组做同样的事，而且所有 Agent 都能用。"),
  };
  const emptyView: Record<ProviderViewKind, { title: string; detail: string; action: string; run: () => void }> = {
    direct: { title: t("这个分页还没有直连供应商"), detail: t("直连分页只显示 Responses 直连且未配置模型路由的供应商；其他供应商在其余分页，配置不会丢失。"), action: t("添加供应商"), run: openCatalog },
    routing: { title: t("这个分页还没有需协议转换的供应商"), detail: t("Chat Completions 协议或配置了模型路由的供应商会显示在这里；其他供应商在其余分页。"), action: t("添加供应商"), run: openCatalog },
    // 新用户优先推荐网关（所有 Agent 都能用）；只想让 Codex 用的，仍可以在这里建。
    failover: { title: t("还没有故障转移配置"), detail: t("推荐用网关「路由」里的模型组：所有 Agent 都能用。只想给 Codex 直连用，在这里建。"), action: t("创建故障转移配置"), run: () => createNewAggregateProfile("provider") },
    modelAggregation: { title: t("还没有模型聚合配置"), detail: t("推荐用网关的供应商池：本身就按模型分发，所有 Agent 都能用。只想给 Codex 直连用，在这里建。"), action: t("创建模型聚合配置"), run: () => createNewAggregateProfile("model") },
  };
  // 「加入供应商池」的提示：还有直连供应商没进池就提一次；网关接管期间同样可以迁移。
  // 不想换到网关的直连用户可以「不再提示」（只关提示，每行的「加入供应商池」照旧在）；已经在用网关的总是提示。
  // 收在「直连供应商」标题右侧的一行：几个还没进池 · 全部加入 · ×（不再提示）。
  const poolHint = normalized.relayProfilesEnabled && poolLink.pending.length && (takenOverByGateway || !nudgeDismissed) ? (
    <span className="cx-pool-hint" title={t("加入后 Codex、Claude Code 等所有接入网关的 Agent 都能用，支持多 Key 轮换和故障转移；直连配置照旧保留。")}>
      <Waypoints aria-hidden="true" />
      {tf("{0} 个还没加入供应商池", [poolLink.pending.length])}
      <button className="cx-secondary" disabled={poolLink.importing !== null} onClick={() => void poolLink.addAll()} type="button">
        {poolLink.importing === "all" ? <Loader2 className="animate-spin" /> : null}
        {t("全部加入")}
      </button>
      {takenOverByGateway ? null : (
        <button aria-label={t("不再提示")} className="cx-icon-button" onClick={dismissNudge} title={t("继续只用直连；以后想换，每个供应商右侧的「加入供应商池」还在")} type="button">
          <X />
        </button>
      )}
    </span>
  ) : null;

  return (
    <div className="mg-page relay-management-page cx-page cx-codex-connection-page">
      <PageHeader
        icon={<KeyRound className="h-5 w-5" />}
        title={t("连接与供应商")}
        actions={(
          <>
            {/* 「从第三方导入」接管期间也保留（#2801）：导入只写直连供应商列表、不动 Codex 配置，
                导进来的供应商可以直接多开，也能「加入供应商池」给网关用。 */}
            <div className="third-party-import" ref={thirdPartyImportTriggerRef}>
              <button className="cx-secondary" onClick={openThirdPartyImport} type="button">
                <Download />
                {t("从第三方导入")}
              </button>
              {thirdPartyImportOpen ? (
                <div
                  className="third-party-import-menu"
                  style={thirdPartyImportPosition
                    ? {
                      position: "fixed",
                      top: thirdPartyImportPosition.top,
                      right: thirdPartyImportPosition.right,
                      maxHeight: thirdPartyImportPosition.maxHeight,
                    }
                    : undefined}
                >
                  <div className="third-party-import-head">
                    <strong>{t("从 cc-switch 导入")}</strong>
                    <small>{t("只读取 cc-switch 的数据库，不会改动它。")}</small>
                  </div>
                  <div className="third-party-import-source">
                    <div className="third-party-import-path">
                      <Input
                        aria-label={t("cc-switch 数据库路径")}
                        onChange={(event) => setCcsDbPathDraft(event.currentTarget.value)}
                        placeholder={t("默认 ~/.cc-switch/cc-switch.db")}
                        value={ccsDbPathDraft}
                      />
                      <Button disabled={ccsPathSaving} onClick={() => void selectCcsDatabase()} size="sm" title={t("选择数据库")} variant="secondary">
                        <FolderOpen className="h-4 w-4" />
                        {t("选择")}
                      </Button>
                      {ccsPathDirty ? (
                        <Button disabled={ccsPathSaving} onClick={() => void saveCcsDbPath()} size="sm">
                          {ccsPathSaving ? t("应用中") : t("应用")}
                        </Button>
                      ) : null}
                    </div>
                    {ccsProviders && isSuccessStatus(ccsProviders.status) ? (
                      <small className="third-party-import-meta" title={ccsProviders.dbPath}>{t("实际来源")}：{ccsProviders.dbPath}</small>
                    ) : null}
                    {ccsProviders?.fallbackReason ? <small className="third-party-import-warning">{ccsProviders.fallbackReason}</small> : null}
                  </div>
                  {/* 供应商列表单独滚动：cc-switch 里几十个供应商时，底部按钮仍然可见可点（issue #1906）。 */}
                  <div className="third-party-import-list">
                    {ccsProviders?.providers.length ? (
                      <div className="third-party-import-list-head">
                        <span>{tf("Codex 供应商 {0} 个", [ccsProviders.providers.length])}</span>
                        <span>{t("导入为")}</span>
                      </div>
                    ) : (
                      <p className="third-party-import-empty">{ccsProviderSummary(ccsProviders)}</p>
                    )}
                    {ccsProviders?.providers.map((provider) => (
                      <label className="third-party-import-row" key={provider.sourceId}>
                        <span className="third-party-import-name">
                          <strong title={provider.name}>{provider.name}</strong>
                          <small title={provider.baseUrl}>{urlHost(provider.baseUrl) || t("未填写地址")}</small>
                        </span>
                        <AppSelect
                          ariaLabel={tf("{0} 导入为", [provider.name])}
                          menuWidth={240}
                          onChange={(next) => setCcsBindings((current) => ({ ...current, [provider.sourceId]: next }))}
                          options={[
                            { value: "", label: t("新增（同名自动关联）") },
                            ...normalized.relayProfiles.map((profile) => ({ value: profile.id, label: `${t("关联已有")}：${profile.name}` })),
                          ]}
                          size="sm"
                          value={ccsBindings[provider.sourceId] || ""}
                        />
                      </label>
                    ))}
                  </div>
                  <div className="third-party-import-actions">
                    {ccsPathDirty ? <small className="third-party-import-warning">{t("路径已修改，点「应用」读取新数据库后再导入。")}</small> : null}
                    <div className="third-party-import-footer">
                      <Button aria-label={t("刷新列表")} onClick={() => void actions.refreshCcsProviders()} size="icon" title={t("刷新列表")} variant="ghost">
                        <RefreshCw className="h-4 w-4" />
                      </Button>
                      <Button
                        disabled={ccsPathSaving || ccsPathDirty || !ccsProviders?.providers.length}
                        onClick={() => {
                          setThirdPartyImportOpen(false);
                          void actions.importCcsProviders(ccsBindings);
                        }}
                      >
                        <Download className="h-4 w-4" />
                        {ccsProviders?.providers.length ? tf("导入 {0} 个供应商", [ccsProviders.providers.length]) : t("导入")}
                      </Button>
                    </div>
                  </div>
                </div>
              ) : null}
            </div>
              <button className="cx-primary" onClick={openCatalog} type="button">
                <Plus />
                {takenOverByGateway ? t("添加直连供应商") : t("添加供应商")}
              </button>
          </>
        )}
      />
      {connection}
      <NativeAuthControl onRestart={() => actions.restart()} />
      <EnvConflictNotice envConflicts={envConflicts} actions={actions} />
      <SshRemoteLoopbackNotice notice={envConflicts?.sshRemoteLoopback ?? null} />
      {!normalized.relayProfilesEnabled ? (
        <CxBanner
          detail={takenOverByGateway
            ? t("Codex++ 无法重新接入 Codex。重新开启只恢复配置写入能力，不会切换直连供应商或断开当前网关。")
            : t("Codex++ 不会写入 Codex 的 config.toml / auth.json，直连供应商列表已隐藏。")}
          icon={<PowerOff />}
          title={t("供应商配置切换已关闭")}
          tone="warn"
        >
          <button className="cx-primary" onClick={() => void saveRelaySettings({ ...normalized, relayProfilesEnabled: true })} type="button">{t("打开配置切换")}</button>
        </CxBanner>
      ) : null}
      {normalized.relayProfilesEnabled && takenOverByGateway ? (
        <CxCard className="cx-takeover">
          <CxEmpty
            detail={t("可以先添加或编辑直连档案。保存后继续使用网关，点上方「直连」预览并确认还原。")}
            title={t("Codex 正由 Codex++ 网关接管")}
          >
            {onOpenGateway ? <button className="cx-secondary" onClick={onOpenGateway} type="button">{t("打开供应商池")}</button> : null}
          </CxEmpty>
          {directProfiles.map((profile) => (
            <CxRow key={profile.id} title={profile.name || t("未命名供应商")} detail={relayProfileModeLabel(profile)}>
              <button className="cx-secondary" onClick={() => void editRelayProfile(profile.id)} type="button">{t("编辑档案")}</button>
            </CxRow>
          ))}
        </CxCard>
      ) : null}
      {/* 总开关关闭时供应商切换不会写入 Codex：分页和列表都没有意义，整块隐藏，只留上方提示条。 */}
      {normalized.relayProfilesEnabled && !takenOverByGateway ? (
        <CxSection
          aside={(
            <>
              {poolHint}
              {(providerView === "failover" || providerView === "modelAggregation") && visibleProfiles.length ? (
                <button className="cx-secondary" onClick={() => createNewAggregateProfile(providerView === "failover" ? "provider" : "model")} type="button">
                  <Plus />
                  {providerView === "failover" ? t("新建故障转移") : t("新建模型聚合")}
                </button>
              ) : null}
            </>
          )}
          count={directProfiles.length}
          title={t("直连供应商")}
        >
          <CxFilters
            items={(["direct", "modelAggregation", "routing", "failover"] as ProviderViewKind[]).map((view) => ({
              id: view,
              label: <span title={viewTip[view]}>{/* 「直连」和分区标题「直连供应商」撞名：按是否要协议转换来叫，和「需协议转换」对照。 */}{view === "direct" ? t("无需转换") : view === "modelAggregation" ? t("模型聚合") : view === "routing" ? t("需协议转换") : t("故障转移")}</span>,
              count: viewCount(view),
            }))}
            label={t("供应商配置模式")}
            onChange={selectProviderView}
            value={providerView}
          />
          {(providerView === "modelAggregation" || providerView === "failover") && visibleProfiles.length ? (
            <p className="cx-aggregate-note">
              <Waypoints aria-hidden="true" />
              <span>{providerView === "modelAggregation"
                ? t("模型聚合是 Codex 专用的小网关。网关的供应商池本身就按模型分发，所有 Agent 都能用；在下面点「加入供应商池」，成员就进了池子。")
                : t("故障转移是 Codex 专用的小网关。网关「路由」里的模型组做的是同一件事，所有 Agent 都能用；在下面点「加入供应商池」，会建好同名模型组。")}</span>
              {onOpenGateway ? <button className="cx-link" onClick={onOpenGateway} type="button">{t("打开供应商池")}</button> : null}
            </p>
          ) : null}
          {!visibleProfiles.length ? (
            <CxCard>
              <CxEmpty detail={emptyView[providerView].detail} title={emptyView[providerView].title}>
                <button className="cx-secondary" onClick={emptyView[providerView].run} type="button">{emptyView[providerView].action}</button>
              </CxEmpty>
            </CxCard>
          ) : null}
          <RelayProfileList
            form={normalized}
            visibleProfiles={visibleProfiles}
            onEdit={(profileId) => void editRelayProfile(profileId)}
            onFormChange={saveRelaySettings}
            onReorder={reorderRelaySettings}
            onSwitchProfile={switchRelayProfileFromList}
            disabled={actions.relaySwitching}
            poolLink={poolLink}
            actions={actions}
          />
        </CxSection>
      ) : null}
      {/* 「切换时保留」对直连和网关都生效（网关重写 Codex 配置也走同一条切换流程），
          所以放在两种模式都看得到的地方；以前藏在直连供应商编辑的「通用配置文件」里，网关用户找不到（#3115）。 */}
      {normalized.relayProfilesEnabled ? (
        <CxSection>
          <CxCard>
            <CxRow
              className="cx-preserve-row"
              detail={<>
                {t("打开后，切换供应商、接入或重写网关、重启 Codex 时：实时 config.toml 里改过的 MCP 服务器（命令、参数、env、超时）以实时文件为准，缺失的项目信任设置和通知命令从实时文件补回。直连和网关都适用。")}
                <br />
                {t("始终按目标重写的供应商设置：model_provider、model、供应商地址与凭据、模型目录。不论开关都保留：Codex++ 没有托管的 MCP / 插件条目、[[skills.config]] 技能启停；网关模式下 developer_instructions 等根级个人设置也会跟着走。")}
              </>}
              title={t("切换时保留实时配置里的改动")}
            >
              <label className="switch-row compact relay-master-switch relay-master-switch-compact">
                <input
                  aria-label={t("切换时保留实时配置里的改动")}
                  checked={normalized.relayPreserveNonproviderSettings === true}
                  onChange={(event) => {
                    const next = { ...normalized, relayPreserveNonproviderSettings: event.currentTarget.checked };
                    void saveRelaySettings(next);
                  }}
                  type="checkbox"
                />
                <span>{t("保留")}</span>
                <ToggleVisual />
              </label>
            </CxRow>
          </CxCard>
        </CxSection>
      ) : null}
      {/* 高级：配置切换总开关、目录外模型映射、供应商独立代理、环境检测（原「旧版代理设置」页）。
          都是直连专用：被网关接管时整块不显示；配置切换被关掉时仍显示，好把它重新打开。 */}
      {takenOverByGateway && normalized.relayProfilesEnabled ? null : (
      <CxSection>
        <CxCard>
          <details className="cx-set-disclosure cx-set-disclosure-first cx-codex-advanced" id="codex-connection-advanced" open={advancedOpen || undefined}>
            <summary>
              {t("高级")}
              <span className="cx-set-summary-hint">{normalized.relayProfilesEnabled ? t("配置切换已开启") : t("配置切换已关闭")}</span>
            </summary>
            <CxRow
              detail={t("关闭后 Codex++ 不再写入 Codex 的 config.toml / auth.json；启动 Codex 时始终不会自动改这些文件。")}
              title={t("供应商配置切换")}
              tooltip={takenOverByGateway ? t("重新开启配置切换只恢复 Codex++ 写入 Codex 配置的能力，不会切换直连供应商或断开当前网关。") : undefined}
            >
              <label className="switch-row compact relay-master-switch relay-master-switch-compact">
                <input
                  aria-label={t("启用供应商配置切换")}
                  checked={normalized.relayProfilesEnabled}
                  onChange={(event) => {
                    const next = { ...normalized, relayProfilesEnabled: event.currentTarget.checked };
                    void saveRelaySettings(next);
                  }}
                  type="checkbox"
                />
                <span>{t("配置切换")}</span>
                <ToggleVisual />
              </label>
            </CxRow>
            <RelayProfileLinkContext.Provider value={openRelayProfileSection}>
              {advanced}
            </RelayProfileLinkContext.Provider>
          </details>
        </CxCard>
      </CxSection>
      )}
    </div>
  );
}

function CodexMultiLaunchScreen({
  form,
  relayFiles,
  onOpenRelay,
  actions,
}: {
  form: BackendSettings;
  relayFiles: RelayFilesResult | null;
  onOpenRelay: () => void;
  actions: Actions;
}) {
  // normalizeSettings 要解析共享 TOML、规整每个供应商，不随每次渲染重算。
  const normalized = useMemo(() => normalizeSettings(form), [form]);
  // #2801：Codex 被网关接管、或「配置切换」关闭时，旧版供应商页的列表整块隐藏，
  // 多开里这些供应商（多是早年从 cc-switch 导入的）就没地方编辑、删除了，这里直接给入口。
  const [detailProfileId, setDetailProfileId] = useState<string | null>(null);
  const detailProfile = detailProfileId
    ? normalized.relayProfiles.find((profile) => profile.id === detailProfileId) || null
    : null;
  useEffect(() => {
    if (detailProfileId && !normalized.relayProfiles.some((profile) => profile.id === detailProfileId)) {
      setDetailProfileId(null);
    }
  }, [detailProfileId, normalized.relayProfiles]);
  const deleteProfile = async (profile: RelayProfile) => {
    const next = await confirmRemoveRelayProfile(normalized, profile, actions, "multiLaunch");
    if (next) await actions.saveSettingsValue(next, true);
  };
  if (detailProfile) {
    return (
      <RelayProfileDetail
        profile={detailProfile}
        relayFiles={detailProfile.id === normalized.activeRelayId ? relayFiles : null}
        form={normalized}
        onBack={() => setDetailProfileId(null)}
        onFormChange={(next) => actions.saveSettingsValue(next, true)}
        onSaved={() => setDetailProfileId(null)}
        actions={actions}
      />
    );
  }
  const { CxAddRow, CxBadge, CxCard, CxEmpty, CxRow, CxSection, CxStatusIcon } = AgentCx;
  const instances = actions.multiLaunchInstances;
  return (
    <div className="mg-page relay-management-page cx-page multi-launch-page">
      <PageHeader
        icon={<Layers3 className="h-5 w-5" />}
        title={t("Codex多开")}
        actions={(
          // 一个刷新同时读供应商和运行实例。
          <button
            className="cx-secondary"
            disabled={actions.multiLaunchInstancesLoading}
            onClick={() => {
              void actions.refreshSettings(true);
              void actions.refreshMultiLaunchInstances(false);
            }}
            type="button"
          >
            <RefreshCw className={actions.multiLaunchInstancesLoading ? "animate-spin" : ""} />
            {t("刷新")}
          </button>
        )}
      />
      <CxSection count={instances.length} title={t("运行中的实例")}>
        <CxCard className="multi-launch-running-panel">
          {instances.length ? instances.map((instance) => {
            const status = multiLaunchInstanceStatus(instance);
            const running = multiLaunchInstanceRunning(instance);
            const mainInstance = multiLaunchInstanceIsMain(instance);
            const canStop = multiLaunchInstanceCanStop(instance);
            const canRestart = multiLaunchInstanceCanRestart(instance);
            const pending = actions.multiLaunchActionPending === instance.instanceId;
            const label = formatMultiLaunchInstanceLabel(instance, { main: t("主 Codex"), fallback: t("Codex 实例") });
            // 启动失败的原因必须让用户看得见：Windows 上多开曾经完全不吭声。
            const failure = multiLaunchInstanceFailure(instance);
            const detail = [
              instance.isMain ? t("主进程") : instance.instanceId,
              typeof instance.pid === "number" ? `PID ${instance.pid}` : "",
              typeof instance.debugPort === "number" ? `CDP ${instance.debugPort}` : "",
              typeof instance.helperPort === "number" ? `API ${instance.helperPort}` : "",
              failure ? `${instance.launchResult?.status === "cleanup_failed" ? t("失败") : t("启动失败")}：${failure}` : "",
            ].filter(Boolean).join(" · ");
            const tone = statusClass(status);
            // Disabled buttons do not receive hover, so the hint sits on the wrapper
            // and the button itself ignores pointer events.
            const restartTitle = mainInstance
              ? t("主 Codex 请使用顶部的「重启 Codex」。")
              : t("重启此 Codex 实例；修改供应商或 Key 后重启才会生效");
            const stopTitle = mainInstance
              ? t("主 Codex 请使用侧栏的「停止 Codex」。")
              : canStop
                ? t("结束此 Codex 实例")
                : t("实例当前未运行");
            return (
              <div className="multi-launch-instance" data-instance-id={instance.instanceId} key={instance.instanceId}>
                <CxRow
                  badges={<CxBadge tone={tone === "good" ? "ok" : tone === "bad" ? "danger" : tone === "warn" ? "warn" : "neutral"}>{statusLabel(status)}</CxBadge>}
                  detail={<span data-failed={failure ? "true" : undefined} title={detail}>{detail || t("未提供进程详情")}</span>}
                  icon={<CxStatusIcon tone={running ? "ok" : failure ? "danger" : "off"}>{instance.isMain ? <AgentIcon agent="codex" className="multi-launch-main-icon" /> : <span className="multi-launch-initial">{providerInitial(label)}</span>}</CxStatusIcon>}
                  title={label}
                >
                  <span title={mainInstance ? restartTitle : undefined}>
                    <button
                      className="cx-secondary"
                      disabled={!canRestart || pending}
                      onClick={() => void actions.restartMultiLaunchInstance(instance)}
                      style={mainInstance ? { pointerEvents: "none" } : undefined}
                      title={restartTitle}
                      type="button"
                    >
                      <RotateCcw />
                      {pending ? t("处理中") : t("重启")}
                    </button>
                  </span>
                  {/* 停止带文字，和「重启」并排；只有图标时看不出是什么。 */}
                  <span title={mainInstance ? stopTitle : undefined}>
                    <button
                      aria-label={stopTitle}
                      className="cx-secondary"
                      disabled={!canStop || pending}
                      onClick={() => void actions.stopMultiLaunchInstance(instance)}
                      style={mainInstance ? { pointerEvents: "none" } : undefined}
                      title={stopTitle}
                      type="button"
                    >
                      <PowerOff />
                      {t("停止")}
                    </button>
                  </span>
                </CxRow>
              </div>
            );
          }) : (
            <CxEmpty title={actions.multiLaunchInstancesLoading ? t("正在读取运行实例…") : t("当前没有已发现的 Codex 实例。")} />
          )}
        </CxCard>
      </CxSection>
      <CxSection
        aside={<span title={t("每个身份使用独立的配置、会话和用户数据目录，可与其他单一 Responses 或 Chat Completions 供应商同时运行。")}><Info className="multi-launch-hint-icon" /></span>}
        count={normalized.relayProfiles.length}
        title={t("按供应商启动")}
      >
        <CxCard className="relay-profile-list">
          {normalized.relayProfiles.map((profile) => {
            const aggregate = isAggregateRelayProfile(profile);
            const unsupported = !multiLaunchSupported(profile);
            const pending = actions.multiLaunchPending === profile.id;
            // 网关托管的那一项由「Agent 接入」管理，和旧版列表一样不给编辑、删除。
            const gatewayManaged = profile.id === CODEX_GATEWAY_RELAY_ID;
            const active = profile.id === normalized.activeRelayId;
            const runningCount = runningInstancesForProfile(actions.multiLaunchInstances, profile.id).length;
            const name = profile.name || t("未命名供应商");
            return (
              <div className="multi-launch-card" data-relay-profile-id={profile.id} key={profile.id}>
                <CxRow
                  badges={(
                    <>
                      {active ? <CxBadge>{t("使用中")}</CxBadge> : null}
                      {runningCount ? <CxBadge tone="ok">{tf("{0} 个实例运行中", [runningCount])}</CxBadge> : null}
                    </>
                  )}
                  detail={`${relayProfileModeLabel(profile)} · ${relayProtocolLabel(profile.protocol)} · ${relayProfileConfigBrief(profile)}`}
                  icon={<span className="multi-launch-initial">{providerInitial(profile.name)}</span>}
                  muted={unsupported}
                  onClick={gatewayManaged ? undefined : () => setDetailProfileId(profile.id)}
                  title={name}
                >
                  <button
                    className="cx-secondary"
                    disabled={unsupported || pending}
                    onClick={() => void actions.launchMultiInstance(profile)}
                    title={aggregate
                      ? t("聚合供应商暂不支持独立多开")
                      : unsupported
                        ? t("仅支持单一 Responses 或 Chat Completions 供应商，且不能配置模型路由")
                        : t("启动此供应商的独立 Codex 实例")}
                    type="button"
                  >
                    <Rocket />
                    {pending ? t("启动中") : t("启动")}
                  </button>
                  {gatewayManaged ? null : (
                    <Menu label={tf("供应商「{0}」的操作", [name])}>
                      <MenuItem onClick={() => setDetailProfileId(profile.id)}>{t("编辑")}</MenuItem>
                      <MenuItem danger disabled={normalized.relayProfiles.length <= 1} onClick={() => void deleteProfile(profile)}>{t("删除供应商")}</MenuItem>
                    </Menu>
                  )}
                </CxRow>
              </div>
            );
          })}
          {!normalized.relayProfiles.length ? <CxEmpty title={t("暂无供应商，请先添加供应商配置。")} /> : null}
        </CxCard>
        {/* #2754：新版的 API 多加在「供应商池」，多开却只读旧版供应商配置；添加入口直接说明这一点。 */}
        <CxAddRow
          detail={t("多开用「连接与供应商」里的直连供应商；供应商池（网关）里的不在这里")}
          onClick={onOpenRelay}
          title={t("导入或添加供应商")}
        />
      </CxSection>
    </div>
  );
}

function ProviderCatalogScreen({
  form,
  query,
  onQueryChange: setQuery,
  onBack,
  onSelect,
}: {
  form: BackendSettings;
  query: string;
  onQueryChange: (value: string) => void;
  onBack: () => void;
  onSelect: (profile: RelayProfile) => void;
}) {
  const categories: Array<{ id: ProviderPreset["category"]; label: string }> = [
    { id: "official", label: t("账号登录") },
    { id: "cn_official", label: t("模型厂商") },
    { id: "aggregator", label: t("聚合/中转") },
    { id: "third_party", label: t("第三方平台") },
  ];
  // 赞助商排在「聚合/中转」最前面。
  const sponsors = useSponsorPresets();
  const sponsorIds = new Set(sponsors.map((preset) => preset.id));
  const filteredPresets = [...sponsors, ...PRESETS.filter((preset) => !sponsorIds.has(preset.id))].filter((preset) => {
    const haystack = `${preset.name} ${preset.baseUrl} ${preset.model}`.toLowerCase();
    return !query.trim() || haystack.includes(query.trim().toLowerCase());
  });
  const selectPreset = (preset?: ProviderPreset) => {
    const draft = createRelayProfile(form);
    const patch = preset ? createPresetPatch(preset) : { name: t("自定义供应商") };
    onSelect(applyRelayProfilePatchToFiles(draft, patch as unknown as Partial<RelayProfile>, { allowGenerateFiles: true }));
  };
  return (
    <div className="provider-catalog-screen provider-add-simple">
      <header className="provider-catalog-header" data-tauri-drag-region>
        <Button onClick={onBack} size="icon" variant="ghost" title={t("返回供应商列表")}>
          <ArrowLeft className="h-5 w-5" />
        </Button>
        <h2>{t("添加新供应商")}</h2>
        <span>Codex</span>
      </header>
      <main className="provider-add-simple-body provider-add-reference-layout">
        <div className="provider-add-search">
          <Search className="h-4 w-4" />
          <Input
            aria-label={t("搜索供应商")}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder={t("搜索名称或网址，例如 kimi、智谱、openrouter")}
            value={query}
          />
        </div>
        <button className="provider-add-custom" onClick={() => selectPreset()} type="button">
          <span className="provider-add-icon"><Sparkles className="h-4 w-4" /></span>
          <span className="provider-add-text">
            <strong>{t("自定义配置")}</strong>
            <small>{t("手动填写请求地址和 API Key，适合中转站和自建服务")}</small>
          </span>
          <ChevronRight className="provider-add-chevron h-4 w-4" />
        </button>
        {categories.map((category) => {
          const presets = filteredPresets.filter((preset) => preset.category === category.id);
          if (!presets.length) return null;
          return (
            <section className="provider-add-section" key={category.id}>
              <h3>{category.label}<span>{presets.length}</span></h3>
              <div className="provider-add-results">
                {presets.map((preset) => (
                  <button className="provider-add-result" key={preset.id} onClick={() => selectPreset(preset)} title={preset.baseUrl} type="button">
                    <span className="provider-add-icon">{providerInitial(preset.name)}</span>
                    <span className="provider-add-text">
                      <strong>{preset.name}</strong>
                      <small>{urlHost(preset.websiteUrl || preset.baseUrl)}</small>
                    </span>
                    <span className="provider-add-protocol">{preset.protocol === "chatCompletions" ? "Chat" : "Responses"}</span>
                  </button>
                ))}
              </div>
            </section>
          );
        })}
        {!filteredPresets.length ? <div className="provider-add-empty">{tf("没有匹配「{0}」的预设，可以用上面的「自定义配置」手动填写。", [query.trim()])}</div> : null}
        <p className="provider-add-hint">{t("选择供应商后会进入编辑页；API Key 和本地配置只在保存时写入当前 Codex。")}</p>
      </main>
    </div>
  );
}

function EnvConflictNotice({
  envConflicts,
  actions,
}: {
  envConflicts: EnvConflictsResult | null;
  actions: Actions;
}) {
  const conflicts = envConflicts?.conflicts ?? [];
  if (!conflicts.length) return null;
  const names = Array.from(new Set(conflicts.map((conflict) => conflict.name))).sort();
  return (
    <div className="env-conflict-notice">
      <div className="env-conflict-icon">
        <ShieldAlert className="h-4 w-4" />
      </div>
      <div className="env-conflict-body">
        <strong>{t("检测到 OPENAI 环境变量")}</strong>
        <p>{t("这些变量可能覆盖当前供应商写入的 config.toml / auth.json；CODEX_HOME 不会被清理。")}</p>
        <div className="env-conflict-tags">
          {conflicts.map((conflict) => (
            <span key={`${conflict.source}-${conflict.name}`}>
              {conflict.name}
              <small>{envConflictSourceLabel(conflict.source)}</small>
            </span>
          ))}
        </div>
      </div>
      <div className="env-conflict-actions">
        <Button onClick={() => void actions.removeEnvConflicts(names)} size="sm">
          <Trash2 className="h-4 w-4" />
          {t("删除")}
        </Button>
        <Button onClick={() => void actions.refreshEnvConflicts(false)} size="sm" variant="secondary">
          <RefreshCw className="h-4 w-4" />
          {t("检测")}
        </Button>
      </div>
    </div>
  );
}

const SSH_REMOTE_LOOPBACK_DISMISS_KEY = "codex-plus-ssh-remote-loopback-dismissed";

// Codex++ 不往 SSH 远端写配置；远端 Codex 若沿用本机 127.0.0.1 中转地址会连到服务器自己（issue #1195）。
// 这里只提示 ssh -R 反向转发的做法。「知道了」按端口 + 远端记住，组合变了再提示。
function SshRemoteLoopbackNotice({ notice }: { notice: SshRemoteLoopbackNoticeData | null }) {
  const signature = notice
    ? `${notice.ports.join(",")}|${notice.remotes.map((remote) => remote.label).join(",")}`
    : "";
  const [dismissed, setDismissed] = useState<string>(() => {
    try { return window.localStorage.getItem(SSH_REMOTE_LOOPBACK_DISMISS_KEY) ?? ""; } catch { return ""; }
  });
  if (!notice || !notice.remotes.length || dismissed === signature) return null;
  const dismiss = () => {
    setDismissed(signature);
    try { window.localStorage.setItem(SSH_REMOTE_LOOPBACK_DISMISS_KEY, signature); } catch { /* 存不了就只影响本次 */ }
  };
  return (
    <div className="env-conflict-notice ssh-remote-loopback-notice" role="note">
      <div className="env-conflict-icon">
        <ShieldAlert className="h-4 w-4" />
      </div>
      <div className="env-conflict-body">
        <strong>{t("SSH 远端连不上本机中转地址")}</strong>
        <p>{t("当前中转地址是本机回环地址，在 SSH 服务器上它指向服务器自己。Codex++ 不会往远端写配置；如果远端 Codex 用的也是这个地址，连接时需要把端口反向转发回本机（本机 Codex++ 要保持运行），或在远端单独配置可访问的上游地址。")}</p>
        <div className="env-conflict-tags">
          {notice.loopbackEndpoints.map((endpoint) => (
            <span key={endpoint.key}>
              {endpoint.url}
              <small>{endpoint.key}</small>
            </span>
          ))}
        </div>
        {notice.remotes.map((remote) => (
          <div className="ssh-remote-loopback-remote" key={remote.label}>
            <p>{tf("连接 {0} 时带上：", [remote.label])}</p>
            <code>{remote.forwardCommand}</code>
            <p>{t("或写进 ~/.ssh/config 对应 Host 下：")}</p>
            <code>{remote.sshConfigLines.join("\n")}</code>
          </div>
        ))}
      </div>
      <div className="env-conflict-actions">
        <Button onClick={dismiss} size="sm" variant="secondary">
          {t("知道了")}
        </Button>
      </div>
    </div>
  );
}

function envConflictSourceLabel(source: string): string {
  if (source === "process") return t("当前进程");
  if (source === "user") return t("用户环境");
  return source || t("环境变量");
}

/// 「启动急救」页：把所有能让 Codex 恢复启动 / 恢复正常显示的开关放在一处——
/// 增强总开关（关掉后 Codex 不再注入，没法在 Codex 里再打开）、总开关关掉后仍生效的独立模式补丁、
/// 本地拓展总开关与「下次启动跳过拓展」、恢复默认皮肤。其余内置拓展的开关、语音输入与下一步建议的
/// 服务配置都只在 Codex 里的 Codex++ 设置页里改。旧的 dreamSkin 路由也落到这一页。
function EnhanceScreen({
  dirty,
  form,
  onFormChange: applyForm,
  settings,
  launchPending,
  skinRestartPending,
  actions,
}: {
  dirty: boolean;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  settings: SettingsResult | null;
  launchPending: boolean;
  /** 恢复默认皮肤（或网页一键换肤）已保存，重启 Codex 才完整生效。 */
  skinRestartPending: boolean;
  actions: Actions;
}) {
  const { CxBadge, CxBanner, CxCard, CxRow, CxSection, CxStatusIcon } = AgentCx;
  const standaloneGroup = managerStandaloneGroup();
  const standaloneCards = useMemo(() => managerBuiltinCards(managerStandaloneGroup().id, enhancementPlatform), []);
  // 说明里写了要重启 Codex 的开关：改动时记下改之前的值，页面顶部汇总成一条「重启 Codex 生效」。
  const restartItems = useMemo(() => standaloneCards
    .flatMap((card) => (card.item ? [{ key: card.item.key, title: card.item.title, description: card.item.description }] : []))
    .filter((item) => RESTART_HINT.test(t(item.description))), [standaloneCards]);
  const restartOriginal = useRef(new Map<string, unknown>());
  const [restartBusy, setRestartBusy] = useState(false);
  const onFormChange = (next: BackendSettings) => {
    for (const item of restartItems) {
      const before = builtinSettingValue(form, item.key);
      if (before !== builtinSettingValue(next, item.key) && !restartOriginal.current.has(item.key)) restartOriginal.current.set(item.key, before);
    }
    applyForm(next);
  };
  const restartPending = restartItems.filter((item) => restartOriginal.current.has(item.key)
    && restartOriginal.current.get(item.key) !== builtinSettingValue(form, item.key));
  // 皮肤的待重启状态在 App 里（重启后清掉）；「稍后」只在本页收起这一项。
  const [skinDismissed, setSkinDismissed] = useState(false);
  const skinPending = skinRestartPending && !skinDismissed;
  const restartLabels = [
    ...restartPending.map((item) => t(item.title).replace(EXPERIMENTAL_TITLE, "")),
    ...(skinPending ? [t("恢复默认皮肤")] : []),
  ];
  const [, setRestartDismissed] = useState(0);
  const clearRestartPending = () => { restartOriginal.current.clear(); setSkinDismissed(true); setRestartDismissed((value) => value + 1); };
  const autoSave = useEnhanceAutoSave(dirty, form, actions.autoSaveSettings);
  const masterEnabled = form.enhancementsEnabled;
  const masterItem = enhancementItem("enhancementsEnabled");
  const inventory = settings?.user_scripts;
  const localEnabled = inventory?.enabled !== false;
  const skipNextStart = Boolean(inventory?.skip_next_start);
  const skinActive = masterEnabled && form.codexAppDreamSkinEnabled && !form.codexAppDreamSkinPaused;
  const skinState = !masterEnabled
    ? t("增强总开关已关闭")
    : !form.codexAppDreamSkinEnabled
      ? t("皮肤未启用")
      : form.codexAppDreamSkinPaused
        ? t("已恢复原始外观")
        : t("皮肤已启用");
  const themeName = form.codexAppDreamSkinThemeConfig?.name?.trim() || t("未记录");
  return (
    <div className="settings-page enhance-page sectioned-page cx-enhance-page">
      <div className="cx-enhance-bar" id="enhance-page-controls">
        <span className="cx-toolbar-spacer" />
        <span className="enhance-save-actions cx-enhance-save">
          <AutoSaveStatus state={autoSave} />
        </span>
        {/* 总开关：一个紧凑开关，说明放悬停。 */}
        <label className="feature-toggle enhance-master-switch cx-enhance-master-switch" title={t(masterItem.description)}>
          <input
            checked={form.enhancementsEnabled}
            onChange={(event) => onFormChange({ ...form, enhancementsEnabled: event.currentTarget.checked })}
            type="checkbox"
          />
          <span><strong>{t(masterItem.title)}</strong></span>
          <ToggleVisual />
        </label>
      </div>
      <div className="sectioned-body cx-enhance-body">
      {restartLabels.length ? (
        <CxBanner
          detail={restartLabels.join("、")}
          icon={<RotateCcw />}
          title={tf("{0} 项改动重启 Codex 后生效", [restartLabels.length])}
          tone="warn"
        >
          <button className="cx-secondary" onClick={clearRestartPending} type="button">{t("稍后")}</button>
          <button
            className="cx-primary"
            disabled={restartBusy || launchPending}
            onClick={() => void (async () => {
              setRestartBusy(true);
              try {
                if (await actions.restart()) clearRestartPending();
              } finally {
                setRestartBusy(false);
              }
            })()}
            type="button"
          >
            {restartBusy ? <Loader2 className="cx-dash-spin" /> : <RotateCcw />}
            {t("重启 Codex")}
          </button>
        </CxBanner>
      ) : null}
      {masterEnabled ? null : (
        <CxBanner icon={<PowerOff />} title={t("内置拓展已全部暂停")} detail={t("独立模式里的补丁仍然生效；服务配置会保留。")} tone="warn">
          <button className="cx-primary" onClick={() => onFormChange({ ...form, enhancementsEnabled: true })} type="button">{t("全部启用")}</button>
        </CxBanner>
      )}
      <CxCard>
        <div className="feature-toggle cx-enhance-moved" data-enhance-moved="codex">
          <span>
            <strong>{t("Codex 打不开或显示异常时在这里处理")}</strong>
            <small>{t("这里只放能让 Codex 恢复启动的开关：增强总开关、独立模式补丁、本地拓展和皮肤。各项内置拓展的开关、语音输入与下一步建议的服务配置在 Codex 左侧栏的「Codex++」设置页里。")}</small>
          </span>
        </div>
      </CxCard>
      <FeatureGroup id="enhance-group-standalone" title={t(standaloneGroup.title)} detail={t(standaloneGroup.description ?? "")}>
        {standaloneCards.map((card) => (card.item ? (
          <EnhancementSettingControl key={card.id} item={card.item} form={form} onFormChange={onFormChange} masterEnabled={masterEnabled} />
        ) : null))}
      </FeatureGroup>
      {/* 本地拓展的整体开关立即生效（不走设置表单的自动保存），和「万物皆拓展」页的三点菜单是同一份状态。 */}
      <CxSection className="cx-feature-group" id="enhance-group-extensions" title={t("本地拓展")}>
        <CxCard className="cx-feature-card">
          <FeatureToggle
            checked={localEnabled}
            detail={t("关掉后 Codex 不再注入任何本地拓展；各拓展的启停设置不会改变")}
            disabled={!inventory}
            onChange={(value) => void actions.setUserScriptsEnabled(value, "codex")}
            title={t("启用本地拓展")}
          />
          <FeatureToggle
            checked={skipNextStart}
            detail={t("拓展导致黑屏或卡死时，下一次启动先跳过拓展；各拓展的启停设置不会改变")}
            disabled={!inventory}
            onChange={(value) => void actions.setUserScriptsSkipNextStart(value)}
            title={t("下次启动跳过拓展")}
          />
        </CxCard>
      </CxSection>
      {/* 皮肤的界面已移到 Codex 里（Codex++ → 设置 → 皮肤，见 96-dream-skin-page.js），这里只留恢复默认皮肤。 */}
      <CxSection className="enhance-skin-section" id="enhance-group-skin" title={t("皮肤")}>
        <CxCard>
          <CxRow
            badges={<CxBadge tone={skinActive ? "ok" : "neutral"}>{skinState}</CxBadge>}
            detail={<span role="status">{tf("当前主题：{0}", [themeName])}</span>}
            icon={<CxStatusIcon tone={skinActive ? "ok" : "off"}><Palette /></CxStatusIcon>}
            title={t("当前皮肤")}
            tooltip={t("皮肤已移到 Codex 里：打开 Codex 左侧的 Codex++ → 设置 → 皮肤，可以换主题、装社区和市场主题、编辑主题。")}
          />
          <CxRow
            detail={t("皮肤让 Codex 显示异常或打不开时使用：关掉皮肤并清掉当前窗口里的皮肤，主题库不受影响；重启 Codex 后完整生效。")}
            icon={<CxStatusIcon tone="warn"><RotateCcw /></CxStatusIcon>}
            title={t("恢复默认皮肤")}
          >
            <button className="cx-secondary" onClick={() => { setSkinDismissed(false); void actions.restoreDreamSkin(); }} type="button">
              <RotateCcw />
              {t("恢复默认皮肤")}
            </button>
          </CxRow>
        </CxCard>
        <p className="dream-skin-about">
          {t("项目来源：Fei-Away/Codex-Dream-Skin · 原作者 Fei-Away · MIT License · 第三方图片需自行确认授权")}
        </p>
      </CxSection>
      </div>
    </div>
  );
}

type BooleanSettingKey = { [K in keyof BackendSettings]-?: BackendSettings[K] extends boolean ? K : never }[keyof BackendSettings];

/// 按设置项清单渲染一个增强设置：bool 是开关卡，select 是带下拉的卡片。
/// 受总开关控制的项在增强关闭时禁用；独立模式补丁（masterGated: false）始终可改，
/// 并按总开关状态切换说明（descriptionWhenEnhancementsOn）。
function EnhancementSettingControl({
  item,
  form,
  onFormChange,
  masterEnabled,
  onToggle,
}: {
  item: EnhancementSettingItem;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  masterEnabled: boolean;
  /** 需要在写入前插入确认等步骤时覆盖默认写入。 */
  onToggle?: (value: boolean) => void;
}) {
  const disabled = isEnhancementItemMasterGated(item) && !masterEnabled;
  const detail = t(masterEnabled && item.descriptionWhenEnhancementsOn ? item.descriptionWhenEnhancementsOn : item.description);
  if (item.control === "select") {
    const key = item.key as keyof BackendSettings;
    return (
      <div className={`feature-toggle ${disabled ? "disabled" : ""}`}>
        <span>
          <strong>{t(item.title)}</strong>
          <small title={detail}>{detail}</small>
        </span>
        <AppSelect<string>
          ariaLabel={t(item.title)}
          value={String(form[key] ?? "")}
          disabled={disabled}
          onChange={(value) => onFormChange({ ...form, [key]: value })}
          options={(item.options ?? []).map((option) => ({ value: option.value, label: t(option.label) }))}
        />
      </div>
    );
  }
  const key = item.key as BooleanSettingKey;
  return (
    <FeatureToggle
      title={t(item.title)}
      detail={detail}
      checked={form[key]}
      disabled={disabled}
      onChange={onToggle ?? ((value) => onFormChange({ ...form, [key]: value }))}
    />
  );
}

function UserScriptsScreen({ host, focus = null, onHostChange, settings, market, onOpenRescue, onOpenPluginMarket, onOpenClaude, actions }: { host: ExtensionHost; focus?: AgentFocus; onHostChange: (host: ExtensionHost) => void; settings: SettingsResult | null; market: ScriptMarketResult | null; onOpenRescue: () => void; onOpenPluginMarket?: () => void; onOpenClaude: () => void; actions: Actions }) {
  const codexHost = host === "codex";
  const [reloading, setReloading] = useState(false);
  const reload = async () => {
    if (reloading) return;
    setReloading(true);
    try {
      await actions.reloadUserScripts();
    } finally {
      setReloading(false);
    }
  };
  const inventory = settings?.user_scripts;
  const scripts = extensionsForHost(inventory?.scripts ?? [], host);
  const marketScripts = market?.market.scripts ?? [];
  const marketPackages = market?.market.packages ?? [];
  // 参照 Cursor 的 Customize：一个搜索框管所有筛选（精选 / 社区 / 本地 / 我发布的 / dsh 插件），
  // 一排筛选胶囊切换来源，列表按「已安装」「可安装」（本地是「已启用」「未启用」）分节。
  const [search, setSearch] = useState("");
  const [communitySort, setCommunitySort] = useState<CommunitySort>("stars");
  const [codexPane, setPane] = useState<"market" | "community" | "local" | "mine" | "dsh">("market");
  const pane = codexHost ? codexPane : "local";
  // 直接打开本页（命令面板、深链）时市场可能还没读过，自动读一次，不再要用户点「刷新」。
  useEffect(() => {
    if (codexHost && !market) void actions.refreshScriptMarket();
  }, [codexHost]);
  const query = search.trim().toLocaleLowerCase();
  const filteredMarketScripts = useMemo(() => {
    if (!query) return marketScripts;
    return marketScripts.filter((script) => {
      const haystack = [
        script.name,
        script.author,
        script.description,
        script.version,
        script.homepage,
        ...script.tags,
      ]
        .filter(Boolean)
        .join(" ")
        .toLocaleLowerCase();
      return haystack.includes(query);
    });
  }, [query, marketScripts]);
  // 用户没手动选排序时跟随数据：清单里每个脚本都标了更新时间就默认「最近更新」，否则保持清单原顺序。
  const [marketSortChoice, setMarketSortChoice] = useState<ScriptMarketSort | null>(null);
  const marketSort = marketSortChoice ?? defaultMarketSort(marketScripts);
  const sortedMarketScripts = useMemo(
    () => sortMarketScripts(filteredMarketScripts, marketSort),
    [filteredMarketScripts, marketSort],
  );
  const hasMarketUpdateTimes = marketScripts.some((script) => parseMarketTime(script.updatedAt) !== null);
  // market_id -> 本地脚本 key。只用 user 来源的脚本：内置脚本走的是 builtin: 前缀，
  // 后端 delete_user_script 只接受 user:，映射过去会直接报错。
  const installedKeyByMarketId = useMemo(() => {
    const map = new Map<string, string>();
    for (const script of scripts) {
      if (script.source !== "user" || !script.market_id) continue;
      map.set(script.market_id, script.key);
    }
    return map;
  }, [scripts]);
  const localEnabled = codexHost ? inventory?.enabled !== false : inventory?.claude_enabled === true;
  const skipNextStart = Boolean(inventory?.skip_next_start);
  const localScripts = (query
    ? scripts.filter((script) => [script.name, script.key, script.package?.id, script.package?.description].filter(Boolean).join(" ").toLocaleLowerCase().includes(query))
    : scripts);
  const enabledScripts = localScripts.filter((script) => extensionEnabledForHost(script, host));
  const idleScripts = localScripts.filter((script) => !extensionEnabledForHost(script, host));
  const scriptRow = (script: (typeof scripts)[number]) => <ScriptRow key={`${host}:${script.key}`} host={host} script={script} globalEnabled={localEnabled} actions={actions} />;
  const marketRow = (script: ScriptMarketItem) => (
    <MarketScriptCard actions={actions} installedKey={installedKeyByMarketId.get(script.id)} key={script.id} script={script} />
  );
  const switchLabel = codexHost ? t("启用本地拓展") : t("启用 Claude 拓展");
  const panes = [
    { id: "market" as const, label: t("精选"), count: marketScripts.length + marketPackages.length },
    { id: "community" as const, label: t("社区") },
    { id: "local" as const, label: t("本地"), count: scripts.length },
    { id: "mine" as const, label: t("我发布的") },
    // dsh 插件不是又一个拓展来源，而是把 dsh 插件导入成 Codex 的 MCP / Skill。
    { id: "dsh" as const, label: t("导入 dsh 插件") },
  ];
  const sortControl = pane === "market" ? (
    <AppSelect
      ariaLabel={t("拓展市场排序")}
      className="script-market-sort"
      onChange={(value) => setMarketSortChoice(value)}
      options={[
        { value: "default", label: t("默认顺序") },
        {
          value: "updated",
          label: t("最近更新"),
          title: hasMarketUpdateTimes ? undefined : t("市场清单暂未提供更新时间"),
        },
        { value: "name", label: t("名称") },
      ]}
      size="sm"
      title={t("拓展市场排序")}
      value={marketSort}
    />
  ) : pane === "community" ? (
    <AppSelect
      ariaLabel={t("社区拓展排序")}
      className="script-market-sort"
      onChange={(value) => setCommunitySort(value)}
      options={[
        { value: "stars", label: t("星数") },
        { value: "updated", label: t("最近更新") },
        { value: "name", label: t("名称") },
      ]}
      size="sm"
      title={t("社区拓展排序")}
      value={communitySort}
    />
  ) : null;
  return (
    <div className="tp-page cx-page user-scripts-page">
      <TopbarActions>
        {codexHost ? <button className="cx-secondary" disabled={reloading} onClick={() => void reload()} title={t("应用本地拓展及开关；旧版脚本可能需要刷新 Codex 页面")} type="button">
          <RefreshCw className={reloading ? "animate-spin" : ""} />
          {t("热重载拓展")}
        </button> : <button className="cx-secondary" onClick={onOpenClaude} type="button">
          <AppWindow />
          {t("Claude 补丁版")}
        </button>}
        {/* 不常用的整体开关收进三点菜单；关着时页面顶部会有一条提示带「启用」。 */}
        <Menu label={t("更多操作")}>
          <MenuItem onClick={() => void (async () => { await actions.refreshScriptMarket(false, false, true); await actions.refreshCurrent(); })()}>{t("刷新列表")}</MenuItem>
          <MenuItem checked={localEnabled} disabled={!inventory} onClick={() => void actions.setUserScriptsEnabled(!localEnabled, host)}>{switchLabel}</MenuItem>
          {codexHost ? <MenuItem checked={skipNextStart} disabled={!inventory} onClick={() => void actions.setUserScriptsSkipNextStart(!skipNextStart)}>{t("下次启动跳过拓展")}</MenuItem> : null}
          {/* 这两个整体开关在「启动急救」页里也有一份（Codex 打不开时从那里一起处理）。 */}
          {codexHost ? <MenuItem onClick={onOpenRescue}>{t("打开启动急救")}</MenuItem> : null}
        </Menu>
      </TopbarActions>
      <ExtensionsHero host={host} focus={focus} onHostChange={onHostChange} />
      {codexHost && onOpenPluginMarket ? (
        <p className="cx-aggregate-note ext-crosslink">
          <span>{t("这里是 Codex++ 拓展；Codex 官方目录里的插件在「Codex 插件」页。")}</span>
          <button className="cx-link" onClick={onOpenPluginMarket} type="button">{t("打开 Codex 插件")}</button>
        </p>
      ) : null}
      <CxToolbar onQuery={setSearch} placeholder={codexHost ? t("搜索拓展：名称、作者、描述或标签") : t("搜索适配 Claude 的拓展")} query={search}>
        {codexHost ? <ExtensionCreatorCard /> : null}
      </CxToolbar>
      {/* 顶部最多一条状态提示：整体关闭优先，其次是「下次启动跳过」。 */}
      {inventory && !localEnabled ? (
        <CxBanner
          detail={codexHost ? t("拓展开着，但「启用本地拓展」整体开关关着，暂不注入") : t("重启 Claude 补丁版后应用")}
          icon={<PowerOff />}
          title={codexHost ? t("本地拓展已整体关闭") : t("Claude 拓展已关闭")}
          tone="warn"
        >
          <button className="cx-secondary" onClick={() => void actions.setUserScriptsEnabled(true, host)} type="button">{t("启用")}</button>
        </CxBanner>
      ) : codexHost && skipNextStart ? (
        <CxBanner detail={t("拓展导致黑屏或卡死时，下一次启动先跳过拓展；各拓展的启停设置不会改变")} icon={<PowerOff />} title={t("下次启动会跳过拓展")} tone="warn">
          <button className="cx-secondary" onClick={() => void actions.setUserScriptsSkipNextStart(false)} type="button">{t("取消跳过")}</button>
        </CxBanner>
      ) : null}
      {codexHost ? <CxFilters items={panes} label={t("拓展")} onChange={setPane} trailing={sortControl} value={pane} /> : null}
      {pane === "market" ? (
        <CuratedPackagesSection
          blockedInstalled={market?.market.blockedInstalled ?? []}
          emptyText={market?.status === "failed" ? market.message : t("正在读取拓展市场…")}
          hasScripts={marketScripts.length > 0}
          installedScripts={sortedMarketScripts.filter((script) => script.installed).map(marketRow)}
          onInstalled={async () => { await actions.reloadUserScripts(); await actions.refreshScriptMarket(false, true); }}
          onOpenUrl={(url) => void actions.openExternalUrl(url)}
          packages={marketPackages}
          search={search}
        >
          {sortedMarketScripts.filter((script) => !script.installed).map(marketRow)}
        </CuratedPackagesSection>
      ) : pane === "community" ? (
        <CommunityExtensionsPanel onCurated={actions.refreshScriptMarket} search={search} sort={communitySort} onInstalled={actions.reloadUserScripts} onOpenUrl={(url) => void actions.openExternalUrl(url)} />
      ) : pane === "mine" ? (
        <MyPublishedPanel onOpenUrl={(url) => void actions.openExternalUrl(url)} search={search} />
      ) : pane === "dsh" ? (
        <DshPluginsPanel search={search} />
      ) : (
        <>
          {enabledScripts.length ? (
            <CxSection count={enabledScripts.length} title={t("已启用")}>
              <CxCard>{enabledScripts.map(scriptRow)}</CxCard>
            </CxSection>
          ) : null}
          {idleScripts.length ? (
            <CxSection count={idleScripts.length} title={t("未启用")}>
              <CxCard>{idleScripts.map(scriptRow)}</CxCard>
            </CxSection>
          ) : null}
          {!localScripts.length ? (
            <CxCard>
              <CxEmpty title={query ? t("没有匹配的拓展。") : codexHost ? t("还没有本地拓展。") : t("还没有适配 Claude 的本地拓展。")}>
                {query ? <button className="cx-secondary" onClick={() => setSearch("")} type="button">{t("清除搜索")}</button>
                  : codexHost ? <button className="cx-secondary" onClick={() => setPane("market")} type="button">{t("浏览精选")}</button> : null}
              </CxEmpty>
            </CxCard>
          ) : null}
          {codexHost && inventory?.builtins?.length ? <BuiltinExtensionsSummary builtins={inventory.builtins} /> : null}
        </>
      )}
    </div>
  );
}

function SessionRepairPanel({ dirty, form, providerSyncProgress, sessionIndexRepairActive, sessionIndexRepairReport, sessionIndexRepairReportError, providerSyncTargets, selectedProviderSyncTarget, onFormChange, actions }: {
  dirty: boolean;
  form: BackendSettings;
  providerSyncProgress: ProviderSyncProgress;
  sessionIndexRepairActive: boolean;
  sessionIndexRepairReport: SessionIndexRepairReport | null;
  sessionIndexRepairReportError: string | null;
  providerSyncTargets: ProviderSyncTargetsResult | null;
  selectedProviderSyncTarget: string;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const providerTargets = providerSyncTargets?.targets ?? [];
  const selectedProviderTarget = providerTargets.find((target) => target.id === selectedProviderSyncTarget);
  const canRepairProviderSessions = Boolean(selectedProviderTarget && isProviderSyncTargetSelectable(selectedProviderTarget));
  const maintenanceBusy = providerSyncProgress.active || sessionIndexRepairActive;
  const maintenanceHasError = Boolean(
    sessionIndexRepairReportError
    || providerSyncProgress.result?.status === "failed"
    || providerSyncProgress.result?.syncStatus === "partial_failure"
    || providerSyncProgress.result?.recoveryErrors?.length
    || sessionIndexRepairReport?.abortedReason
    || sessionIndexRepairReport?.warnings?.length,
  );
  const maintenanceNeedsAttention = maintenanceBusy || maintenanceHasError;
  const [maintenanceExpanded, setMaintenanceExpanded] = useState(maintenanceNeedsAttention);

  useEffect(() => {
    if (maintenanceNeedsAttention) setMaintenanceExpanded(true);
  }, [maintenanceNeedsAttention, providerSyncProgress.active, sessionIndexRepairActive, sessionIndexRepairReportError]);

  return <details
    className="session-maintenance"
    open={maintenanceExpanded}
    onToggle={(event) => setMaintenanceExpanded(event.currentTarget.open)}
  >
    <summary className="session-maintenance-summary">
      <Wrench aria-hidden="true" className="h-4 w-4" />
      <strong>{t("修复与维护")}</strong>
      <span className="session-maintenance-auto-state" data-enabled={form.sessionIndexRepairEnabled ?? true}>
        {(form.sessionIndexRepairEnabled ?? true) ? t("自动修复已开启") : t("自动修复已关闭")}
      </span>
      <span className="session-maintenance-status" data-error={maintenanceHasError} role="status" aria-live="polite">
        {providerSyncProgress.active
          ? <>{t("正在修复历史会话")} {formatProgressPercent(providerSyncProgress.percent)}%</>
          : sessionIndexRepairActive
            ? t("正在检查索引…")
            : maintenanceHasError ? t("需要处理") : null}
      </span>
      <ChevronDown aria-hidden="true" className="session-maintenance-chevron h-4 w-4" />
    </summary>
    <div className="session-maintenance-content">
          {/* 和设置页同一种行：左边标题 + 一行说明，右边控件；开关即改即存（SettingsAutoSave）。 */}
          <div className="session-repair-rows">
            <SettingRow title={t(sessionIndexRepairItem.title)} description={t(sessionIndexRepairItem.description)}>
              <label className="relay-bare-switch">
                <input
                  aria-label={t(sessionIndexRepairItem.title)}
                  checked={form.sessionIndexRepairEnabled ?? true}
                  onChange={(event) => onFormChange({ ...form, sessionIndexRepairEnabled: event.currentTarget.checked })}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
            </SettingRow>
            <SettingRow title={t(providerSyncItem.title)} description={t(providerSyncItem.description)}>
              <label className="relay-bare-switch">
                <input
                  aria-label={t(providerSyncItem.title)}
                  checked={form.providerSyncEnabled}
                  onChange={(event) => onFormChange({ ...form, providerSyncEnabled: event.currentTarget.checked })}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
            </SettingRow>
            <SettingRow title={t("切换模型后旧会话跟随新默认")} description={t("切换供应商或默认模型后，下次启动前把仍停在旧默认模型的会话改为新默认；手动选过模型的会话不变。保存设置后生效。")}>
              <label className="relay-bare-switch">
                <input
                  aria-label={t("切换模型后旧会话跟随新默认")}
                  checked={Boolean(form.threadModelFollowSwitchEnabled)}
                  onChange={(event) => onFormChange({ ...form, threadModelFollowSwitchEnabled: event.currentTarget.checked })}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
            </SettingRow>
            <SettingRow title={t("非官方登录时保留本地项目")} description={t("登录过 ChatGPT 后改用 API Key、网关或聚合供应商，Codex 每次启动都会清空项目列表。开启后由 Codex++ 启动前把最近使用账号的项目带到当前模式，切回官方登录时归还该账号；写入前自动备份。重新启动 Codex 后生效。")}>
              <label className="relay-bare-switch">
                <input
                  aria-label={t("非官方登录时保留本地项目")}
                  checked={Boolean(form.keepProjectsWithoutAccount)}
                  onChange={(event) => onFormChange({ ...form, keepProjectsWithoutAccount: event.currentTarget.checked })}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
            </SettingRow>
            <SettingRow title={t("同步目标")}>
              <AppSelect
                disabled={providerSyncProgress.active || !providerTargets.length}
                value={selectedProviderSyncTarget}
                onChange={(value) => actions.setProviderSyncTarget(value)}
                options={
                  providerTargets.length
                    ? [
                        ...(!selectedProviderSyncTarget
                          ? [{ value: "", label: t("当前配置 provider"), disabled: true }]
                          : []),
                        ...providerTargets.map((target) => ({
                          value: target.id,
                          label: `${target.id}${t("（")}${providerSyncTargetLabel(target)}${t("）")}`,
                          disabled: !isProviderSyncTargetSelectable(target),
                          title: target.unavailableReason ?? undefined,
                        })),
                      ]
                    : [{ value: "", label: t("当前配置 provider"), disabled: true }]
                }
              />
              <Button
                disabled={providerSyncProgress.active || sessionIndexRepairActive || !canRepairProviderSessions}
                onClick={() => void actions.syncProvidersNow()}
                variant="outline"
              >
                <Wrench className="h-4 w-4" />
                {providerSyncProgress.active ? t("正在修复…") : t("修复历史会话")}
              </Button>
            </SettingRow>
            <SettingRow title={t("会话索引")} description={t("仅恢复有本地原文且可确认位置的消息；已打开的会话可能需要重新打开才能显示。")}>
              <Button
                disabled={sessionIndexRepairActive || providerSyncProgress.active}
                onClick={() => void actions.repairSessionIndex()}
                variant="outline"
              >
                <Wrench className="h-4 w-4" />
                {sessionIndexRepairActive ? t("正在检查索引…") : t("修复会话索引")}
              </Button>
            </SettingRow>
            <SettingsAutoSave dirty={dirty} form={form} actions={actions} />
          </div>

          {providerSyncProgress.active || providerSyncProgress.percent > 0 ? (
            <div className="provider-sync-progress session-repair-progress" data-active={providerSyncProgress.active}>
              <div className="provider-sync-progress-head">
                <strong>{providerSyncProgress.active ? t("正在修复历史会话") : t("历史会话修复进度")}</strong>
                <span>{formatProgressPercent(providerSyncProgress.percent)}%</span>
              </div>
              <div
                aria-valuemax={100}
                aria-valuemin={0}
                aria-valuenow={providerSyncProgress.percent}
                className="provider-sync-progress-bar"
                role="progressbar"
              >
                <div className="provider-sync-progress-fill" style={{ width: `${providerSyncProgress.percent}%` }} />
              </div>
              <small>{providerSyncProgress.message}</small>
            </div>
          ) : null}

          {sessionIndexRepairActive ? (
            <p role="status">{t("正在检查全部会话并恢复高可信缺失消息，首次检查可能需要较长时间…")}</p>
          ) : null}
          {sessionIndexRepairReportError ? (
            <p role="alert" className="break-all">{sessionIndexRepairReportError}</p>
          ) : null}
          {sessionIndexRepairReport ? (
            <div className="provider-sync-progress session-repair-progress" aria-live="polite">
              <strong title={`${t("自动检查需要从此界面启动 Codex，并保持 Codex++ 运行，且自动修复开关已开启并保存；每次检查完成后间隔 30 分钟复查。此页面每 15 秒刷新报告，不会单独启动修复；再次检查不保证恢复。")}\n${t("短暂等待最长 30 分钟；原文和记录文件都已超过 24 小时未更新的项目直接转入需核查。缺少对应轮次或结束状态，当前证据不足以安全补回；后续检查仍会核验。")}`}>{t("最近一次会话索引修复报告")}</strong>
              <p>{t("最后检查：")}{sessionIndexRepairReport.checkedAtMs ? formatTime(sessionIndexRepairReport.checkedAtMs) : t("旧版报告未记录时间")}</p>
              <p>
                {t("读取文件")} {sessionIndexRepairReport.scannedFiles} · {t("复用缓存")} {sessionIndexRepairReport.cachedFiles} · {t("耗时")} {(sessionIndexRepairReport.elapsedMs / 1000).toFixed(1)} s
              </p>
              <p>
                {t("恢复消息")} {sessionIndexRepairReport.repairedItems} · {t("已存在")} {sessionIndexRepairReport.alreadyPresent} · {t("短暂等待")} {sessionIndexRepairReport.deferredItems ?? 0} · {t("需核查")} {sessionIndexRepairReport.skippedItems}
              </p>
              {sessionIndexRepairReport.frontendRecovery ? (
                <p role="status" className="break-all">
                  <strong>{t("历史轮次补载：")}</strong>
                  {t(historyRecoveryStatusLabel(sessionIndexRepairReport.frontendRecovery))}
                  {" · "}{sessionIndexRepairReport.frontendRecovery.status}
                  {" · "}{t("消息页")} {sessionIndexRepairReport.frontendRecovery.loadedPages}
                  {" · "}{t("元数据页")} {sessionIndexRepairReport.frontendRecovery.metadataPages}
                  {sessionIndexRepairReport.frontendRecovery.reason ? (
                    <small> · {t("原因：")}{sessionIndexRepairReport.frontendRecovery.reason}</small>
                  ) : null}
                </p>
              ) : null}
              {sessionIndexRepairReport.backupPath ? <p className="break-all">{t("修复前备份：")}{sessionIndexRepairReport.backupPath}</p> : null}
              {sessionIndexRepairReport.abortedReason ? (
                <p role="alert" className="break-all"><strong>{t("修复已中止：")}</strong>{sessionIndexRepairReport.abortedReason}</p>
              ) : null}
              {sessionIndexRepairReport.warnings?.map((warning, index) => (
                <p key={index} role="alert" className="break-all"><strong>{t("修复警告：")}</strong>{warning}</p>
              ))}
              {sessionIndexRepairReport.pendingDetails?.length ? (
                <details>
                  <summary>{t("等待与持续无法恢复详情")} ({sessionIndexRepairReport.pendingDetails.length})</summary>
                  <ul>
                    {sessionIndexRepairReport.pendingDetails.map((item, index) => (
                      <li key={`${item.threadId}-${item.turnId}-${index}`} className="break-all my-3">
                        <strong>{item.state === "waiting" ? t("短暂等待") : t("持续无法恢复")}</strong>
                        <p>{t("任务 ID：")}{item.threadId ?? "—"} · {t("轮次 ID：")}{item.turnId ?? "—"}</p>
                        <p>{t("原因：")}{item.reason}</p>
                        <small>{t("首次发现：")}{formatTime(item.firstSeenAtMs)} · {t("最后检查：")}{formatTime(item.lastCheckedAtMs)} · {t("检查次数：")}{item.checks}</small>
                      </li>
                    ))}
                  </ul>
                </details>
              ) : null}
              {sessionIndexRepairReport.issues.length ? (
                <details>
                  <summary>{t("查看检查详情")} ({sessionIndexRepairReport.issues.length})</summary>
                  <ul>{sessionIndexRepairReport.issues.map((issue, index) => <li key={index} className="break-all">{issue}</li>)}</ul>
                </details>
              ) : null}
              {sessionIndexRepairReport.issuesTruncated ? (
                <p><small>{tf("另有 {0} 条检查详情因报告上限未显示。", [sessionIndexRepairReport.issuesTruncated])}</small></p>
              ) : null}
            </div>
          ) : null}

    {providerSyncProgress.result?.recoveryJournalPath ? <div>
      {!providerSyncProgress.active && providerSyncProgress.percent === 0 ? <p role="alert">{providerSyncProgress.message}</p> : null}
      <p role="alert" className="break-all">{t("恢复记录：")}{providerSyncProgress.result.recoveryJournalPath}</p>
      {providerSyncProgress.result.syncStatus === "partial_failure" && providerSyncProgress.result.backupDir && providerSyncProgress.result.recoveryAvailable !== false ? <Button
        disabled={providerSyncProgress.active || sessionIndexRepairActive}
        onClick={() => void actions.recoverProviderSync()}>{t("恢复未完成修复")}</Button> : null}
    </div> : null}
    {providerSyncProgress.result?.recoveryErrors?.map((error, index) => <p role="alert" key={index}>{error}</p>)}
    {providerSyncProgress.result?.syncStatus === "partial_failure" && Boolean(providerSyncProgress.result.databaseOutcomes?.length || (providerSyncProgress.result.changedSessionFiles ?? 0) > 0 || (providerSyncProgress.result.sqliteRowsUpdated ?? 0) > 0) ? <p role="status">
      {t("仍有改动：")}{providerSyncProgress.result.changedSessionFiles ?? 0} {t("个会话文件，")}
      {providerSyncProgress.result.sqliteRowsUpdated ?? 0} {t("行索引。请查看恢复记录处理剩余冲突。")}
    </p> : null}
    </div>
  </details>;
}

/// 首页：整个程序的落脚页。上面是各 Agent 与网关的状态，中间是精选推荐和其他赞助商，最下面是常用入口。
///
/// 推荐：置顶位全部做大卡片，排成一行均分宽度；推荐池里的赞助商全部列出，两列小卡片，紧跟在精选下面。
/// 置顶位和推荐池里同一家常有两个 id、同一个落地页，按 adIdentity 去重。只展示赞助商，普通推荐不显示。
function HomeScreen({
  ads,
  actions,
  tools,
  focus,
  codexRunning,
  launchPending,
  onLaunchCodex,
  onNavigate,
  onOpenAgent,
  autoTakeover,
  autoTakeoverOn,
  watcherEnabled,
}: {
  ads: AdsResult | null;
  actions: Actions;
  tools: ToolEntry[];
  focus: AgentFocus;
  codexRunning: boolean;
  launchPending: boolean;
  onLaunchCodex: () => void;
  onNavigate: (route: Route) => void;
  onOpenAgent: (agent: ToolId, destination?: Route) => void;
  autoTakeover: AutoTakeoverResult | null;
  /** 设置还没读到时为 null，开关先禁用。 */
  autoTakeoverOn: boolean | null;
  watcherEnabled: boolean | null;
}) {
  const gateway = useGatewayPayload(10_000);
  const usage = useTodayUsage();
  const enhanceOnly = providersHidden(focus);
  const connected = new Set((gateway?.agents ?? []).filter((agent) => agent.enabled).map((agent) => agent.agent as string));
  // 和侧栏一样：没有 Agent 接入网关时不显示网关那一行，也不催其他 Agent 接入。
  const gatewayInUse = connected.size > 0;

  const topAds = featuredAds(ads);
  const topIdentities = new Set(topAds.map(adIdentity));
  const pool = (ads?.ads ?? []).filter((ad) => !isExpiredAd(ad) && !topIdentities.has(adIdentity(ad)));
  // 大横幅只放置顶位（top_ad / topAds），没有置顶位就不显示；推荐池里的赞助商只进下面的列表。
  const heroes = topAds;
  const rest = pool.filter((ad) => ad.type === "sponsor");
  const open = (ad: AdItem) => void actions.openExternalUrl(ad.url);

  // agent：Codex 自己的页面，先切到 Codex 再打开。
  const shortcuts: Array<{ route: Route; label: string; detail: string; icon: LucideIcon; agent?: ToolId }> = [
    { route: "userScripts", label: t("万物皆拓展"), detail: t("给 Codex、Claude 加功能的脚本"), icon: Blocks },
    { route: "context", label: t("MCP 与 Skills"), detail: t("一处管理所有 Agent 的 MCP 和 Skill"), icon: Network },
    { route: "sessions", label: t("会话"), detail: t("找回、继续各 Agent 的本地会话"), icon: MessagesSquare },
    { route: "relay", label: t("连接与供应商"), detail: t("切换 Codex 的供应商与连接方式"), icon: Cable, agent: "codex" },
    { route: "gatewayUsage", label: t("用量"), detail: t("调用次数、Token 与费用"), icon: BarChart3 },
    { route: "settings", label: t("设置"), detail: t("主题、更新、日志与诊断"), icon: Settings },
  ];

  // 自动接管紧挨着 Codex 那一行：打开后自己直接开 Codex 也有增强，不必再从这里点启动。
  // 关掉 Codex++ 就没人监听，所以没开登录自启时在这一行提醒并给开关。
  const takeover = autoTakeoverView(autoTakeover);
  const takeoverNeedsLogin = autoTakeoverOn === true && watcherEnabled === false;
  const takeoverRow = (
    <CxRow
      badges={autoTakeoverOn ? <StatusBadge status={takeover.status} label={takeover.label} /> : null}
      detail={autoTakeoverOn
        ? [takeover.detail, takeoverNeedsLogin ? t("Codex++ 没有设为登录时打开，退出 Codex++ 后不会接管。") : ""].filter(Boolean).join(" ")
        : enhancementPlatform === "windows"
          ? t("打开后，直接打开 Codex（开始菜单、任务栏）也会自动接管并注入增强，不用再从 Codex++ 启动。")
          : t("打开后，直接打开 Codex（Dock、启动台）也会自动接管并注入增强，不用再从 Codex++ 启动。")}
      icon={<CxStatusIcon tone={autoTakeoverOn ? (takeover.status === "ok" ? "ok" : takeover.status === "failed" ? "danger" : "warn") : "off"}><Plug /></CxStatusIcon>}
      key="codex-auto-takeover"
      title={t("自动接管官方 App")}
      tooltip={t("官方 App 已在运行且没有调试端口时，会先退出再带参数重开（可能中断正在进行的操作）。")}
    >
      {takeoverNeedsLogin ? (
        <button className="cx-secondary" onClick={() => void actions.setWatcherEnabled(true)} type="button">{t("登录时打开 Codex++")}</button>
      ) : null}
      <label className="switch-row compact relay-master-switch relay-master-switch-compact">
        <input
          aria-label={t("开启自动接管")}
          checked={autoTakeoverOn === true}
          disabled={autoTakeoverOn === null}
          onChange={(event) => void actions.setAutoTakeover(event.currentTarget.checked)}
          type="checkbox"
        />
        <span>{t("自动接管")}</span>
        <ToggleVisual />
      </label>
    </CxRow>
  );

  const agentRow = (tool: ToolEntry) => {
    const viaGateway = connected.has(tool.id);
    if (tool.id === "codex") {
      const mode = viaGateway ? t("经网关") : tool.activeRelayName ? tf("直连 · {0}", [tool.activeRelayName]) : t("直连");
      return [
        <CxRow
          detail={`${codexRunning ? t("运行中") : t("未运行")} · ${mode}`}
          icon={<CxStatusIcon tone={codexRunning ? "ok" : "off"}><AgentGlyph agent="codex" className="home-agent-glyph" /></CxStatusIcon>}
          key={tool.id}
          onClick={() => onOpenAgent(tool.id)}
          title={tool.name}
        >
          <button className="cx-secondary" disabled={launchPending} onClick={(event) => { event.stopPropagation(); onLaunchCodex(); }} type="button">
            {codexRunning ? <RotateCcw /> : <Play />}
            {codexRunning ? t("重启") : t("启动")}
          </button>
        </CxRow>,
        takeoverRow,
      ];
    }
    return (
      <CxRow
        detail={viaGateway ? t("已经通过 Codex++ 网关接入") : gatewayInUse ? t("未接入网关") : t("直连")}
        icon={<CxStatusIcon tone={viaGateway ? "ok" : "off"}><AgentGlyph agent={tool.id} className="home-agent-glyph" /></CxStatusIcon>}
        key={tool.id}
        onClick={() => onOpenAgent(tool.id)}
        title={tool.name}
      >
        {viaGateway || enhanceOnly || !gatewayInUse ? <ChevronRight aria-hidden="true" className="cx-set-chevron" /> : (
          <button className="cx-secondary" onClick={(event) => { event.stopPropagation(); onNavigate("gateway"); }} type="button">{t("接入")}</button>
        )}
      </CxRow>
    );
  };

  const gatewayDetail = gateway?.helperRunning
    ? [
        tf("运行中 · 端口 {0}", [gateway.port]),
        tf("{0} 个供应商", [gateway.pool.length]),
        tf("{0} 个 Agent 已接入", [connected.size]),
        usage ? tf("今日 {0} 次调用", [usage.totals.calls]) : "",
      ].filter(Boolean).join(" · ")
    : t("网关未运行");

  return (
    <div className="tp-page home-page">
      <TopbarActions>
        <button className="cx-secondary" onClick={() => void actions.refreshAds()} type="button">
          <RefreshCw />
          {t("刷新推荐")}
        </button>
      </TopbarActions>

      <CxSection title={t("状态")}>
        <CxCard>
          {tools.map(agentRow)}
          {enhanceOnly || !gatewayInUse ? null : (
            <CxRow
              detail={gatewayDetail}
              icon={<CxStatusIcon tone={gateway?.helperRunning ? "ok" : "off"}><Waypoints /></CxStatusIcon>}
              onClick={() => onNavigate("hub")}
              title={t("聚合网关")}
            >
              <ChevronRight aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
          )}
        </CxCard>
      </CxSection>

      {heroes.length ? (
        <CxSection aside={<span>{t("赞助商推荐：支持 Codex++ 的服务")}</span>} title={t("精选推荐")}>
          <div className="home-heroes" data-dense={heroes.length >= 3 || undefined} style={{ gridTemplateColumns: `repeat(${heroes.length}, minmax(0, 1fr))` }}>
            {heroes.map((ad) => (
              <div
                className="home-hero"
                key={adIdentity(ad)}
                onClick={() => open(ad)}
                onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); open(ad); } }}
                role="link"
                tabIndex={0}
                title={`${ad.description}\n${ad.url}`}
              >
                {ad.image ? <img alt="" className="home-hero-image" src={ad.image} /> : <span className="home-hero-image home-hero-placeholder"><Sparkles aria-hidden="true" /></span>}
                <div className="home-hero-body">
                  <div className="home-hero-head">
                    <strong>{formatAdTitle(ad.title)}</strong>
                  </div>
                  <p className="home-hero-text">{ad.description}</p>
                  {ad.highlights?.length ? (
                    <div className="home-hero-chips">
                      {ad.highlights.filter(Boolean).slice(0, 4).map((item) => <span key={item}>{item}</span>)}
                    </div>
                  ) : null}
                  <div className="home-hero-foot">
                    <span className="home-hero-host">{adHost(ad.url)}</span>
                    <span className="home-hero-cta">{t("了解更多")}<ArrowRight aria-hidden="true" /></span>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </CxSection>
      ) : null}

      {rest.length ? (
        <CxSection count={rest.length} title={t("更多赞助商")}>
          <div className="home-sponsors">
            {rest.map((ad) => (
              <div
                className="home-sponsor"
                key={adIdentity(ad)}
                onClick={() => open(ad)}
                onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); open(ad); } }}
                role="link"
                tabIndex={0}
                title={`${ad.description}\n${ad.url}`}
              >
                {ad.image ? <img alt="" className="home-sponsor-thumb" loading="lazy" src={ad.image} /> : <span className="home-sponsor-thumb home-hero-placeholder"><Sparkles aria-hidden="true" /></span>}
                <div className="home-sponsor-body">
                  <div className="home-sponsor-head">
                    <strong>{formatAdTitle(ad.title)}</strong>
                    {(ad.highlights ?? []).filter(Boolean).slice(0, 2).map((item) => <span className="home-sponsor-chip" key={item}>{item}</span>)}
                  </div>
                  <p className="home-sponsor-text">{ad.description}</p>
                </div>
                <ExternalLink aria-hidden="true" className="home-sponsor-go" />
              </div>
            ))}
          </div>
        </CxSection>
      ) : null}

      <CxSection title={t("常用")}>
        <div className="home-shortcuts">
          {shortcuts.filter((item) => routeVisible(focus, item.route)).map((item) => {
            const Icon = item.icon;
            return (
              <button className="home-shortcut" key={item.route} onClick={() => (item.agent ? onOpenAgent(item.agent, item.route) : onNavigate(item.route))} type="button">
                <Icon aria-hidden="true" />
                <span>
                  <strong>{item.label}</strong>
                  <small>{item.detail}</small>
                </span>
              </button>
            );
          })}
        </div>
      </CxSection>

    </div>
  );
}

function MaintenanceScreen({
  overview,
  launchPending,
  watcher,
  autoTakeover,
  settings,
  launchForm,
  onLaunchFormChange,
  actions,
  advanced = null,
}: {
  overview: OverviewResult | null;
  launchPending: boolean;
  watcher: WatcherResult | null;
  autoTakeover: AutoTakeoverResult | null;
  settings: SettingsResult | null;
  launchForm: { appPath: string; debugPort: string; helperPort: string };
  onLaunchFormChange: (next: { appPath: string; debugPort: string; helperPort: string }) => void;
  actions: Actions;
  /** 「高级」卡片里排在手动启动前面的内容（额外启动参数）。 */
  advanced?: ReactNode;
}) {
  const savedCodexAppPath = settings?.settings.codexAppPath ?? "";
  const takeoverOn = Boolean(settings?.settings.codexAppAutoTakeover);
  const takeover = autoTakeoverView(autoTakeover);
  return (
    <>
      {/* Codex 应用一行：识别到的路径 + 状态；换路径、清除保存的路径都在它的菜单里（原来「检查与修复」「应用路径」两节显示同一个路径三次）。 */}
      <CxSection
        aside={<button className="cx-secondary" onClick={() => void actions.checkHealth()} type="button"><RefreshCw />{t("检查")}</button>}
        id="codex-settings-check"
        title={t("应用与入口")}
      >
        <CxCard>
          <CxRow
            badges={savedCodexAppPath ? <CxBadge title={savedCodexAppPath}>{t("已手动指定")}</CxBadge> : null}
            detail={<code className="cx-set-path">{overview?.codex_app.path || savedCodexAppPath || t("未记录路径")}</code>}
            title={t("Codex 应用")}
            tooltip={t("免安装版或解包版只需要选择一次，之后从界面启动会自动复用")}
          >
            <StatusBadge status={overview?.codex_app.status} />
            <Menu>
              <MenuItem onClick={() => void actions.chooseCodexAppPath("folder")}>{t("选择应用目录")}</MenuItem>
              <MenuItem onClick={() => void actions.chooseCodexAppPath("file")}>{t("选择 Codex.exe")}</MenuItem>
              <MenuItem danger disabled={!savedCodexAppPath} onClick={() => void actions.clearCodexAppPath()}>{t("清除保存路径")}</MenuItem>
            </Menu>
          </CxRow>
          <CxRow detail={<code className="cx-set-path">{overview?.management_shortcut.path || t("未记录路径")}</code>} title={t("Codex++ 应用入口")}>
            <StatusBadge status={overview?.management_shortcut.status} />
            <Menu>
              <MenuItem onClick={() => void actions.repairShortcuts()}>{t("修复快捷方式")}</MenuItem>
            </Menu>
          </CxRow>
        </CxCard>
      </CxSection>
      <CxSection id="codex-settings-startup" title={t("开机启动")}>
        {/* 「登录时」的两个子选项只在开启登录打开后出现；「打开时启动 Codex」对手动打开也生效，单独一行。 */}
        <CxCard className="cx-set-switches">
          <label className="switch-row" title={t("开启后注册系统登录项，关闭时移除；是否自动启动 Codex 由下面的独立开关决定。")}>
            <input
              checked={Boolean(watcher?.enabled)}
              disabled={!watcher}
              onChange={(event) => void actions.setWatcherEnabled(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("登录时自动打开")}</strong>
              <small>{t("登录系统后打开 Codex++")}</small>
            </span>
            <ToggleVisual />
          </label>
          {watcher?.enabled ? (
            <>
              <label className="switch-row cx-set-sub" title={t("仅登录启动 Codex++ 时自动启动 Codex；手动打开 Codex++ 仍不会自动启动。")}>
                <input
                  checked={Boolean(watcher?.start_codex)}
                  disabled={!watcher}
                  onChange={(event) => void actions.setWatcherStartCodex(event.currentTarget.checked)}
                  type="checkbox"
                />
                <span>
                  <strong>{t("登录时自动启动 Codex")}</strong>
                  <small>{t("只在开机登录打开时生效")}</small>
                </span>
                <ToggleVisual />
              </label>
              <label className="switch-row cx-set-sub" title={t("仅登录启动时隐藏到托盘；手动打开 Codex++ 仍显示主窗口。")}>
                <input
                  checked={Boolean(watcher?.start_minimized)}
                  disabled={!watcher}
                  onChange={(event) => void actions.setWatcherStartMinimized(event.currentTarget.checked)}
                  type="checkbox"
                />
                <span>
                  <strong>{t("启动时自动最小化")}</strong>
                  <small>{t("开机登录时只留在托盘")}</small>
                </span>
                <ToggleVisual />
              </label>
            </>
          ) : null}
          <label className="switch-row" title={t("仅手动打开 Codex++ 时生效；已有 Codex 会先恢复，不会重复启动。")}>
            <input
              checked={Boolean(watcher?.start_codex_on_open)}
              disabled={!watcher}
              onChange={(event) => void actions.setWatcherStartCodexOnOpen(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("打开 Codex++ 时自动启动 Codex")}</strong>
              <small>{t("已有 Codex 会先恢复，不会重复启动")}</small>
            </span>
            <ToggleVisual />
          </label>
        </CxCard>
      </CxSection>
      <CxSection id="codex-settings-takeover" title={t("自动接管官方 App")}>
        <CxCard className="cx-set-switches">
          <label
            className="switch-row"
            title={`${t("开启后，你自己启动官方 App 时 Codex++ 会自动接管。官方 App 已在运行且没有调试端口时，会先退出再带参数重开（可能中断正在进行的操作）。")}\n${t("仅在 Codex++ 正在运行时生效；建议同时开启上面的「开机打开 Codex++」。")}`}
          >
            <input
              checked={takeoverOn}
              onChange={(event) => void actions.setAutoTakeover(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("开启自动接管")}</strong>
              <small>{t("你自己双击启动官方 App 时，Codex++ 自动接管并注入增强")}</small>
            </span>
            <ToggleVisual />
          </label>
          {takeoverOn ? (
            <CxRow
              badges={<StatusBadge status={takeover.status} label={takeover.label} />}
              detail={takeover.detail}
              title={t("接管状态")}
              tooltip={t("仅在 Codex++ 正在运行时生效；需要重启官方 App 时会自动重启。")}
            >
              <button className="cx-secondary" onClick={() => void actions.takeOverNow()} type="button">{t("立即接管")}</button>
              <Menu>
                <MenuItem onClick={() => void actions.pauseAutoTakeover()}>{t("本次不接管")}</MenuItem>
                <MenuItem onClick={() => void actions.refreshAutoTakeover()}>{t("刷新状态")}</MenuItem>
              </Menu>
            </CxRow>
          ) : null}
        </CxCard>
      </CxSection>
      <CxSection id="codex-settings-manual-launch" title={t("高级")}>
        <CxCard>
          {advanced}
          {/* 给少数人用：用别的路径或端口启动一次。默认收起。 */}
          <details className={`cx-set-disclosure ${advanced ? "" : "cx-set-disclosure-first"}`.trim()}>
            <summary>{t("手动启动")}<span className="cx-set-summary-hint">{t("应用路径留空时使用已保存路径；没有保存路径时使用自动探测")}</span></summary>
            <CxRow title={t("应用路径覆盖")}>
              <Input
                className="cx-set-input-wide"
                value={launchForm.appPath}
                onChange={(event) => onLaunchFormChange({ ...launchForm, appPath: event.currentTarget.value })}
                placeholder={savedCodexAppPath || t("例如 C:\\Program Files\\WindowsApps\\OpenAI.Codex...\\app")}
              />
            </CxRow>
            <CxRow title={t("Debug 端口")}>
              <Input
                className="cx-set-input-port"
                value={launchForm.debugPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, debugPort: event.currentTarget.value })}
              />
            </CxRow>
            <CxRow title={t("Helper 端口")}>
              <Input
                className="cx-set-input-port"
                value={launchForm.helperPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, helperPort: event.currentTarget.value })}
              />
            </CxRow>
            <div className="cx-set-footer">
              <button className="cx-secondary" onClick={() => void actions.saveManualCodexAppPath()} type="button">{t("保存为默认路径")}</button>
              <button className="cx-primary" disabled={launchPending} onClick={() => void actions.launch()} type="button"><Play />{t("启动 Codex")}</button>
            </div>
          </details>
        </CxCard>
      </CxSection>
    </>
  );
}

function AboutScreen({
  sections,
  overview,
  update,
  updateDownload = null,
  updateInstallProgress,
  logs,
  diagnostics,
  actions,
  form,
  onFormChange,
}: {
  /** 当前显示的设置分类里的卡片 id；其余卡片不渲染。 */
  sections: string[];
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  overview: OverviewResult | null;
  update: UpdateResult | null;
  /** 后台下载更新的状态（「有可用更新时自动下载」）。 */
  updateDownload?: UpdateDownloadState | null;
  updateInstallProgress: TaskProgress;
  logs: LogsResult | null;
  diagnostics: DiagnosticsResult | null;
  actions: Actions;
}) {
  const version = overview?.current_version ?? update?.currentVersion ?? "-";
  const openUrl = (url: string) => void actions.openExternalUrl(url);
  const shown = (id: string) => sections.includes(id);
  return (
    <>
      {shown("app-settings-update") ? <div className="cx-set-group" id="app-settings-update">
        <CxSection>
          <CxCard>
            {/* 一行摘要 + 一个操作：有新版本时主按钮是「安装」（已在后台下好）或「下载并安装」，检查更新退到旁边。 */}
            {(() => {
              const latest = update?.updateAvailable ? update.latestVersion ?? "" : "";
              const download = latest && updateDownload?.version === latest ? updateDownload : null;
              const ready = Boolean(latest) && (update?.downloaded === true || download?.state === "ready");
              const downloading = !ready && download?.state === "downloading";
              const detail = latest
                ? ready
                  ? t("新版本已下载，点「安装」更新；安装时 Codex++ 会关闭并自动重启")
                  : downloading
                    ? download?.percent != null
                      ? tf("正在后台下载 {0}%，下完不会自动安装", [Math.round(download.percent)])
                      : t("正在后台下载，下完不会自动安装")
                    : download?.state === "failed"
                      ? t("后台下载没成功，点「下载并安装」重试")
                      : t("从 GitHub Release 下载安装包更新")
                : update?.latestVersion
                  ? t("已是最新版本")
                  : t("尚未检查更新");
              return (
                <CxRow
                  badges={latest ? <CxBadge tone="ok">{ready ? tf("{0} 已下载", [latest]) : tf("有新版本 {0}", [latest])}</CxBadge> : null}
                  detail={<>{detail}{latest && update?.assetName ? ` · ${update.assetName}` : ""}</>}
                  icon={<img alt="" src={APP_LOGO_URL} />}
                  title={`Codex++ ${version}`}
                  tooltip={download?.state === "failed" ? download.message : undefined}
                >
                  <button className="cx-secondary" onClick={() => void actions.checkUpdate()} type="button"><RefreshCw />{t("检查更新")}</button>
                  {latest ? (
                    <button className="cx-primary" disabled={updateInstallProgress.active} onClick={() => void actions.performUpdate()} type="button">
                      <Download />
                      {updateInstallProgress.active ? t("正在安装…") : ready ? t("安装") : t("下载并安装")}
                    </button>
                  ) : null}
                </CxRow>
              );
            })()}
            {/* 改的是设置草稿，由同页 GlobalSettingsScreen 的自动保存落盘（#1272）。 */}
            <CxRow
              detail={t("打开时及运行期间每隔几小时检查一次")}
              title={t("启动时自动检查更新")}
              tooltip={t("打开 Codex++ 时及运行期间每隔几小时在后台检查新版本并提示。关闭后不再自动检查，仍可随时点「检查更新」。")}
            >
              <Switch label={t("启动时自动检查更新")} on={form.updateAutoCheck !== false} onToggle={() => onFormChange({ ...form, updateAutoCheck: form.updateAutoCheck === false })} />
            </CxRow>
            <CxRow
              detail={t("在后台把安装包下好，但不安装；点「安装」才更新")}
              title={t("有可用更新时自动下载")}
              tooltip={t("检查到新版本后，在后台下载安装包并校验，下好后这里和侧栏会提示「安装」。不会自动安装，也不会重启 Codex++。")}
            >
              <Switch label={t("有可用更新时自动下载")} on={form.updateAutoDownload !== false} onToggle={() => onFormChange({ ...form, updateAutoDownload: form.updateAutoDownload === false })} />
            </CxRow>
            {update?.updateAvailable && update.releaseSummary ? (
              <details className="app-update-notes cx-set-disclosure">
                <summary>{tf("{0} 更新说明", [update.latestVersion ?? ""])}</summary>
                <div className="cx-set-block"><ReleaseNotes markdown={update.releaseSummary} onOpenUrl={openUrl} /></div>
              </details>
            ) : null}
          </CxCard>
          <TaskProgressBox completedTitle={t("上次更新结果")} progress={updateInstallProgress} title={t("安装包更新进度")} />
        </CxSection>
      </div> : null}
      {shown("app-settings-about") ? <div className="cx-set-group" id="app-settings-about">
        <CxSection>
          <CxCard>
            <CxRow detail={t("本地 Codex 增强、管理工具和安装包维护")} icon={<img alt="" src={APP_LOGO_URL} />} title={`Codex++ ${version}`} />
            <CxRow title={t("Codex 版本")}>
              <span className="cx-set-value">{overview?.codex_version ?? t("未检测到")}</span>
            </CxRow>
            <CxRow detail="github.com/BigPizzaV3/CodexPlusPlus" onClick={() => openUrl("https://github.com/BigPizzaV3/CodexPlusPlus")} title={t("项目地址")}>
              <ExternalLink aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
          </CxCard>
        </CxSection>
        <CxSection title={t("反馈与社区")}>
          <CxCard>
            <CxRow detail={t("在 GitHub Issues 报告问题或提建议")} icon={<Github />} onClick={() => openUrl("https://github.com/BigPizzaV3/CodexPlusPlus/issues")} title={t("反馈问题")}>
              <ExternalLink aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
            <CxRow detail="discord.gg/y96kX7A76v" icon={<MessageCircle />} onClick={() => openUrl("https://discord.gg/y96kX7A76v")} title="Discord">
              <ExternalLink aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
            <CxRow detail="t.me/CodexPlusPlus" icon={<MessageCircle />} onClick={() => openUrl("https://t.me/CodexPlusPlus")} title="Telegram">
              <ExternalLink aria-hidden="true" className="cx-set-chevron" />
            </CxRow>
          </CxCard>
        </CxSection>
      </div> : null}
      {shown("app-settings-logs") ? <div className="cx-set-group" id="app-settings-logs"><LogsPanel logs={logs} actions={actions} /></div> : null}
      {shown("app-settings-diagnostics") ? <div className="cx-set-group" id="app-settings-diagnostics"><DiagnosticsPanel diagnostics={diagnostics} actions={actions} /></div> : null}
    </>
  );
}

/// Codex 的额外启动参数：设置模式「Codex」分类（agentSettings）里，维护页最后「高级」卡片的内容。
/// 应用级选项在整体设置里，见 [GlobalSettingsScreen]。
function AgentSettingsScreen({
  dirty,
  form,
  onFormChange,
  actions,
}: {
  dirty: boolean;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const count = form.codexExtraArgs.length;
  return (
    <>
      <TopbarActions><SettingsAutoSave className="cx-set-autosave" dirty={dirty} form={form} actions={actions} /></TopbarActions>
      {/* 放在「高级」卡片里，默认收起；填了参数时标题旁显示条数，不用展开也知道有没有设。 */}
      <details className="cx-set-disclosure cx-set-disclosure-first" open={count > 0 || undefined}>
        <summary>
          {t("额外启动参数")}
          <span className="cx-set-summary-hint">{count ? tf("已设 {0} 个", [count]) : t("启动 Codex App 时追加的命令行参数")}</span>
        </summary>
        <CxRow detail={t("每行一个参数，例如 --force_high_performance_gpu。不需要填写 open 或 --args。")} title={t("额外参数")} tooltip={t("启动 Codex App 时追加到默认 CDP 参数后。留空则保持默认启动行为。")} />
        <div className="cx-set-block">
          <Textarea
              aria-label={t("Codex 启动参数")}
              className="launch-args-input"
              placeholder="--force_high_performance_gpu"
              spellCheck={false}
              value={codexExtraArgsToInput(form.codexExtraArgs)}
              onChange={(event) =>
                onFormChange({
                  ...form,
                  codexExtraArgs: inputToCodexExtraArgs(event.currentTarget.value),
                })
              }
            />
        </div>
      </details>
    </>
  );
}

/// 托盘快捷面板开关。值仍存在 gateway.json 的 `trayPanel`（老版本在网关设置里改过的选择照样有效），
/// 由后端命令读写，后端同时更新托盘行为和内存缓存。
/** 设置页里直接读写后端的一个开关（托盘快捷面板、轻量模式）：读不到时按开着显示。 */
function useBackendToggle(readCommand: string, writeCommand: string) {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let cancelled = false;
    void invoke<boolean>(readCommand)
      .then((value) => { if (!cancelled) setEnabled(value !== false); })
      .catch(() => { if (!cancelled) setEnabled(true); });
    return () => { cancelled = true; };
  }, [readCommand]);
  const toggle = async () => {
    if (enabled === null || saving) return;
    setSaving(true);
    setError("");
    try {
      setEnabled(await invoke<boolean>(writeCommand, { enabled: !enabled }));
    } catch (failure) {
      setError(stringifyError(failure));
    } finally {
      setSaving(false);
    }
  };
  return { enabled, saving, error, toggle };
}


/// 整体设置页（最左侧齿轮）。
///
/// 只放与当前 Agent 无关的应用级选项，内容不随 Agent 切换而变。
/// Agent 专属选项放在各 Agent 菜单的「设置」里，见 [AgentSettingsScreen]。
function GlobalSettingsScreen({
  sections,
  dirty,
  form,
  settings,
  theme,
  themePreference,
  actions,
  onOpenTour,
  agentFocus,
  onOpenAgentFocus,
}: {
  /** 当前显示的设置分类里的卡片 id；其余卡片不渲染。 */
  sections: string[];
  onOpenTour: () => void;
  agentFocus: AgentFocus;
  onOpenAgentFocus: () => void;
  dirty: boolean;
  form: BackendSettings;
  settings: SettingsResult | null;
  theme: Theme;
  themePreference: ThemePreference;
  actions: Actions;
  onOpenAgentSettings: () => void;
}) {
  const settingsPath = settings?.settings_path ?? "";
  const shown = (id: string) => sections.includes(id);
  const trayPanel = useBackendToggle("tray_panel_enabled", "set_tray_panel_enabled");
  const lightweight = useBackendToggle("lightweight_mode_enabled", "set_lightweight_mode_enabled");
  const themeOptions: AppSelectOption<ThemePreference>[] = [
    { value: "system", label: t("跟随系统") },
    { value: "light", label: t("浅色") },
    { value: "dark", label: t("深色") },
  ];
  // 参照 Cursor 设置页：页面标题就是分类名，每节一个卡片外的小标题 + 一张行卡片；
  // 说明只写一行，长说明放悬停提示。自动保存状态挂到标题栏，只在保存中 / 出错时出现。
  return (
    <>
      <TopbarActions><SettingsAutoSave className="cx-set-autosave" dirty={dirty} form={form} actions={actions} /></TopbarActions>
      {shown("app-settings-general") ? <div className="cx-set-group" id="app-settings-general">
        <CxSection title={t("外观")}>
          <CxCard>
            <CxRow
              title={t("界面主题")}
              detail={themePreference === "system"
                ? `${t("跟随系统外观，当前为")}${theme === "dark" ? t("深色") : t("浅色")}${t("模式。")}`
                : `${t("当前为")}${theme === "dark" ? t("深色") : t("浅色")}${t("模式。")}`}
            >
              <AppSelect ariaLabel={t("界面主题")} className="cx-set-select" onChange={(value) => actions.setThemePreference(value)} options={themeOptions} size="sm" value={themePreference} />
            </CxRow>
            <DisplaySettings />
          </CxCard>
        </CxSection>
        <CxSection title={t("窗口与托盘")}>
          <CxCard>
            <CxRow
              detail={trayPanel.error || t("单击托盘图标查看状态、启动 Codex、切换供应商")}
              title={t("点击托盘图标弹出快捷面板")}
              tooltip={t("面板里能看 Codex 状态、一键启动或重启、切换供应商、看用量，网关用户还能给各 Agent 换模型。右键仍是原来的菜单，双击打开主窗口；关掉恢复原来的点击行为。")}
            >
              <Switch disabled={trayPanel.enabled === null || trayPanel.saving} label={t("点击托盘图标弹出快捷面板")} on={trayPanel.enabled ?? true} onToggle={() => void trayPanel.toggle()} />
            </CxRow>
            <CxRow
              detail={lightweight.error || t("后台服务照常运行，再次打开时重新加载界面")}
              title={t("关闭窗口后释放界面内存")}
              tooltip={t("关闭主窗口时连同界面一起释放，后台服务、网关和托盘照常运行，从托盘再打开时重新加载界面。未保存的编辑内容会丢失；关掉则关闭窗口时只隐藏。")}
            >
              <Switch disabled={lightweight.enabled === null || lightweight.saving} label={t("关闭窗口后释放界面内存")} on={lightweight.enabled ?? false} onToggle={() => void lightweight.toggle()} />
            </CxRow>
          </CxCard>
        </CxSection>
        <CxSection title={t("入门")}>
          <CxCard>
            <CxRow detail={agentFocus === "codex" ? t("只管理 Codex") : agentFocus === "enhance" ? t("只用增强功能") : t("全部功能")} title={t("使用模式")} tooltip={agentFocusDescription(agentFocus)}>
              <button className="cx-secondary" onClick={onOpenAgentFocus} type="button">{t("修改")}</button>
            </CxRow>
            <CxRow detail={t("完成配置、验证连接并开始使用，也可以查看各个入口的导览。")} title={t("新手引导")}>
              <button className="cx-secondary" onClick={onOpenTour} type="button">{t("重新打开引导")}</button>
            </CxRow>
          </CxCard>
        </CxSection>
      </div> : null}
      {shown("app-settings-doctor") ? <ConfigDoctorPanel /> : null}
      {shown("app-settings-config") ? <div className="cx-set-group" id="app-settings-config">
        <CxSection title={t("配置备份与迁移")}>
          <CxCard>
            <CxRow detail={t("包含供应商 API Key 等敏感信息，请妥善保管")} title={t("导出完整配置")}>
              <button className="cx-secondary" onClick={() => void actions.exportSettings()} type="button"><Upload />{t("导出")}</button>
            </CxRow>
            <CxRow detail={t("读入为草稿，检查无误后自动保存")} title={t("导入配置草稿")} tooltip={t("导出文件包含供应商 API Key 等敏感信息；导入只会读入草稿，检查后仍需点击保存设置。")}>
              <button className="cx-secondary" onClick={() => void actions.importSettings()} type="button"><Download />{t("导入")}</button>
            </CxRow>
          </CxCard>
        </CxSection>
        <CxSection title={t("存放位置")}>
          <CxCard>
            <CxRow
              detail={<code className="cx-set-path" title={settingsPath}>{settingsPath || "—"}</code>}
              title={t("配置文件路径")}
              tooltip={t("只迁移 settings.json；日志、备份、Skills 和运行状态仍留在原状态目录。更换后建议重新打开管理器。")}
            >
              <button className="cx-secondary" onClick={() => void actions.chooseSettingsPath()} type="button">{t("更改…")}</button>
              <Menu>
                <MenuItem onClick={() => void actions.resetSettingsPath()}>{t("恢复默认路径")}</MenuItem>
              </Menu>
            </CxRow>
          </CxCard>
        </CxSection>
      </div> : null}
    </>
  );
}

function LogsPanel({ logs, actions }: { logs: LogsResult | null; actions: Actions }) {
  // 每行一条结构化日志：时间、事件名、一行摘要；点开看完整明细。最新的在前，可搜索、可只看出错。
  const entries = useMemo(() => parseLogText(logs?.text ?? "", logs?.truncated === true), [logs?.text, logs?.truncated]);
  const [query, setQuery] = useState("");
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [limit, setLimit] = useState(200);
  const shown = useMemo(() => filterLogEntries(entries, query, errorsOnly), [entries, query, errorsOnly]);
  const errorCount = useMemo(() => entries.filter((entry) => entry.tone === "error").length, [entries]);
  const logDetail = logs
    ? logs.truncated
      ? tf("日志大小 {0}，仅显示末尾 {1} 行", [formatBytes(logs.fileSize), logs.lines])
      : tf("日志大小 {0}", [formatBytes(logs.fileSize)])
    : "";
  const now = Date.now();
  return (
    <CxSection
      aside={
        <>
          {logDetail ? <span title={logs?.path ?? ""}>{logDetail}</span> : null}
          <button className="cx-secondary" onClick={() => void actions.copyLogs()} type="button"><Copy />{t("复制")}</button>
          <Menu>
            <MenuItem onClick={() => void actions.refreshLogs()}>{t("刷新")}</MenuItem>
            <MenuItem danger onClick={() => void actions.clearLogs()}>{t("清理日志")}</MenuItem>
          </Menu>
        </>
      }
      title={t("最近日志")}
    >
      <div className="cx-log-toolbar">
        <CxFilters
          items={[
            { id: "all", label: t("全部"), count: entries.length },
            { id: "errors", label: t("出错"), count: errorCount },
          ]}
          label={t("日志筛选")}
          onChange={(value) => setErrorsOnly(value === "errors")}
          value={errorsOnly ? "errors" : "all"}
        />
        <label className="cx-search cx-log-search">
          <Search aria-hidden="true" />
          <input aria-label={t("搜索日志")} onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("搜索事件名或内容")} value={query} />
        </label>
      </div>
      <CxCard className="cx-log-card">
        {shown.slice(0, limit).map((entry) => (
          <details className="cx-log-row" data-tone={entry.tone} key={entry.index}>
            <summary title={entry.pid ? tf("第 {0} 行 · 进程 {1}", [entry.index, entry.pid]) : tf("第 {0} 行", [entry.index])}>
              <i aria-hidden="true" className="cx-log-dot" />
              <span className="cx-log-time">{formatLogTime(entry.timestampMs, now)}</span>
              {entry.event ? <span className="cx-log-event">{entry.event}</span> : null}
              <span className="cx-log-summary">{entry.summary}</span>
            </summary>
            <pre className="cx-log-body">{entry.body}</pre>
          </details>
        ))}
        {shown.length > limit ? (
          <button className="cx-log-more" onClick={() => setLimit((current) => current + 300)} type="button">
            {tf("再显示 {0} 条", [Math.min(300, shown.length - limit)])}
          </button>
        ) : null}
        {!shown.length ? <CxEmpty title={entries.length ? t("没有匹配的日志") : t("暂无日志。")} /> : null}
      </CxCard>
      {logs?.path ? <code className="cx-set-caption">{logs.path}</code> : null}
    </CxSection>
  );
}

function DiagnosticsPanel({ diagnostics, actions }: { diagnostics: DiagnosticsResult | null; actions: Actions }) {
  // 先列关键信息，完整报告（反馈问题时复制给开发者）收在下面。
  const report = diagnostics?.report ?? "";
  const facts = useMemo(() => diagnosticFacts(report), [report]);
  return (
    <CxSection
      aside={
        <>
          <button className="cx-secondary" onClick={() => void actions.copyDiagnostics()} type="button"><Copy />{t("复制报告")}</button>
          <Menu>
            <MenuItem onClick={() => void actions.refreshDiagnostics()}>{t("重新生成")}</MenuItem>
          </Menu>
        </>
      }
      title={<span title={t("包含版本、路径、设置和平台信息；反馈问题时点「复制报告」附上即可")}>{t("诊断报告")}</span>}
    >
      <CxCard className="cx-set-mono-card cx-diagnostic-card">
        {facts.map((fact) => (
          <CxRow key={fact.label} title={t(fact.label)}>
            <span className="cx-diagnostic-value" data-tone={fact.tone} title={fact.value}>
              {[fact.status ? t(fact.status) : "", fact.value ?? ""].filter(Boolean).join(" · ")}
            </span>
          </CxRow>
        ))}
        <details className="cx-set-disclosure" open={facts.length ? undefined : true}>
          <summary>{t("完整报告")}</summary>
          <Textarea aria-label={t("诊断报告")} className="log-view" readOnly title={t("包含版本、路径、设置和平台信息")} value={report || t("尚未生成诊断报告。")} />
        </details>
      </CxCard>
    </CxSection>
  );
}

function RelayProfileList({
  form,
  visibleProfiles = form.relayProfiles,
  onFormChange,
  onReorder,
  onEdit,
  onSwitchProfile,
  disabled = false,
  poolLink,
  actions,
}: {
  form: BackendSettings;
  /** 和网关供应商池的对应：每张卡片右侧显示「已在供应商池 / 加入供应商池」。 */
  poolLink?: CodexPoolLink;
  visibleProfiles?: RelayProfile[];
  onFormChange: (value: BackendSettings) => void;
  /// 拖拽排序落手专用：只走乐观更新路径；复制、删除等表单改动仍走 onFormChange。
  onReorder: (value: BackendSettings) => void;
  onEdit: (id: string) => void;
  onSwitchProfile: (id: string) => void;
  disabled?: boolean;
  actions: Actions;
}) {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const next = reorderRelayProfiles(form, String(active.id), String(over.id));
    if (next !== form) onReorder(next);
  };
  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
      <SortableContext items={visibleProfiles.map((profile) => profile.id)} strategy={verticalListSortingStrategy}>
        <div className="relay-profile-list">
          {visibleProfiles.map((profile, index) => (
            <SortableRelayProfileCard
              poolLink={poolLink}
              actions={actions}
              form={form}
              index={index}
              key={profile.id}
              onEdit={onEdit}
              onFormChange={onFormChange}
              onSwitchProfile={onSwitchProfile}
              disabled={disabled}
              profile={profile}
            />
          ))}
        </div>
      </SortableContext>
    </DndContext>
  );
}

function SortableRelayProfileCard({
  form,
  profile,
  index,
  onFormChange,
  onEdit,
  onSwitchProfile,
  disabled = false,
  poolLink,
  actions,
}: {
  form: BackendSettings;
  poolLink?: CodexPoolLink;
  profile: RelayProfile;
  index: number;
  onFormChange: (value: BackendSettings) => void;
  onEdit: (id: string) => void;
  onSwitchProfile: (id: string) => void;
  disabled?: boolean;
  actions: Actions;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: profile.id });
  const active = profile.id === form.activeRelayId;
  // 网关托管的供应商：由「Agent 接入」管理，这里只能「使用」（重新接入）。
  const gatewayManaged = profile.id === CODEX_GATEWAY_RELAY_ID;
  const billing = useRelayBillingProbe(
    !gatewayManaged && relayBillingProbeEligible(profile) ? relayBillingCacheKey(profile) : "",
    () => invoke<CommandResult<RelayBillingProbe>>("probe_relay_billing", { profile }),
  );
  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    // 拖动中跟手、松手时 200ms 滑到新位置。
    transition: isDragging ? "none" : transition ?? "transform 200ms ease",
  };

  return (
    <div
      className={`relay-profile-card ${active ? "active" : ""} ${isDragging ? "dragging" : ""}`}
      data-gateway-managed={gatewayManaged || undefined}
      data-relay-profile-id={profile.id}
      key={profile.id}
      onKeyDown={(event) => {
        if (event.key === "Enter" && !gatewayManaged) onEdit(profile.id);
      }}
      // 点整行打开编辑（按钮、菜单自己拦住点击）。
      onClick={(event) => {
        if (gatewayManaged || (event.target as HTMLElement).closest("button, a, input, [role=menu]")) return;
        onEdit(profile.id);
      }}
      ref={setNodeRef}
      style={style}
      tabIndex={0}
    >
      <button
        aria-label={t("拖动排序")}
        className="relay-drag"
        title={t("拖动排序")}
        type="button"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="h-4 w-4" />
      </button>
      <span className="relay-index" title={profile.name || t("未命名供应商")}>
        {providerInitial(profile.name)}
      </span>
      <span className="relay-summary">
        {/* 参照 CC Switch：接入方式、非 Responses 协议做成名字旁的标签，第二行只放地址。 */}
        <strong>
          <span className="relay-summary-name">{profile.name || t("未命名供应商")}</span>
          {gatewayManaged ? (
            <>
              <span className="relay-tag" data-tone="gateway">{t("新版网关")}</span>
              {codexGatewayKeepsLogin(profile) ? <span className="relay-tag" data-tone="official">{t("ChatGPT 登录")}</span> : null}
            </>
          ) : (
            <span className="relay-tag" data-tone={relayProfileTagTone(profile)}>{relayProfileModeLabel(profile)}</span>
          )}
          {!gatewayManaged && !isAggregateRelayProfile(profile) && profile.protocol !== "responses" ? (
            <span className="relay-tag" data-tone="protocol">{relayProtocolShortLabel(profile.protocol)}</span>
          ) : null}
          {active ? <span className="relay-active-badge">{t("使用中")}</span> : null}
          {!billing.probe && billing.settled ? (
            <RelayBillingRetry failure={billing.failure} loading={billing.loading} onRefresh={billing.refresh} />
          ) : null}
        </strong>
        <small>
          {gatewayManaged
            ? codexGatewayModelCount(profile)
              ? tf("经 Codex++ 本地网关，用供应商池里的 {0} 个模型；在「Agent 接入」里管理", [codexGatewayModelCount(profile)])
              : t("经 Codex++ 本地网关，用供应商池里的模型；在「Agent 接入」里管理")
            : profile.relayMode === "official"
            ? profile.officialMixApiKey
              ? profile.baseUrl || t("未填写 URL")
              : t("使用 Codex 当前登录的 ChatGPT 账号")
            : relayProfileConfigBrief(profile)}
        </small>
        {billing.probe ? (
          <RelayBillingLine failure={billing.failure} loading={billing.loading} onRefresh={billing.refresh} probe={billing.probe} />
        ) : null}
      </span>
      <span className="relay-card-actions">
        <Button
          className={`relay-use-button ${active ? "active" : ""}`}
          disabled={disabled}
          onClick={(event) => {
            event.stopPropagation();
            if (disabled) return;
            onSwitchProfile(profile.id);
          }}
          size="sm"
          title={
            disabled
              ? t("供应商切换中")
              : !form.relayProfilesEnabled
                ? t("供应商配置总开关已关闭，点击后按提示开启")
                : active
                  ? t("当前正在使用")
                  : gatewayManaged
                    ? t("重新接入 Codex++ 网关")
                    : t("设为当前")
          }
          variant={active ? "secondary" : "outline"}
        >
          <CheckCircle2 className="h-4 w-4" />
          {active ? t("使用中") : t("使用")}
        </Button>
        {gatewayManaged || !poolLink ? null : <PoolLinkControl link={poolLink} profileId={profile.id} />}
        {/* 次要操作收进三点菜单（原来是一排五个图标按钮）：多开、测试、编辑、复制、删除。点整行也是编辑。 */}
        {gatewayManaged ? null : (
        <span className="relay-card-extra" onClick={(event) => event.stopPropagation()}>
          <Menu label={t("供应商操作")}>
            <MenuItem onClick={() => onEdit(profile.id)}>{t("编辑")}</MenuItem>
            <MenuItem
              disabled={isAggregateRelayProfile(profile)}
              onClick={() => { if (!isAggregateRelayProfile(profile)) void actions.testRelayProfile(profile); }}
            >
              <span title={isAggregateRelayProfile(profile) ? t("聚合供应商会在真实对话中轮转成员，请测试成员供应商") : t("发送 hi 测试")}>{t("测试")}</span>
            </MenuItem>
            <MenuItem
              disabled={!multiLaunchSupported(profile) || actions.multiLaunchPending === profile.id}
              onClick={() => { if (multiLaunchSupported(profile)) void actions.launchMultiInstance(profile); }}
            >
              <span title={isAggregateRelayProfile(profile)
                ? t("聚合供应商暂不支持独立多开")
                : multiLaunchSupported(profile)
                  ? t("启动独立 Codex 实例")
                  : t("仅支持单一 Responses 或 Chat Completions 供应商，且不能配置模型路由")}>{t("用这个供应商多开")}</span>
            </MenuItem>
            <MenuItem onClick={() => onFormChange(duplicateRelayProfile(form, profile.id))}>{t("复制")}</MenuItem>
            <MenuItem
              danger
              disabled={form.relayProfiles.length <= 1}
              onClick={() => {
                void confirmRemoveRelayProfile(form, profile, actions, "relay").then((next) => {
                  if (next) onFormChange(next);
                });
              }}
            >
              {t("删除供应商")}
            </MenuItem>
          </Menu>
        </span>
        )}
      </span>
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

/// 市场脚本更新时间的相对描述：刚刚 / N 小时前 / N 天前 / N 个月前 / N 年前。
function formatRelativeAge(time: number, now = Date.now()): string {
  const age = relativeAge(time, now);
  switch (age.unit) {
    case "justNow":
      return t("刚刚");
    case "hours":
      return tf("{0} 小时前", [age.value]);
    case "days":
      return tf("{0} 天前", [age.value]);
    case "months":
      return tf("{0} 个月前", [age.value]);
    case "years":
      return tf("{0} 年前", [age.value]);
  }
}

function MarketScriptCard({
  script,
  actions,
  installedKey,
}: {
  script: ScriptMarketItem;
  actions: Actions;
  /// 已安装市场脚本在本地清单里的 key（`user:market-*.js`）。后端按 id 生成文件名，
  /// 前端不重算，直接取自 user_scripts 清单；为空表示未安装或不可删除。
  installedKey?: string;
}) {
  // 「未安装」是默认状态：满屏同一个徽标只是噪音，只给已安装 / 可更新打标。
  const status = script.updateAvailable ? t("可更新") : script.installed ? tf("已安装 {0}", [script.installedVersion]) : null;
  const isGitHubHomepage = script.homepage ? isGitHubRepositoryHomepage(script.homepage) : false;
  const updatedTime = parseMarketTime(script.updatedAt);
  // 头像：清单给的图标，否则主页仓库作者的 GitHub 头像；主页就是市场仓库时退回名字首字，免得全是同一个头像。
  const homepageRepo = isGitHubHomepage ? script.homepage.replace(/^https?:\/\/github\.com\//, "").split(/[/?#]/).slice(0, 2).join("/") : "";
  const icon = script.icon || (homepageRepo && !SCRIPT_MARKET_REPOSITORY_URL.includes(homepageRepo) ? githubAvatar(homepageRepo) : "");
  const updatedText = updatedTime !== null ? tf("{0}更新", [formatRelativeAge(updatedTime)]) : "";
  const meta = [script.author || t("未知作者"), `v${script.version}`, updatedText].filter(Boolean).join(" · ");
  // 已安装且是最新版时不再给按钮（重新安装收进菜单），未安装 / 可更新时右侧一个按钮。
  const showInstall = !script.installed || script.updateAvailable;
  return (
    <ExtensionCard
      description={script.description}
      icon={icon}
      link={script.homepage ? { url: script.homepage, text: isGitHubHomepage ? tf("在 GitHub 上支持作者：{0}", [script.name]) : t("主页") } : null}
      menu={
        <>
          {script.installed && !script.updateAvailable ? (
            <MenuItem onClick={() => void actions.installMarketScript(script.id)}>
              <span className="ext-row-menu-item"><Download size={14} />{t("重新安装")}</span>
            </MenuItem>
          ) : null}
          {installedKey ? (
            <MenuItem danger onClick={() => void actions.deleteUserScript(installedKey)}>
              <span className="ext-row-menu-item"><Trash2 size={14} />{t("卸载")}</span>
            </MenuItem>
          ) : null}
        </>
      }
      meta={meta}
      name={script.name}
      onOpenUrl={(url) => void actions.openExternalUrl(url)}
      primary={
        showInstall ? (
          <button className="cx-secondary" onClick={() => void actions.installMarketScript(script.id)} type="button">
            <Download />
            {script.updateAvailable ? t("更新") : t("安装")}
          </button>
        ) : null
      }
      status={status ? { text: status, highlight: script.updateAvailable } : null}
      tags={script.tags}
    />
  );
}

// 增强页的一组开关：卡片外的小标题（分组说明收进悬停）+ 一张卡，卡里一行一项。
function FeatureGroup({ title, detail, children, id, hidden = false }: { title: string; detail: string; children: ReactNode; id?: string; hidden?: boolean }) {
  const { CxCard, CxSection } = AgentCx;
  if (hidden) return null;
  return (
    <CxSection className="cx-feature-group" id={id} title={<span title={detail || undefined}>{title}</span>}>
      <CxCard className="cx-feature-card">{children}</CxCard>
    </CxSection>
  );
}

function FeatureToggle({
  title,
  detail,
  checked,
  disabled = false,
  onChange,
}: {
  title: string;
  detail: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}) {
  const { CxBadge } = AgentCx;
  // 「（实验）」从标题里拿出来做成小标签；说明里写了要重启 Codex 的，标题旁加「需重启」。
  const experimental = EXPERIMENTAL_TITLE.test(title);
  const needsRestart = RESTART_HINT.test(detail);
  return (
    <label className={`feature-toggle ${disabled ? "disabled" : ""}`}>
      <input
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
        type="checkbox"
      />
      <span>
        <strong>
          {experimental ? title.replace(EXPERIMENTAL_TITLE, "") : title}
          {experimental ? <CxBadge tone="warn">{t("实验")}</CxBadge> : null}
          {needsRestart ? <CxBadge>{t("需重启")}</CxBadge> : null}
        </strong>
        <small title={detail}>{detail}</small>
      </span>
      <ToggleVisual />
    </label>
  );
}

/** 打开设置页时滚到指定分组（例如启动时提示更新 → 版本与更新），滚完清掉。 */
function AppSettingsFocus({ focus, onDone }: { focus: string | null; onDone: () => void }) {
  useEffect(() => {
    if (!focus) return;
    const frame = window.requestAnimationFrame(() => {
      document.getElementById(focus)?.scrollIntoView({ block: "start" });
      onDone();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [focus, onDone]);
  return null;
}

const EXPERIMENTAL_TITLE = /\s*(?:（实验性?）|\(experimental\))$/i;
const RESTART_HINT = /重启 ?Codex|重新启动 ?Codex|下次启动 ?Codex|restart(?:ing)? Codex|relaunch/i;

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function UpdateDialog({ update, progress, downloaded = false, onClose, onSkipVersion, onMuteToday, onUpdate, onOpenUrl }: {
  update: UpdateResult;
  progress: TaskProgress;
  /** 安装包已在后台下好：主按钮写「立即安装」。 */
  downloaded?: boolean;
  onClose: () => void;
  onSkipVersion: () => void;
  onMuteToday: () => void;
  onUpdate: () => void;
  onOpenUrl: (url: string) => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => { if (dialog?.open) dialog.close(); };
  }, []);

  return (
    <dialog className="modal-card update-dialog" ref={dialogRef} aria-labelledby="update-dialog-title" aria-describedby="update-dialog-description" onCancel={(event) => { event.preventDefault(); onClose(); }}>
      <div className="modal-head">
        <div>
          <h2 id="update-dialog-title">{tf("发现新版本 {0}", [update.latestVersion ?? ""])}</h2>
          <p id="update-dialog-description">{tf("当前版本 {0}", [update.currentVersion])}</p>
        </div>
        <button className="toast-close" onClick={onClose} type="button" aria-label={t("关闭更新提示")}>×</button>
      </div>
      <div className="update-dialog-body">
        <ReleaseNotes markdown={update.releaseSummary || t("发现可用更新。")} onOpenUrl={onOpenUrl} />
      </div>
      <TaskProgressBox completedTitle={t("上次更新结果")} progress={progress} title={t("安装包更新进度")} />
      <p className="update-dialog-hint">{t("也可随时在「设置 → 版本与更新」里检查和安装。")}</p>
      {!update.assetName || !update.assetUrl ? <p className="update-dialog-hint">{t("此版本暂未提供适用于当前系统的安装包。")}</p> : null}
      <Toolbar className="update-dialog-actions">
        <Button autoFocus disabled={progress.active || !update.assetName || !update.assetUrl} onClick={onUpdate}>
          <Download className="h-4 w-4" />
          {progress.active ? t("正在安装…") : downloaded ? t("立即安装") : t("立即更新")}
        </Button>
        <Button variant="secondary" onClick={onClose}>{t("关闭提示")}</Button>
        <Button variant="ghost" onClick={onMuteToday}>{t("今日不再提示")}</Button>
        <Button variant="ghost" onClick={onSkipVersion}>{t("跳过此版本")}</Button>
      </Toolbar>
    </dialog>
  );
}

function NoticeDialog({
  notice,
  onClose,
}: {
  notice: NoticeState;
  onClose: () => void;
}) {
  const tone = noticeTone(notice.status);
  const duration = noticeDuration(tone, notice.message);
  const [paused, setPaused] = useState(false);
  const [copied, setCopied] = useState(false);
  const remaining = useRef(duration);

  useEffect(() => {
    if (paused) return;
    const startedAt = Date.now();
    const timer = window.setTimeout(onClose, Math.max(0, remaining.current));
    return () => {
      window.clearTimeout(timer);
      remaining.current -= Date.now() - startedAt;
    };
    // onClose 每次渲染都是新函数，倒计时只随暂停状态变化。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paused]);

  const Icon = tone === "error" ? CircleX : tone === "warning" ? TriangleAlert : tone === "info" ? Info : CheckCircle2;
  const copyDetails = () => {
    void navigator.clipboard?.writeText(`${notice.title}\n${notice.message}`).then(() => setCopied(true)).catch(() => undefined);
  };

  return (
    <div className="toast-wrap" role={tone === "error" ? "alert" : "status"} aria-live={tone === "error" ? "assertive" : "polite"}>
      <div
        className="toast-card"
        data-tone={tone}
        onBlur={() => setPaused(false)}
        onFocus={() => setPaused(true)}
        onMouseEnter={() => setPaused(true)}
        onMouseLeave={() => setPaused(false)}
      >
        <span className="toast-icon"><Icon aria-hidden="true" /></span>
        <div className="toast-body">
          <h2>{notice.title}</h2>
          {notice.message ? <p>{notice.message}</p> : null}
          {notice.action ? (
            <button className="toast-action" onClick={() => { notice.action?.run(); onClose(); }} type="button">
              {notice.action.label}
            </button>
          ) : null}
          {tone === "error" && notice.message ? (
            <button className="toast-action" onClick={copyDetails} type="button">
              {copied ? <CheckCircle2 aria-hidden="true" /> : <Copy aria-hidden="true" />}
              {copied ? t("已复制") : t("复制详情")}
            </button>
          ) : null}
        </div>
        <button aria-label={t("关闭提示")} className="toast-dismiss" onClick={onClose} type="button">
          <X aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

/** 切换时实时配置 endpoint 被外部改过（#2666）：让用户选导入还是放弃，不再只给报错。 */
function LiveConflictDialog({
  profileName,
  onChoose,
}: {
  profileName: string;
  onChoose: (choice: LiveConflictResolution | null) => void;
}) {
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card confirm-modal">
        <div className="modal-head">
          <div>
            <h2>{t("实时配置已被修改")}</h2>
          </div>
          <button className="toast-close" onClick={() => onChoose(null)} type="button">×</button>
        </div>
        <div className="confirm-modal-body">
          <p className="modal-message">
            {tf("当前生效的 config.toml / auth.json 里的供应商地址，和「{0}」保存的不一样。可能是其它工具改过，也可能是你手动改的，Codex++ 无法自动判断该保留哪一份。", [profileName])}
          </p>
          <p className="modal-message">{tf("导入为新供应商：把当前生效的地址和 Key 保存成一个新供应商，「{0}」保持原样，然后继续切换。", [profileName])}</p>
          <p className="modal-message">{tf("放弃实时改动：先把当前生效的文件备份一份，「{0}」保持原样，然后继续切换。", [profileName])}</p>
        </div>
        <Toolbar className="confirm-modal-actions">
          <Button onClick={() => onChoose("importAsNewProfile")}>
            <Download className="h-4 w-4" />
            {t("导入为新供应商")}
          </Button>
          <Button onClick={() => onChoose("discardLiveChanges")} variant="secondary">
            {t("放弃实时改动、继续切换")}
          </Button>
          <Button onClick={() => onChoose(null)} variant="ghost">{t("取消")}</Button>
        </Toolbar>
      </div>
    </div>
  );
}

function SessionIndexCleanupDialog({
  request,
  onConfirm,
  onCancel,
}: {
  request: { candidates: SessionIndexCleanupCandidate[] };
  onConfirm: (selectedIds: string[]) => void;
  onCancel: () => void;
}) {
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const allSelected = request.candidates.length > 0 && selectedIds.size === request.candidates.length;
  const toggleCandidate = (id: string, selected: boolean) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (selected) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card session-index-cleanup-modal">
        <div className="modal-head">
          <div>
            <h2>{t("清理幽灵任务索引")}</h2>
            <p className="modal-message">
              {tf("发现 {0} 条仅存在于 session_index.jsonl、未在本地数据库或 rollout 中找到来源的候选记录。它们也可能是云端或尚未落盘的任务，请逐项核对。任务标题仅用于预览，实际按 thread ID 与数据来源判断。清理前请先完全退出 Codex App / ChatGPT。", [request.candidates.length])}
            </p>
          </div>
          <button className="toast-close" onClick={onCancel} type="button">×</button>
        </div>
        <label className="session-index-cleanup-select-all">
          <input
            checked={allSelected}
            onChange={(event) => {
              setSelectedIds(event.target.checked ? new Set(request.candidates.map((candidate) => candidate.id)) : new Set());
            }}
            type="checkbox"
          />
          <span>{t("选择全部候选记录")}</span>
        </label>
        <div className="session-index-cleanup-list">
          {request.candidates.map((candidate) => (
            <label className="session-index-cleanup-item" key={candidate.id}>
              <input
                checked={selectedIds.has(candidate.id)}
                onChange={(event) => toggleCandidate(candidate.id, event.target.checked)}
                type="checkbox"
              />
              <span>
                <strong>{candidate.threadName || t("未命名任务")}</strong>
                <code>{candidate.id}</code>
                <small>{candidate.updatedAt}</small>
              </span>
            </label>
          ))}
        </div>
        <Toolbar>
          <Button disabled={selectedIds.size === 0} onClick={() => onConfirm(Array.from(selectedIds))}>
            <Trash2 className="h-4 w-4" />
            {tf("确认清理 {0} 条", [selectedIds.size])}
          </Button>
          <Button onClick={onCancel} variant="secondary">{t("取消")}</Button>
        </Toolbar>
      </div>
    </div>
  );
}

function PendingProviderImportDialog({
  request,
  onConfirm,
  onDismiss,
}: {
  request: ProviderImportRequest;
  onConfirm: () => void;
  onDismiss: () => void;
}) {
  const toPool = request.target === "pool";
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card provider-import-modal">
        <div className="modal-head">
          <div>
            <h2>{t("导入 Codex++ 供应商")}</h2>
            <p>{toPool ? t("检测到来自网页的供应商导入请求，确认后会加进网关供应商池，并自动从上游获取模型。") : t("检测到来自网页的供应商配置导入请求，确认后会写入本机 Codex++。")}</p>
          </div>
          <button className="toast-close" onClick={onDismiss} type="button">×</button>
        </div>
        <div className="metric-list">
          <Metric label={t("名称")} value={request.name || t("未命名供应商")} />
          <Metric label={t("导入到")} value={toPool ? t("网关供应商池") : t("直连模式供应商")} />
          <Metric label="Base URL" value={request.baseUrl || t("未填写")} />
          <Metric label={t("协议")} value={providerImportWireApiLabel(request.wireApi)} />
          {toPool ? null : <Metric label={t("模式")} value={providerImportRelayModeLabel(request.relayMode)} />}
          <Metric label="API Key" value={maskSecret(request.apiKey)} />
        </div>
        {toPool ? (
          <div className="hint-line" role="note">
            {t("导入后供应商默认启用，会按「Agent 接入」里各 Agent 的「添加供应商时自动加入」设置立即承接请求。")}
          </div>
        ) : null}
        <div className="hint-line" role="note">
          {t("安全提示：网页链接中的自定义 config.toml 和 auth.json 不会执行；管理工具只会使用上方字段生成受管配置。")}
        </div>
        <Toolbar>
          <Button onClick={onConfirm}>
            <Download className="h-4 w-4" />
            {t("确认导入")}
          </Button>
          <Button onClick={onDismiss} variant="secondary">{t("取消")}</Button>
        </Toolbar>
      </div>
    </div>
  );
}

function DreamSkinCommunityLinkDialog({
  versionId,
  installPending,
  onConfirm,
  onDismiss,
}: {
  versionId: string;
  installPending: boolean;
  onConfirm: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card provider-import-modal">
        <div className="modal-head">
          <div>
            <h2>{t("从 DreamSkin.cc 安装主题")}</h2>
            <p>{t("检测到网页一键换肤请求。确认后会从固定社区 API 下载，并在本机重新校验大小、SHA-256、ZIP 清单与 Safe CSS。")}</p>
          </div>
          <button aria-label={t("关闭")} className="toast-close" disabled={installPending} onClick={onDismiss} type="button">×</button>
        </div>
        <div className="metric-list">
          <Metric label={t("主题版本 ID")} value={versionId} />
          <Metric label={t("来源")} value="api.dreamskin.cc" />
        </div>
        <div className="hint-line" role="note">
          {t("链接不能携带任意下载地址、文件路径或命令；安装后主题会进入“我的主题”，不会自动重启 Codex。")}
        </div>
        <Toolbar>
          <Button aria-busy={installPending} disabled={installPending} onClick={onConfirm}>
            {installPending
              ? <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
              : <Download className="h-4 w-4" />}
            {installPending ? t("安装中…") : t("下载并安装")}
          </Button>
          <Button disabled={installPending} onClick={onDismiss} variant="secondary">{t("取消")}</Button>
        </Toolbar>
        {installPending ? <DreamSkinInstallProgress /> : null}
      </div>
    </div>
  );
}

function DreamSkinInstallProgress({ label = t("正在下载、校验并安装主题包…") }: { label?: string }) {
  return (
    <div aria-live="polite" className="dream-skin-install-progress" role="status">
      <span>{label}</span>
      <div aria-label={label} className="dream-skin-install-progress-bar" role="progressbar">
        <div />
      </div>
    </div>
  );
}

/** Claude 在供应商没有声明窗口时自己用的默认上下文窗口。 */
const CLAUDE_DEFAULT_CONTEXT_WINDOW = 200_000;

/**
 * 上下文窗口展示：1000000 -> `1M`、1100000 -> `1.1M`、200000 -> `200k`。
 * 复用网关用量的 `formatTokens`（B/M/K），只把这里的 K 换成小写，和模型列表里的 `[200k]` 写法一致。
 */
function formatContextWindow(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value) || value <= 0) return "";
  return formatTokens(value).replace(/K$/, "k");
}

/** 录制版本的显示：前端构建日期 + build id，例如「2026-11-14 · a4ced7f85b」；没有构建时间时只显示 build id。 */
function claudeTemplateLabel(template: ClaudeTemplateVersion): string {
  if (!template.builtAt) return template.build;
  const date = new Date(template.builtAt * 1000);
  const day = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
  return `${day} · ${template.build}`;
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

/**
 * Claude Desktop 页。
 *
 * - `mode="local"`（默认）：完整功能模式——页头的原版启动、本地状态、汉化/听写/电脑控制/后台模式；
 *   增强副本那一块搬到「补丁 Desktop」页，这一页不再提它，也不放跳转。
 * - `mode="patch"`：「补丁 Desktop」页——只渲染改过代码的 Desktop 副本（制作/更新/检查/启动、副本开关、
 *   Bash 时限）；不渲染本地状态那部分，副本块常开。
 */
function ClaudeLocalizationScreen(
  { mode = "local", onLocalStatus }: {
    mode?: "local" | "patch";
    /** 每次读到新的原版本地状态（检测、启动、停止、配置）时交给 App，侧栏的播放键要跟着变。 */
    onLocalStatus?: (status: CommandResult<ClaudeLocalStatus>) => void;
  } = {},
) {
  const patchOnly = mode === "patch";
  const [status, setStatus] = useState<CommandResult<ClaudeLocalizationStatus> | null>(claudeLocalizationMemo.status);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  /** 补丁页里副本块恒开；「Desktop 增强」页收起时就不轮询副本状态，避免白跑检测。 */
  const advancedShown = patchOnly || advancedOpen;
  const [localStatus, setLocalStatus] = useState<CommandResult<ClaudeLocalStatus> | null>(claudeLocalMemo.status);
  const [localBusy, setLocalBusy] = useState<string | null>(claudeLocalMemo.autoDetected ? null : "detect");
  const [localError, setLocalError] = useState<string | null>(null);
  const localPending = useRef(false);
  const localGeneration = useRef(0);
  /** 制作增强版时宿主推送的阶段进度（0-100）。 */
  const [progress, setProgress] = useState<{ percent: number; message: string } | null>(null);
  const mounted = useRef(true);
  const pending = useRef(false);
  const statusGeneration = useRef(0);
  const [bashTimeoutDraft, setBashTimeoutDraft] = useState<ClaudeBashTimeoutSettings | null>(
    claudeLocalizationMemo.status?.status === "ok"
      ? claudeLocalizationMemo.status.bashTimeout
      : null,
  );
  // 编辑时保留空字符串，避免清空输入框立刻补 1 后把新数字拼在后面。
  const [bashTimeoutDefaultText, setBashTimeoutDefaultText] = useState<string | null>(null);
  const [bashTimeoutMaxText, setBashTimeoutMaxText] = useState<string | null>(null);
  const bashTimeoutSaving = useRef(false);
  const bashTimeoutQueued = useRef<ClaudeBashTimeoutSettings | null>(null);
  /**
   * 页头右上角的操作槽：原版启动放在和 Codex 页「启动 Codex」相同的位置。
   * 页头与本页都按路由重建、同一次提交挂载，所以在 layout effect 里取，首帧绘制前就到位。
   */
  const [topbarSlot, setTopbarSlot] = useState<HTMLElement | null>(null);
  useLayoutEffect(() => {
    setTopbarSlot(document.querySelector<HTMLElement>("[data-topbar-actions]"));
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<{ percent: number; message: string }>("claude-localization-progress", (event) => {
      if (mounted.current) setProgress(event.payload);
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);
  const accept = (result: CommandResult<ClaudeLocalizationStatus>) => {
    // 先记下有效结果再判断挂载：检测中切走页面，回来时仍能看到这次的结果。
    if (result.status === "ok") claudeLocalizationMemo.status = result;
    if (!mounted.current) return;
    if (result.status === "ok") {
      setStatus(result);
      setBashTimeoutDraft(result.bashTimeout);
      setBashTimeoutDefaultText(null);
      setBashTimeoutMaxText(null);
    }
    else setError(result.message);
  };
  const acceptLocal = (result: CommandResult<ClaudeLocalStatus>) => {
    if (result.status === "ok") {
      claudeLocalMemo.status = result;
      // 通知 App：侧栏 Claude 行和这里共用一份状态（回调是普通函数，别塞进 effect 依赖）。
      onLocalStatus?.(result);
    }
    if (!mounted.current) return;
    if (result.status === "ok") setLocalStatus(result);
    else setLocalError(result.message);
  };
  const refreshLocal = async () => {
    if (localPending.current) return;
    localPending.current = true;
    localGeneration.current++;
    setLocalBusy("detect"); setLocalError(null);
    try { acceptLocal(await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_status")); }
    catch (cause) { if (mounted.current) setLocalError(stringifyError(cause)); }
    finally {
      localPending.current = false;
      if (mounted.current) setLocalBusy(null);
    }
  };
  useEffect(() => {
    if (claudeLocalMemo.autoDetected) return;
    claudeLocalMemo.autoDetected = true;
    void refreshLocal();
  }, []);
  const localAction = async (action: "launch" | "stop" | "trust" | "untrust" | "trust_machine" | "prepare" | "sync_sessions") => {
    if (localPending.current || pending.current) return;
    localPending.current = true;
    localGeneration.current++;
    setLocalBusy(action); setLocalError(null);
    try {
      acceptLocal(await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_action", { action }));
    } catch (cause) { if (mounted.current) setLocalError(stringifyError(cause)); }
    finally {
      localPending.current = false;
      if (mounted.current) setLocalBusy(null);
    }
  };
  const configureLocal = async (config: ClaudeLocalConfig) => {
    if (localPending.current || pending.current) return;
    localPending.current = true;
    localGeneration.current++;
    setLocalBusy("configure"); setLocalError(null);
    try {
      const result = await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_configure", { config });
      acceptLocal(result);
      if (result.status === "ok") announceBackendSettingsChanged();
    } catch (cause) { if (mounted.current) setLocalError(stringifyError(cause)); }
    finally {
      localPending.current = false;
      if (mounted.current) setLocalBusy(null);
    }
  };
  useEffect(() => {
    if ((!localStatus?.running && !localStatus?.claudeRunning) || localBusy !== null) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const generation = localGeneration.current;
    const poll = async () => {
      try {
        const result = await invoke<CommandResult<ClaudeLocalStatus>>("claude_local_status");
        if (cancelled || !mounted.current || localPending.current || generation !== localGeneration.current) return;
        if (result.status === "ok") acceptLocal(result);
      } catch { /* 状态探测失败保留上次结果，手动检测仍可显示具体错误。 */ }
      if (!cancelled && mounted.current && generation === localGeneration.current) {
        timer = setTimeout(() => void poll(), 3000);
      }
    };
    timer = setTimeout(() => void poll(), 3000);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [localStatus?.running, localStatus?.claudeRunning, localBusy]);
  const refresh = async () => {
    if (pending.current) return;
    pending.current = true;
    statusGeneration.current++;
    setBusy("detect"); setError(null);
    try { accept(await invoke<CommandResult<ClaudeLocalizationStatus>>("claude_localization_status")); }
    catch (cause) { if (mounted.current) setError(stringifyError(cause)); }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  useEffect(() => {
    if (!advancedShown || claudeLocalizationMemo.autoDetected) return;
    claudeLocalizationMemo.autoDetected = true;
    void refresh();
  }, [advancedShown]);
  /**
   * Claude Desktop 是否已在 Computer Use 页接入 codex-plus-cu（含自定义目录）。
   * 和这里的电脑控制同时开会让同一个对话里出现两套工具，用来提示二选一；读不到时按未接入处理。
   */
  const [claudeDesktopCuConnected, setClaudeDesktopCuConnected] = useState(false);
  useEffect(() => {
    const connected = (state?: string) => state === "registered" || state === "needs_update";
    void invoke<{ targets?: Record<string, { state?: string }>; customTargets?: Array<{ kind?: string; state?: string }> }>("computer_use_status")
      .then((result) => {
        if (!mounted.current) return;
        setClaudeDesktopCuConnected(connected(result.targets?.claudeDesktop?.state)
          || Boolean(result.customTargets?.some((target) => target.kind === "claudeDesktop" && connected(target.state))));
      })
      .catch(() => undefined);
  }, []);
  const act = async (action: "prepare" | "launch" | "restore" | "check") => {
    if (pending.current || localPending.current) return false;
    pending.current = true;
    statusGeneration.current++;
    setBusy(action); setError(null);
    const launching = action === "launch" || action === "restore";
    const clearLaunchNotice = () => {
      setStatus((current) => current?.status === "ok"
        ? { ...current, bashTimeoutWarning: null, bashTimeoutPending: false } : current);
    };
    if (launching) {
      const timeout = bashTimeoutDraft ?? (status?.status === "ok" ? status.bashTimeout : null);
      const awaitingConfirmation = Boolean(timeout?.enabled && (timeout.defaultEnabled || timeout.maxEnabled));
      setStatus((current) => current?.status === "ok" ? {
        ...current,
        bashTimeoutWarning: awaitingConfirmation ? "已准备，暂未确认。将在补丁版启动后自动确认。" : null,
        bashTimeoutPending: awaitingConfirmation,
      } : current);
    }
    if (action === "prepare") setProgress({ percent: 0, message: t("准备开始…") });
    let succeeded = false;
    try {
      const result = await invoke<CommandResult<ClaudeLocalizationStatus>>("claude_localization_action", { action });
      // 失败响应里的 data 是空对象：直接覆盖状态会让下面的渲染读到不存在的字段
      // （曾经就是这样整页白屏的），所以只有成功时才更新状态。
      if (launching && result.status !== "ok") clearLaunchNotice();
      accept(result);
      succeeded = result.status === "ok";
    } catch (cause) { if (mounted.current) { if (launching) clearLaunchNotice(); setError(stringifyError(cause)); } }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
      if (action === "prepare") setTimeout(() => { if (mounted.current) setProgress(null); }, 1500);
    }
    return succeeded;
  };
  /// 补丁版的三个开关（汉化、语音输入、Computer Use）共用：一次只跑一个，成功时才更新状态。
  const setClaudeFlag = async (
    key: "localization" | "voiceInput" | "computerUse",
    command: "claude_localization_set_localization" | "claude_localization_set_voice_input" | "claude_localization_set_computer_use",
    enabled: boolean,
  ) => {
    if (pending.current || localPending.current) return;
    pending.current = true;
    statusGeneration.current++;
    setBusy(key); setError(null);
    try {
      const result = await invoke<CommandResult<ClaudeLocalizationStatus>>(command, { enabled });
      // 失败响应里的 data 是空对象：直接覆盖状态会让下面的渲染读到不存在的字段
      // （曾经就是这样整页白屏的），所以只有成功时才更新状态。
      accept(result);
    } catch (cause) { if (mounted.current) setError(stringifyError(cause)); }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  const toggleVoiceInput = (enabled: boolean) => setClaudeFlag("voiceInput", "claude_localization_set_voice_input", enabled);
  const toggleComputerUse = (enabled: boolean) => setClaudeFlag("computerUse", "claude_localization_set_computer_use", enabled);
  const saveBashTimeout = async (next: ClaudeBashTimeoutSettings) => {
    if (localPending.current || (pending.current && !bashTimeoutSaving.current)) return;
    bashTimeoutQueued.current = next;
    setBashTimeoutDraft(next);
    if (bashTimeoutSaving.current) return;
    let confirmed = status?.status === "ok" ? status.bashTimeout : null;
    pending.current = true;
    statusGeneration.current++;
    bashTimeoutSaving.current = true;
    setBusy("bashTimeout"); setError(null);
    try {
      // 失焦保存先于开关点击；串行处理后续草稿，避免吞掉这次点击。
      while (bashTimeoutQueued.current) {
        const timeout = bashTimeoutQueued.current;
        bashTimeoutQueued.current = null;
        const result = await invoke<CommandResult<ClaudeLocalizationStatus>>(
          "claude_localization_set_bash_timeout",
          { timeout },
        );
        if (result.status !== "ok") throw new Error(result.message);
        confirmed = result.bashTimeout;
        claudeLocalizationMemo.status = result;
        if (mounted.current) setStatus(result);
        if (!bashTimeoutQueued.current) accept(result);
      }
    } catch (cause) {
      bashTimeoutQueued.current = null;
      if (mounted.current) {
        setBashTimeoutDraft(confirmed);
        setError(stringifyError(cause));
      }
    }
    finally {
      bashTimeoutSaving.current = false;
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  const toggleLocalization = (enabled: boolean) => setClaudeFlag("localization", "claude_localization_set_localization", enabled);
  const s = status;
  const bashTimeout = bashTimeoutDraft ?? s?.bashTimeout;
  useEffect(() => {
    if (!advancedShown || !s?.bashTimeoutPending || !s.running || busy !== null) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const generation = statusGeneration.current;
    const poll = async () => {
      try {
        const result = await invoke<CommandResult<{ warning: string | null; pending: boolean }>>(
          "claude_localization_bash_timeout_status",
        );
        if (cancelled || !mounted.current || pending.current || statusGeneration.current !== generation) return;
        if (result.status === "ok") {
          // 只同步提示，不覆盖正在编辑的秒数或其他设置草稿。
          const update = { bashTimeoutWarning: result.warning, bashTimeoutPending: result.pending };
          setStatus((current) => current?.status === "ok" ? { ...current, ...update } : current);
          if (claudeLocalizationMemo.status?.status === "ok") {
            claudeLocalizationMemo.status = { ...claudeLocalizationMemo.status, ...update };
          }
          if (!result.pending) return;
        }
      } catch { /* 临时读取失败保留提示，下一轮继续确认。 */ }
      if (!cancelled && mounted.current && statusGeneration.current === generation) {
        timer = setTimeout(() => void poll(), 1000);
      }
    };
    timer = setTimeout(() => void poll(), 1000);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [advancedShown, s?.bashTimeoutPending, s?.running, busy]);
  // 只有明确点击可选的增强版启动，才允许制作副本。
  const needsPrepare = Boolean(s && (!s.prepared || s.needsRebuild));
  const canLaunch = Boolean(s?.supported && s.nodeAvailable && (s.prepared || needsPrepare));
  const launchEnhanced = async () => {
    if (needsPrepare && !(await act("prepare"))) return;
    await act("launch");
  };
  const { CxBadge, CxCard, CxRow, CxSection, CxStatusIcon } = AgentCx;
  const statusTone = !s ? "pending" : s.prepared ? (s.needsRebuild ? "warn" : "ok") : "idle";
  const statusText = busy === "detect" ? t("检测中…") : !s ? t("未检测") : !s.supported ? t("未检测到 Claude") : s.prepared ? (s.needsRebuild ? t("需要更新") : t("已准备")) : t("未准备");
  const meta = [
    tf("官方版本 {0}", [!s ? t("未检测") : s.sourceVersion || t("未检测到")]),
    tf("副本版本 {0}", [s?.copyVersion || t("未准备")]),
    tf("外壳语言 {0}", [s?.shellLocale || t("未设置")]),
    s?.catalogEntries != null ? tf("官方词条 {0} 条", [s.catalogEntries.toLocaleString()]) : tf("官方词条 {0}", [t("未检测")]),
  ].join(" · ");
  const prepareLabel = busy === "prepare"
    ? t("制作中…")
    : s?.needsRebuild
      ? t("更新副本")
      : s?.prepared
        ? t("重新制作补丁版")
        : t("制作补丁版");
  const prepareHint = t("首次制作会创建独立副本并执行完整性校验；官方应用、登录态和 app.asar 不会被改写。macOS 可能需要钥匙串授权，Windows 会使用独立副本启动。");
  const local = localStatus;
  const localState = !local ? t("未检测")
    : local.connected ? t("本地模式已连接")
      : local.claudeRunning ? (local.config.enabled ? t("原版运行，未连接本地服务") : t("原版已运行"))
        : local.running ? t("本地服务已就绪") : t("未启动");
  const localTone = local?.connected ? "ok" : local?.config.enabled && local?.claudeRunning ? "warn" : "neutral";
  const canLaunchOriginal = Boolean(local?.supported && (!local.config.enabled || local.nodeAvailable));
  const localDisabled = localBusy !== null || busy !== null;
  const localTemplates = local?.templates ?? [];
  const localDetail = local
    ? (local.supported ? tf("原版版本 {0}", [local.version || t("未检测到")]) : t("未检测到支持的 Claude Desktop"))
      + ` · ${local.contextWindow != null && local.contextWindow > 0
        ? tf("上下文窗口 {0}（供应商声明）", [formatContextWindow(local.contextWindow)])
        : t("上下文窗口 未声明（Claude 按 200k）")}`
    : t("正在读取原版 Claude 状态");
  const defaultModelWindow = local?.contextWindow != null && local.contextWindow > 0
    ? formatContextWindow(local.contextWindow)
    : formatContextWindow(CLAUDE_DEFAULT_CONTEXT_WINDOW);
  return (
    <div className="claude-localization-page cx-page">
      {!patchOnly && topbarSlot
        ? createPortal(
            <Button
              disabled={localDisabled || !canLaunchOriginal}
              onClick={() => void localAction("launch")}
              title={t("启动原版 Claude Desktop")}
            >
              <Rocket className="h-4 w-4" />
              {localBusy === "launch" ? t("启动中…") : t("启动 Claude")}
            </Button>,
            topbarSlot,
          )
        : null}
      {!patchOnly && localError ? <div className="claude-l10n-error" role="alert">{localError}</div> : null}
      {!patchOnly && local?.warning ? <div className="claude-l10n-warning" role="status">{local.warning}</div> : null}
      {!patchOnly && local?.needsRestart ? <p className="claude-l10n-hint" role="status">{t("设置已保存，完全退出 Claude 后重新启动。")}</p> : null}
      {!patchOnly ? <CxCard className="claude-l10n-hero">
        <CxRow
          badges={<CxBadge tone={localTone}>{localBusy === "detect" ? t("检测中…") : localState}</CxBadge>}
          detail={localDetail}
          icon={<CxStatusIcon tone={local?.connected ? "ok" : "off"}><Rocket /></CxStatusIcon>}
          title="Claude Desktop"
        >
          <div className="claude-l10n-actions">
            {local?.running ? (
              <button className="cx-secondary" disabled={localDisabled} onClick={() => void localAction("stop")} type="button">
                <Square />{t("停止本地服务")}
              </button>
            ) : null}
            <button className="cx-secondary" disabled={localDisabled} onClick={() => void refreshLocal()} type="button">
              <RefreshCw />{t("检测原版 Claude")}
            </button>
          </div>
        </CxRow>
      </CxCard> : null}
      {/* 窗口比默认模型小的模型不会列给 Claude（Claude 按默认模型的窗口压所有模型）；说清楚它们去哪了。 */}
      {!patchOnly ? <CxSection className="claude-local-settings" title={t("启动设置")}>
        <CxCard>
          <CxRow detail={t("从这里启动后生效；已打开的 Claude 需完全退出重开。")} title={t("启动模式")}>
            <AppSelect
              ariaLabel={t("启动模式")}
              disabled={localDisabled || !local?.supported}
              onChange={(next) => {
                if (local) void configureLocal({ ...local.config, enabled: next === "full" });
              }}
              options={[
                { value: "3p", label: t("3p 模式") },
                { value: "full", label: t("完整功能模式") },
              ]}
              value={local?.config.enabled ? "full" : "3p"}
            />
          </CxRow>
          {/* 模型跟着「Agent 接入」里的 Claude Desktop 绑定走：这里只说明，不放第二个入口。 */}
          <CxRow detail={t("模型在「Agent 接入」里的 Claude Desktop 行选择，改完需要重新启动本地服务。")} title={t("默认模型")} />
          <CxRow
            detail={t("默认只列窗口不小于默认模型的模型（Claude 的窗口是启动时一份全局值，切到更小的模型不会跟着变小）；打开后池里的聊天模型全列出来，代价是切到小窗口模型可能超上限。")}
            title={t("列出全部模型")}
          >
            <label className="switch-row compact">
              <input
                aria-label={t("列出全部模型")}
                checked={Boolean(local?.config.listAllModels)}
                disabled={localDisabled || !local?.supported}
                onChange={(event) => {
                  if (local) void configureLocal({ ...local.config, listAllModels: event.currentTarget.checked });
                }}
                type="checkbox"
              />
              <span>{local?.config.listAllModels ? t("已开启") : t("已关闭")}</span>
              <ToggleVisual />
            </label>
          </CxRow>
          <CxRow
            detail={t("Claude 自带的 Opus、Sonnet、Haiku 等条目不对应任何供应商，选了也会被换成默认模型。打开后选择器里只留供应商池里的模型；改完需要重新启动本地服务。")}
            title={t("隐藏无供应商的模型")}
          >
            <label className="switch-row compact">
              <input
                aria-label={t("隐藏无供应商的模型")}
                checked={local?.config.hideUnroutedModels ?? true}
                disabled={localDisabled || !local?.supported}
                onChange={(event) => {
                  if (local) void configureLocal({ ...local.config, hideUnroutedModels: event.currentTarget.checked });
                }}
                type="checkbox"
              />
              <span>{local?.config.hideUnroutedModels ?? true ? t("已开启") : t("已关闭")}</span>
              <ToggleVisual />
            </label>
          </CxRow>
          <CxRow detail={t("只改 Claude 里显示的账号信息，从这里启动后生效。")} title={t("用户名")}>
            <Input
              aria-label={t("用户名")}
              defaultValue={local?.config.displayName ?? ""}
              disabled={localDisabled || !local?.supported}
              key={local?.config.displayName ?? ""}
              onBlur={(event) => {
                if (local && event.currentTarget.value !== (local.config.displayName ?? "")) void configureLocal({ ...local.config, displayName: event.currentTarget.value });
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
              placeholder={t("Local User")}
            />
          </CxRow>
          <CxRow detail={t("只改 Claude 里显示的账号邮箱，从这里启动后生效。")} title={t("邮箱")}>
            <Input
              aria-label={t("邮箱")}
              defaultValue={local?.config.email ?? ""}
              disabled={localDisabled || !local?.supported}
              key={local?.config.email ?? ""}
              onBlur={(event) => {
                if (local && event.currentTarget.value !== (local.config.email ?? "")) void configureLocal({ ...local.config, email: event.currentTarget.value });
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
              placeholder="local@claude.local"
              type="email"
            />
          </CxRow>
          <CxRow title={t("套餐")}>
            <AppSelect
              ariaLabel={t("套餐")}
              disabled={localDisabled || !local?.supported}
              onChange={(next) => {
                if (local) void configureLocal({ ...local.config, plan: next });
              }}
              options={[
                { value: "free", label: "Free" },
                { value: "pro", label: "Pro" },
                { value: "max", label: "Max" },
                { value: "ultra", label: "Ultra" },
              ]}
              value={local?.config.plan || "ultra"}
            />
          </CxRow>
          <CxRow
            detail={(local?.config.codeShareHome ?? true)
              ? t("Code 页签和命令行的 Claude Code 共用 ~/.claude：会话、MCP、插件是同一份；~/.claude/settings.json 里的接口地址（env）会优先于网关。从这里启动后生效。")
              : t("Code 页签用独立目录，请求走 Codex++ 网关；平时在命令行 / 3p 模式建的会话要靠会话同步才能打开。")}
            title={t("和命令行共用 ~/.claude")}
          >
            <label className="switch-row compact">
              <input
                aria-label={t("和命令行共用 ~/.claude")}
                checked={local?.config.codeShareHome ?? true}
                disabled={localDisabled || !local?.supported}
                onChange={(event) => {
                  if (local) void configureLocal({ ...local.config, codeShareHome: event.currentTarget.checked });
                }}
                type="checkbox"
              />
              <span>{(local?.config.codeShareHome ?? true) ? t("已开启") : t("已关闭")}</span>
              <ToggleVisual />
            </label>
          </CxRow>
          <CxRow
            detail={t("启动 Claude 前，把终端、BaoCode 等命令行里建的会话加进左侧列表（已在列表里的、删过的不会重复加）。")}
            title={t("显示命令行会话")}
          >
            <label className="switch-row compact">
              <input
                aria-label={t("显示命令行会话")}
                checked={local?.config.importCliSessions ?? true}
                disabled={localDisabled || !local?.supported}
                onChange={(event) => {
                  if (local) void configureLocal({ ...local.config, importCliSessions: event.currentTarget.checked });
                }}
                type="checkbox"
              />
              <span>{(local?.config.importCliSessions ?? true) ? t("已开启") : t("已关闭")}</span>
              <ToggleVisual />
            </label>
          </CxRow>
          <CxRow detail={t("启动 Claude 前同步 3p 模式与完整功能模式的 Code 会话")} title={t("会话同步")}>
            <div className="claude-l10n-actions">
              <label className="switch-row compact">
                <input
                  aria-label={t("会话同步")}
                  checked={local?.config.sessionSync === true}
                  disabled={localDisabled || !local?.supported}
                  onChange={(event) => {
                    if (local) void configureLocal({ ...local.config, sessionSync: event.currentTarget.checked });
                  }}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
              <button className="cx-secondary" disabled={localDisabled || !local?.supported || local?.claudeRunning} onClick={() => void localAction("sync_sessions")} title={t("立即同步")} type="button">{t("立即同步")}</button>
            </div>
          </CxRow>
        </CxCard>
        {local?.config.enabled ? (
          <>
            <CxCard>
              <CxRow badges={<CxBadge tone={local.caTrusted ? "ok" : "neutral"}>{local.caTrusted ? t("已信任") : local.caReady ? t("未信任") : t("未准备")}</CxBadge>} title={t("本地证书")}>
                <div className="claude-l10n-actions">
                  {!local.caReady ? (
                    <button className="cx-secondary" disabled={localDisabled} onClick={() => void localAction("prepare")} type="button"><PackageOpen />{t("准备证书")}</button>
                  ) : null}
                  {!local.caTrusted ? (
                    <button className="cx-secondary" disabled={localDisabled || !local.caReady} onClick={() => void localAction("trust")} type="button"><ShieldCheck />{t("信任证书")}</button>
                  ) : (
                    <button className="cx-secondary" disabled={localDisabled} onClick={() => void localAction("untrust")} type="button"><ShieldCheck />{t("取消信任证书")}</button>
                  )}
                </div>
              </CxRow>
              {/* Cowork 虚拟机以系统服务运行，只读系统级根证书：只有 Windows 需要，Cowork 关着时用不上。 */}
              {(local.config.cowork ?? true) && local.caMachineSupported ? (
                <CxRow
                  badges={<CxBadge tone={local.caMachineTrusted ? "ok" : "neutral"}>{local.caMachineTrusted ? t("已安装") : t("未安装")}</CxBadge>}
                  detail={t("Cowork 虚拟机以系统服务运行，只读取系统级根证书；安装需要管理员授权。")}
                  title={t("系统证书")}
                >
                  {!local.caMachineTrusted ? (
                    <div className="claude-l10n-actions">
                      <button className="cx-secondary" disabled={localDisabled} onClick={() => void localAction("trust_machine")} type="button"><ShieldCheck />{t("安装系统证书")}</button>
                    </div>
                  ) : null}
                </CxRow>
              ) : null}
              <CxRow badges={<CxBadge tone={local.dictationReady ? "ok" : "neutral"}>{local.dictationReady ? t("已就绪") : t("未就绪")}</CxBadge>} title={t("语音识别")} />
              <CxRow badges={<CxBadge tone={local.nodeAvailable ? "ok" : "warn"}>{local.nodeAvailable ? t("已就绪") : t("缺少 Node.js")}</CxBadge>} title={t("运行环境")} />
              {localTemplates.length > 1 ? (
                <CxRow
                  detail={t("录制 claude.ai 界面时的版本。新版本有问题时可以换回旧的；前端代码按所选版本从 Anthropic 下载。")}
                  title={t("界面版本")}
                >
                  <AppSelect
                    ariaLabel={t("界面版本")}
                    disabled={localDisabled}
                    menuWidth={300}
                    onChange={(next) => void configureLocal({ ...local.config, template: next })}
                    options={[
                      { value: "", label: tf("最新（{0}）", [claudeTemplateLabel(localTemplates[0])]) },
                      ...localTemplates.map((template) => ({ value: template.build, label: claudeTemplateLabel(template) })),
                    ]}
                    value={localTemplates.some((template) => template.build === local.config.template) ? local.config.template ?? "" : ""}
                  />
                </CxRow>
              ) : null}
            </CxCard>
            <CxCard className="claude-l10n-feature-grid">
              {([
                ["localization", "界面汉化", "本地界面汉化", ""],
                ["voiceInput", "原生听写", "本地原生听写", ""],
                ["computerUse", "电脑控制", "本地电脑控制", ""],
                ["backgroundMode", "后台模式", "本地后台模式", ""],
                ["forceBackgroundMode", "新系统也开后台模式", "本地后台模式不限系统版本", "Claude 在较新的 macOS 上会提示「暂不支持后台模式」；打开后不再限制系统版本，未经 Claude 验证，可能不稳定"],
                ["cowork", "Cowork", "本地 Cowork", "关掉后按 Claude 自己的开关，新版 Claude 通常只剩 Chat / Code。系统条件（Windows 需商店版、虚拟化可用）仍由 Claude 检查"],
                ["hubHome", "合并式主页（实验）", "本地合并式主页", "Claude 正在小范围试验的主页布局：对话和 Cowork 合在一个输入框里，没有「对话 | Cowork」切换。关着时用有切换的经典主页"],
              ] as const).map(([key, label, ariaLabel, hint]) => (
                <label
                  className="switch-row compact"
                  key={key}
                  title={hint ? t(hint) : undefined}
                >
                  <input
                    aria-label={t(ariaLabel)}
                    // 只有 Cowork 默认是开的。
                    checked={local.config[key] ?? key === "cowork"}
                    disabled={localDisabled}
                    onChange={(event) => {
                      const checked = event.currentTarget.checked;
                      // 关后台模式时一并关掉「新系统也开」，否则保存时会被它重新打开。
                      const extra = key === "backgroundMode" && !checked ? { forceBackgroundMode: false } : {};
                      void configureLocal({ ...local.config, [key]: checked, ...extra });
                    }}
                    type="checkbox"
                  />
                  <span><strong>{t(label)}</strong></span>
                  <ToggleVisual />
                </label>
              ))}
            </CxCard>
          </>
        ) : null}
      </CxSection> : null}
      {patchOnly || advancedOpen ? <section className={`claude-local-advanced${patchOnly ? " claude-local-advanced-page" : ""}`}>
        <fieldset className="claude-local-advanced-body" disabled={localBusy !== null}>
      {/* 顶部最多几条一次性的提示：检测中 / 未安装、错误、Bash 时限确认、制作进度。 */}
      {!s ? (
        <p className="claude-l10n-hint">
          {busy === "detect"
            ? t("正在读取 Claude 安装状态，只读检测，不会修改应用。")
            : t("没有读到安装状态，点击“检测 Claude”重试；检测不会修改应用。")}
        </p>
      ) : null}
      {error ? <div className="claude-l10n-error" role="alert">{error}</div> : null}
      {bashTimeout?.enabled && s?.bashTimeoutWarning ? (
        <div className="claude-l10n-warning" role="status">{t("Bash 时限补丁：")} {t(s.bashTimeoutWarning)}</div>
      ) : null}
      {progress ? (
        <div
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={progress.percent}
          className="claude-l10n-progress"
          role="progressbar"
        >
          <div className="claude-l10n-progress-head">
            <span>{progress.message || t("处理中…")}</span>
            <span className="claude-l10n-progress-pct">{progress.percent}%</span>
          </div>
          <div className="claude-l10n-progress-track">
            <div
              className="claude-l10n-progress-fill"
              style={{ width: `${Math.min(100, Math.max(2, progress.percent))}%` }}
            />
          </div>
        </div>
      ) : null}

      {/* 增强版状态一行：版本与词表收成一行小字；制作 / 检测 / 检查在右侧，启动在页头右上角。 */}
      <CxCard className="claude-l10n-hero">
        <CxRow
          badges={<CxBadge tone={statusTone === "ok" ? "ok" : statusTone === "warn" ? "warn" : "neutral"}>{statusText}</CxBadge>}
          detail={<span title={t("所有补丁的前置条件：官方 Claude.app 与 app.asar 保持原样，补丁通过副本和启动时注入生效。")}>{s && !s.supported
            ? t("未检测到 Claude，请先安装官方应用。你可以继续使用 Codex++ 的其他功能。")
            : s?.supported && !s.prepared ? t("首次启动会先制作独立副本；官方 Claude.app 与 app.asar 不改动") : meta}</span>}
          icon={<CxStatusIcon tone={statusTone === "ok" ? "ok" : statusTone === "warn" ? "warn" : "off"}><PackageOpen /></CxStatusIcon>}
          title={t("补丁版 Claude")}
        >
          {/* 增强副本仅在这里手动制作和启动；检查与重新检测收进菜单。 */}
          <div className="claude-l10n-actions">
            <button className="cx-secondary" disabled={busy !== null || localBusy !== null || !canLaunch} onClick={() => void launchEnhanced()} title={needsPrepare ? t("制作副本并启动补丁版") : t("启动补丁版")} type="button">
              <Rocket />{busy === "launch" ? t("启动中…") : busy === "prepare" ? t("制作中…") : t("启动补丁版")}
            </button>
            {needsPrepare || !s ? (
              <button className="cx-secondary" disabled={busy !== null || !s?.supported} onClick={() => void act("prepare")} title={prepareHint} type="button">
                <PackageOpen />
                {prepareLabel}
              </button>
            ) : null}
            <Menu>
              {needsPrepare || !s ? null : (
                <MenuItem disabled={busy !== null || !s?.supported} onClick={() => void act("prepare")}>{prepareLabel}</MenuItem>
              )}
              <MenuItem disabled={busy !== null || !s?.prepared} onClick={() => void act("check")}>{t("检查补丁版")}</MenuItem>
              <MenuItem disabled={busy !== null} onClick={() => void refresh()}>{busy === "detect" ? t("检测中…") : t("检测 Claude")}</MenuItem>
            </Menu>
          </div>
        </CxRow>
      </CxCard>
      {/* 增强开关一节：每项一行，各自独立、即点即存，重启增强版后应用。 */}
      <CxSection
        aside={s?.supported && !s.prepared ? (
          <span className="claude-l10n-section-note">{t("补丁版还没制作：开关会先存下来，制作并启动补丁版后生效。")}</span>
        ) : (
          <span className="claude-l10n-section-note" title={t("各自独立开关，互不影响；副本只是它们共同的前置条件，改动在重启副本后应用。")}>{t("重启补丁版后应用")}</span>
        )}
        className="claude-l10n-features"
        title={t("补丁项")}
      >
        <CxCard className="claude-l10n-feature-grid">
          <label className="switch-row compact">
            <input
              checked={Boolean(s?.localization)}
              disabled={busy !== null || !s?.supported}
              onChange={(event) => void toggleLocalization(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("界面汉化")}</strong>
              <small>{t("启动时注入官方中文词表，界面语言切到简体中文；关掉后副本按原语言启动。")}</small>
            </span>
            <ToggleVisual />
          </label>
          <label className="switch-row compact">
            <input
              checked={Boolean(s?.voiceInput)}
              disabled={busy !== null || !s?.supported}
              onChange={(event) => void toggleVoiceInput(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("语音输入")}</strong>
              <small>{t("在 Claude 输入框右下角加一个麦克风入口，识别结果只写进输入框、不自动发送。")}</small>
            </span>
            <ToggleVisual />
          </label>
          <label className="switch-row compact">
            <input
              checked={Boolean(s?.computerUse)}
              disabled={busy !== null || !s?.supported || s?.computerUseSupported === false}
              onChange={(event) => void toggleComputerUse(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("Computer Use")}</strong>
              <small title={s?.computerUseSupported === false
                ? t("暂仅支持 macOS：Windows 的副本构建流程不会改写 app.asar，打开这个开关不会有任何效果。")
                : `${t("解锁桌面控制：订阅与灰度校验已解除，API Key（3p）模式同样可用；会改写副本的 app.asar 并关闭 Electron 的 asar 完整性校验，原件不动。首次启动补丁版会提示授权辅助功能与屏幕录制。")}\n${t("这里的电脑控制和「Computer Use」页给 Claude Desktop 接入的 codex-plus-cu 是两套独立实现，同一个 Claude Desktop 里二选一即可，不要同时开。")}`}
              >{s?.computerUseSupported === false
                ? t("暂仅支持 macOS：Windows 的副本构建流程不会改写 app.asar，打开这个开关不会有任何效果。")
                : t("解锁桌面控制：订阅与灰度校验已解除，API Key（3p）模式同样可用；会改写副本的 app.asar 并关闭 Electron 的 asar 完整性校验，原件不动。首次启动补丁版会提示授权辅助功能与屏幕录制。")}</small>
            </span>
            <ToggleVisual />
          </label>
          {s?.computerUseSupported !== false ? (
            s?.computerUse && claudeDesktopCuConnected ? (
              <p className="claude-l10n-hint is-warning" role="alert">
                {t("Claude Desktop 同时接入了「Computer Use」页的 codex-plus-cu：同一个对话里会有两套电脑控制，Claude 可能混着调用、互相抢前台，同一个应用也要确认两次。建议二选一：保留这里的开关，就到「Computer Use」页撤销 Claude Desktop 的接入；或者关掉这里的开关。")}
              </p>
            ) : null
          ) : null}
          <label className="switch-row compact claude-bash-timeout-master">
            <input
              aria-label={t("解锁 Bash 时间限制")}
              checked={Boolean(bashTimeout?.enabled)}
              disabled={(busy !== null && busy !== "bashTimeout") || !bashTimeout}
              onChange={(event) => {
                if (bashTimeout) void saveBashTimeout({ ...bashTimeout, enabled: event.currentTarget.checked });
              }}
              type="checkbox"
            />
            <span>
              <strong>{t("解锁 Bash 时间限制")}</strong>
              <small title={t("配置 Claude Code Bash 的默认值与最大值；Cowork 还需启动注入成功且模块版本兼容。适配失败时提示并保持该项原生时限，不影响其他补丁。重启补丁版后应用。")}>{t("配置 Claude Code Bash 的默认值与最大值；Cowork 还需启动注入成功且模块版本兼容。适配失败时提示并保持该项原生时限，不影响其他补丁。重启补丁版后应用。")}</small>
            </span>
            <ToggleVisual />
          </label>
          {bashTimeout?.enabled ? (
            <div className="claude-bash-timeout-editor">
              <p>{t("两项可分别启停，单位均为秒。修改数字后离开输入框即保存。")}</p>
              <label className="switch-row compact claude-bash-timeout-toggle">
                <input
                  aria-label={t("修改默认等待时间")}
                  checked={bashTimeout.defaultEnabled}
                  disabled={busy !== null && busy !== "bashTimeout"}
                  onChange={(event) => void saveBashTimeout({
                    ...bashTimeout,
                    defaultEnabled: event.currentTarget.checked,
                  })}
                  type="checkbox"
                />
                <span>
                  <strong>{t("修改默认等待时间")}</strong>
                  <small>{t("未显式传 timeout / timeout_ms 时使用；Claude Code 与本地 Cowork Bash 原生默认 120 秒。")}</small>
                </span>
                <ToggleVisual />
              </label>
              <div className="claude-bash-timeout-value">
                <span>{t("默认等待时间")}</span>
                <Input
                  aria-label={t("默认等待时间（秒）")}
                  disabled={busy !== null || !bashTimeout.defaultEnabled}
                  max={2147423}
                  min={1}
                  onChange={(event) => {
                    setBashTimeoutDefaultText(event.currentTarget.value);
                    const seconds = clampNumber(Number(event.currentTarget.value), 1, 2147423);
                    setBashTimeoutDraft((current) => current ? { ...current, defaultSeconds: seconds } : current);
                  }}
                  onBlur={() => {
                    setBashTimeoutDefaultText(null);
                    void saveBashTimeout(bashTimeout);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") event.currentTarget.blur();
                  }}
                  type="number"
                  value={bashTimeoutDefaultText ?? bashTimeout.defaultSeconds}
                />
                <span>{t("秒")}</span>
              </div>
              <label className="switch-row compact claude-bash-timeout-toggle">
                <input
                  aria-label={t("修改最大等待时间")}
                  checked={bashTimeout.maxEnabled}
                  disabled={busy !== null && busy !== "bashTimeout"}
                  onChange={(event) => void saveBashTimeout({
                    ...bashTimeout,
                    maxEnabled: event.currentTarget.checked,
                  })}
                  type="checkbox"
                />
                <span>
                  <strong>{t("修改最大等待时间")}</strong>
                  <small>{t("显式传 timeout / timeout_ms 时的上限；原生参数上限 600 秒，Cowork 还受外层等待时限约束。")}</small>
                </span>
                <ToggleVisual />
              </label>
              <div className="claude-bash-timeout-value">
                <span>{t("最大等待时间")}</span>
                <Input
                  aria-label={t("最大等待时间（秒）")}
                  disabled={busy !== null || !bashTimeout.maxEnabled}
                  max={2147423}
                  min={1}
                  onChange={(event) => {
                    setBashTimeoutMaxText(event.currentTarget.value);
                    const seconds = clampNumber(Number(event.currentTarget.value), 1, 2147423);
                    setBashTimeoutDraft((current) => current ? { ...current, maxSeconds: seconds } : current);
                  }}
                  onBlur={() => {
                    setBashTimeoutMaxText(null);
                    void saveBashTimeout(bashTimeout);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") event.currentTarget.blur();
                  }}
                  type="number"
                  value={bashTimeoutMaxText ?? bashTimeout.maxSeconds}
                />
                <span>{t("秒")}</span>
              </div>
            </div>
          ) : null}
        </CxCard>
      </CxSection>
        </fieldset>
      </section> : null}
    </div>
  );
}



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

function settingsNavEntries(focus: AgentFocus, codexAvailable: boolean): SettingsNavEntry[] {
  const entries: SettingsNavEntry[] = [
    { key: "general", label: t("通用"), icon: Settings, route: "settings", sections: ["app-settings-general"], group: "app", keywords: "theme mode tour 主题 模式 引导 托盘" },
    { key: "config", label: t("配置文件"), icon: FileCode2, route: "settings", sections: ["app-settings-config"], group: "app", keywords: "backup import export 备份 导入 导出 路径" },
    { key: "doctor", label: t("配置体检"), icon: Stethoscope, route: "settings", sections: ["app-settings-doctor"], group: "app", keywords: "doctor ai config check health 体检 检查 诊断 配置" },
    { key: "update", label: t("版本与更新"), icon: Download, route: "settings", sections: ["app-settings-update"], group: "app", keywords: "update version 更新 版本" },
    { key: "logs", label: t("日志与诊断"), icon: ScrollText, route: "settings", sections: ["app-settings-logs", "app-settings-diagnostics"], group: "app", keywords: "logs diagnostics 日志 诊断" },
    { key: "about", label: t("关于"), icon: Info, route: "settings", sections: ["app-settings-about"], group: "app", keywords: "about github 关于" },
    { key: "codex", label: "Codex", icon: AppWindow, route: "agentSettings", group: "agent", keywords: "codex launch args takeover 启动参数 接管 维护 修复" },
    { key: "gateway", label: t("网关"), icon: SlidersHorizontal, route: "gatewaySettings", group: "agent", keywords: "gateway port key 网关 端口 监听" },
    { key: "cache", label: t("缓存清理"), icon: Trash2, route: "agentCache", group: "tools", keywords: "cache clean 缓存 清理" },
    { key: "computerUse", label: "Computer Use", icon: MousePointerClick, route: "computerUse", group: "beta", keywords: "computer use mcp" },
    { key: "agentCollaboration", label: t("Agent 协作"), icon: GitBranch, route: "agentCollaboration", group: "beta", keywords: "collaboration 协作" },
  ];
  return entries.filter((entry) => routeVisible(focus, entry.route) && (entry.key !== "codex" || codexAvailable));
}
/** 设置分类当前是哪一个：整体设置页按显示的卡片判断，其余按路由。 */
function activeSettingsEntry(entries: SettingsNavEntry[], route: Route, settingsSections: string[]) {
  if (route === "settings") return entries.find((entry) => entry.route === "settings" && entry.sections?.[0] === settingsSections[0])?.key ?? "general";
  return entries.find((entry) => entry.route === route)?.key ?? null;
}
/** 某张设置卡片所在的分类（深链「滚到版本与更新」之类）。 */
function settingsSectionsFor(sectionId: string): string[] {
  const entry = settingsNavEntries(null, true).find((candidate) => candidate.sections?.includes(sectionId));
  return entry?.sections ?? ["app-settings-general"];
}

/** 「MCP 与 Skills」页顶部的胶囊。 */
function CustomizeTabs({ route, activeTool, onNavigate }: { route: Route; activeTool: ToolId; onNavigate: (route: Route) => void }) {
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
function AppSidebar({
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
  soloAgent,
  focus,
  extensionPages = [],
  activeExtensionKey = null,
  onOpenExtensionPage,
  settingsSections = ["app-settings-general"],
  onOpenSettings,
  onExitSettings,
  setupCard = null,
}: {
  /** 入门清单（没完成、没关掉时）：放在侧栏底部，替代网关状态卡。 */
  setupCard?: ReactNode;
  /** 拓展贡献的管理器页面：声明了 manager.nav 的放进对应分组。 */
  extensionPages?: ManagerExtensionPage[];
  activeExtensionKey?: string | null;
  onOpenExtensionPage?: (key: string) => void;
  /** 单 Agent 模式下的那个 Agent：侧栏不再出现「接入更多 Agent」的入口和多 Agent 工具。 */
  soloAgent: string | null;
  /** 使用模式：只用增强功能时没有网关入口，底部状态卡不讲网关，Agent 行不画接入状态点。 */
  focus: AgentFocus;
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
  const [settingsQuery, setSettingsQuery] = useState("");
  // 收起了页面列表的 Agent：再点一次当前 Agent 收起 / 展开它下面的页面（#2913）。切到别的 Agent 时清掉。
  const [collapsedAgent, setCollapsedAgent] = useState<string | null>(null);
  const enhanceOnly = providersHidden(focus);
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

  if (settingsMode) {
    const codexAvailable = tools.some((tool) => tool.id === "codex" && tool.switchable);
    const entries = settingsNavEntries(focus, codexAvailable);
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
        <nav aria-label={t("设置分类")} className="cx-side-nav" data-tour="settings">
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
  // 第一组是首页（赞助商推荐）、万物皆拓展（Codex / Claude 下）、MCP 与 Skills、会话、用量；聚合网关排在 Agent 增强后面，只在有 Agent 接入网关时出现。
  type SideEntry = { key: string; route: Route; label: string; icon: LucideIcon; match: Route[]; tour?: string };
  // 用量也记直连代理的请求，不算网关专属。
  const general: SideEntry[] = [
    { key: "home", route: "home", label: t("首页"), icon: House, match: ["home"] },
    // 万物皆拓展是独立入口，不随侧栏当前选的 Agent 显隐；宿主在页内切换。
    { key: "extensions", route: "userScripts", label: t("万物皆拓展"), icon: Blocks, match: ["userScripts"], tour: "management" },
    { key: "customize", route: "context", label: t("MCP 与 Skills"), icon: Network, match: customizeRoutes },
    { key: "sessions", route: "sessions", label: t("会话"), icon: MessagesSquare, match: ["sessions"] },
    { key: "usage", route: "gatewayUsage", label: t("用量"), icon: BarChart3, match: ["gatewayUsage", "requestLog"] },
  ];
  const gatewayEntries: SideEntry[] = [
    { key: "hub", route: "hub", label: t("网关总览"), icon: Waypoints, match: ["hub"], tour: "hub" },
    { key: "providers", route: "gatewayProviders", label: t("供应商池"), icon: Server, match: ["gatewayProviders"], tour: "nav-gatewayProviders" },
    { key: "gateway", route: "gateway", label: soloAgent ? tf("{0} 接入", [agentName(soloAgent)]) : t("Agent 接入"), icon: Plug, match: ["gateway"], tour: "nav-gateway" },
    { key: "routing", route: "gatewayRouting", label: t("路由"), icon: Split, match: ["gatewayRouting"] },
  ];
  const shownGeneral = general.filter((item) => routeVisible(focus, item.route));
  // 还没用网关时整组不显示（⌘K 和 Codex「连接与供应商」页仍能进去）；停在网关页上时照常显示，好在组里来回切。
  const gatewayShown = gatewayInUse || gatewayEntries.some((item) => item.match.includes(route));
  const shownGateway = gatewayShown ? gatewayEntries.filter((item) => routeVisible(focus, item.route)) : [];
  // 声明在网关分组的拓展页面跟在聚合网关后面；网关一组不显示时并进其余拓展页面那一节。
  const gatewayExtensionItems = managerNavPages(extensionPages, "gateway");
  const extensionItems = [...(shownGateway.length ? [] : gatewayExtensionItems), ...(["management", "tools"] as ManagerNavGroup[]).flatMap((group) => managerNavPages(extensionPages, group))];
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

      <nav aria-label={t("主导航")} className="cx-side-nav">
        <div className="cx-side-group">{shownGeneral.map(renderEntry)}</div>

        <div className="cx-side-group" data-tour="agents">
          <div className="cx-side-heading">
            <span>{t("Agent 增强")}</span>
          </div>
          {/* 每个 Agent 一行；点一下切到它，回到上次看的页面（没有就是落脚页）。它的页面在内容区顶部的胶囊里。 */}
          {tools.map((tool) => {
            const current = tool.id === activeTool && !onGlobalPage;
            const expanded = current && collapsedAgent !== tool.id;
            // 这一行同时管 Claude Code（命令行配置概览）和 Claude Desktop（增强），侧栏只写 Claude，免得误以为增强针对的是命令行（#2913）。
            const displayName = tool.id === "claude" ? "Claude" : tool.name;
            const title = tool.switchable
              ? tf("{0}｜{1}｜{2} 个供应商", [tool.name, tool.homeDir || t("未配置目录"), tool.relayCount])
              : tf("{0}｜{1}｜供应商配置尚未接入", [tool.name, tool.homeDir || t("未配置目录")]);
            // 状态点表示接入方式：接入网关为绿色；Codex 没接入网关时是直连，由 Codex++ 直接管理供应商配置。
            const access = connectedAgents.has(tool.id)
              ? { tone: "ok", label: t("已经通过 Codex++ 网关接入") }
              : tool.id === "codex"
                ? { tone: "direct", label: t("Codex 的供应商配置由 Codex++ 直接管理") }
                : { tone: "off", label: t("未接入网关") };
            return (
              <Fragment key={tool.id}>
              <div className={`cx-side-agent ${tool.id === "codex" && tool.switchable ? "has-run" : ""}`.trim()}>
                <button
                  aria-current={current ? "true" : undefined}
                  aria-expanded={current ? expanded : undefined}
                  aria-label={tool.switchable ? `${displayName} · ${access.label}` : `${displayName} · ${t("待接入")}`}
                  className={`cx-side-item ${current ? "active" : ""} ${expanded && agentTabGroups(navigationSections, routeTool, tool.id).flatMap((entry) => visibleRoutes(focus, entry.routes)).length > 1 ? "has-subnav" : ""}`.trim()}
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
                  {enhanceOnly || !gatewayInUse ? null : <i aria-hidden="true" className="side-dot" data-tone={access.tone} />}
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
                {tool.id === "claude" ? (() => {
                  // Claude Desktop 本地模式：和 Codex 行同一套播放键，启动 / 停止本地服务。
                  const label = claudeRun.running ? t("停止本地服务") : t("启动 Claude");
                  return (
                    <button
                      aria-label={label}
                      className="cx-side-run"
                      data-running={claudeRun.running}
                      data-unavailable={!claudeRun.running && Boolean(claudeRun.unavailableReason)}
                      disabled={claudeRun.busy || (!claudeRun.running && Boolean(claudeRun.unavailableReason))}
                      onClick={claudeRun.running ? claudeRun.onStop : claudeRun.onStart}
                      title={claudeRun.busy ? t("处理中…") : claudeRun.running ? t("停止本地服务") : claudeRun.unavailableReason ?? t("启动 Claude")}
                      type="button"
                    >
                      {claudeRun.busy ? <Loader2 aria-hidden="true" className="animate-spin" /> : claudeRun.running ? <Square aria-hidden="true" /> : <Play aria-hidden="true" />}
                    </button>
                  );
                })() : null}
              </div>
              {/* 当前 Agent 的页面：缩进列在它那一行下面（参照 Cursor 侧栏里仓库下面的会话列表），不再占内容区顶部一排胶囊。 */}
{expanded ? (() => {
            const pages = agentTabGroups(navigationSections, routeTool, activeTool)
              .flatMap((entry) => visibleRoutes(focus, entry.routes));
            if (pages.length < 2) return null;
            return (
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
            );
})() : null}
              </Fragment>
            );
          })}
        </div>

        {shownGateway.length ? (
          <div className="cx-side-group" data-group="gateway">
            <div className="cx-side-heading" title={t("任意 Agent 经 Codex++ 接入任意供应商：一个 Agent 可以同时用多家，按模型路由、出错自动换下一家")}>
              <span>{t("聚合网关")}</span>
            </div>
            {shownGateway.map(renderEntry)}
            {gatewayExtensionItems.map(renderExtensionPage)}
          </div>
        ) : null}

        {extensionItems.length ? (
          <div className="cx-side-group">
            <div className="cx-side-heading"><span>{t("拓展页面")}</span></div>
            {extensionItems.map(renderExtensionPage)}
          </div>
        ) : null}
      </nav>

      {/* 底部状态卡（对应 Cursor 的 Getting Started）：入门清单没完成时是它；有新版本时是更新提示。
          网关状态不再单独占一张卡：有 Agent 接入时侧栏已经有「聚合网关」一组，它的「总览」就是同一个入口。 */}
      {setupCard}
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

function Badge({ status }: { status: string }) {
  return <UiBadge className={statusClass(status)} variant="secondary">{statusLabel(status)}</UiBadge>;
}

/// 本地脚本合并状态的文案与悬停说明。
function localScriptStateLabel(state: LocalScriptState, host: ExtensionHost = "codex"): { label: string; hint: string } {
  if (host === "claude") {
    if (state === "off") return { label: t("已关闭"), hint: t("该拓展已在 Claude 中关闭") };
    if (state === "globalOff") return { label: t("整体已关闭"), hint: t("Claude 拓展整体开关已关闭") };
    return { label: t("重启后应用"), hint: t("重启 Claude 补丁版后应用") };
  }
  switch (state) {
    case "off":
      return { label: t("已关闭"), hint: t("该拓展已关闭，不会注入 Codex") };
    case "globalOff":
      return { label: t("整体已关闭"), hint: t("拓展开着，但「启用本地拓展」整体开关关着，暂不注入") };
    case "loaded":
      return { label: t("已加载"), hint: t("已在 Codex 里运行") };
    case "loading":
      return { label: t("加载中"), hint: t("正在 Codex 里加载") };
    case "failed":
      return { label: t("加载失败"), hint: t("拓展在 Codex 里运行出错") };
    case "unknown":
      return { label: t("状态待确认"), hint: t("请检查实际效果") };
    case "notLoaded":
      return { label: t("未加载"), hint: t("Codex 未运行或尚未热重载拓展") };
  }
}

type ExtensionCreatorStatus = {
  status: "connected" | "not_connected" | "disconnected" | "conflict";
  message: string;
  configPath?: string;
  extensionsDirectory?: string;
};

/// 「对话造拓展」：把拓展创作 MCP 接入 Codex 后，用户在 Codex 对话里描述需求，agent 直接写出拓展并热重载。
/// 拓展页顶部的理念：Codex++ 的增强能力都以拓展的形式开放，下面列出拓展能改到的地方（放在悬停里）。
const extensionCapabilities = [
  "界面与页面", "斜杠命令", "消息与面板", "模型工具", "Codex 钩子", "网关中间件", "常驻后台", "会话历史", "拓展间服务", "多语言",
] as const;

function ExtensionsHero({ host, focus = null, onHostChange }: { host: ExtensionHost; focus?: AgentFocus; onHostChange: (host: ExtensionHost) => void }) {
  // 页面标题就是「万物皆拓展」：这里只留一句理念（能改到的地方放在悬停里），宿主切换放在右侧；只有一个宿主时不显示切换。
  const hosts = (["codex", "claude"] as const).filter((target) => agentVisible(focus, target));
  return (
    <section className="ext-hero" aria-label={t("万物皆拓展")}>
      <p className="ext-hero-lead" title={host === "codex" ? `${t("Codex++ 构建于万物皆拓展的架构之上：界面、命令、模型能用的工具、agent 循环的钩子、发往模型的请求与回复，都能由拓展接管。在 Codex 对话里说一句，就能造一个属于自己的拓展。")}\n${extensionCapabilities.map((label) => t(label)).join(" · ")}` : undefined}>
        {host === "codex"
          ? t("界面、命令、工具、钩子、模型请求都能由拓展接管；在 Codex 对话里说一句就能造一个。")
          : t("给 Claude 补丁版装拓展：只显示声明了适配 Claude 的拓展。")}
      </p>
      {hosts.length > 1 ? (
        <div className="tp-tabs cx-pills ext-host-switch" role="group" aria-label={t("拓展宿主")}>
          {hosts.map((target) => (
            <button key={target} type="button" aria-pressed={host === target} data-active={host === target} onClick={() => onHostChange(target)}>
              <AgentGlyph agent={target} />
              {target === "codex" ? "Codex" : "Claude"}
            </button>
          ))}
        </div>
      ) : null}
    </section>
  );
}

/// 工具条右侧的主按钮「对话造拓展」：一键把拓展创作 MCP 接入 Codex；接入后按钮变成「已接入」，再点一次取消接入。
/// 说明放在悬停里，不再单独占一张卡。
function ExtensionCreatorCard() {
  const [state, setState] = useState<ExtensionCreatorStatus | null>(null);
  const [busy, setBusy] = useState<"connect" | "disconnect" | null>(null);
  const [error, setError] = useState("");
  const request = async (action: "status" | "connect" | "disconnect") => {
    setBusy(action === "status" ? null : action);
    setError("");
    try {
      setState(await invoke<ExtensionCreatorStatus>("extension_creator_request", { action }));
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(null);
    }
  };
  useEffect(() => {
    void request("status");
  }, []);
  const connected = state?.status === "connected";
  const hint = error || (connected
    ? t("新开一个 Codex 对话，说「帮我做个拓展：…」即可；不造拓展时可取消接入，省下工具说明占用的上下文")
    : state?.status === "disconnected" ? t("已取消接入，新开的 Codex 对话不再加载拓展创作工具；要造拓展时再接入")
    : state?.status === "conflict" ? state.message : t("在 Codex 对话里描述需求，agent 直接写出拓展并热重载"));
  return (
    <button
      aria-pressed={connected}
      className={connected ? "cx-secondary ext-creator" : "cx-primary ext-creator"}
      data-error={error ? "true" : undefined}
      data-status={state?.status ?? "loading"}
      disabled={busy !== null || !state || (!connected && state.status === "conflict")}
      onClick={() => void request(connected ? "disconnect" : "connect")}
      title={`${hint}${connected ? `\n${t("再点一次取消接入")}` : ""}${state?.extensionsDirectory ? `\n${state.extensionsDirectory}` : ""}`}
      type="button"
    >
      {connected ? <Check /> : <Sparkles />}
      {busy === "connect" ? t("接入中…") : busy === "disconnect" ? t("取消中…") : connected ? t("对话造拓展已接入") : t("对话造拓展")}
    </button>
  );
}

/// 拓展包 locale/<语言>.json 里的本地化名称与描述：先找当前界面语言，再找 en，最后用清单原值。
function localizedPackageText(
  locales: Record<string, { title?: string | null; description?: string | null }> | undefined,
  field: "title" | "description",
  fallback: string,
): string {
  if (!locales) return fallback;
  for (const lang of [getLanguage(), "en"]) {
    const hit = Object.keys(locales).find((key) => key.toLowerCase() === lang || key.toLowerCase().startsWith(`${lang}-`));
    const value = hit ? locales[hit]?.[field] : null;
    if (typeof value === "string" && value.trim()) return value;
  }
  return fallback;
}

/// 本地拓展列表底部的内置拓展摘要：Codex++ 自带的功能都是内置拓展，开关与参数统一在 Codex 里的 Codex++ 设置页管理，
/// 这里只给个数和被顶替的项（只读，不可点），免得两处各有一套开关。
function BuiltinExtensionsSummary({ builtins }: { builtins: BuiltinExtension[] }) {
  const enabled = builtins.filter((builtin) => builtin.enabled && !builtin.replacedBy).length;
  const replaced = builtins.filter((builtin) => builtin.replacedBy);
  return (
    <CxSection title={t("内置")}>
      <CxCard>
        <CxRow
          badges={<CxBadge tone={enabled ? "ok" : "neutral"}>{tf("已启用 {0} / 共 {1} 项", [String(enabled), String(builtins.length)])}</CxBadge>}
          detail={replaced.length
            ? tf("已被顶替：{0}", [replaced.map((builtin) => `${t(builtin.name)} → ${String(builtin.replacedBy).replace(/^user:/, "")}`).join("、")])
            : t("Codex++ 自带的功能，开关与参数在 Codex 里的「Codex++」设置页管理")}
          icon={<Blocks />}
          title={t("内置拓展")}
          tooltip={tf("Codex++ 自带的功能都是内置拓展：已启用 {0} / 共 {1} 项。可以被声明 replaces 的拓展顶替。", [String(enabled), String(builtins.length)])}
        />
      </CxCard>
    </CxSection>
  );
}

/// 本地脚本一行：名称 / 来源 / 开关 + 状态 / 操作。
/// 以前「启用 / 关闭」和 loaded / disabled 各占一列、还另有启用按钮，三处说同一件事（#2781）；
/// 现在开关直接放进状态列，状态只补充开关之外的信息（是否真正加载、失败原因）。
function ScriptRow({ host, script, globalEnabled, actions }: { host: ExtensionHost; script: NonNullable<UserScriptInventory["scripts"]>[number]; globalEnabled: boolean; actions: Actions }) {
  const codexHost = host === "codex";
  const enabled = extensionEnabledForHost(script, host);
  const isPackage = script.kind === "package";
  const source = script.market_id
    ? tf("市场 · {0}", [script.version || t("未知版本")])
    : isPackage
      ? tf("拓展包 · {0}", [script.package?.version || t("未知版本")])
      : script.source === "builtin" ? t("内置") : t("用户");
  // 拓展包是文件夹：后端整个移进废纸篓（可找回，#2919），单文件脚本直接删除。
  const canDelete = codexHost && script.source === "user";
  const state = extensionStateForHost(script, host, globalEnabled);
  const { label, hint } = localScriptStateLabel(state, host);
  const stateHint = codexHost && state === "failed" && script.error ? `${hint}\n${script.error}` : hint;
  const configSchema = codexHost ? script.package?.configSchema ?? [] : [];
  // 拓展自己的设置页（settings.page）优先，没有时用按清单 config 生成的通用表单。
  const settingsPage = codexHost && !!script.package?.settingsPage;
  const hasSettings = settingsPage || configSchema.length > 0;
  const [configOpen, setConfigOpen] = useState(false);
  // 拓展自己的管理器面板（manager.page），启用时才能打开。
  const managerPage = codexHost && enabled ? script.package?.managerPage ?? null : null;
  const [panelOpen, setPanelOpen] = useState(false);
  // 用户自己的拓展包可以一键发布到 GitHub（单文件脚本、内置拓展没有这个入口）。
  const canPublish = codexHost && isPackage && script.source === "user" && !!script.package?.id;
  const [publishOpen, setPublishOpen] = useState(false);
  const displayName = localizedPackageText(script.package?.locales, "title", script.name);
  const displayDescription = localizedPackageText(script.package?.locales, "description", script.package?.description ?? "");
  // 拓展声明的能力，排成一行小标签；警示类（资源超限）用警告色。
  const pkg = script.package;
  const joined = (parts: unknown[]) => parts.filter((part): part is string => typeof part === "string" && part !== "").join("、");
  type CapabilityTag = { text: string; tone?: "warn"; title?: string };
  const capabilityCandidates: Array<CapabilityTag | null> = codexHost ? [
    pkg?.claudeDesktop ? { text: "Claude Code" } : null,
    pkg?.hookEvents?.length ? { text: tf("Codex 钩子：{0}", [pkg.hookEvents.join("、")]) } : null,
    pkg?.tools?.length ? { text: tf("模型工具：{0}", [pkg.tools.join("、")]) } : null,
    pkg?.backend ? { text: t("常驻后台") } : null,
    pkg?.services?.length ? { text: tf("提供服务：{0}", [pkg.services.join("、")]) } : null,
    pkg?.providers?.length ? { text: tf("模型供应商：{0}", [pkg.providers.map((provider) => provider.name || provider.id).join("、")]) } : null,
    pkg?.toolPipeline && (pkg.toolPipeline.preExecute || pkg.toolPipeline.execute || pkg.toolPipeline.postExecute)
      ? { text: tf("工具管道：{0}", [joined([pkg.toolPipeline.preExecute && t("改参数"), pkg.toolPipeline.execute && t("代为执行"), pkg.toolPipeline.postExecute && t("改结果")])]) }
      : null,
    pkg?.gatewayRequest || pkg?.gatewayResponse || pkg?.gatewayError
      ? { text: tf("网关中间件：{0}", [joined([!!pkg.gatewayRequest && t("请求"), !!pkg.gatewayResponse && t("响应流"), !!pkg.gatewayError && t("失败重试")])]) }
      : null,
    pkg?.codex?.skills?.length ? { text: tf("技能：{0}", [pkg.codex.skills.join("、")]) } : null,
    pkg?.codex?.agents?.length ? { text: tf("子 agent：{0}", [pkg.codex.agents.join("、")]) } : null,
    pkg?.codex?.mcpServers?.length ? { text: tf("MCP 服务器：{0}", [pkg.codex.mcpServers.join("、")]) } : null,
    pkg?.codex?.instructions ? { text: t("全局指令") } : null,
    pkg?.codex?.prompts?.length || pkg?.codex?.resources?.length
      ? { text: tf("提示词 / 资源：{0}", [[...(pkg.codex.prompts ?? []), ...(pkg.codex.resources ?? [])].join("、")]) }
      : null,
    pkg?.schedules?.length ? { text: tf("定时任务：{0}", [pkg.schedules.map((item) => item.at ? `${item.method} ${item.at}` : tf("{0} 每 {1} 分钟", [item.method, String(item.everyMinutes ?? "")])).join("、")]) } : null,
    pkg?.http ? { text: tf("HTTP 路由：/ext/{0}/", [pkg.id]) } : null,
    pkg?.skippedAssets?.length
      ? { text: tf("资源超出大小上限未加载：{0}", [pkg.skippedAssets.join("、")]), tone: "warn" as const, title: pkg.skippedAssets.join("\n") }
      : null,
  ] : [
    pkg?.claudeDesktop ? { text: "Claude Code" } : null,
    pkg?.backend ? { text: t("常驻后台") } : null,
  ];
  const capabilityTags = capabilityCandidates.filter((tag): tag is CapabilityTag => tag !== null);
  // 一行只放一句说明：有描述用描述，没有就列能力；完整的能力清单放进悬停。警示类能力单独打橙色标签。
  const plainTags = capabilityTags.filter((tag) => tag.tone !== "warn");
  const warnTags = capabilityTags.filter((tag) => tag.tone === "warn");
  const detailText = displayDescription || plainTags.map((tag) => tag.text).join(" · ");
  const detailHover = [displayDescription, ...plainTags.map((tag) => tag.text)].filter(Boolean).join("\n");
  // 状态点：已加载 = 绿，加载失败 = 红，整体关闭 / 未加载 = 橙，关着 = 灰。
  const stateTone = state === "loaded" ? "ok" : state === "failed" ? "danger" : state === "off" ? "off" : "warn";
  // 开关本身已经说明「开 / 关」，状态文字只在开关之外还有话说时显示（未加载、失败、整体关闭）。
  const quietState = state === "loaded" || state === "off";
  return (
    <>
    <CxRow
      badges={
        <>
          <CxBadge>{source}</CxBadge>
          {warnTags.map((tag) => <CxBadge key={tag.text} title={tag.title ?? tag.text} tone="warn">{t("资源超限")}</CxBadge>)}
        </>
      }
      className="ext-local-row"
      detail={detailText ? <span title={detailHover}>{detailText}</span> : undefined}
      icon={<CxStatusIcon tone={stateTone}><Puzzle /></CxStatusIcon>}
      muted={!enabled}
      title={displayName}
    >
      <span className="user-script-state-label" data-state={state} hidden={quietState || undefined} title={stateHint}>{label}</span>
      {hasSettings ? (
        <button
          type="button"
          className="mg-icon-button"
          title={configOpen ? t("收起设置") : t("设置")}
          aria-label={tf("{0} 设置", [displayName])}
          aria-expanded={configOpen}
          data-active={configOpen || undefined}
          onClick={() => setConfigOpen((open) => !open)}
        >
          <Settings size={16} />
        </button>
      ) : null}
      <label className="relay-bare-switch" title={enabled ? t("关闭该拓展") : t("启用该拓展")}>
        <input
          aria-label={tf("启用 {0}", [script.name])}
          checked={enabled}
          onChange={() => void actions.setUserScriptEnabled(script.key, !enabled, host)}
          type="checkbox"
        />
        <ToggleVisual />
      </label>
      {canPublish || canDelete || managerPage ? (
        <Menu label={tf("{0} 的更多操作", [displayName])}>
          {managerPage ? (
            <MenuItem onClick={() => setPanelOpen((open) => !open)}>
              <span className="user-script-menu-item"><AppWindow className="h-4 w-4" />{panelOpen ? t("收起面板") : t("打开面板")}</span>
            </MenuItem>
          ) : null}
          {canPublish ? (
            <MenuItem onClick={() => setPublishOpen(true)}>
              <span className="user-script-menu-item"><Upload className="h-4 w-4" />{t("发布到 GitHub")}</span>
            </MenuItem>
          ) : null}
          {canDelete ? (
            <MenuItem danger onClick={() => void actions.deleteUserScript(script.key)}>
              <span className="user-script-menu-item"><Trash2 className="h-4 w-4" />{t("删除")}</span>
            </MenuItem>
          ) : null}
        </Menu>
      ) : <span className="user-script-menu-spacer" aria-hidden="true" />}
    </CxRow>
    {publishOpen && script.package?.id ? (
      <ExtensionPublishDialog id={script.package.id} onClose={() => setPublishOpen(false)} onOpenUrl={(url) => void actions.openExternalUrl(url)} />
    ) : null}
    {panelOpen && managerPage ? (
      <div className="cx-row-expand"><ExtensionManagerPanel
        scriptKey={script.key}
        config={script.package?.config ?? {}}
        onSaveConfig={(values) => actions.setExtensionConfig(script.key, values)}
        onToast={(message) => actions.showExtensionNotice(managerPage.title || displayName, message)}
      /></div>
    ) : null}
    {configOpen && settingsPage ? (
      <div className="cx-row-expand"><ExtensionManagerPanel
        kind="settings"
        scriptKey={script.key}
        config={script.package?.config ?? {}}
        onSaveConfig={(values) => actions.setExtensionConfig(script.key, values, { silent: true })}
        onToast={(message) => actions.showExtensionNotice(displayName, message)}
      /></div>
    ) : null}
    {configOpen && !settingsPage && configSchema.length ? (
      <div className="cx-row-expand"><ExtensionConfigForm
        fields={configSchema}
        values={script.package?.config ?? {}}
        onCancel={() => setConfigOpen(false)}
        onSave={async (values) => {
          if (await actions.setExtensionConfig(script.key, values)) setConfigOpen(false);
        }}
      /></div>
    ) : null}
    </>
  );
}

/// 卡片右上角显示落地页域名，点之前就知道会打开哪里。
function adHost(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return "";
  }
}

/// 广告标题原样展示。
///
/// 以前这里会在 `｜` / `|` 处截断，把「火山引擎｜方舟 Agent Plan」显示成
/// 「火山引擎」—— 后半段是作者写的产品名，不该被我们悄悄丢掉。既然卡片
/// 已经改成按内容自适应高度，就不再需要在标题上省这一点空间。
function formatAdTitle(title: string) {
  return title.trim() || title;
}

/// 广告的「同一家」判据。
///
/// 只比 id 是不够的：置顶位和推荐池里的同一条赞助商往往有两个 id
/// （`jojocode-top` vs `jojocode-codex-relay`），但指向同一个去处。
/// 所以按落地 URL（去 query / 尾斜杠）比，退回标题。
function adIdentity(ad: AdItem): string {
  const url = (ad.url || "").trim();
  if (url) {
    try {
      const parsed = new URL(url);
      return `${parsed.host}${parsed.pathname}`.replace(/\/+$/, "").toLowerCase();
    } catch {
      return url.replace(/[?#].*$/, "").replace(/\/+$/, "").toLowerCase();
    }
  }
  return (ad.title || "").trim().toLowerCase();
}

function isExpiredAd(ad: AdItem) {
  if (!ad.expires_at) return false;
  const expiresAt = Date.parse(ad.expires_at);
  return Number.isFinite(expiresAt) && expiresAt < Date.now();
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

/** 「填入常用」：限流与常见的服务端/网关瞬时错误。 */
const COMMON_RETRY_STATUS_CODES = [429, 500, 502, 503, 504];

function parsePort(value: string, fallback: number): number {
  const parsed = Number.parseInt(value, 10);
  return Number.isInteger(parsed) && parsed >= 1 && parsed <= 65535 ? parsed : fallback;
}

function codexExtraArgsToInput(args: string[] | undefined) {
  return (args ?? []).join("\n");
}

function inputToCodexExtraArgs(value: string) {
  return value === "" ? [] : value.split(/\r?\n/);
}

/// 网址只显示主机名；解析不了就原样返回。
function urlHost(value: string | undefined): string {
  const text = (value ?? "").trim();
  if (!text) return "";
  try {
    return new URL(text).host;
  } catch {
    return text.replace(/^https?:\/\//, "").replace(/\/.*$/, "");
  }
}

function ccsProviderSummary(result: CcsProvidersResult | null): string {
  if (!result) return t("读取 ~/.cc-switch/cc-switch.db");
  if (!isSuccessStatus(result.status)) return result.message || t("读取 cc-switch 供应商失败。");
  const count = result.providers.length;
  return count ? tf("发现 {0} 个 Codex 供应商", [count]) : t("未发现可导入供应商");
}

function providerImportWireApiLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "chat" || normalized === "chat_completions" || normalized === "chat-completions") {
    return "Chat Completions";
  }
  if (normalized === "anthropic" || normalized === "anthropic_messages" || normalized === "anthropic-messages" || normalized === "messages") {
    return "Anthropic Messages";
  }
  return "Responses";
}

function providerImportRelayModeLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "official") return t("官方登录");
  if (normalized === "mixedapi" || normalized === "mixed-api" || normalized === "mixed_api") return t("官方登录 + API");
  if (normalized === "aggregate") return t("聚合供应商");
  return t("纯 API");
}

function maskSecret(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return t("未填写");
  if (trimmed.length <= 10) return `${trimmed.slice(0, 2)}…${trimmed.slice(-2)}`;
  return `${trimmed.slice(0, 6)}…${trimmed.slice(-4)}`;
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

/** 新用户自动显示入门清单；看过旧导览的用户可从设置主动重开。 */
function initialSetupOpen(): boolean {
  if (typeof window === "undefined") return false;
  if (new URLSearchParams(window.location.search).has("previewRoute")) return false;
  try {
    return shouldShowSetup(window.localStorage.getItem(SETUP_STORAGE_KEY), window.localStorage.getItem(TOUR_STORAGE_KEY));
  } catch {
    return true;
  }
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
