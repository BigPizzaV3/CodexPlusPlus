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
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { open, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  ArrowRight,
  ArrowRightLeft,
  Bell,
  Blocks,
  Cable,
  BookOpen,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Camera,
  CircleHelp,
  Copy,
  Download,
  Edit3,
  Eye,
  GripVertical,
  Info,
  ImagePlus,
  Github,
  GitBranch,
  ExternalLink,
  Hammer,
  KeyRound,
  Layers3,
  Languages,
  LayoutGrid,
  LayoutDashboard,
  List,
  Loader2,
  Palette,
  Play,
  MessageCircle,
  MoreHorizontal,
  PackageOpen,
  FileCode2,
  FolderOpen,
  Moon,
  MousePointerClick,
  PawPrint,
  Network,
  Power,
  PowerOff,
  Plus,
  RefreshCw,
  RotateCcw,
  Rocket,
  ScanLine,
  Save,
  Search,
  Settings,
  Sparkles,
  ShieldCheck,
  ShieldAlert,
  Square,
  Star,
  Store,
  Stethoscope,
  Sun,
  TestTube,
  Trash2,
  Wrench,
  ArrowUp,
  AppWindow,
  Bug,
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
  type LucideIcon,
} from "lucide-react";
import { createPresetPatch } from "@/components/ProviderPresetSelector";
import { PRESETS, type ProviderPreset } from "./presets";
import { createPortal } from "react-dom";
import { Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type CSSProperties, type ReactNode } from "react";

import { Badge as UiBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { isGitHubRepositoryHomepage } from "./github-repository";
import { NativeBrowserStatusView, nativeBrowserConsent } from "./native-browser-settings";
import { AgentCachePanel } from "./agent-cache";
import { rebaseSettingsEdits } from "./settings-rebase";
import { summarizeRecentRequests, type RecentRequestSummary } from "./overview-model";
import { AgentCollaborationPage } from "./AgentCollaborationPage";
import { SectionLabel, Switch } from "./gateway-ui";
import { parseTrayRequest, TRAY_REQUEST_EVENT, TRAY_RESULT_EVENT, type TrayRequest, type TrayResult } from "./tray-panel-model";
import { AppSelect, type AppSelectOption } from "./AppSelect";
import { AppDialogHost, appAlert, appConfirm, appPrompt } from "./app-dialog";
import { GatewayPage, type GatewayTab } from "./GatewayPage";
import { AgentGlyph, HubOverview, useGatewayPayload } from "./HubOverview";
import { CommandPalette, type PaletteCommand } from "./CommandPalette";
import { SessionsView } from "./GatewaySessions";
import { agentName, maskAccountName, type GatewayResult } from "./gateway-model";
import { SessionAgentTabs, type SessionTab } from "./SessionAgentTabs";
import { noticeDuration, noticeTone } from "./notice-model";
import { agentTabGroups, agentTarget } from "./page-groups";
import { TopbarActions } from "./TopbarActions";
import { OnboardingTour } from "./OnboardingTour";
import { AgentDashboard, type DashboardAgent, type DashboardStat, type DashboardStatus } from "./AgentDashboard";
import { shouldShowTour, TOUR_STORAGE_KEY, TOUR_VERSION } from "./onboarding-model";
import "./ToolPages.css";
import { BACKEND_SETTINGS_CHANGED_EVENT, ComputerUsePage } from "./ComputerUsePage";
import { PluginMarketScreen } from "./PluginMarketScreen";
import { ReleaseNotes } from "./ReleaseNotes";
import { UPDATE_CHECK_INTERVAL_MS, UpdateNotifications } from "./update-notification";
import { McpPage, PageHeader } from "./ManagementPages";
import { SkillsPage } from "./ManagementExtra";
import { ProjectSessionPage } from "./ProjectSessionPage";
import { ENHANCEMENT_SECTION_IDS, managerNavigationDestination, type EnhancementTab, type ManagerNavigationIntent } from "./enhancement-navigation";
import {
  enhancementGroup,
  enhancementGroupAnchorId,
  enhancementItem,
  enhancementPlatformFromUserAgent,
  isEnhancementItemMasterGated,
  managerGeneralGroups,
  managerGroupItems,
  managerStandaloneGroup,
  type EnhancementSettingItem,
} from "./enhancement-catalog";
import { DEFAULT_AUTO_COMPACT_PERCENT, autoCompactOptions, normalizeAutoCompactPercent } from "./auto-compact";
import { agentPetSettingsIssue, defaultAgentPetSettings, normalizeAgentPetSettings, type AgentPetIssue, type AgentPetSettings } from "./agent-pet-settings";
import { AgentPetPage } from "./AgentPetPage";
import {
  applyDictationPreset,
  defaultDictationSettings,
  dictationPreset,
  dictationSettingsIssue,
  normalizeDictationSettings,
  type DictationSettings,
  type DictationSettingsIssue,
  type DictationPreset,
} from "./dictation-settings";
import {
  builtinEntryToImportDocument,
  builtinRowBackfillValue,
  clearModelMetadataForSlug,
  importDocumentSyncPatch,
  importPanelControls,
  importSaveDecision,
  metadataMatchesBuiltin,
  metadataSourceTags,
  modelSlugFromRowName,
  suffixWindowString,
  normalizeTokenCountInput,
  modelMetadataKey,
  modelFastSupportMode,
  synchronizeModelMetadataLiteSupport,
  synchronizeModelMetadataFastSupport,
  type ModelFastSupportMode,
  parseModelMetadataDocument,
  parseModelMetadataMap,
  remapModelMetadataSlugs,
  replaceModelMetadataForSlug,
  resolveModelMetadataRowKey,
  retainModelMetadataForSlugs,
  serializeModelMetadataDocument,
  synchronizeModelMetadataDocumentLimitsPreview,
  type ActiveImportDraft,
  createActiveImportDraft,
  updateActiveImportDraft,
  cancelActiveImportDraft,
  rematchActiveImportDraft,
  builtinMetadataQueryState,
  type BuiltinMetadataQueryState,
  type BuiltinModelMetadataMatch,
  type ImportedModelMetadata,
} from "./model-metadata";
import {
  independentProxyEnabled,
  setIndependentProxyConfigured,
  requestCompatibilityEnabled,
  findRelayModelRouteIssue,
  modelRouteSaveRequiresRestart,
  normalizeRelayModelRoutes,
  webSearchHistoryCompatEnabled,
  PROTOCOL_PROXY_BASE_URL,
  type RelayModelRoute,
} from "./model-routes";
import {
  mergeModelWindowRows,
  modelWindowRowsFromProfile,
  modelWindowRowsValidationError,
  serializeModelWindowRows,
  reorderModelWindowRows,
  type ImageHandling,
  type ModelWindowRowsValidationIssue,
  type ModelWindowRow,
} from "./model-windows";
import {
  reasoningParameterMode,
  setModelReasoningParameterMode,
  type ReasoningParameterMode,
  formatReasoningLevelsLabel,
  normalizeReasoningLevels,
  REASONING_LEVELS,
  resolveModelReasoningLevels,
  setModelReasoningLevelsForSlug,
} from "./model-reasoning";
import { clampAggregateRoutePriority, normalizeAggregateRoutes, validateAggregateRoutes } from "./aggregate-routes";
import {
  aggregateMemberCandidates,
  normalizeAggregateMembers,
  isAggregateProfile,
  isOfficialLoginMemberProfile,
  normalizeAggregateDefaultRelayId,
} from "./aggregate-members";
import { relayAuthForLiveDraft, shouldBackfillRelayProfileBeforeSwitch } from "./relay-live-files";
import { apiKeyPreview, createRelayApiKey, normalizeRelayApiKeys, relayProfileWithNormalizedApiKeys, relayProfileWithUpdatedApiKey, type RelayApiKey } from "./provider-api-keys";
import { relayHeadersValidationMessage, serializeRelayHeaders } from "./relay-headers";
import { sessionProviderForProtocol } from "./relay-session";
import { usePopoverLayer } from "./use-popover-layer";
import { resolveAnchoredMenuLayout, type AnchoredMenuLayout } from "./popover-layer";
import { providerAvatarText, resolveProviderName } from "./provider-name";
import {
  providerSyncStreamPercent,
  resolveProviderSyncCompletion,
  type ProviderSyncStreamProgress,
} from "./provider-sync-flow";
import { isProviderSyncTargetSelectable, preferredProviderSyncTarget } from "./provider-sync-target";
import { resolveLaunchStatus, launchCompletionNotice } from "./launch-status";
import {
  defaultDreamSkinTheme,
  defaultDreamSkinColors,
  isDreamSkinDraftDirty,
  normalizeDreamSkinTheme,
  runExclusiveDreamSkinInstall,
  type DreamSkinCheck,
  type DreamSkinColors,
  type DreamSkinCommunityResult,
  type DreamSkinCommunityTheme,
  type DreamSkinImageResult,
  type DreamSkinMarketResult,
  type DreamSkinMarketTheme,
  type DreamSkinRuntimeResult,
  type DreamSkinThemeActivationResult,
  type DreamSkinThemeConfig,
  type DreamSkinThemeDraft,
  type DreamSkinThemeDraftResult,
  type DreamSkinThemeLibrary,
  type DreamSkinThemeLibraryResult,
  type DreamSkinThemeSummary,
  type DreamSkinVerificationResult,
} from "./dream-skin";
import { subscribeLanguage, getLanguage, t, tf, toggleLanguage } from "@/i18n";
import { vlmTestTranslation } from "./vlm-test-translation";
import {
  multiLaunchInstanceFailure,
  multiLaunchInstanceLabel as formatMultiLaunchInstanceLabel,
  multiLaunchInstanceRunning,
  multiLaunchInstanceStatus,
  type MultiLaunchInstance,
} from "./multi-launch-model";

const isWindowsPlatform = /\bWindows\b/i.test(navigator.userAgent);
/// 增强设置项清单按平台过滤（例如桌宠跟随鼠标只在 Windows / macOS 显示）。
const enhancementPlatform = enhancementPlatformFromUserAgent(navigator.userAgent);
/// 会话维护页的「启动前自动修复历史会话」与 Codex 设置页共用清单里的同一条文案。
const providerSyncItem = enhancementItem("providerSyncEnabled");

/// 多开启动后轮询「就绪/失败」结论的节奏：worker 最长等 90 秒就绪，这里给到 2 分钟。
const MULTI_LAUNCH_POLL_INTERVAL_MS = 3000;
const MULTI_LAUNCH_POLL_TIMEOUT_MS = 120_000;
const dreamSkinWindowsPreviewUrl = new URL("../../../assets/inject/upstream/dream-skin/windows/dream-reference.jpg", import.meta.url).href;
const dreamSkinMacPreviewUrl = new URL("../../../assets/inject/upstream/dream-skin/macos/portal-hero.png", import.meta.url).href;
const dreamSkinCompanionDataUrlLimit = 240_000;
const dreamSkinCompanionMimeTypes = new Set(["image/png", "image/jpeg", "image/webp", "image/gif"]);

type Status = "ok" | "failed" | "not_implemented" | "not_checked" | string;

function modelWindowRowsValidationMessage(issue: ModelWindowRowsValidationIssue | null): string | null {
  if (!issue) return null;
  if (issue.code === "duplicateModel") return tf("模型名称重复：{0}", [issue.model]);
  if (issue.code === "invalidWindow") {
    return tf("模型 {0} 的上下文窗口无效；请输入正整数，或使用 K/M 整数后缀。", [issue.model]);
  }
  return tf("模型 {0} 的自动压缩百分比无效；请输入 0 到 100 之间、最多 6 位小数的十进制数。", [issue.model]);
}

type CommandResult<T> = T & {
  status: Status;
  message: string;
};

type PendingDreamSkinCommunityResult = CommandResult<{ versionId: string }>;

type PendingDreamSkinRestart = {
  currentThemeKey: string | null;
  currentThemeName: string;
  pendingThemeKey: string;
  pendingThemeName: string;
};

type DreamSkinInstallOperation = {
  source: "market" | "community" | "pending-community" | "import";
  id: string;
  name: string;
};

type PathState = {
  status: string;
  path: string | null;
};

type LaunchStatus = {
  status: string;
  message: string;
  /** 阶段；侧栏「停止 Codex」写的是 user_stopped，用来和意外退出区分。 */
  phase?: string | null;
  started_at_ms: number;
  debug_port: number | null;
  helper_port: number | null;
  codex_app: string | null;
  aumid: string | null;
};

type RuntimeServiceState = { status: string; address: string | null; message: string };

type OverviewResult = CommandResult<{
  codex_app: PathState;
  codex_version: string | null;
  silent_shortcut: PathState;
  management_shortcut: PathState;
  latest_launch: LaunchStatus | null;
  runtime_health: { codex_app: RuntimeServiceState; local_server: RuntimeServiceState; debugger: RuntimeServiceState; protocol_conversion_enabled: boolean | null };
  current_version: string;
  update_status: string;
  settings_path: string;
  logs_path: string;
}>;

type LaunchCommandResult = CommandResult<{
  launchStartedAtMs?: number;
  nativeBrowserRestoreFailed?: boolean;
}>;

type MultiLaunchResult = CommandResult<{
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

type MultiLaunchInstancesResult = CommandResult<{
  instances: MultiLaunchInstance[];
}>;

type BackendSettings = {
  revision?: string;
  /** Agent 桌宠（与 Rust `agent_pet` 对应）。 */
  agentPet: AgentPetSettings;
  codexAppPath: string;
  /** 自动接管官方 App（与 Rust `codex_app_auto_takeover` 对应）。 */
  codexAppAutoTakeover: boolean;
  codexAppAutoTakeoverWhenActive: string;
  codexExtraArgs: string[];
  dictation: DictationSettings;
  providerSyncEnabled: boolean;
  threadModelFollowSwitchEnabled?: boolean;
  ccsDbPath: string;
  providerSyncSavedProviders: string[];
  providerSyncManualProviders: string[];
  providerSyncLastSelectedProvider: string;
  relayProfilesEnabled: boolean;
  localConfigProviderImported: boolean;
  enhancementsEnabled: boolean;
  codexAppStandaloneQuotaUnlock: boolean;
  codexAppPluginMarketplaceUnlock: boolean;
  codexAppModelWhitelistUnlock: boolean;
  codexAppSessionDelete: boolean;
  codexAppSideChatSave: boolean;
  codexAppMarkdownExport: boolean;
  codexAppSessionShare: boolean;
  codexAppUsageStatusLine: boolean;
  codexAppPasteFix: boolean;
  codexAppForceChineseLocale: boolean;
  codexAppTypingEffect: TypingEffect;
  codexAppThreadIdBadge: boolean;
  codexAppConversationView: boolean;
  codexAppThreadScrollRestore: boolean;
  codexAppNativeMenuPlacement: boolean;
  codexAppNativeBrowserRequireIdentification: boolean;
  codexAppNativeFastUnlock: boolean;
  /** 纯 API Key 模式启动 Codex 前把 Work 视图写回 Codex，避开 Work browser access 报错。 */
  codexAppDisableWorkView: boolean;
  codexAppPetRealMouseLook: boolean;
  codexAppActivityOverlayRight: boolean;
  /** 协议代理内部自动重试（与 Rust `protocol_proxy_auto_retry` 对应）。 */
  protocolProxyAutoRetry: boolean;
  protocolProxyAutoRetryMaxAttempts: number;
  protocolProxyAutoRetryIntervalSeconds: number;
  /** 命中这些上游 HTTP 状态码时由代理退避重发；空数组表示不重试。与 Rust `protocol_proxy_auto_retry_status_codes` 对应。 */
  protocolProxyAutoRetryStatusCodes: number[];
  protocolProxyAutoRetryMode: "streaming" | "streaming-legacy" | "buffered";
  /** 流式避震下首段可见输出后到提交的延迟（毫秒）；0 表示一出现可见输出立即提交。与 Rust `protocol_proxy_auto_retry_commit_delay_ms` 对应。 */
  protocolProxyAutoRetryCommitDelayMs: number;
  /** 把不在当前供应商模型列表里的请求模型（Codex 后台任务的 gpt-* / codex-auto-review）改写为该供应商默认模型。与 Rust `protocol_proxy_map_unlisted_models` 对应。 */
  protocolProxyMapUnlistedModels: boolean;
  /** 本地协议代理单次请求体上限（MiB，32~512，默认 128）。与 Rust `helper_max_request_body_mib` 对应。 */
  helperMaxRequestBodyMib: number;
  codexAppCapacityAutoRetry: boolean;
  codexAppTurnRewind: boolean;
  codexAppStepwiseEnabled: boolean;
  codexAppAnswerOutlineEnabled: boolean;
  codexAppStepwiseDirectSend: boolean;
  codexAppStepwiseProtocol: StepwiseProtocol;
  codexAppStepwiseGenerationMode: StepwiseGenerationMode;
  codexAppStepwiseBaseUrl: string;
  codexAppStepwiseApiKey: string;
  codexAppStepwiseApiKeyEnv: string;
  codexAppStepwiseModel: string;
  codexAppStepwiseMaxItems: number;
  codexAppStepwiseMaxInputChars: number;
  codexAppStepwiseMaxOutputTokens: number;
  codexAppStepwiseTimeoutMs: number;
  codexAppImageOverlayEnabled: boolean;
  codexAppImageOverlayPath: string;
  codexAppImageOverlayOpacity: number;
  codexAppImageOverlayFitMode: ImageOverlayFitMode;
  codexAppDreamSkinEnabled: boolean;
  codexAppDreamSkinPaused: boolean;
  codexAppDreamSkinTheme: string;
  codexAppDreamSkinThemeConfig: DreamSkinThemeConfig;
  codexAppDreamSkinImagePath: string;
  codexGoalsEnabled: boolean;
  weixinConnectEnabled: boolean;
  weixinConnectBaseUrl: string;
  weixinConnectToken: string;
  weixinConnectAccountId: string;
  weixinConnectAllowFrom: string;
  weixinConnectRouteTag: string;
  weixinConnectWorkDir: string;
  weixinConnectModel: string;
  weixinConnectSandbox: "read-only" | "workspace-write" | "danger-full-access";
  weixinConnectCodexPath: string;
  relayBaseUrl: string;
  relayApiKey: string;
  relayProfiles: RelayProfile[];
  aggregateRelayProfiles: AggregateRelayProfile[];
  activeAggregateRelayId: string;
  relayCommonConfigContents: string;
  relayPreserveNonproviderSettings: boolean;
  relayContextConfigContents: string;
  activeRelayId: string;
  relayTestModel: string;
  /** 按工具分区的配置镜像，键为工具 id（codex / grok / …）。 */
  tools?: Record<string, ToolShard>;
  /** 侧栏当前聚焦的工具。只影响管理器的展示，不影响 Codex 的启动配置。 */
  activeTool?: string;
};

/** settings.json 里单个工具的配置分片。Codex 分片由后端从扁平字段镜像生成。 */
type ToolShard = {
  relayProfiles?: RelayProfile[];
  activeRelayId?: string;
  aggregateRelayProfiles?: AggregateRelayProfile[];
  activeAggregateRelayId?: string;
  relayCommonConfigContents?: string;
  relayContextConfigContents?: string;
  relayTestModel?: string;
};

type ToolEntry = {
  id: string;
  name: string;
  homeDir: string;
  switchable: boolean;
  active: boolean;
  activeRelayName: string | null;
  relayCount: number;
};

type ToolsResult = {
  status: string;
  message: string;
  tools: ToolEntry[];
  activeTool: string;
};

type ImageOverlayFitMode = "fill" | "fit" | "stretch" | "tile" | "center";
type TypingEffect = "off" | "rainbow" | "fireworks" | "stars";

export type RelayProfile = {
  id: string;
  name: string;
  model: string;
  baseUrl: string;
  upstreamBaseUrl: string;
  apiKey: string;
  apiKeys?: RelayApiKey[];
  activeApiKeyId?: string;
  protocol: RelayProtocol;
  relayMode: RelayMode;
  sessionProvider?: RelaySessionProvider;
  officialMixApiKey: boolean;
  hideOfficialUsageAlert: boolean;
  testModel: string;
  configContents: string;
  authContents: string;
  useCommonConfig: boolean;
  contextSelection: RelayContextSelection;
  contextSelectionInitialized: boolean;
  contextWindow: string;
  autoCompactLimit: string;
  modelList: string;
  modelWindows: string;
  modelAutoCompact: string;
  modelMetadata: string;
  modelVlm: string;
  vlmApiKey: string;
  vlmModel: string;
  vlmBaseUrl: string;
  userAgent: string;
  autoReviewModel?: string;
  customHeaders: { key: string; value: string }[];
  sub2apiEnabled: boolean;
  sub2apiMultiplier: string;
  noAuth: boolean;
  modelRoutes?: RelayModelRoute[];
  standardOpenaiProtocol: boolean;
  webSearchHistoryCompat: boolean;
  localCompactionPromptAtEnd?: boolean;
  trustUpstreamCatalogLite?: boolean;
  strictResponsesReasoning?: boolean;
  /** Anthropic 上游的认证字段：`x-api-key`（默认）或 `authorization`。 */
  anthropicAuthHeader?: string;
  /** Anthropic 的 `max_tokens` 兜底值，0/缺省表示用内置 8192；填了则优先于请求值。 */
  anthropicMaxTokens?: number;
  /** 注入 Anthropic 提示缓存断点，默认开启。 */
  anthropicPromptCache?: boolean;
  /** 伪装 Claude Code 客户端（仅网关限制「只能通过 Claude Code 使用」时需要）。 */
  anthropicImpersonateClaudeCode?: boolean;
  /** 思考模式：空/auto、adaptive（等级透传）、budget（经典预算）、off。 */
  anthropicThinkingMode?: string;
  aggregate?: RelayAggregateConfig | null;
};

type RelayAggregateStrategy = "failover" | "conversationRoundRobin" | "requestRoundRobin" | "weightedRoundRobin";
type AggregateMode = "provider" | "model";
type RelayAggregateMember = {
  profileId: string;
  weight: number;
};
type RelayAggregateRoute = {
  pattern: string;
  profileId: string;
  priority: number;
};
type RelayAggregateConfig = {
  strategy: RelayAggregateStrategy;
  members: RelayAggregateMember[];
  routes?: RelayAggregateRoute[];
  modelAggregationEnabled?: boolean;
  /** 默认供应商：解析不到成员的普通模型请求交给该成员转发；空串表示未设置。 */
  defaultRelayId?: string;
  /** 把默认成员的官方登录同步进 Codex（写 auth.json 的 tokens 段）。默认关。 */
  syncDefaultMemberLogin?: boolean;
};
type AggregateRelayMember = {
  relayId: string;
  weight: number;
};
type AggregateRelayProfile = {
  id: string;
  name: string;
  sessionProvider?: RelaySessionProvider;
  modelAggregationEnabled?: boolean;
  strategy: RelayAggregateStrategy;
  members: AggregateRelayMember[];
  routes?: { pattern: string; relayId: string; priority: number }[];
  defaultRelayId?: string;
  /** 把默认成员的官方登录同步进 Codex（auth.json 的 tokens 段）。默认关。 */
  syncDefaultMemberLogin?: boolean;
};

type RelayContextSelection = {
  mcpServers: string[];
  skills: string[];
  plugins: string[];
};

type ContextKind = "mcp" | "skill" | "plugin";

type CodexContextEntry = {
  id: string;
  kind: ContextKind;
  title: string;
  summary: string;
  tomlBody: string;
  enabled: boolean;
};

type CodexContextEntries = {
  mcpServers: CodexContextEntry[];
  skills: CodexContextEntry[];
  plugins: CodexContextEntry[];
};

type RelayProtocol = "responses" | "chatCompletions" | "anthropicMessages";
type StepwiseProtocol = "auto" | "chat_completions" | "responses" | "anthropic_messages";
type StepwiseGenerationMode = "auto" | "manual";
type RelayMode = "official" | "mixedApi" | "pureApi" | "aggregate";
type RelaySessionProvider = "custom" | "openai";
type RelayEditorMode = "official" | "mixedApi" | "pureApi";
const CHAT_UPSTREAM_BASE_URL_KEY = "codex_plus_chat_base_url";
const SCRIPT_MARKET_REPOSITORY_URL = "https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket";

const emptyContextSelection = (): RelayContextSelection => ({
  mcpServers: [],
  skills: [],
  plugins: [],
});

type UserScriptInventory = {
  enabled?: boolean;
  skip_next_start?: boolean;
  scripts?: Array<{
    key: string;
    name: string;
    source: string;
    enabled: boolean;
    status: string;
    error: string;
    market_id?: string;
    version?: string;
    installed?: boolean;
    source_url?: string;
    homepage?: string;
  }>;
};

type RequestLogUsage = {
  inputTokens: number;
  outputTokens: number;
  cachedTokens?: number;
  reasoningTokens?: number;
};

type RequestLogAttemptDetail = {
  attempt: number;
  code?: string | null;
  message?: string | null;
  delayMs?: number | null;
};

type RequestLogEntry = {
  id: string;
  /** 网关模式下发起请求的 Agent（claude / grok）；Codex 直连代理的记录为空。 */
  agent?: string | null;
  timestampMs: number;
  startedAtMs: number;
  durationMs: number;
  method: string;
  path: string;
  statusCode: number;
  success: boolean;
  stream: boolean;
  model: string;
  upstreamModel: string;
  relayId: string;
  relayName: string;
  wireApi: string;
  /** Codex 会话/线程 id（来自请求头；旧日志没有该字段）。 */
  threadId?: string | null;
  /** 会话标题（后端按 threadId 从 Codex 本地会话库补齐；可能缺失）。 */
  threadTitle?: string | null;
  attempts: number;
  autoRetries: number;
  retryReasons?: string[];
  /** 每次自动重试前的失败尝试摘要；旧日志没有该字段。 */
  attemptDetails?: RequestLogAttemptDetail[] | null;
  /** 请求进行中时为 running；旧日志没有该字段。 */
  state?: string | null;
  /** 首字耗时（毫秒）。 */
  firstTokenMs?: number | null;
  /** 运行中按流式增量粗估的输出 token。 */
  estimatedOutputTokens?: number | null;
  error?: string | null;
  streamOutcome?: string | null;
  usage?: RequestLogUsage | null;
  cacheHitRatio?: number | null;
};

type RequestLogsResult = {
  status: Status;
  message: string;
  entries: RequestLogEntry[];
};

type SettingsResult = CommandResult<{
  revision: string;
  settings: BackendSettings;
  settings_path: string;
  user_scripts: UserScriptInventory;
}>;

type WeixinConnectStatusResult = CommandResult<{
  state: string;
  message: string;
  accountId: string;
  hasToken: boolean;
  lastPeerId: string;
  lastMessageAtMs: number;
  processedMessages: number;
}>;

type WeixinQrResult = CommandResult<{
  qrStatus: string;
  qrContent: string;
  qrSvg: string;
  accountId: string;
  linkedUserId: string;
  hasToken: boolean;
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

type RelayFilesResult = CommandResult<{
  configPath: string;
  authPath: string;
  configContents: string;
  authContents: string;
}>;

type LocalSession = {
  id: string;
  title: string;
  cwd: string;
  modelProvider: string;
  archived: boolean;
  updatedAtMs: number | null;
  rolloutPath: string;
  dbPath: string;
};

type LocalSessionsResult = CommandResult<{
  dbPath: string;
  dbPaths: string[];
  sessions: LocalSession[];
  offset: number;
  limit: number;
  hasMore: boolean;
  totalCount: number;
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

type McpImportPreviewResult = CommandResult<{
  entries: Array<{ id: string; tomlBody: string }>;
  warnings: string[];
}>;

type LiveContextEntriesResult = CommandResult<{
  entries: CodexContextEntries;
}>;

type ExtractRelayCommonConfigResult = CommandResult<{
  commonConfigContents: string;
  profileConfigContents: string;
}>;

type RelaySwitchResult = CommandResult<{
  settings: BackendSettings;
  settingsPath: string;
  user_scripts: unknown;
  relay: RelayPayload;
  /** 实时配置 endpoint 被外部改过而停下时才有（#2666）。 */
  liveConflict?: { profileId: string; profileName: string };
}>;

/** 冲突弹窗里的两种继续方式，对应后端 LiveConflictResolution。 */
type LiveConflictResolution = "importAsNewProfile" | "discardLiveChanges";

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

type StepwiseTestResult = CommandResult<{
  itemCount: number;
  error: string;
}>;

type RelayProfileModelsResult = CommandResult<{
  models: string[];
  endpoint: string;
}>;

type ProviderDoctorCheck = {
  id: string;
  title: string;
  status: Status;
  detail: string;
};

type ProviderDoctorResult = CommandResult<{
  profileName: string;
  model: string;
  summary: string;
  recommendation: string;
  checks: ProviderDoctorCheck[];
}>;

type CcsProviderImport = {
  sourceId: string;
  name: string;
  baseUrl: string;
  apiKey: string;
  protocol: RelayProtocol;
  configContents: string;
  authContents: string;
};

type CcsProvidersResult = CommandResult<{
  dbPath: string;
  configuredDbPath: string;
  fallbackReason?: string | null;
  providers: CcsProviderImport[];
}>;

type SkillInventoryResult = CommandResult<{
  skills: Array<{ id: string; name: string; description: string; enabled: boolean; bundled: boolean }>;
  codexSkillsDir: string;
}>;

type ProviderImportRequest = {
  name: string;
  baseUrl: string;
  apiKey: string;
  wireApi: string;
  relayMode: string;
  configContents: string;
  authContents: string;
};

type PendingProviderImportResult = CommandResult<{
  pending: ProviderImportRequest | null;
}>;

type EnvConflict = {
  name: string;
  source: "process" | "user" | string;
  valuePresent: boolean;
};

type EnvConflictsResult = CommandResult<{
  conflicts: EnvConflict[];
}>;

type RelayEnvironmentResult = CommandResult<{
  clashVergeTun: {
    enabled: boolean;
    configPath: string | null;
  };
  proxyEnvironment: {
    variables: Array<{
      name: string;
      source: "process" | "user" | "system" | string;
    }>;
  };
  codexEnvFile: {
    exists: boolean;
    path: string;
  };
}>;

type RemoveEnvConflictsResult = CommandResult<{
  removed: Array<{
    name: string;
    removedProcess: boolean;
    removedUser: boolean;
  }>;
  backupPath: string | null;
  remaining: EnvConflict[];
}>;

type ProviderSyncPayload = {
  syncStatus?: string;
  syncMessage?: string;
  recoveryJournalPath?: string | null;
  recoveryAvailable?: boolean;
  recoveryErrors?: string[];
  databaseOutcomes?: Array<{ database: string; attemptedRows: number; restoredRows: number; remainingRows: number }>;
  targetProvider?: string;
  changedSessionFiles?: number;
  skippedLockedRolloutFiles?: string[];
  sqliteRowsUpdated?: number;
  sqliteProviderRowsUpdated?: number;
  sqliteUserEventRowsUpdated?: number;
  sqliteCwdRowsUpdated?: number;
  sqliteCatalogRowsInserted?: number;
  sqliteCatalogRowsRemoved?: number;
  updatedWorkspaceRoots?: number;
  prunedSessionIndexEntries?: number;
  encryptedContentWarning?: string | null;
  repairAudit?: {
    catalogOnlySessions: number;
    catalogOnlyWithCurrentRollout: number;
    catalogOnlyWithBackupDatabase: number;
    catalogOnlyWithoutRecoverySource: number;
  };
  backupDir?: string | null;
};

type SessionIndexCleanupCandidate = {
  id: string;
  threadName: string;
  updatedAt: string;
};

type SessionIndexCleanupPreviewPayload = {
  snapshotSha256: string;
  candidates: SessionIndexCleanupCandidate[];
};

type SessionIndexCleanupApplyPayload = {
  prunedEntries?: number;
  backupDir?: string | null;
};

type ProviderSyncTargetSource = "config" | "rollout" | "sqlite" | "manual";

type ProviderSyncTargetOption = {
  id: string;
  sources: ProviderSyncTargetSource[];
  isCurrentProvider: boolean;
  isManual: boolean;
  isSaved: boolean;
  isResolvable: boolean;
  unavailableReason: string | null;
};

type ProviderSyncTargetsPayload = {
  currentProvider: string;
  targets: ProviderSyncTargetOption[];
};

type ProviderSyncTargetsResult = CommandResult<ProviderSyncTargetsPayload>;

type ProviderSyncProgress = {
  active: boolean;
  percent: number;
  message: string;
  result: CommandResult<ProviderSyncPayload> | null;
};

type TaskProgress = {
  active: boolean;
  percent: number;
  message: string;
  indeterminate?: boolean;
  failed?: boolean;
};

type UpdateStage = "downloading" | "writing" | "launching";

type UpdateProgressEvent = {
  stage: UpdateStage;
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number | null;
  elapsedMs: number;
};

type SessionIndexRepairReport = {
  scannedFiles: number;
  cachedFiles: number;
  repairedItems: number;
  alreadyPresent: number;
  skippedItems: number;
  deferredItems?: number;
  issues: string[];
  issuesTruncated?: number;
  abortedReason?: string | null;
  warnings?: string[];
  backupPath: string | null;
  elapsedMs: number;
  checkedAtMs?: number;
  pendingDetails?: {
    threadId: string | null;
    turnId: string | null;
    reason: string;
    state: "waiting" | "blocked";
    firstSeenAtMs: number;
    lastCheckedAtMs: number;
    checks: number;
  }[];
};

type LogsResult = CommandResult<{
  path: string;
  text: string;
  lines: number;
  truncated: boolean;
  fileSize: number;
}>;

type DiagnosticsResult = CommandResult<{
  report: string;
}>;

type WatcherResult = CommandResult<{
  enabled: boolean;
  disabled_flag: string;
  start_minimized: boolean;
  start_codex: boolean;
  start_codex_on_open: boolean;
}>;

// 与 Rust 侧 `auto_takeover::AutoTakeoverPayload`（rename_all = "camelCase"）逐字段对齐。
// 少一个字段的表现是「状态行永远空白」，所以两边要一起改。
type AutoTakeoverResult = CommandResult<{
  state: string;
  enabled: boolean;
  platformSupported: boolean;
  activePolicy: string;
  lastTakeoverAtMs?: number | null;
  lastReason?: string | null;
  lastError?: string | null;
  actualDebugPort?: number | null;
  appRunning?: boolean | null;
  sessionOwned: boolean;
  sessionStatus?: string | null;
  restartsLastHour: number;
  restartsTotal: number;
  pausedInstance?: string | null;
  running: boolean;
}>;

type InstallResult = CommandResult<{
  silent_shortcut: { installed: boolean; path: string | null };
  management_shortcut: { installed: boolean; path: string | null };
}>;

type UpdateResult = CommandResult<{
  currentVersion: string;
  latestVersion?: string | null;
  releaseSummary?: string;
  assetName?: string | null;
  assetUrl?: string | null;
  updateAvailable?: boolean;
  installedPath?: string;
  progress?: number;
}>;

type AdItem = {
  id?: string;
  type: "sponsor" | "normal" | string;
  title: string;
  description: string;
  url: string;
  image?: string;
  highlights?: string[];
  expires_at?: string;
};

type AdsResult = CommandResult<{
  version: number;
  ads: AdItem[];
  /// 兼容旧数据和旧管理端消费者的首条置顶赞助位。
  topAd?: AdItem;
  /// 独立置顶赞助位列表，供概览横幅轮播。
  topAds?: AdItem[];
}>;

type ScriptMarketItem = {
  id: string;
  name: string;
  description: string;
  version: string;
  author: string;
  tags: string[];
  homepage: string;
  script_url: string;
  installed: boolean;
  installedVersion: string;
  updateAvailable: boolean;
};

type ScriptMarketResult = CommandResult<{
  market: {
    status: string;
    message: string;
    indexUrl: string;
    updatedAt: string;
    scripts: ScriptMarketItem[];
  };
  user_scripts: UserScriptInventory;
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

/** 左侧 Agent 切换栏的工具标识。后端 `list_tools` 返回同名字符串。 */
type ToolId = string;

/** Codex++ 自己的 logo：与应用图标（src-tauri/icons/icon.png）同一张图。 */
const APP_LOGO_URL = new URL("./assets/codex-plus-logo.png", import.meta.url).href;

type Route = "overview" | "relay" | "multiLaunch" | "claude" | "relayEnvironment" | "requestLog" | "sessions" | "context" | "skills" | "weixin" | "enhance" | "dreamSkin" | "userScripts" | "pluginMarket" | "recommendations" | "agentCache" | "agentCollaboration" | "computerUse" | "agentPet" | "hub" | "gateway" | "gatewayProviders" | "gatewayRouting" | "gatewayUsage" | "gatewaySettings" | "maintenance" | "about" | "settings" | "agentSettings";
type Theme = "dark" | "light";

const MANAGER_NAVIGATION_EVENT = "manager-navigation-requested";

/**
 * 导航项归属。
 *
 * - `"codex"` / `"grok"`：这一页只属于某个工具，切到别的工具时隐藏。
 *   绝大部分功能（会话、MCP、皮肤、脚本市场、安装维护…）都是 Codex 专属的。
 * - 不写 `tool`：与工具无关的应用级页面（设置、关于），任何工具下都显示。
 *
 * 新增页面时**必须**想清楚归属：默认可见会让 Codex 专属功能在 Grok 下露出来。
 */
const routes: Array<{ id: Route; label: string; icon: LucideIcon; badge?: string; tool?: string }> = [
  // 概览在两个工具下都可见：它承载共用的置顶推荐位，以及各自的状态。
  { id: "overview", get label() { return t("概览"); }, icon: LayoutDashboard },
  { id: "relay", get label() { return t("旧版供应商配置"); }, icon: KeyRound, tool: "codex" },
  { id: "multiLaunch", get label() { return t("多开"); }, icon: Layers3, tool: "codex" },
  { id: "claude", get label() { return t("增强"); }, icon: Languages, tool: "claude" },
  // 请求日志、会话、MCP、Skills 管理所有 Agent，不属于某一个 Agent。
  { id: "sessions", get label() { return t("会话"); }, icon: MessagesSquare },
  { id: "context", label: "MCP", icon: Network },
  { id: "skills", label: "Skills", icon: BookOpen },
  { id: "weixin", get label() { return t("微信连接"); }, icon: ScanLine, tool: "codex" },
  { id: "enhance", get label() { return t("增强"); }, icon: Hammer, tool: "codex" },
  { id: "dreamSkin", get label() { return t("皮肤管理"); }, icon: Palette, tool: "codex" },
  { id: "userScripts", get label() { return t("脚本"); }, icon: FileCode2, tool: "codex" },
  { id: "pluginMarket", get label() { return t("插件市场"); }, icon: Store, tool: "codex" },
  { id: "recommendations", get label() { return t("推荐内容"); }, icon: ExternalLink },
  // 缓存清理覆盖多个 AI 应用，属于应用级页面，在各工具下均可访问。
  { id: "agentCache", get label() { return t("AI Agent 缓存清理"); }, icon: Trash2 },
  // 跨客户端协作属于应用级能力，各工具下共用同一组任务。
  { id: "agentCollaboration", get label() { return t("Agent 协作"); }, icon: GitBranch, badge: "实验" },
  { id: "computerUse", get label() { return t("Computer Use"); }, icon: MousePointerClick, badge: "实验" },
  // 桌宠服务所有 Agent（目前只接入了 Codex），与具体 Agent 无关，放在「工具」里。
  { id: "agentPet", get label() { return t("Agent 桌宠"); }, icon: PawPrint },
  // 网关：任意 Agent 经 Codex++ 接入任意供应商，与具体 Agent 无关，是应用级页面。
  { id: "hub", get label() { return t("总览"); }, icon: Waypoints },
  // 先有供应商，再接 Agent；路由是进阶设置，放在 Agent 接入之后。
  { id: "gatewayProviders", get label() { return t("供应商池"); }, icon: Server },
  { id: "gateway", get label() { return t("Agent 接入"); }, icon: Plug },
  { id: "gatewayRouting", get label() { return t("路由"); }, icon: Split },
  { id: "gatewayUsage", get label() { return t("用量"); }, icon: BarChart3 },
  { id: "gatewaySettings", get label() { return t("网关设置"); }, icon: SlidersHorizontal },
  { id: "maintenance", get label() { return t("安装维护"); }, icon: Wrench, tool: "codex" },
  { id: "about", get label() { return t("关于"); }, icon: Info },
  { id: "settings", get label() { return t("整体设置"); }, icon: Settings },
  // Agent 专属设置：目前只有 Codex 有专属选项（图片覆盖层、启动参数）。
  // Claude / Grok 没有独立设置项，原来那页只有一个跳回各自主页的按钮，所以不再给它们显示。
  { id: "agentSettings", get label() { return t("设置"); }, icon: Settings, tool: "codex" },
  { id: "relayEnvironment", get label() { return t("旧版代理设置"); }, icon: ShieldCheck, tool: "codex" },
  { id: "requestLog", get label() { return t("请求日志"); }, icon: List },
];

/**
 * Agent 的页面分组：侧栏只列 Agent，点开后它的全部页面都在内容区顶部的页签里，分组之间用细线隔开。
 * 每个页面仍按 `routes` 里的 `tool` 归属过滤；只有一个页面的 Agent 不显示页签。
 */
const navigationSections: Array<{ id: string; routes: Route[] }> = [
  { id: "overview", routes: ["overview"] },
  { id: "providers", routes: ["relay", "relayEnvironment", "multiLaunch"] },
  { id: "enhance", routes: ["claude", "enhance", "dreamSkin", "userScripts", "pluginMarket", "weixin"] },
  // 安装维护并进了设置页（排在设置下方），不再单独占一个页签；"maintenance" 路由保留为设置页的别名。
  { id: "settings", routes: ["agentSettings"] },
];
const routeTool = (routeId: Route) => routes.find((candidate) => candidate.id === routeId)?.tool;
/** 某个 Agent 能看到的全部页面，按页签顺序。 */
const agentRoutes = (toolId: ToolId) => agentTabGroups(navigationSections, routeTool, toolId).flatMap((entry) => entry.routes);
/** 每个 Agent 上次打开的页面：在侧栏点 Agent 时回到那里。 */
const lastAgentRoute = new Map<string, Route>();

/**
 * 与具体 Agent 无关的应用级页面：网关、工具、关于、整体设置。打开它们时侧栏里没有 Agent 处于选中态。
 * Computer Use 同时接入 Claude Code / Claude Desktop / DSH 等多个客户端，不属于任何一个 Agent，
 * 所以和 Agent 协作一样放在「工具」里，而不是重复出现在每个 Agent 的「扩展」分组里。
 */
const globalNavigationRoutes: Route[] = ["hub", "gatewayProviders", "gateway", "gatewayRouting", "gatewayUsage", "gatewaySettings", "requestLog", "sessions", "context", "skills", "computerUse", "agentPet", "agentCollaboration", "agentCache", "recommendations", "about", "settings"];
/** 侧栏「网关」分组：任意 Agent 经 Codex++ 接入任意供应商。 */
const gatewayNavigationRoutes: Route[] = ["hub", "gatewayProviders", "gateway", "gatewayRouting", "gatewayUsage", "gatewaySettings"];
/** 侧栏「全局管理」分组：同时管理所有 Agent 的请求日志、会话、MCP 与 Skills。 */
const managementNavigationRoutes: Route[] = ["requestLog", "sessions", "context", "skills"];
/** 侧栏「工具」分组：服务所有 Agent，但与供应商无关。关于、整体设置放在侧栏底部。 */
const toolNavigationRoutes: Route[] = ["recommendations", "computerUse", "agentPet", "agentCollaboration", "agentCache"];
/** 网关的各个页签由侧栏直接选中，页内不再显示标签条。 */
const gatewayTabByRoute: Partial<Record<Route, GatewayTab>> = {
  gateway: "agents",
  gatewayProviders: "providers",
  gatewayRouting: "routing",
  gatewayUsage: "usage",
  gatewaySettings: "gateway",
};
// 网关里的「会话」已并进全局管理的「会话」页。
const gatewayRouteByTab = { ...Object.fromEntries(Object.entries(gatewayTabByRoute).map(([route, tab]) => [tab, route])), sessions: "sessions" } as Record<GatewayTab, Route>;
/**
 * 每个 Agent 在侧栏里的落脚页：点 Agent 名字先到这里。
 * Claude Code / Grok 的供应商不再在 Codex++ 里单独配置：要么经网关接入供应商池（「Agent 接入」），
 * 要么不接入、照它自己的配置文件直连。
 */
const agentHomeRoute: Record<string, Route> = { codex: "overview", claude: "claude", grok: "overview" };
const agentEntryRoute = (toolId: ToolId) => agentTarget(agentRoutes(toolId), lastAgentRoute.get(toolId), agentHomeRoute[toolId] ?? "overview");

const defaultSettings: BackendSettings = {
  agentPet: defaultAgentPetSettings(),
  codexAppPath: "",
  codexAppAutoTakeover: false,
  codexAppAutoTakeoverWhenActive: "defer",
  codexExtraArgs: [],
  dictation: defaultDictationSettings(),
  providerSyncEnabled: false,
  threadModelFollowSwitchEnabled: false,
  ccsDbPath: "",
  providerSyncSavedProviders: [],
  providerSyncManualProviders: [],
  providerSyncLastSelectedProvider: "",
  relayProfilesEnabled: true,
  localConfigProviderImported: false,
  enhancementsEnabled: true,
  codexAppStandaloneQuotaUnlock: false,
  codexAppPluginMarketplaceUnlock: true,
  codexAppModelWhitelistUnlock: true,
  codexAppSessionDelete: true,
  codexAppSideChatSave: false,
  codexAppMarkdownExport: true,
  codexAppSessionShare: true,
  codexAppUsageStatusLine: true,
  codexAppPasteFix: false,
  codexAppForceChineseLocale: false,
  codexAppTypingEffect: "off",
  codexAppThreadIdBadge: false,
  codexAppConversationView: false,
  codexAppThreadScrollRestore: true,
  codexAppNativeMenuPlacement: true,
  codexAppNativeBrowserRequireIdentification: false,
  codexAppNativeFastUnlock: false,
  codexAppDisableWorkView: false,
  codexAppPetRealMouseLook: false,
  codexAppActivityOverlayRight: false,
  protocolProxyAutoRetry: true,
  protocolProxyAutoRetryMaxAttempts: 5,
  protocolProxyAutoRetryIntervalSeconds: 2,
  protocolProxyAutoRetryStatusCodes: [],
  protocolProxyAutoRetryMode: "streaming",
  protocolProxyAutoRetryCommitDelayMs: 1000,
  protocolProxyMapUnlistedModels: false,
  helperMaxRequestBodyMib: 128,
  codexAppCapacityAutoRetry: false,
  codexAppTurnRewind: false,
  codexAppStepwiseEnabled: false,
  codexAppAnswerOutlineEnabled: false,
  codexAppStepwiseDirectSend: false,
  codexAppStepwiseProtocol: "chat_completions",
  codexAppStepwiseGenerationMode: "auto",
  codexAppStepwiseBaseUrl: "",
  codexAppStepwiseApiKey: "",
  codexAppStepwiseApiKeyEnv: "CODEX_STEPWISE_API_KEY",
  codexAppStepwiseModel: "",
  codexAppStepwiseMaxItems: 4,
  codexAppStepwiseMaxInputChars: 6000,
  codexAppStepwiseMaxOutputTokens: 500,
  codexAppStepwiseTimeoutMs: 8000,
  codexAppImageOverlayEnabled: false,
  codexAppImageOverlayPath: "",
  codexAppImageOverlayOpacity: 35,
  codexAppImageOverlayFitMode: "fit",
  codexAppDreamSkinEnabled: false,
  codexAppDreamSkinPaused: false,
  codexAppDreamSkinTheme: "pink",
  codexAppDreamSkinThemeConfig: defaultDreamSkinTheme(),
  codexAppDreamSkinImagePath: "",
  codexGoalsEnabled: false,
  weixinConnectEnabled: false,
  weixinConnectBaseUrl: "https://ilinkai.weixin.qq.com",
  weixinConnectToken: "",
  weixinConnectAccountId: "",
  weixinConnectAllowFrom: "",
  weixinConnectRouteTag: "",
  weixinConnectWorkDir: "",
  weixinConnectModel: "",
  weixinConnectSandbox: "read-only",
  weixinConnectCodexPath: "",
  relayBaseUrl: "",
  relayApiKey: "",
  relayProfiles: [
    {
      id: "default",
      name: t("默认中转"),
      model: "",
      baseUrl: "",
      upstreamBaseUrl: "",
      apiKey: "",
      protocol: "responses",
      relayMode: "official",
      officialMixApiKey: false,
      hideOfficialUsageAlert: false,
      testModel: "",
      configContents: "",
      authContents: "",
      useCommonConfig: true,
      contextSelection: emptyContextSelection(),
      contextSelectionInitialized: true,
      contextWindow: "",
      autoCompactLimit: "",
      modelList: "",
      modelWindows: "",
      modelAutoCompact: "",
      modelMetadata: "",
      modelVlm: "",
      vlmApiKey: "",
      vlmModel: "",
      vlmBaseUrl: "",
      userAgent: "",
      customHeaders: [],
      sub2apiEnabled: false,
      noAuth: false,
      sub2apiMultiplier: "",
      standardOpenaiProtocol: false,
      webSearchHistoryCompat: false,
    },
  ],
  relayCommonConfigContents: "",
  relayPreserveNonproviderSettings: false,
  relayContextConfigContents: "",
  activeRelayId: "default",
  aggregateRelayProfiles: [],
  activeAggregateRelayId: "",
  relayTestModel: "gpt-5.4-mini",
  tools: {},
  activeTool: "codex",
};

export function App() {
  useSyncExternalStore(subscribeLanguage, getLanguage, getLanguage);
  const [theme, setTheme] = useState<Theme>(() => loadInitialTheme());
  const [route, setRoute] = useState<Route>(() => loadInitialRoute());
  /** 合并后的「设置」页里要滚到的分组（原来跳「关于」的地方滚到版本与更新、日志）。 */
  const [settingsFocus, setSettingsFocus] = useState<string | null>(() => initialSettingsFocus());
  const [enhancementTab, setEnhancementTab] = useState<EnhancementTab>("general");
  const [pendingEnhancementSection, setPendingEnhancementSection] = useState<ManagerNavigationIntent["section"] | null>(null);
  const [notice, setNotice] = useState<{ title: string; message: string; status?: Status } | null>(null);
  const [confirmDialog, setConfirmDialog] = useState<{
    title: string;
    message: string;
    confirmText: string;
    cancelText: string;
    resolve: (confirmed: boolean) => void;
  } | null>(null);
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
  // 外部改设置时要拿「上次读到的设置」与当前编辑比对，监听回调里读这个 ref 拿最新值。
  const settingsRef = useRef<SettingsResult | null>(null);
  settingsRef.current = settings;
  const [weixinStatus, setWeixinStatus] = useState<WeixinConnectStatusResult | null>(null);
  const [weixinQr, setWeixinQr] = useState<WeixinQrResult | null>(null);
  const [relay, setRelay] = useState<RelayResult | null>(null);
  const [relayFiles, setRelayFiles] = useState<RelayFilesResult | null>(null);
  const [envConflicts, setEnvConflicts] = useState<EnvConflictsResult | null>(null);
  const [relayEnvironment, setRelayEnvironment] = useState<RelayEnvironmentResult | null>(null);
  const [ccsProviders, setCcsProviders] = useState<CcsProvidersResult | null>(null);
  const [pendingProviderImport, setPendingProviderImport] = useState<ProviderImportRequest | null>(null);
  const [localSessions, setLocalSessions] = useState<LocalSessionsResult | null>(null);
  const [liveContextEntries, setLiveContextEntries] = useState<CodexContextEntries | null>(null);
  const [logs, setLogs] = useState<LogsResult | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsResult | null>(null);
  const [watcher, setWatcher] = useState<WatcherResult | null>(null);
  const [dreamSkinStatus, setDreamSkinStatus] = useState<DreamSkinRuntimeResult | null>(null);
  const [dreamSkinVerification, setDreamSkinVerification] = useState<DreamSkinVerificationResult | null>(null);
  const [dreamSkinLibrary, setDreamSkinLibrary] = useState<DreamSkinThemeLibrary | null>(null);
  const [dreamSkinMarket, setDreamSkinMarket] = useState<DreamSkinMarketResult | null>(null);
  const [dreamSkinCommunity, setDreamSkinCommunity] = useState<DreamSkinCommunityResult | null>(null);
  const [dreamSkinLibraryLoading, setDreamSkinLibraryLoading] = useState(false);
  const [dreamSkinMarketLoading, setDreamSkinMarketLoading] = useState(false);
  const [dreamSkinCommunityLoading, setDreamSkinCommunityLoading] = useState(false);
  const [dreamSkinInstallOperation, setDreamSkinInstallOperation] = useState<DreamSkinInstallOperation | null>(null);
  const dreamSkinInstallLockRef = useRef(false);
  const [pendingDreamSkinCommunity, setPendingDreamSkinCommunity] = useState("");
  const [selectedDreamSkinTheme, setSelectedDreamSkinTheme] = useState("builtin");
  const [savedDreamSkinThemeDraft, setSavedDreamSkinThemeDraft] = useState<DreamSkinThemeDraft | null>(null);
  const [dreamSkinThemeDraft, setDreamSkinThemeDraft] = useState<DreamSkinThemeDraft | null>(null);
  const [pendingDreamSkinRestart, setPendingDreamSkinRestart] = useState<PendingDreamSkinRestart | null>(null);
  const [dreamSkinUnsavedDialog, setDreamSkinUnsavedDialog] = useState(false);
  const dreamSkinPendingActionRef = useRef<(() => void) | null>(null);
  const [update, setUpdate] = useState<UpdateResult | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [tourOpen, setTourOpen] = useState(() => initialTourOpen());
  const closeTour = useCallback(() => {
    // 看完或跳过都记下来，下次不再自动弹出；设置页里可以重新打开。
    try { window.localStorage.setItem(TOUR_STORAGE_KEY, String(TOUR_VERSION)); } catch { /* 存不了就只影响本次 */ }
    setTourOpen(false);
  }, []);
  // 「会话」页默认看所有 Agent；切到 Codex 页签才是带删除 / 归档 / 修复的完整会话管理。
  const [sessionTab, setSessionTab] = useState<SessionTab>("all");
  /** 请求日志打开时的 Agent 过滤（从 Agent 概览跳过去时设置，用一次就清掉）。 */
  const [requestLogAgentFilter, setRequestLogAgentFilter] = useState<"all" | "codex" | "claude" | "grok">("all");
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
  const [updatePrompt, setUpdatePrompt] = useState<UpdateResult | null>(null);
  const updateNotifications = useRef(new UpdateNotifications());
  const updateCheckPending = useRef<{ manual: boolean } | null>(null);
  const updateInstallPending = useRef(false);
  const [updateInstallProgress, setUpdateInstallProgress] = useState<TaskProgress>({
    active: false,
    percent: 0,
    message: t("尚未运行安装包更新。"),
  });
  const [ads, setAds] = useState<AdsResult | null>(null);
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
  const dreamSkinDraftDirty = Boolean(
    savedDreamSkinThemeDraft
      && dreamSkinThemeDraft
      && isDreamSkinDraftDirty(savedDreamSkinThemeDraft, dreamSkinThemeDraft),
  );
  const settingsDirty = useMemo(
    () => Boolean(settings && !backendSettingsEqual(settingsForm, settings.settings)),
    [settings, settingsForm],
  );

  const call = <T,>(command: string, args?: Record<string, unknown>) => {
    const snapshot = args?.settings as BackendSettings | undefined;
    return invoke<T>(command, command === "save_settings"
      ? { ...args, expectedRevision: snapshot?.revision || "" }
      : args).then((result) => {
        const response = result as { message?: string };
        if (command === "save_settings" && response?.message?.includes("settings_revision_conflict")) {
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
      setToolEntries(result.tools ?? []);
      setActiveTool(result.activeTool || "codex");
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
    if (currentRoute?.tool && currentRoute.tool !== toolId) {
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

  // Computer Use 页的开关由后端单独写进 settings.json：重新读取，只合并这几个键和版本号，
  // 别处没保存的草稿保持不动；否则表单里的旧版本号会让之后的保存全部报冲突。
  useEffect(() => {
    const sync = async () => {
      const result = await run(() => call<SettingsResult>("load_settings"));
      if (!result) return;
      const fresh = normalizeSettings(result.settings) as BackendSettings & Record<string, unknown>;
      setSettings(result);
      setSettingsForm((current) => {
        const merged: Record<string, unknown> = { ...current, revision: fresh.revision };
        for (const key of ["computerUseMode", "computerUseApproval", "computerUsePip"]) merged[key] = fresh[key];
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
      setActiveTool(normalized.activeTool || "codex");
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

  const dreamSkinRequest = (screenshotPath?: string) => ({
    request: {
      debugPort: overview?.latest_launch?.debug_port ?? parsePort(launchForm.debugPort, 9229),
      helperPort: overview?.latest_launch?.helper_port ?? parsePort(launchForm.helperPort, 57321),
      screenshotPath: screenshotPath || null,
    },
  });

  const refreshDreamSkinStatus = async (silent = false) => {
    const result = await run(() => call<DreamSkinRuntimeResult>("dream_skin_status", dreamSkinRequest()));
    if (result) {
      setDreamSkinStatus(result);
      if (!silent || !isSuccessStatus(result.status)) {
        showResultNotice(t("Dream Skin 状态"), result, { silentSuccess: true });
      }
    }
    return result;
  };

  const refreshScriptMarket = async (silent = false) => {
    const result = await run(() => call<ScriptMarketResult>("refresh_script_market"));
    if (result) {
      setScriptMarket(result);
      setSettings((current) => (current ? { ...current, user_scripts: result.user_scripts } : current));
      if (!silent || !isSuccessStatus(result.status)) showResultNotice(t("脚本"), result, { silentSuccess: true });
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

  const reloadUserScripts = async () => {
    const result = await run(() => call<SettingsResult>("reload_user_scripts"));
    if (result) {
      setSettings(result);
      setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
      showResultNotice(t("本地脚本"), result);
    }
  };

  const installMarketScript = async (id: string) => {
    const result = await run(() => call<ScriptMarketResult>("install_market_script", { id }));
    if (result) {
      setScriptMarket(result);
      setSettings((current) => (current ? { ...current, user_scripts: result.user_scripts } : current));
      showResultNotice(t("脚本"), result);
    }
  };

  const setUserScriptsEnabled = async (enabled: boolean) => {
    const result = await run(() => call<SettingsResult>("set_user_scripts_enabled", { enabled }));
    if (!result) return;
    setSettings(result);
    setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
    showResultNotice(t("本地脚本"), result);
    if (isSuccessStatus(result.status)) await reloadUserScripts();
  };

  const setUserScriptsSkipNextStart = async (skip: boolean) => {
    const result = await run(() => call<SettingsResult>("set_user_scripts_skip_next_start", { skip }));
    if (!result) return;
    setSettings(result);
    setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
    showResultNotice(t("本地脚本"), result);
  };

  const setUserScriptEnabled = async (key: string, enabled: boolean) => {
    const result = await run(() => call<SettingsResult>("set_user_script_enabled", { key, enabled }));
    if (result) {
      setSettings(result);
      setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
      showResultNotice(t("本地脚本"), result);
      await refreshUserScriptInventory();
    }
  };

  const deleteUserScript = async (key: string) => {
    const script = settings?.user_scripts?.scripts?.find((item) => item.key === key);
    const name = script?.name || key;
    if (!(await appConfirm(tf("删除脚本“{0}”？此操作会移除本地脚本文件。", [name]), { title: t("删除脚本"), confirmText: t("删除"), danger: true }))) return;
    const result = await run(() => call<SettingsResult>("delete_user_script", { key }));
    if (result) {
      setSettings(result);
      setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
      showResultNotice(t("本地脚本"), result);
      await refreshUserScriptInventory();
    }
  };

  const refreshRelay = async (silent = false) => {
    const result = await run(() => call<RelayResult>("relay_status"));
    if (result) {
      setRelay(result);
      if (!silent) showResultNotice(t("登录状态"), result, { silentSuccess: true });
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
      setEnvConflicts({
        status: result.status,
        message: result.message,
        conflicts: result.remaining,
      });
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
      setPendingProviderImport(result.pending);
      if (!silent && !isSuccessStatus(result.status)) showResultNotice(t("Codex++ 导入"), result, { silentSuccess: true });
    }
    return result;
  };

  const confirmPendingProviderImport = async () => {
    const result = await run(() => call<SettingsResult>("confirm_pending_provider_import"));
    if (result) {
      setPendingProviderImport(null);
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showResultNotice(t("Codex++ 导入"), result);
      await refreshCcsProviders(true);
    }
  };

  const dismissPendingProviderImport = async () => {
    const result = await run(() => call<PendingProviderImportResult>("dismiss_pending_provider_import"));
    if (result) {
      setPendingProviderImport(null);
      showResultNotice(t("Codex++ 导入"), result, { silentSuccess: true });
    }
  };

  const refreshLocalSessions = async (silent = false, offset = 0): Promise<LocalSessionsResult | null> => {
    const result = await run(() =>
      call<LocalSessionsResult>("list_local_sessions", {
        request: { offset, limit: 50 },
      }),
    );
    if (result) {
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

  const confirmSessionDelete = (title: string, message: string) =>
    new Promise<boolean>((resolve) => {
      setConfirmDialog({
        title,
        message,
        confirmText: t("确认删除"),
        cancelText: t("取消"),
        resolve,
      });
    });

  const setDreamSkinDraftSelection = (
    key: string,
    draft: DreamSkinThemeDraft,
  ) => {
    setSelectedDreamSkinTheme(key);
    setSavedDreamSkinThemeDraft(draft);
    setDreamSkinThemeDraft(draft);
  };

  const refreshDreamSkinLibrary = async (silent = false) => {
    setDreamSkinLibraryLoading(true);
    try {
      const result = await run(() => call<DreamSkinThemeLibraryResult>("list_dream_skin_themes"));
      if (!result) return null;
      const library: DreamSkinThemeLibrary = {
        themes: result.themes,
        activeDraft: result.activeDraft,
      };
      setDreamSkinLibrary(library);
      const active = library.themes.find((item) => item.active) ?? library.themes[0];
      if (active) {
        const draft = active.builtin
          ? { config: defaultDreamSkinTheme(), imagePath: "", builtin: true }
          : library.activeDraft;
        setDreamSkinDraftSelection(active.key, draft);
      }
      if (!silent && !isSuccessStatus(result.status)) {
        showResultNotice(t("主题库"), result);
      }
      return library;
    } finally {
      setDreamSkinLibraryLoading(false);
    }
  };

  const refreshDreamSkinMarket = async (silent = false) => {
    setDreamSkinMarketLoading(true);
    try {
      const result = await run(() => call<DreamSkinMarketResult>("refresh_dream_skin_market"));
      if (result) {
        setDreamSkinMarket(result);
        if (!silent || !isSuccessStatus(result.status)) {
          showResultNotice(t("主题市场"), result, { silentSuccess: true });
        }
      }
      return result;
    } finally {
      setDreamSkinMarketLoading(false);
    }
  };

  const refreshDreamSkinCommunity = async (silent = false) => {
    setDreamSkinCommunityLoading(true);
    try {
      const result = await run(() => call<DreamSkinCommunityResult>("refresh_dream_skin_community"));
      if (result) {
        setDreamSkinCommunity(result);
        if (!silent || !isSuccessStatus(result.status)) {
          showResultNotice(t("DreamSkin 社区"), result, { silentSuccess: true });
        }
      }
      return result;
    } finally {
      setDreamSkinCommunityLoading(false);
    }
  };

  const runDreamSkinInstallOperation = async <T,>(
    operation: DreamSkinInstallOperation,
    install: () => Promise<T>,
  ): Promise<T | null> => runExclusiveDreamSkinInstall(
    dreamSkinInstallLockRef,
    (pending) => setDreamSkinInstallOperation(pending ? operation : null),
    install,
  );

  const installDreamSkinCommunityTheme = async (theme: DreamSkinCommunityTheme) => {
    const installed = await runDreamSkinInstallOperation(
      { source: "community", id: theme.id, name: theme.name },
      async () => {
        const result = await run(() => call<DreamSkinCommunityResult>(
          "install_dream_skin_community_theme",
          { id: theme.id },
        ));
        if (!result) return false;
        setDreamSkinCommunity(result);
        showResultNotice(t("DreamSkin 社区"), result);
        if (!isSuccessStatus(result.status)) return false;
        await refreshDreamSkinLibrary(true);
        const draft = await loadDreamSkinThemeDraft(theme.themeId);
        if (draft) setDreamSkinDraftSelection(`stored:${theme.themeId}`, draft);
        return true;
      },
    );
    return installed ?? false;
  };

  const refreshPendingDreamSkinCommunity = async () => {
    const result = await run(() => call<PendingDreamSkinCommunityResult>("load_pending_dream_skin_community"));
    if (result) setPendingDreamSkinCommunity(result.versionId);
    return result;
  };

  const confirmPendingDreamSkinCommunity = async () => {
    await runDreamSkinInstallOperation(
      { source: "pending-community", id: pendingDreamSkinCommunity, name: t("网页一键换肤主题") },
      async () => {
        const result = await run(() => call<DreamSkinCommunityResult>("confirm_pending_dream_skin_community"));
        if (!result) return false;
        setDreamSkinCommunity(result);
        showResultNotice(t("DreamSkin 社区"), result);
        if (!isSuccessStatus(result.status)) return false;
        setPendingDreamSkinCommunity("");
        setRoute("dreamSkin");
        await refreshDreamSkinLibrary(true);
        if (result.installedThemeId) {
          const draft = await loadDreamSkinThemeDraft(result.installedThemeId);
          if (draft) {
            setDreamSkinDraftSelection(`stored:${result.installedThemeId}`, draft);
            await activateDreamSkinDraft(draft);
          }
        }
        return true;
      }
    );
  };

  const dismissPendingDreamSkinCommunity = async () => {
    if (dreamSkinInstallLockRef.current && dreamSkinInstallOperation?.source === "pending-community") return;
    const result = await run(() => call<PendingDreamSkinCommunityResult>("dismiss_pending_dream_skin_community"));
    if (!result) return;
    if (isSuccessStatus(result.status)) setPendingDreamSkinCommunity("");
    else showResultNotice(t("DreamSkin 社区"), result);
  };

  const importDreamSkinThemePackage = async () => {
    let selected: string | string[] | null;
    try {
      selected = await open({
        title: t("导入 DreamSkin 主题包"),
        multiple: false,
        directory: false,
        filters: [{ name: "DreamSkin ZIP", extensions: ["zip"] }],
      });
    } catch (error) {
      showNotice(t("主题库"), tf("打开选择器失败：{0}", [stringifyError(error)]), "failed");
      return;
    }
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) return;
    await runDreamSkinInstallOperation(
      { source: "import", id: "local-package", name: t("本地主题包") },
      async () => {
        const previousIds = new Set(dreamSkinLibrary?.themes.map((item) => item.id) ?? []);
        const result = await run(() => call<DreamSkinThemeLibraryResult>(
          "import_dream_skin_theme_package",
          { path },
        ));
        if (!result) return false;
        showResultNotice(t("主题库"), result);
        if (!isSuccessStatus(result.status)) return false;
        const library = { themes: result.themes, activeDraft: result.activeDraft };
        setDreamSkinLibrary(library);
        const imported = result.themes.find((item) => item.kind === "stored" && !previousIds.has(item.id));
        if (imported) {
          const draft = await loadDreamSkinThemeDraft(imported.id);
          if (draft) setDreamSkinDraftSelection(imported.key, draft);
        }
        await refreshDreamSkinCommunity(true);
        return true;
      },
    );
  };

  const installDreamSkinMarketTheme = async (theme: DreamSkinMarketTheme) => {
    const installed = await runDreamSkinInstallOperation(
      { source: "market", id: theme.id, name: theme.name },
      async () => {
        const result = await run(() => call<DreamSkinMarketResult>("install_dream_skin_market_theme", { id: theme.id }));
        if (!result) return false;
        setDreamSkinMarket(result);
        showResultNotice(t("主题市场"), result);
        if (!isSuccessStatus(result.status)) return false;
        await refreshDreamSkinLibrary(true);
        const draft = await loadDreamSkinThemeDraft(theme.id);
        if (draft) setDreamSkinDraftSelection(`stored:${theme.id}`, draft);
        return true;
      },
    );
    return installed ?? false;
  };

  const runAfterDreamSkinDraftGuard = (action: () => void) => {
    if (!dreamSkinDraftDirty) {
      action();
      return;
    }
    dreamSkinPendingActionRef.current = action;
    setDreamSkinUnsavedDialog(true);
  };

  const loadDreamSkinThemeDraft = async (id: string) => {
    const result = await run(() => call<DreamSkinThemeDraftResult>("load_dream_skin_theme", { id }));
    if (!result || !isSuccessStatus(result.status)) {
      if (result) showResultNotice(t("主题库"), result);
      return null;
    }
    return {
      config: result.config,
      imagePath: result.imagePath,
      builtin: result.builtin,
    } satisfies DreamSkinThemeDraft;
  };

  const selectDreamSkinTheme = (item: DreamSkinThemeSummary) => {
    if (item.key === selectedDreamSkinTheme) return;
    runAfterDreamSkinDraftGuard(() => {
      void (async () => {
        if (item.builtin) {
          setDreamSkinDraftSelection(item.key, {
            config: defaultDreamSkinTheme(),
            imagePath: "",
            builtin: true,
          });
          return;
        }
        if (item.active && dreamSkinLibrary) {
          setDreamSkinDraftSelection(item.key, dreamSkinLibrary.activeDraft);
          return;
        }
        const draft = await loadDreamSkinThemeDraft(item.id);
        if (draft) setDreamSkinDraftSelection(item.key, draft);
      })();
    });
  };

  const saveDreamSkinThemeDraft = async (): Promise<DreamSkinThemeDraft | null> => {
    if (!dreamSkinThemeDraft) return null;
    const selected = dreamSkinLibrary?.themes.find((item) => item.key === selectedDreamSkinTheme);
    const saveAsNew = dreamSkinThemeDraft.builtin || selected?.kind === "activeUnsaved";
    const draft: DreamSkinThemeDraft = saveAsNew
      ? {
          ...dreamSkinThemeDraft,
          config: {
            ...dreamSkinThemeDraft.config,
            id: dreamSkinThemeDraft.builtin
              ? `theme-${Date.now()}`
              : dreamSkinThemeDraft.config.id,
            name: dreamSkinThemeDraft.config.name === "Dream Skin"
              ? t("Dream Skin 副本")
              : dreamSkinThemeDraft.config.name,
          },
          builtin: false,
        }
      : dreamSkinThemeDraft;
    const result = await run(() => call<DreamSkinThemeLibraryResult>("save_dream_skin_theme", { draft }));
    if (!result || !isSuccessStatus(result.status)) {
      if (result) showResultNotice(t("主题库"), result);
      return null;
    }
    const stored = await loadDreamSkinThemeDraft(draft.config.id);
    if (!stored) return null;
    setDreamSkinLibrary({ themes: result.themes, activeDraft: result.activeDraft });
    setDreamSkinDraftSelection(`stored:${draft.config.id}`, stored);
    return stored;
  };

  const createDreamSkinTheme = async () => {
    let selected: unknown;
    try {
      selected = await open({
        directory: false,
        multiple: false,
        title: t("选择皮肤图片"),
        filters: [{
          name: t("图片"),
          extensions: isWindowsPlatform
            ? ["png", "jpg", "jpeg", "webp", "gif", "bmp"]
            : ["png", "jpg", "jpeg", "heic", "tif", "tiff", "webp"],
        }],
      });
    } catch (error) {
      showNotice(t("主题库"), tf("打开选择器失败：{0}", [stringifyError(error)]), "failed");
      return;
    }
    if (typeof selected !== "string" || !selected.trim()) return;
    const result = await run(() => call<DreamSkinThemeDraftResult>("create_dream_skin_theme", { path: selected.trim() }));
    if (!result || !isSuccessStatus(result.status)) {
      if (result) showResultNotice(t("主题库"), result);
      return;
    }
    const draft: DreamSkinThemeDraft = {
      config: result.config,
      imagePath: result.imagePath,
      builtin: result.builtin,
    };
    await refreshDreamSkinLibrary(true);
    setDreamSkinDraftSelection(`stored:${draft.config.id}`, draft);
  };

  const chooseDreamSkinDraftImage = async () => {
    let selected: unknown;
    try {
      selected = await open({
        directory: false,
        multiple: false,
        title: t("选择皮肤图片"),
        filters: [{
          name: t("图片"),
          extensions: isWindowsPlatform
            ? ["png", "jpg", "jpeg", "webp", "gif", "bmp"]
            : ["png", "jpg", "jpeg", "heic", "tif", "tiff", "webp"],
        }],
      });
    } catch (error) {
      showNotice(t("主题库"), tf("打开选择器失败：{0}", [stringifyError(error)]), "failed");
      return;
    }
    if (typeof selected === "string" && selected.trim()) {
      setDreamSkinThemeDraft((current) => current ? { ...current, imagePath: selected.trim() } : current);
    }
  };

  const activateDreamSkinDraft = async (initialDraft: DreamSkinThemeDraft) => {
    const currentTheme = pendingDreamSkinRestart
      ? {
          key: pendingDreamSkinRestart.currentThemeKey,
          name: pendingDreamSkinRestart.currentThemeName,
        }
      : dreamSkinLibrary?.themes.find((item) => item.active) ?? null;
    let draft = initialDraft;
    if (draft.builtin && dreamSkinDraftDirty) {
      const stored = await saveDreamSkinThemeDraft();
      if (!stored) return false;
      draft = stored;
    }
    const saved = await persistDreamSkinSettings({
      ...settingsForm,
      codexAppDreamSkinEnabled: true,
      codexAppDreamSkinPaused: false,
    });
    if (!saved) return false;
    const ports = dreamSkinRequest().request;
    const result = await run(() => call<DreamSkinThemeActivationResult>("activate_dream_skin_theme", {
      request: {
        draft,
        debugPort: ports.debugPort,
        helperPort: ports.helperPort,
      },
    }));
    if (!result || !isSuccessStatus(result.status)) {
      if (result) showResultNotice(t("主题库"), result);
      return false;
    }
    setDreamSkinLibrary(result.library);
    setDreamSkinStatus({ ...result.runtime, status: result.status, message: result.message });
    const active = result.library.themes.find((item) => item.active);
    if (active) setDreamSkinDraftSelection(active.key, result.library.activeDraft);
    await refreshSettings(true);
    if (result.savedForNextLaunch) {
      setPendingDreamSkinRestart({
        currentThemeKey: currentTheme?.key ?? null,
        currentThemeName: currentTheme?.name ?? t("当前皮肤"),
        pendingThemeKey: active?.key ?? selectedDreamSkinTheme,
        pendingThemeName: active?.name ?? draft.config.name,
      });
      showNotice(t("主题库"), t("主题已保存并设为待应用，不会自动重启 Codex。"), "not_checked");
    } else {
      setPendingDreamSkinRestart(null);
    }
    return true;
  };

  const activateDreamSkinTheme = async () => {
    if (!dreamSkinThemeDraft) return;
    await activateDreamSkinDraft(dreamSkinThemeDraft);
  };

  const renameDreamSkinTheme = async (item: DreamSkinThemeSummary) => {
    const name = (await appPrompt(t("重命名主题"), { defaultValue: item.name, placeholder: t("输入新的主题名称") }))?.trim();
    if (!name || name === item.name) return;
    const result = await run(() => call<DreamSkinThemeLibraryResult>("rename_dream_skin_theme", { id: item.id, name }));
    if (!result || !isSuccessStatus(result.status)) {
      if (result) showResultNotice(t("主题库"), result);
      return;
    }
    setDreamSkinLibrary({ themes: result.themes, activeDraft: result.activeDraft });
    if (selectedDreamSkinTheme === item.key) {
      setDreamSkinThemeDraft((current) => current
        ? { ...current, config: { ...current.config, name } }
        : current);
      setSavedDreamSkinThemeDraft((current) => current
        ? { ...current, config: { ...current.config, name } }
        : current);
    }
  };

  const deleteDreamSkinTheme = async (item: DreamSkinThemeSummary) => {
    const confirmed = await confirmSessionDelete(
      t("删除主题"),
      tf("删除主题“{0}”？此操作无法撤销。", [item.name]),
    );
    if (!confirmed) return;
    const result = await run(() => call<DreamSkinThemeLibraryResult>("delete_dream_skin_theme", { id: item.id }));
    if (!result || !isSuccessStatus(result.status)) {
      if (result) showResultNotice(t("主题库"), result);
      return;
    }
    setDreamSkinLibrary({ themes: result.themes, activeDraft: result.activeDraft });
    const active = result.themes.find((candidate) => candidate.active) ?? result.themes[0];
    if (active) {
      const draft = active.builtin
        ? { config: defaultDreamSkinTheme(), imagePath: "", builtin: true }
        : result.activeDraft;
      setDreamSkinDraftSelection(active.key, draft);
    }
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
    const confirmed = await confirmSessionDelete(t("删除会话"), tf("删除会话“{0}”？此操作会删除本地数据库记录和 rollout 文件，并创建备份。", [title]));
    if (!confirmed) return false;
    const result = await run(() => requestDeleteLocalSession(session));
    if (result) {
      showResultNotice(t("会话删除"), result);
      await refreshLocalSessions(true, localSessions?.offset ?? 0);
    }
    return !!result && isSuccessStatus(result.status);
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
    const confirmed = await confirmSessionDelete(
      t("批量删除会话"),
      tf("删除选中的 {0} 个会话？此操作会删除本地数据库记录和 rollout 文件，并为每个会话创建备份。\n\n{1}{2}", [uniqueSessions.length, preview, extraCount]),
    );
    if (!confirmed) return false;

    let succeeded = 0;
    const failed: string[] = [];
    for (const session of uniqueSessions) {
      const result = await run(() => requestDeleteLocalSession(session));
      if (result && isSuccessStatus(result.status)) {
        succeeded += 1;
      } else {
        failed.push(session.title || session.id);
      }
    }

    if (failed.length) {
      showNotice(
        t("批量删除会话"),
        tf("已删除 {0} 个，失败 {1} 个：{2}", [succeeded, failed.length, failed.slice(0, 3).map(truncateSessionDeletePreview).join(t("、"))]),
        succeeded ? "ok" : "failed",
      );
    } else {
      showNotice(t("批量删除会话"), tf("已删除 {0} 个会话。", [succeeded]), "ok");
    }
    await refreshLocalSessions(true, localSessions?.offset ?? 0);
    return true;
  };

  const archiveLocalSessions = async (sessions: LocalSession[], archived = true) => {
    const uniqueSessions = Array.from(new Map(sessions.map((session) => [session.id, session])).values());
    if (!uniqueSessions.length) return false;
    const confirmed = await confirmSessionDelete(
      archived ? t("归档会话") : t("取消归档会话"),
      tf("{0} 个会话？这只更新本地归档状态，不删除 rollout 文件。", [uniqueSessions.length]),
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
   */
  const refreshAutoTakeover = async (silent = false) => {
    if (settings && !settings.settings.codexAppAutoTakeover) {
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
      await refreshAutoTakeover(true);
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

  const navigate = async (target: Route, skipDreamSkinDraftGuard = false) => {
    // 安装维护并进了 Codex 设置页；关于并进了设置页（滚到版本与更新）。
    if (target === "about") setSettingsFocus("app-settings-update");
    const next: Route = target === "maintenance" ? "agentSettings" : target === "about" ? "settings" : target;
    if (!skipDreamSkinDraftGuard && route === "dreamSkin" && next !== "dreamSkin" && dreamSkinDraftDirty) {
      runAfterDreamSkinDraftGuard(() => void navigate(next, true));
      return;
    }
    setRoute(next);
    if (next === "overview") await refreshOverview(true);
    if (next === "relay" || next === "multiLaunch") {
      await refreshSettings(true);
      await refreshWeixinStatus(true);
      await refreshRelay(true);
      await refreshRelayFiles(true);
      await refreshEnvConflicts(true);
      await refreshCcsProviders(true);
    }
    if (next === "multiLaunch") await refreshMultiLaunchInstances(true);
    if (next === "relayEnvironment") await refreshRelayEnvironment(true);
    if (next === "gatewaySettings") await refreshOverview(true);
    if (next === "sessions") {
      await refreshSettings(true);
      await refreshLocalSessions(true);
      await refreshProviderSyncTargets(true);
      await refreshSessionIndexRepairReport();
    }
    if (next === "context") {
      await refreshSettings(true);
      await refreshRelayFiles(true);
      await refreshLiveContextEntries(true);
    }
    if (next === "weixin") {
      await refreshSettings(true);
      await refreshWeixinStatus(true);
      await refreshLocalSessions(true);
    }
    if (next === "dreamSkin") {
      await refreshSettings(true);
      await refreshOverview(true);
      await refreshDreamSkinStatus(true);
      await refreshDreamSkinLibrary(true);
      await refreshDreamSkinMarket(true);
      await refreshDreamSkinCommunity(true);
    }
    if (next === "settings" || next === "agentSettings" || next === "enhance") await refreshSettings(true);
    if (next === "userScripts") {
      await refreshSettings(true);
      await refreshScriptMarket(true);
      await refreshUserScriptInventory();
    }
    if (next === "recommendations") await refreshAds(true);
    if (next === "settings") {
      await refreshOverview(true);
      await refreshLogs(true);
      await refreshDiagnostics(true);
    }
    if (next === "agentSettings") {
      await refreshOverview(true);
      await refreshWatcher(true);
      await refreshAutoTakeover(true);
    }
  };

  const consumePendingManagerNavigation = async (): Promise<boolean> => {
    try {
      const navigation = await invoke<ManagerNavigationIntent | null>("consume_pending_manager_navigation");
      if (!navigation) return false;
      const destination = managerNavigationDestination(navigation);
      await refreshSettings(true);
      if (destination.route === "enhance" || destination.route === "pluginMarket") {
        setActiveTool("codex");
      }
      if (destination.route === "enhance") {
        setEnhancementTab(destination.section ?? "general");
      }
      setPendingEnhancementSection(destination.section);
      setRoute(destination.route);
      return true;
    } catch (error) {
      logDiagnostic("manager.navigation_failed", { error: stringifyError(error) });
    }
    return false;
  };

  const launchMultiInstance = async (profile: RelayProfile) => {
    if (multiLaunchPending) return;
    setMultiLaunchPending(profile.id);
    try {
      const result = await run(() => call<MultiLaunchResult>("launch_codex_multi_instance", {
        request: { profileId: profile.id },
      }));
      if (!result) return;
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
        setMultiLaunchInstances(
          Array.isArray(result.instances)
            ? result.instances.filter((instance) => multiLaunchInstanceRunning(instance) || Boolean(multiLaunchInstanceFailure(instance)))
            : [],
        );
        if (!silent && !isSuccessStatus(result.status)) showNotice(t("当前运行实例"), result.message, result.status);
      }
      return result;
    } finally {
      multiLaunchInstancesRequestRef.current = false;
      if (!silent) setMultiLaunchInstancesLoading(false);
    }
  };

  const restartMultiLaunchInstance = async (instance: MultiLaunchInstance) => {
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
      if (succeeded) setPendingDreamSkinRestart(null);
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

  const checkUpdate = async (silent = false) => {
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
      if (updateNotifications.current.shouldPrompt(result, request.manual)) {
        setUpdatePrompt(result);
      } else if (request.manual) {
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

  const performUpdate = async (selectedUpdate = update) => {
    if (updateInstallPending.current) return;
    const release =
      selectedUpdate?.latestVersion && selectedUpdate.assetName && selectedUpdate.assetUrl
        ? {
            version: selectedUpdate.latestVersion,
            url: "",
            body: selectedUpdate.releaseSummary ?? "",
            asset_name: selectedUpdate.assetName,
            asset_url: selectedUpdate.assetUrl,
          }
        : null;
    if (!release) {
      showNotice(t("更新安装"), t("请先检查更新并选择可下载的 Release asset。"), "failed");
      return;
    }
    updateInstallPending.current = true;
    setUpdateInstallProgress({
      active: true,
      percent: 0,
      indeterminate: true,
      message: t("正在准备下载安装包…"),
    });
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
    const petIssue = agentPetValidationMessage(agentPetSettingsIssue(next.agentPet));
    if (petIssue) {
      showNotice(t("Agent 桌宠"), petIssue, "failed");
      return;
    }
    const dictationIssue = dictationSettingsValidationMessage(dictationSettingsIssue(next.dictation));
    if (dictationIssue) {
      showNotice(t("语音输入"), dictationIssue, "failed");
      return;
    }
    const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
    if (result) {
      if (isSuccessStatus(result.status)) {
        setSettings(result);
        setSettingsForm(normalizeSettings(result.settings));
      }
      showNotice(t("设置保存"), result.message, result.status);
    }
  };

  const saveSettingsValue = async (next: BackendSettings, silent = true) => {
    const normalized = normalizeSettings(next);
    const petIssue = agentPetValidationMessage(agentPetSettingsIssue(normalized.agentPet));
    if (petIssue) {
      showNotice(t("Agent 桌宠"), petIssue, "failed");
      return null;
    }
    const dictationIssue = dictationSettingsValidationMessage(dictationSettingsIssue(normalized.dictation));
    if (dictationIssue) {
      showNotice(t("语音输入"), dictationIssue, "failed");
      return null;
    }
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
    const issue = agentPetValidationMessage(agentPetSettingsIssue(normalized.agentPet))
      || dictationSettingsValidationMessage(dictationSettingsIssue(normalized.dictation));
    if (issue) return { status: "invalid", message: issue };
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

  const resetImageOverlaySettings = async () => {
    const result = await run(() => call<SettingsResult>("reset_image_overlay_settings"));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showNotice(t("图片覆盖层"), result.message, result.status);
    }
  };

  const refreshAds = async (silent = false) => {
    const result = await run(() => call<AdsResult>("load_ads"));
    if (result) {
      setAds(result);
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
          return result.report;
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
      setRelay(result);
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
      setRelay(result);
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice(t("纯 API 模式"), result.message, result.status);
    }
    return !!result && isSuccessStatus(result.status) && result.configured;
  };

  const clearRelayInjection = async (silent = false) => {
    const result = await run(() => call<RelayResult>("clear_relay_injection"));
    if (result) {
      setRelay(result);
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
      await refreshRelay(true);
    }
  };

  const upsertContextEntry = async (next: BackendSettings, kind: ContextKind, id: string, tomlBody: string) => {
    const result = await run(() =>
      call<ContextEntriesResult>("upsert_context_entry", {
        request: { settings: next, kind, id, tomlBody },
      }),
    );
    if (!result) return null;
    let normalized = normalizeSettings(result.settings);
    const saveResult = await run(() => call<SettingsResult>("save_settings", { settings: normalized }));
    if (saveResult) {
      setSettings(saveResult);
      normalized = normalizeSettings(saveResult.settings);
    }
    setSettingsForm(normalized);
    if (!isSuccessStatus(result.status)) showResultNotice(t("MCP&插件"), result);
    return normalized;
  };

  const deleteContextEntry = async (next: BackendSettings, kind: ContextKind, id: string) => {
    const result = await run(() =>
      call<ContextEntriesResult>("delete_context_entry", {
        request: { settings: next, kind, id },
      }),
    );
    if (!result) return null;
    let normalized = normalizeSettings(result.settings);
    const saveResult = await run(() => call<SettingsResult>("save_settings", { settings: normalized }));
    if (saveResult) {
      setSettings(saveResult);
      normalized = normalizeSettings(saveResult.settings);
    }
    setSettingsForm(normalized);
    if (!isSuccessStatus(result.status)) showResultNotice(t("MCP&插件"), result);
    return normalized;
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
    if (result) showNotice("Provider Doctor", result.message, result.status);
    return result ?? null;
  };

  const testStepwiseSettings = async (settings: BackendSettings) => {
    const result = await run(() => call<StepwiseTestResult>("test_stepwise_settings", { settings }));
    if (result) showNotice(t("测试连接"), result.message, result.status);
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

  const switchRelayProfile = async (
    next: BackendSettings,
    previousActiveRelayId = settingsForm.activeRelayId,
    liveConflictResolution?: LiveConflictResolution,
  ): Promise<boolean> => {
    if (relaySwitching) {
      showNotice(t("供应商切换中"), t("上一次切换还没有完成，请稍后再试。"), "failed");
      return false;
    }
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
      setRelay({
        status: result.status,
        message: result.message,
        ...result.relay,
      });
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
    return switchRelayProfile(next, previousActiveRelayId, choice);
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

  const showNotice = (title: string, message: string, status?: Status) => {
    setNotice({ title, message: t(message), status });
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

  useEffect(() => {
    // 工具栏只依赖本地配置，不能排在广告等网络请求之后。
    // 切换语言会重载窗口，串行等待会让三个 Agent 图标长时间消失。
    void refreshTools(true);
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
      await refreshOverview(true);
      // 概览页的赞助商区块要显示真实广告源内容，所以启动就拉一次，
      // 不要等到用户点进「推荐内容」才加载。
      await refreshAds(true);
      if (!handledNavigation) await refreshSettings(true);
      await refreshRelay(true);
      await refreshEnvConflicts(true);
      await refreshProviderSyncTargets(true);
      await refreshPendingProviderImport(true);
      await refreshPendingDreamSkinCommunity();
    })();
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
      if (!disposed) void consumePendingManagerNavigation();
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
      void trayRequestHandlerRef.current(request)
        .catch((error): TrayResult => ({ id: request.id, ok: false, message: stringifyError(error) }))
        .then((result) => emit(TRAY_RESULT_EVENT, result))
        .catch(() => {});
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

  useEffect(() => {
    if (route !== "sessions") return;
    let disposed = false;
    void call<CommandResult<ProviderSyncPayload>>("load_provider_sync_recovery").then((result) => {
      if (disposed || !result.recoveryJournalPath) return;
      setProviderSyncProgress((current) => current.active ? current : ({ active: false, percent: 0, message: result.message, result }));
    }).catch(() => {});
    return () => { disposed = true; };
  }, [route]);

  useEffect(() => {
    if (route !== "sessions") return;
    let disposed = false;
    const refresh = () => void refreshSessionIndexRepairReport(() => !disposed);
    refresh();
    const timer = window.setInterval(refresh, 15_000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [route]);

  useEffect(() => {
    if (route !== "multiLaunch") return;
    let disposed = false;
    const refresh = () => {
      if (disposed) return;
      void refreshMultiLaunchInstances(true);
    };
    refresh();
    const timer = window.setInterval(refresh, 3_000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [route]);

  useEffect(() => {
    if (route !== "enhance" || !pendingEnhancementSection) return;
    let secondFrame = 0;
    const firstFrame = window.requestAnimationFrame(() => {
      secondFrame = window.requestAnimationFrame(() => {
        const target = document.getElementById(ENHANCEMENT_SECTION_IDS[pendingEnhancementSection]);
        if (!target) return;
        target.scrollIntoView({
          behavior: "smooth",
          block: "start",
        });
        setPendingEnhancementSection(null);
      });
    });
    return () => {
      window.cancelAnimationFrame(firstFrame);
      if (secondFrame) window.cancelAnimationFrame(secondFrame);
    };
  }, [pendingEnhancementSection, route]);

  useEffect(() => {
    if (getLanguage() === "en") {
      void invoke("update_tray_labels", {
        showLabel: "Show window",
        quitLabel: "Quit",
        windowTitle: "Codex++",
      });
    }
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void refreshPendingProviderImport(true);
      void refreshPendingDreamSkinCommunity();
    }, 1200);
    return () => window.clearInterval(timer);
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

  useEffect(() => {
    if (route !== "weixin") return;
    const timer = window.setInterval(() => void refreshWeixinStatus(true), 2_000);
    return () => window.clearInterval(timer);
  }, [route]);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    window.localStorage.setItem("codex-plus-theme", theme);
  }, [theme]);

  useEffect(() => {
    const refresh = () => { void refreshOverview(true); };
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

  const persistDreamSkinSettings = async (next: BackendSettings) => {
    const normalized = normalizeSettings(next);
    const result = await run(() => call<SettingsResult>("save_settings", { settings: normalized }));
    if (!result) return null;
    setSettings(result);
    setSettingsForm(normalizeSettings(result.settings));
    if (!isSuccessStatus(result.status)) {
      showNotice(t("皮肤管理"), result.message, result.status);
      return null;
    }
    return result;
  };

  const restoreDreamSkin = async () => {
    const currentTheme = pendingDreamSkinRestart
      ? {
          key: pendingDreamSkinRestart.currentThemeKey,
          name: pendingDreamSkinRestart.currentThemeName,
        }
      : dreamSkinLibrary?.themes.find((item) => item.active) ?? null;
    const result = await run(() => call<DreamSkinRuntimeResult>("restore_dream_skin", dreamSkinRequest()));
    if (!result) return;
    setDreamSkinStatus(result);
    await refreshSettings(true);
    showResultNotice(t("皮肤管理"), result);
    if (isSuccessStatus(result.status)) {
      setPendingDreamSkinRestart({
        currentThemeKey: currentTheme?.key ?? null,
        currentThemeName: currentTheme?.name ?? t("当前皮肤"),
        pendingThemeKey: "codex-original-appearance",
        pendingThemeName: t("Codex 原始外观"),
      });
    }
  };

  const verifyDreamSkin = async (withScreenshot: boolean) => {
    let screenshotPath: string | undefined;
    if (withScreenshot) {
      try {
        const selected = await saveDialog({
          title: t("保存 Dream Skin 截图"),
          defaultPath: "codex-dream-skin-verification.png",
          filters: [{ name: "PNG", extensions: ["png"] }],
        });
        if (!selected) return;
        screenshotPath = selected;
      } catch (error) {
        showNotice(t("保存截图"), tf("打开选择器失败：{0}", [stringifyError(error)]), "failed");
        return;
      }
    }
    const result = await run(() =>
      call<DreamSkinVerificationResult>("verify_dream_skin", dreamSkinRequest(screenshotPath)),
    );
    if (!result) return;
    setDreamSkinVerification(result);
    showResultNotice(withScreenshot ? t("保存截图") : t("实机验证"), result);
    await refreshDreamSkinStatus(true);
  };

  const actions = useMemo(
    () => ({
      refreshCurrent: () => navigate(route),
      launch,
      launchMultiInstance,
      multiLaunchPending,
      multiLaunchInstances,
      multiLaunchInstancesLoading,
      multiLaunchActionPending,
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
      refreshSettings,
      resetSettings,
      exportSettings,
      importSettings,
      chooseSettingsPath,
      resetSettingsPath,
      resetImageOverlaySettings,
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
      chooseImageOverlayPath: async () => {
        let selected: unknown;
        try {
          selected = await open({
            directory: false,
            multiple: false,
            title: t("选择覆盖图片"),
            filters: [{ name: t("图片"), extensions: ["png", "jpg", "jpeg", "webp", "gif", "bmp"] }],
          });
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          showNotice(t("图片覆盖层"), tf("打开选择器失败：{0}", [message]), "failed");
          return;
        }
        if (typeof selected === "string" && selected.trim()) {
          setSettingsForm((current) => ({
            ...current,
            codexAppImageOverlayEnabled: true,
            codexAppImageOverlayPath: selected.trim(),
          }));
        }
      },
      chooseDreamSkinImagePath: chooseDreamSkinDraftImage,
      resetDreamSkinImage: async () => runAfterDreamSkinDraftGuard(() => {
        setDreamSkinThemeDraft((current) => current ? { ...current, imagePath: "" } : current);
      }),
      resetDreamSkinTheme: async () => runAfterDreamSkinDraftGuard(() => {
        setDreamSkinThemeDraft((current) => {
          if (!current) return current;
          if (isWindowsPlatform) {
            const config = { ...current.config };
            delete config.colors;
            delete config.palette;
            return { ...current, config };
          }
          const defaults = defaultDreamSkinTheme();
          return {
            ...current,
            config: current.builtin
              ? defaults
              : { ...defaults, id: current.config.id, name: current.config.name },
            imagePath: "",
          };
        });
      }),
      refreshDreamSkinLibrary,
      refreshDreamSkinMarket,
      refreshDreamSkinCommunity,
      installDreamSkinMarketTheme,
      installDreamSkinCommunityTheme,
      importDreamSkinThemePackage,
      createDreamSkinTheme: async () => runAfterDreamSkinDraftGuard(() => void createDreamSkinTheme()),
      saveDreamSkinTheme: saveDreamSkinThemeDraft,
      selectDreamSkinTheme,
      renameDreamSkinTheme,
      deleteDreamSkinTheme: async (item: DreamSkinThemeSummary) => {
        if (item.key === selectedDreamSkinTheme && dreamSkinDraftDirty) {
          runAfterDreamSkinDraftGuard(() => void deleteDreamSkinTheme(item));
          return;
        }
        await deleteDreamSkinTheme(item);
      },
      activateDreamSkinTheme,
      refreshDreamSkinStatus,
      restoreDreamSkin,
      verifyDreamSkin: () => verifyDreamSkin(false),
      saveDreamSkinScreenshot: () => verifyDreamSkin(true),
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
      refreshRelay,
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
      reloadUserScripts,
      installMarketScript,
      setUserScriptsEnabled,
      setUserScriptsSkipNextStart,
      setUserScriptEnabled,
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
      upsertContextEntry,
      deleteContextEntry,
      previewMcpServersJson,
      importMcpServersJson,
      extractRelayCommonConfig,
      testRelayProfile,
      diagnoseRelayProfile,
      testStepwiseSettings,
      fetchRelayProfileModels,
      switchRelayProfile,
      relaySwitching,
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
        await refreshOverview(true);
        await refreshRelay(true);
        await refreshWatcher(true);
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
      toggleTheme: () => setTheme((current) => (current === "dark" ? "light" : "dark")),
    }),
    [route, launchForm, settingsForm, settings, overview, update, updateInstallProgress.active, logs, diagnostics, theme, relayFiles, localSessions, selectedProviderSyncTarget, envConflicts, relayEnvironment, ccsProviders, multiLaunchPending, multiLaunchInstances, multiLaunchInstancesLoading, multiLaunchActionPending, dreamSkinLibrary, dreamSkinMarket, dreamSkinCommunity, selectedDreamSkinTheme, savedDreamSkinThemeDraft, dreamSkinThemeDraft, dreamSkinDraftDirty, pendingDreamSkinRestart, getLanguage()],
  );
  const isGlobalPage = globalNavigationRoutes.includes(route);
  const isManagementScreen = route === "relay" || route === "multiLaunch" || route === "sessions" || route === "context" || route === "skills";
  const codexAppRunning = overview?.runtime_health?.codex_app?.status === "running";
  const activeToolName = toolEntries.find((entry) => entry.id === activeTool)?.name ?? t("当前 Agent");

  const isGatewayPage = gatewayNavigationRoutes.includes(route);
  const crumbGroup = isGatewayPage ? t("网关") : managementNavigationRoutes.includes(route) ? t("全局管理") : isGlobalPage ? "Codex++" : activeToolName;
  // ⌘K：页面、各 Agent 的页面和常用操作。非当前 Agent 的页面会先切到那个 Agent。
  const paletteCommands: PaletteCommand[] = [];
  const routeCommand = (routeId: Route, group: string, run: () => void, keywords = ""): PaletteCommand | null => {
    const item = routes.find((candidate) => candidate.id === routeId);
    if (!item) return null;
    const Icon = item.icon;
    return { id: `${group}:${routeId}`, group, label: item.label, keywords: `${routeId} ${keywords}`, icon: <Icon className="h-4 w-4" />, run };
  };
  const pushCommand = (command: PaletteCommand | null) => { if (command) paletteCommands.push(command); };
  gatewayNavigationRoutes.forEach((routeId) => pushCommand(routeCommand(routeId, t("网关"), () => void navigate(routeId), "gateway")));
  managementNavigationRoutes.forEach((routeId) => pushCommand(routeCommand(routeId, t("全局管理"), () => void navigate(routeId))));
  for (const tool of toolEntries.filter((entry) => entry.switchable)) {
    agentRoutes(tool.id).forEach((routeId) => pushCommand(routeCommand(routeId, tool.name, () => void switchTool(tool.id, routeId), tool.id)));
  }
  [...toolNavigationRoutes, "settings" as Route].forEach((routeId) => pushCommand(routeCommand(routeId, t("工具"), () => void navigate(routeId))));
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
    { id: "action:language", group: t("操作"), label: getLanguage() === "en" ? t("切换到中文") : t("切换到英文"), keywords: "language", icon: <Languages className="h-4 w-4" />, run: () => toggleLanguage() },
  );

  return (
    <div className={`shell ${theme}`}>
      <AppSidebar
        tools={toolEntries}
        activeTool={activeTool}
        route={route}
        theme={theme}
        version={overview?.current_version ?? update?.currentVersion ?? ""}
        onOpenAgent={(toolId, destination) => void switchTool(toolId, destination ?? agentEntryRoute(toolId))}
        onNavigate={(next) => void navigate(next)}
        onToggleTheme={actions.toggleTheme}
        onOpenPalette={() => setPaletteOpen(true)}
        pendingUpdateVersion={update?.updateAvailable && update.latestVersion ? update.latestVersion : null}
        onOpenUpdate={() => { if (update) setUpdatePrompt(update); }}
        codexRun={{ running: codexAppRunning, busy: launchPending, onStart: () => void actions.launch(), onStop: () => void actions.stopCodex() }}
      />
      <CommandPalette commands={paletteCommands} onClose={() => setPaletteOpen(false)} open={paletteOpen} />
      {tourOpen ? <OnboardingTour onClose={closeTour} /> : null}
      <main className="workspace">
        <header className="topbar" key={`topbar-${route}`}>
          <div className="topbar-heading">
            <div className="topbar-title">
              <span className="topbar-crumb">{crumbGroup}</span>
              <ChevronRight aria-hidden="true" className="topbar-crumb-sep" />
              <h1>{route === "agentSettings" ? tf("{0} 设置", [activeToolName]) : routeTitle(route)}</h1>
            </div>
            <p>
              {route === "agentSettings"
                ? tf("只作用于 {0}；界面主题、配置备份等整体设置在侧栏底部的齿轮里。", [activeToolName])
                : routeSubtitle(route)}
            </p>
          </div>
          <div className="topbar-actions">
            {/* data-topbar-actions：页面级操作的挂载点（TopbarActions / PageHeader），Claude 增强页把「启动增强版」portal 进来。
                Codex 的启动 / 停止在侧栏 Codex 行的播放键上，顶栏不再重复。 */}
            <div className="topbar-page-actions" data-topbar-actions />
          </div>
        </header>
        <section
          className={`screen ${isManagementScreen ? "management-screen" : ""} ${route === "requestLog" ? "request-log-screen" : ""}`.trim()}
          key={route}
        >
          <AgentPageTabs activeTool={activeTool} onNavigate={(next) => void navigate(next)} route={route} />
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
          {route === "relay" ? (
            <LegacyRelayGatewayHint
              form={settingsForm}
              onDisconnect={async () => {
                try {
                  const result = await invoke<{ status: string; message: string }>("disable_gateway_binding", { agent: "codex" });
                  await showNotice(result.status === "ok" ? t("已断开网关") : t("断开失败"), result.message, result.status === "ok" ? "ok" : "failed");
                } catch (error) {
                  await showNotice(t("断开失败"), String(error), "failed");
                }
                await refreshSettings(true);
              }}
              onManageAgents={() => void navigate("gateway")}
              onOpenGateway={() => void navigate("gatewayProviders")}
            />
          ) : null}
          {route === "relay" ? (
            <RelayScreen
              settings={settings}
              relayFiles={relayFiles}
              envConflicts={envConflicts}
              ccsProviders={ccsProviders}
              form={settingsForm}
              actions={actions}
            />
          ) : null}
          {route === "multiLaunch" ? (
            <CodexMultiLaunchScreen form={settingsForm} actions={actions} />
          ) : null}
          {route === "relayEnvironment" ? (
            <ProxySettingsScreen
              dirty={settingsDirty}
              form={settingsForm}
              relayEnvironment={relayEnvironment}
              onFormChange={setSettingsForm}
              onOpenGatewaySettings={() => void navigate("gatewaySettings")}
              actions={actions}
            />
          ) : null}
          {route === "requestLog" ? (
            <RequestLogScreen initialAgent={requestLogAgentFilter} onConsumeInitialAgent={() => setRequestLogAgentFilter("all")} />
          ) : null}
          {route === "claude" ? <ClaudeLocalizationScreen /> : null}
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
              onRefresh={async (offset) => { await actions.refreshLocalSessions(false, offset ?? 0); }}
              onDelete={actions.deleteLocalSession}
              onDeleteMany={actions.deleteLocalSessions}
              onArchiveMany={actions.archiveLocalSessions}
              onListProjectSessions={actions.listProjectSessions}
              repairPanel={<SessionRepairPanel dirty={settingsDirty} form={settingsForm} providerSyncProgress={providerSyncProgress}
                sessionIndexRepairActive={sessionIndexRepairActive} sessionIndexRepairReport={sessionIndexRepairReport}
                sessionIndexRepairReportError={sessionIndexRepairReportError} providerSyncTargets={providerSyncTargets}
                selectedProviderSyncTarget={selectedProviderSyncTarget} onFormChange={setSettingsForm} actions={actions} />}
            />
          ) : null}
          {route === "context" ? (
            <McpPage
              entries={[...contextEntriesByKind(contextEntriesWithLiveEntries(settingsForm, liveContextEntries), "mcp"), ...contextEntriesByKind(contextEntriesWithLiveEntries(settingsForm, liveContextEntries), "plugin")]}
              onSave={async (kind, id, body) => {
                const next = await actions.upsertContextEntry(settingsForm, kind, id, body);
                if (!next) return false;
                const synced = await actions.syncLiveContextEntries(next, true);
                return Boolean(synced && isSuccessStatus(synced.status));
              }}
              onDelete={async (entry) => {
                const next = await actions.deleteContextEntry(settingsForm, entry.kind, entry.id);
                if (!next) return false;
                const synced = await actions.syncLiveContextEntries(next, true);
                return Boolean(synced && isSuccessStatus(synced.status));
              }}
              onRefresh={async () => { await actions.refreshLiveContextEntries(); await actions.refreshRelayFiles(); }}
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
          {route === "enhance" ? (
            <EnhanceScreen
              dirty={settingsDirty}
              activeTab={enhancementTab}
              onTabChange={setEnhancementTab}
              form={settingsForm}
              onFormChange={setSettingsForm}
              actions={actions}
            />
          ) : null}
          {route === "dreamSkin" ? (
            <DreamSkinScreen
              form={settingsForm}
              launchPending={launchPending}
              library={dreamSkinLibrary}
              libraryLoading={dreamSkinLibraryLoading}
              market={dreamSkinMarket}
              marketLoading={dreamSkinMarketLoading}
              community={dreamSkinCommunity}
              communityLoading={dreamSkinCommunityLoading}
              installOperation={dreamSkinInstallOperation}
              draft={dreamSkinThemeDraft}
              dirty={dreamSkinDraftDirty}
              pendingRestart={pendingDreamSkinRestart}
              selectedTheme={selectedDreamSkinTheme}
              status={dreamSkinStatus}
              verification={dreamSkinVerification}
              onFormChange={setSettingsForm}
              onDraftChange={setDreamSkinThemeDraft}
              actions={actions}
            />
          ) : null}
          {route === "userScripts" ? <UserScriptsScreen settings={settings} market={scriptMarket} actions={actions} /> : null}
          {route === "pluginMarket" ? <PluginMarketScreen /> : null}
          {route === "recommendations" ? <RecommendationsScreen ads={ads} actions={actions} /> : null}
          {route === "agentCache" ? <AgentCachePanel /> : null}
          {route === "agentCollaboration" ? <AgentCollaborationPage /> : null}
          {route === "computerUse" ? <ComputerUsePage /> : null}
          {route === "agentPet" ? (
            <AgentPetPage
              settings={settingsForm.agentPet}
              onChange={(agentPet) => setSettingsForm((current) => ({ ...current, agentPet }))}
              autoSave={<SettingsAutoSave dirty={settingsDirty} form={settingsForm} actions={actions} />}
            />
          ) : null}
          {/* 置顶推荐位只放在网关总览一处，内容与「推荐内容」页同源；各 Agent 的概览不再重复。 */}
          {route === "hub" ? <SponsorBoard ads={ads} actions={actions} /> : null}
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
              notify={actions.showMessage}
              onTabChange={(tab) => void navigate(gatewayRouteByTab[tab])}
              tab={gatewayTabByRoute[route]}
            />
          ) : null}
          {route === "agentSettings" || route === "maintenance" ? (
            // 设置与安装维护合在一页：左侧分类目录，右侧限宽列表。
            <div className="settings-page codex-settings-page sectioned-page">
              <PageSectionNav label={t("设置分类")} sections={codexSettingsSections()} />
              <div className="sectioned-body">
                <AgentSettingsScreen
                  dirty={settingsDirty}
                  form={settingsForm}
                  onFormChange={setSettingsForm}
                  actions={actions}
                />
                <MaintenanceScreen
                  overview={overview}
                  launchPending={launchPending}
                  watcher={watcher}
                  autoTakeover={autoTakeover}
                  settings={settings}
                  launchForm={launchForm}
                  onLaunchFormChange={setLaunchForm}
                  actions={actions}
                />
              </div>
            </div>
          ) : null}
          {route === "settings" || route === "about" ? (
            // 设置与关于合在一页：左侧分类目录，右侧限宽。
            <div className="settings-page app-settings-page sectioned-page">
              <PageSectionNav label={t("设置分类")} sections={appSettingsSections()} />
              <div className="sectioned-body">
                <AppSettingsFocus focus={settingsFocus} onDone={() => setSettingsFocus(null)} />
                <GlobalSettingsScreen
                  onOpenTour={() => setTourOpen(true)}
                  dirty={settingsDirty}
                  form={settingsForm}
                  settings={settings}
                  theme={theme}
                  actions={actions}
                  onOpenAgentSettings={() => void (async () => {
                    if (activeTool !== "codex") await switchTool("codex");
                    await navigate("agentSettings");
                  })()}
                />
                <AboutScreen
                  overview={overview}
                  update={update}
                  updateInstallProgress={updateInstallProgress}
                  logs={logs}
                  diagnostics={diagnostics}
                  actions={actions}
                />
              </div>
            </div>
          ) : null}
        </section>
      </main>
      {updatePrompt ? (
        <UpdateDialog
          update={updatePrompt}
          progress={updateInstallProgress}
          onClose={closeUpdatePrompt}
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
      {confirmDialog ? (
        <ConfirmDialog
          confirm={confirmDialog}
          onCancel={() => {
            confirmDialog.resolve(false);
            setConfirmDialog(null);
          }}
          onConfirm={() => {
            confirmDialog.resolve(true);
            setConfirmDialog(null);
          }}
        />
      ) : null}
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
      {dreamSkinUnsavedDialog ? (
        <DreamSkinUnsavedDialog
          onCancel={() => {
            dreamSkinPendingActionRef.current = null;
            setDreamSkinUnsavedDialog(false);
          }}
          onDiscard={() => {
            const pending = dreamSkinPendingActionRef.current;
            dreamSkinPendingActionRef.current = null;
            setDreamSkinThemeDraft(savedDreamSkinThemeDraft);
            setDreamSkinUnsavedDialog(false);
            pending?.();
          }}
          onSave={() => void (async () => {
            const saved = await saveDreamSkinThemeDraft();
            if (!saved) return;
            const pending = dreamSkinPendingActionRef.current;
            dreamSkinPendingActionRef.current = null;
            setDreamSkinUnsavedDialog(false);
            pending?.();
          })()}
        />
      ) : null}
      {pendingProviderImport ? (
        <PendingProviderImportDialog
          request={pendingProviderImport}
          onConfirm={() => void confirmPendingProviderImport()}
          onDismiss={() => void dismissPendingProviderImport()}
        />
      ) : null}
      {pendingDreamSkinCommunity ? (
        <DreamSkinCommunityLinkDialog
          installBlocked={Boolean(dreamSkinInstallOperation)}
          installPending={dreamSkinInstallOperation?.source === "pending-community"}
          versionId={pendingDreamSkinCommunity}
          onConfirm={() => void confirmPendingDreamSkinCommunity()}
          onDismiss={() => void dismissPendingDreamSkinCommunity()}
        />
      ) : null}
    </div>
  );
}

// 模型行「思考等级」列：多选档位 + 默认档。选择直接写回该模型的
// supported_reasoning_levels，不下发本地草稿，面板与「模型配置」JSON 共用一份数据。
function ModelReasoningLevelsCell({
  levels,
  inherited,
  defaultLevel,
  parameterMode,
  onParameterModeChange,
  onChange,
}: {
  levels: string[];
  parameterMode: ReasoningParameterMode;
  onParameterModeChange: (mode: ReasoningParameterMode) => void;
  /// true = 当前值来自内置声明而非该模型的自定义配置。
  inherited: boolean;
  defaultLevel: string | null;
  onChange: (levels: string[], defaultLevel: string | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const label = parameterMode === "omit" ? t("不发送思考参数") : formatReasoningLevelsLabel(levels);
  const toggleLevel = (level: string) => {
    const picked = levels.includes(level)
      ? levels.filter((item) => item !== level)
      : [...levels, level];
    const next = normalizeReasoningLevels(picked);
    // 默认档必须落在选中集合里，取消勾选时一起清掉。
    onChange(next, defaultLevel && next.includes(defaultLevel) ? defaultLevel : null);
  };
  const triggerTip = levels.length
    ? `${levels.join(", ")}${defaultLevel ? ` · ${t("默认等级")}: ${defaultLevel}` : ""}`
    : t("未设置时沿用内置默认档位");
  const { rootRef, menuRef, menuStyle } = usePopoverLayer({
    open,
    onOpenChange: setOpen,
    align: "end",
    menuWidth: 240,
    minWidth: 240,
    maxHeight: 320,
  });
  return (
    <div
      className={`app-select relay-reasoning ${open ? "open" : ""} ${inherited ? "inherited" : ""}`.trim()}
      ref={rootRef}
    >
      <button
        aria-expanded={open}
        aria-haspopup="listbox"
        className="app-select-trigger"
        onClick={() => setOpen((current) => !current)}
        title={triggerTip}
        type="button"
      >
        <span className={label ? "" : "relay-reasoning-empty"}>{label || t("未设置")}</span>
        <ChevronDown className="h-4 w-4" />
      </button>
      {open ? createPortal(
        <div
          aria-multiselectable="true"
          className="app-select-menu app-select-menu-portal relay-reasoning-menu"
          ref={menuRef}
          role="listbox"
          style={menuStyle}
        >
          <div className="relay-reasoning-default">
            <span>{t("请求参数")}</span>
            <div className="relay-reasoning-chips">
              {([ ["auto", t("自动兼容")], ["passthrough", t("原样传等级（Chat）")], ["omit", t("不发送")] ] as const).map(([mode, text]) =>
                <button key={mode} type="button" className={parameterMode === mode ? "selected" : ""} onClick={() => onParameterModeChange(mode)}>{text}</button>)}
            </div>
          </div>
          {REASONING_LEVELS.map((level) => {
            const selected = levels.includes(level);
            return (
              <button
                aria-selected={selected}
                className={`app-select-option ${selected ? "selected" : ""}`.trim()}
                key={level}
                disabled={parameterMode === "omit"}
                onClick={() => toggleLevel(level)}
                onMouseDown={(event) => event.preventDefault()}
                role="option"
                type="button"
              >
                <span className="app-select-option-label">{level}</span>
                {selected ? <Check aria-hidden className="app-select-check" /> : null}
              </button>
            );
          })}
          <div className="relay-reasoning-default">
            <span>{t("默认等级")}</span>
            <div className="relay-reasoning-chips">
              <button
                className={defaultLevel ? "" : "selected"}
                disabled={parameterMode === "omit"}
                onClick={() => onChange(levels, null)}
                type="button"
              >
                {t("自动")}
              </button>
              {levels.map((level) => (
                <button
                  className={defaultLevel === level ? "selected" : ""}
                  key={level}
                  disabled={parameterMode === "omit"}
                  onClick={() => onChange(levels, level)}
                  type="button"
                >
                  {level}
                </button>
              ))}
            </div>
          </div>
          <p className="relay-reasoning-hint">
            {parameterMode === "omit" ? t("请求不发送思考参数；档位草稿保留，切回自动兼容后可恢复。")
              : parameterMode === "passthrough" ? t("Chat 上游按原值接收 reasoning_effort，不按模型名改写档位。")
              : inherited
              ? t("当前为内置默认，修改后保存为该模型的自定义思考等级。")
              : t("未设置时沿用内置默认；清空全部档位即恢复内置。")}
          </p>
        </div>,
        document.body,
      ) : null}
    </div>
  );
}

type AutoSaveOutcome = { status: "saved" } | { status: "invalid" | "failed"; message: string };

type Actions = {
  refreshCurrent: () => Promise<void>;
  launch: () => Promise<boolean>;
  launchMultiInstance: (profile: RelayProfile) => Promise<void>;
  multiLaunchPending: string | null;
  multiLaunchInstances: MultiLaunchInstance[];
  multiLaunchInstancesLoading: boolean;
  multiLaunchActionPending: string | null;
  refreshMultiLaunchInstances: (silent?: boolean) => Promise<MultiLaunchInstancesResult | null>;
  restartMultiLaunchInstance: (instance: MultiLaunchInstance) => Promise<void>;
  stopMultiLaunchInstance: (instance: MultiLaunchInstance) => Promise<void>;
  restart: (syncActiveRelay?: boolean) => Promise<boolean>;
  stopCodex: () => Promise<boolean>;
  repairShortcuts: () => Promise<void>;
  checkUpdate: () => Promise<void>;
  performUpdate: () => Promise<void>;
  saveSettings: () => Promise<void>;
  saveSettingsValue: (settings: BackendSettings, silent?: boolean) => Promise<BackendSettings | null>;
  autoSaveSettings: (settings: BackendSettings) => Promise<AutoSaveOutcome>;
  refreshSettings: (silent?: boolean) => Promise<BackendSettings | null>;
  resetSettings: () => Promise<void>;
  exportSettings: () => Promise<void>;
  importSettings: () => Promise<void>;
  chooseSettingsPath: () => Promise<void>;
  resetSettingsPath: () => Promise<void>;
  resetImageOverlaySettings: () => Promise<void>;
  chooseCodexAppPath: (mode: "folder" | "file") => Promise<void>;
  clearCodexAppPath: () => Promise<void>;
  chooseImageOverlayPath: () => Promise<void>;
  chooseDreamSkinImagePath: () => Promise<void>;
  resetDreamSkinImage: () => Promise<void>;
  resetDreamSkinTheme: () => Promise<void>;
  refreshDreamSkinLibrary: (silent?: boolean) => Promise<DreamSkinThemeLibrary | null>;
  refreshDreamSkinMarket: (silent?: boolean) => Promise<DreamSkinMarketResult | null>;
  installDreamSkinMarketTheme: (theme: DreamSkinMarketTheme) => Promise<boolean>;
  refreshDreamSkinCommunity: (silent?: boolean) => Promise<DreamSkinCommunityResult | null>;
  installDreamSkinCommunityTheme: (theme: DreamSkinCommunityTheme) => Promise<boolean>;
  importDreamSkinThemePackage: () => Promise<void>;
  createDreamSkinTheme: () => Promise<void>;
  saveDreamSkinTheme: () => Promise<DreamSkinThemeDraft | null>;
  selectDreamSkinTheme: (item: DreamSkinThemeSummary) => void;
  renameDreamSkinTheme: (item: DreamSkinThemeSummary) => Promise<void>;
  deleteDreamSkinTheme: (item: DreamSkinThemeSummary) => Promise<void>;
  activateDreamSkinTheme: () => Promise<void>;
  refreshDreamSkinStatus: (silent?: boolean) => Promise<DreamSkinRuntimeResult | null>;
  restoreDreamSkin: () => Promise<void>;
  verifyDreamSkin: () => Promise<void>;
  saveDreamSkinScreenshot: () => Promise<void>;
  saveManualCodexAppPath: () => Promise<void>;
  syncProvidersNow: () => Promise<void>;
  recoverProviderSync: () => Promise<void>;
  repairSessionIndex: () => Promise<void>;
  refreshProviderSyncTargets: (silent?: boolean) => Promise<ProviderSyncTargetsResult | null>;
  setProviderSyncTarget: (provider: string) => void;
  refreshRelay: () => Promise<void>;
  refreshRelayFiles: () => Promise<RelayFilesResult | null>;
  refreshEnvConflicts: (silent?: boolean) => Promise<EnvConflictsResult | null>;
  refreshRelayEnvironment: (silent?: boolean) => Promise<RelayEnvironmentResult | null>;
  removeEnvConflicts: (names: string[]) => Promise<void>;
  refreshCcsProviders: (silent?: boolean) => Promise<CcsProvidersResult | null>;
  importCcsProviders: (bindings?: Record<string, string>) => Promise<void>;
  refreshLiveContextEntries: () => Promise<LiveContextEntriesResult | null>;
  syncLiveContextEntries: (settings: BackendSettings, silent?: boolean) => Promise<LiveContextEntriesResult | null>;
  refreshAds: () => Promise<void>;
  refreshScriptMarket: () => Promise<void>;
  refreshUserScriptInventory: () => Promise<SettingsResult | null>;
  reloadUserScripts: () => Promise<void>;
  installMarketScript: (id: string) => Promise<void>;
  setUserScriptsEnabled: (enabled: boolean) => Promise<void>;
  setUserScriptsSkipNextStart: (skip: boolean) => Promise<void>;
  setUserScriptEnabled: (key: string, enabled: boolean) => Promise<void>;
  deleteUserScript: (key: string) => Promise<void>;
  refreshLocalSessions: (silent?: boolean, offset?: number) => Promise<LocalSessionsResult | null>;
  listProjectSessions: (cwd: string) => Promise<LocalSession[]>;
  // 返回是否真正执行（用户取消时为 false），调用方据此决定是否清空选择。
  deleteLocalSession: (session: LocalSession) => Promise<boolean>;
  deleteLocalSessions: (sessions: LocalSession[]) => Promise<boolean>;
  archiveLocalSessions: (sessions: LocalSession[], archived?: boolean) => Promise<boolean>;
  openExternalUrl: (url: string) => Promise<void>;
  applyRelayInjection: () => Promise<boolean>;
  applyPureApiInjection: () => Promise<boolean>;
  clearRelayInjection: () => Promise<boolean>;
  saveRelayFile: (kind: "config" | "auth", contents: string, silent?: boolean) => Promise<void>;
  upsertContextEntry: (
    settings: BackendSettings,
    kind: ContextKind,
    id: string,
    tomlBody: string,
  ) => Promise<BackendSettings | null>;
  deleteContextEntry: (settings: BackendSettings, kind: ContextKind, id: string) => Promise<BackendSettings | null>;
  previewMcpServersJson: (json: string) => Promise<McpImportPreviewResult | null>;
  importMcpServersJson: (settings: BackendSettings, json: string) => Promise<BackendSettings | null>;
  extractRelayCommonConfig: (configContents: string) => Promise<ExtractRelayCommonConfigResult | null>;
  testRelayProfile: (profile: RelayProfile) => Promise<void>;
  diagnoseRelayProfile: (profile: RelayProfile) => Promise<ProviderDoctorResult | null>;
  testStepwiseSettings: (settings: BackendSettings) => Promise<void>;
  fetchRelayProfileModels: (profile: RelayProfile) => Promise<string[] | null>;
  switchRelayProfile: (settings: BackendSettings, previousActiveRelayId?: string) => Promise<boolean>;
  relaySwitching: boolean;
  switchOfficialMode: () => Promise<void>;
  switchPureApiMode: () => Promise<void>;
  refreshLogs: () => Promise<void>;
  clearLogs: () => Promise<void>;
  refreshDiagnostics: () => Promise<void>;
  showMessage: (title: string, message: string, status?: Status) => Promise<void>;
  copyLogs: () => Promise<void>;
  copyDiagnostics: () => Promise<void>;
  goLogs: () => Promise<void>;
  setWatcherEnabled: (enabled: boolean) => Promise<void>;
  setWatcherStartMinimized: (minimized: boolean) => Promise<void>;
  setWatcherStartCodex: (startCodex: boolean) => Promise<void>;
  setWatcherStartCodexOnOpen: (enabled: boolean) => Promise<void>;
  setAutoTakeover: (enabled: boolean) => Promise<void>;
  refreshAutoTakeover: () => Promise<void>;
  takeOverNow: () => Promise<void>;
  pauseAutoTakeover: () => Promise<void>;
  toggleTheme: () => void;
  checkHealth: () => Promise<void>;
};

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
  const workDirOptions = useMemo(
    () => Array.from(new Set(sessions.map((session) => session.cwd.trim()).filter(Boolean))).sort(),
    [sessions],
  );
  const runtimeState = status?.state ?? "stopped";
  const running = ["starting", "running", "retrying"].includes(runtimeState);
  const stopping = runtimeState === "stopping";
  const statusLabel = {
    starting: t("正在启动"),
    running: t("运行中"),
    retrying: t("正在重试"),
    stopping: t("正在停止"),
    error: t("异常"),
    stopped: t("已停止"),
  }[runtimeState] ?? runtimeState;

  return (
    <div className="weixin-connect-page sectioned-page">
      <PageSectionNav label={t("微信连接分类")} sections={[
        { id: "weixin-section-status", label: t("连接状态") },
        { id: "weixin-section-account", label: t("账号") },
        { id: "weixin-section-session", label: t("会话管理") },
        { id: "weixin-section-cli", label: "Codex CLI" },
      ]} />
      <div className="sectioned-body">
      <Panel className={`weixin-status-panel is-${runtimeState}`} id="weixin-section-status">
        <CardContent className="weixin-status-content">
          <div className="weixin-connect-head">
            <div className="weixin-status-primary">
              <div className="weixin-status-icon" aria-hidden="true">
                <MessageCircle className="h-5 w-5" />
              </div>
              <div>
                <div className="section-heading-row">
                  <h2>{t("个人微信连接")}</h2>
                  <UiBadge variant={runtimeState === "running" ? "default" : runtimeState === "error" ? "outline" : "secondary"}>
                    {statusLabel}
                  </UiBadge>
                </div>
                <p className="muted">{status?.message ?? t("微信连接未启动。")}</p>
              </div>
            </div>
            <div className="toolbar weixin-connect-actions">
              {autoSave}
              <Button onClick={onQrLogin} variant="outline">
                <ScanLine className="h-4 w-4" />
                {form.weixinConnectToken ? t("重新登录") : t("扫码登录")}
              </Button>
              {running || stopping ? (
                <Button disabled={stopping} onClick={onStop} variant="outline">
                  <PowerOff className="h-4 w-4" />
                  {stopping ? t("正在停止") : t("停止")}
                </Button>
              ) : (
                <Button disabled={!form.weixinConnectToken} onClick={onStart}>
                  <Play className="h-4 w-4" />
                  {t("启动")}
                </Button>
              )}
            </div>
          </div>
          <div className="weixin-runtime-meta">
            <div>
              <span>{t("账号")}</span>
              <code title={status?.accountId || form.weixinConnectAccountId || t("未登录")}>
                {status?.accountId || form.weixinConnectAccountId || t("未登录")}
              </code>
            </div>
            <div>
              <span>{t("已处理消息")}</span>
              <strong>{status?.processedMessages ?? 0}</strong>
            </div>
            <div>
              <span>{t("最近联系人")}</span>
              <code title={status?.lastPeerId || t("暂无")}>{status?.lastPeerId || t("暂无")}</code>
            </div>
          </div>
        </CardContent>
      </Panel>

      {qr?.qrContent ? (
        <Panel>
          <CardHeader>
            <CardTitle>{qr.qrStatus === "scaned" ? t("已扫码，请在手机上确认") : t("微信扫码登录")}</CardTitle>
            <CardDescription>{t("在手机微信中打开登录链接，或复制到可生成二维码的设备完成确认。")}</CardDescription>
          </CardHeader>
          <CardContent>
            {qr.qrSvg ? (
              <div className="weixin-qr-image" dangerouslySetInnerHTML={{ __html: qr.qrSvg }} />
            ) : null}
            <div className="weixin-qr-content">{qr.qrContent}</div>
            <div className="toolbar">
              <Button onClick={() => onOpenQr(qr.qrContent)}>
                <ExternalLink className="h-4 w-4" />
                {t("打开登录链接")}
              </Button>
              <Button onClick={() => onCopyQr(qr.qrContent)} variant="outline">
                <Copy className="h-4 w-4" />
                {t("复制链接")}
              </Button>
            </div>
          </CardContent>
        </Panel>
      ) : null}

      <Panel className="weixin-settings-panel">
        <CardHeader className="weixin-settings-head">
          <div>
            <CardTitle>{t("连接设置")}</CardTitle>
            <CardDescription>{t("每个微信联系人会映射到独立的 Codex 会话。")}</CardDescription>
          </div>
        </CardHeader>
        <CardContent className="weixin-connect-form">
          <section className="weixin-form-section" id="weixin-section-account">
            <div className="weixin-form-section-title">
              <KeyRound className="h-4 w-4" />
              <strong>{t("账号")}</strong>
            </div>
            <div className="weixin-form-fields">
              <label className="field">
                <span>{t("iLink API 地址")}</span>
                <Input
                  className="h-10"
                  onChange={(event) => onFormChange({ ...form, weixinConnectBaseUrl: event.target.value })}
                  value={form.weixinConnectBaseUrl}
                />
              </label>
              <label className="field">
                <span>{t("登录凭据")}</span>
                <Input
                  autoComplete="off"
                  className="h-10"
                  onChange={(event) => onFormChange({ ...form, weixinConnectToken: event.target.value })}
                  placeholder={t("扫码后自动保存，也可粘贴已有 Bearer token")}
                  type="password"
                  value={form.weixinConnectToken}
                />
              </label>
              <label className="field">
                <span>{t("允许的微信用户 ID")}</span>
                <Input
                  className="h-10"
                  onChange={(event) => onFormChange({ ...form, weixinConnectAllowFrom: event.target.value })}
                  placeholder="user@im.wechat"
                  value={form.weixinConnectAllowFrom}
                />
              </label>
              <label className="field">
                <span>{t("账号标识")}</span>
                <Input
                  className="h-10"
                  onChange={(event) => onFormChange({ ...form, weixinConnectAccountId: event.target.value })}
                  placeholder={t("扫码后自动填写")}
                  value={form.weixinConnectAccountId}
                />
              </label>
              <label className="field">
                <span>{t("SKRouteTag")}</span>
                <Input
                  className="h-10"
                  onChange={(event) => onFormChange({ ...form, weixinConnectRouteTag: event.target.value })}
                  placeholder={t("仅在网关要求时填写")}
                  value={form.weixinConnectRouteTag}
                />
              </label>
            </div>
          </section>

          <section className="weixin-form-section" id="weixin-section-session">
            <div className="weixin-form-section-title">
              <MessageCircle className="h-4 w-4" />
              <strong>{t("会话管理")}</strong>
            </div>
            <div className="weixin-form-fields">
              <label className="field">
                <span>{t("工作目录")}</span>
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
                  <Button onClick={onChooseWorkDir} size="icon" title={t("选择工作目录")} type="button" variant="outline">
                    <ExternalLink className="h-4 w-4" />
                  </Button>
                </div>
              </label>
              <label className="field">
                <span>{t("已有会话")}</span>
                <SearchableSessionPicker
                  onSelect={(session) => {
                    setSelectedSessionId(session?.id ?? "");
                    if (session?.cwd) onFormChange({ ...form, weixinConnectWorkDir: session.cwd });
                  }}
                  selectedId={selectedSessionId}
                  sessions={sessions}
                />
                <small className="weixin-field-hint">{t("选择后自动带入该会话的工作目录，微信联系人仍保持独立会话。")}</small>
              </label>
              <label className="field">
                <span>{t("模型")}</span>
                <Input
                  className="h-10"
                  onChange={(event) => onFormChange({ ...form, weixinConnectModel: event.target.value })}
                  placeholder={t("留空时使用 Codex 当前默认模型")}
                  value={form.weixinConnectModel}
                />
              </label>
              <label className="field">
                <span>{t("沙箱权限")}</span>
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
              </label>
            </div>
          </section>

          <section className="weixin-form-section" id="weixin-section-cli">
            <div className="weixin-form-section-title">
              <Settings className="h-4 w-4" />
              <strong>Codex CLI</strong>
            </div>
            <div className="weixin-form-fields">
              <label className="field">
                <span>{t("Codex CLI 路径")}</span>
                <div className="weixin-path-row weixin-cli-path-row">
                  <Input
                    className="h-10"
                    onChange={(event) => onFormChange({ ...form, weixinConnectCodexPath: event.target.value })}
                    placeholder={t("留空时从 PATH 查找 codex")}
                    value={form.weixinConnectCodexPath}
                  />
                  <Button
                    className="weixin-bundled-cli-button"
                    onClick={onUseDesktopCodexCli}
                    size="sm"
                    title={t("使用桌面版内置 CLI")}
                    type="button"
                    variant="secondary"
                  >
                    <PackageOpen className="h-4 w-4" />
                    {t("使用桌面版内置 CLI")}
                  </Button>
                  <Button onClick={onChooseCodexPath} size="icon" title={t("选择 Codex CLI")} type="button" variant="outline">
                    <ExternalLink className="h-4 w-4" />
                  </Button>
                </div>
              </label>
            </div>
          </section>
        </CardContent>
      </Panel>
      </div>
    </div>
  );
}

/// 网关总览顶部的两个独立广告位。
///
/// 这里只读取广告源的 `top_ad/topAds`，不会读取普通 `ads` 推荐池。
/// `top_ad` 可以是单个对象，也可以是数组；最多显示前两个有效广告。
function SponsorBoard({ ads, actions }: { ads: AdsResult | null; actions: Actions }) {
  const sourceAds = ads?.topAds?.length ? ads.topAds : ads?.topAd ? [ads.topAd] : [];
  const featured = sourceAds.filter((ad) => !isExpiredAd(ad)).slice(0, 2);

  if (featured.length === 0) return null;

  return (
    <div className={`sponsor-board ${featured.length === 1 ? "sponsor-board-single" : ""}`}>
      {featured.map((ad) => (
        <Panel className="jojocode-overview sponsor-slot" key={ad.id || ad.title}>
          <CardContent>
            <div className="sponsor-slot-content">
              <div className="sponsor-slot-main">
                {ad.image ? (
                  <img alt="" className="sponsor-logo" src={ad.image} />
                ) : (
                  <div className="jojocode-overview-mark">
                    <Network className="h-5 w-5" />
                  </div>
                )}
                <div className="sponsor-slot-copy">
                  <span className="eyebrow">{t("推荐内容")}</span>
                  <h2>{formatAdTitle(ad.title)}</h2>
                  <p className="recommendation-description" title={ad.description}>{ad.description}</p>
                </div>
              </div>
              {ad.highlights?.length ? (
                <div className="jojocode-model-tags sponsor-slot-tags">
                  {ad.highlights.map((item) => (
                    <span key={item}>{item}</span>
                  ))}
                </div>
              ) : null}
              <div className="sponsor-slot-actions">
                <Button onClick={() => void actions.openExternalUrl(ad.url)}>
                  <ExternalLink className="h-4 w-4" />
                  {t("打开推荐内容")}
                </Button>
              </div>
            </div>
          </CardContent>
        </Panel>
      ))}
    </div>
  );
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
  const health = healthItems(overview);
  const tool = toolEntries.find((entry) => entry.id === activeTool);
  const normalized = normalizeSettings(form);
  const activeProfile = normalized.relayProfiles.find((profile) => profile.id === normalized.activeRelayId);
  // 最近请求只在 Codex 概览里用；失败时静默显示空状态，不打断概览。
  const [recent, setRecent] = useState<RecentRequestSummary | null>(null);
  useEffect(() => {
    if (activeTool !== "codex") return;
    let cancelled = false;
    void invoke<RequestLogsResult>("load_request_logs", { limit: 300 })
      .then((result) => {
        // 网关记录里也有 Claude Code / Grok 的请求，Codex 概览只统计 Codex 自己的。
        if (!cancelled) setRecent(summarizeRecentRequests(result.status === "ok" ? result.entries.filter((entry) => requestLogAgent(entry) === "codex") : [], Date.now()));
      })
      .catch(() => { if (!cancelled) setRecent(summarizeRecentRequests([], Date.now())); });
    return () => { cancelled = true; };
  }, [activeTool]);
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
      <Button onClick={() => onNavigate("gateway")} size="sm">{t("管理接入")}</Button>
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
      <Button onClick={() => onNavigate("relay")} size="sm" variant="secondary">{t("管理供应商")}</Button>
    </>
  ) : (
    <Button onClick={() => onNavigate("relay")} size="sm">{t("添加供应商")}</Button>
  );
  const codexStats: DashboardStat[] = [
    { label: t("24 小时请求"), value: recent ? recent.total : "—" },
    { label: t("成功率"), value: recent?.successRate == null ? "—" : `${Math.round(recent.successRate * 100)}%`, tone: recent?.failed ? "warn" : "ok" },
    { label: t("平均耗时"), value: recent?.avgDurationMs == null ? "—" : `${(recent.avgDurationMs / 1000).toFixed(1)}s` },
    { label: t("常用模型"), value: recent?.topModel ?? "—", mono: false },
  ];
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
          stats={codexStats}
          status={codexStatus}
          version={overview?.codex_version}
        >
        <Panel>
          <CardHeader className="panel-head overview-health-header">
            <CardTitle>{t("健康检查")}</CardTitle>
            <div className="overview-health-header-actions">
              <CardDescription>{t("概览只展示关键问题，具体配置在对应页面处理")}</CardDescription>
              <Button
                aria-label={t("刷新状态")}
                title={t("刷新状态")}
                size="icon"
                variant="outline"
                className="shrink-0"
                onClick={() => void actions.refreshCurrent()}
              >
                <RefreshCw className="h-4 w-4" />
              </Button>
            </div>
          </CardHeader>
          <CardContent>
            <div className="health-grid overview-health-grid">
              <div className={`health-item ${overview?.codex_version ? "ok" : "needs-fix"}`}>
                <span className={`health-icon ${overview?.codex_version ? "good" : "warn"}`}><Tag className="h-4 w-4" /></span>
                <div>
                  <strong>{t("Codex 版本")}</strong>
                  <span title={overview?.codex_version ?? undefined}>{overview?.codex_version ?? t("未检测到 Codex 应用版本。")}</span>
                </div>
                <Badge status={overview?.codex_version ? "ok" : "not_checked"} />
              </div>
              {health.map((item) => (
                <div className={`health-item ${item.ok ? "ok" : "needs-fix"}`} key={item.title}>
                  <span className={`health-icon ${statusClass(item.status)}`}><item.icon className="h-4 w-4" /></span>
                  <div>
                    <strong>{item.title}</strong>
                    <span title={item.detail}>{item.detail}</span>
                  </div>
                  <Badge status={item.status} />
                </div>
              ))}
            </div>
          </CardContent>
        </Panel>
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
  return (
    <div className="gw-shared-settings">
      <SectionLabel aside={autoSave}>{t("本地服务（新旧共用）")}</SectionLabel>
      <div className="gw-card">
        {health.map((item) => (
          <div className="gw-setting-row" key={item.title}>
            <div>
              <strong>{item.title}</strong>
              <small title={item.detail}>{item.detail}</small>
            </div>
            <span />
            <span className="gw-pill" data-tone={tone(item)}>{statusLabel(item.status)}</span>
          </div>
        ))}
        <div className="gw-setting-row">
          <div>
            <strong>{t("请求体上限")}</strong>
            <small title={t("默认 128 MiB，可填 32~512。长上下文会话（1M 窗口 + 截图历史）的请求体会超过旧的 32 MiB 硬上限并触发 413；调大后本地代理处理单次请求的内存占用也会增加。保存后立即对本地代理生效；环境变量 CODEX_PLUS_MAX_HTTP_BODY_BYTES（字节）优先于本设置。")}>
              {t("长上下文会话超过上限会报 413；调大会增加本地服务的内存占用。")}
            </small>
          </div>
          <span />
          {number(form.helperMaxRequestBodyMib, 32, 512, "MiB", (next) => onFormChange({ ...form, helperMaxRequestBodyMib: next }), 16)}
        </div>
      </div>

      <SectionLabel>{t("请求避震（新旧共用）")}</SectionLabel>
      <div className="gw-card">
        <div className="gw-setting-row">
          <div>
            <strong>{t("启用请求避震")}</strong>
            <small>
              {retry
                ? t("在本地服务内消化上游过载与限流，尽量不把错误交给 Agent。作用于经过网关和旧版本地协议代理的请求。")
                : t("已关闭：上游过载或限流时，错误直接交给 Agent。")}
            </small>
          </div>
          <span />
          <Switch label={t("启用请求避震")} on={retry} onToggle={() => onFormChange({ ...form, protocolProxyAutoRetry: !retry })} />
        </div>
        {retry ? (
          <>
            <div className="gw-setting-row">
              <div>
                <strong>{t("最多自动重试次数")}</strong>
                <small>{t("0–20 次；次数用尽后把错误交给 Agent。")}</small>
              </div>
              <span />
              {number(form.protocolProxyAutoRetryMaxAttempts, 0, 20, t("次"), (next) => onFormChange({ ...form, protocolProxyAutoRetryMaxAttempts: next }))}
            </div>
            <div className="gw-setting-row">
              <div>
                <strong>{t("首次重试间隔")}</strong>
                <small>{t("之后按 2 倍退避，单次最长 30 秒；有 Retry-After 时优先采用。")}</small>
              </div>
              <span />
              {number(form.protocolProxyAutoRetryIntervalSeconds, 0, 30, t("秒"), (next) => onFormChange({ ...form, protocolProxyAutoRetryIntervalSeconds: next }))}
            </div>
            <div className="gw-setting-row">
              <div>
                <strong>{t("避震模式")}</strong>
                <small title={t("流式：保留实时输出，正文提交前遇到过载由代理重试，之后交给 Codex；「首字提交延迟」为出现正文后再缓冲的时长，0 表示立即提交。整段缓冲：始终由代理重试，但要等整段生成完才显示。")}>
                  {form.protocolProxyAutoRetryMode === "buffered"
                    ? t("始终由代理重试，但要等整段生成完才显示。")
                    : form.protocolProxyAutoRetryMode === "streaming-legacy"
                      ? t("旧版流式行为，仅在新版有问题时使用。")
                      : t("保留实时输出；正文出现前遇到过载由代理重试。")}
                </small>
              </div>
              <span />
              <AppSelect
                label={t("避震模式")}
                onChange={(next) => onFormChange({ ...form, protocolProxyAutoRetryMode: next })}
                options={[
                  { value: "streaming", label: t("流式（推荐）"), hint: t("保留实时输出；正文出现前遇到过载由代理重试。") },
                  { value: "streaming-legacy", label: t("流式（旧行为）"), hint: t("旧版流式行为，仅在新版有问题时使用。") },
                  { value: "buffered", label: t("整段缓冲"), hint: t("始终由代理重试，但要等整段生成完才显示。") },
                ]}
                value={form.protocolProxyAutoRetryMode}
                menuWidth={300}
              />
            </div>
            {form.protocolProxyAutoRetryMode === "streaming" ? (
              <div className="gw-setting-row">
                <div>
                  <strong>{t("首字提交延迟")}</strong>
                  <small>{t("出现正文后再缓冲的时长，0 表示立即提交。")}</small>
                </div>
                <span />
                {number(form.protocolProxyAutoRetryCommitDelayMs, 0, 10000, "ms", (next) => onFormChange({ ...form, protocolProxyAutoRetryCommitDelayMs: next }), 100)}
              </div>
            ) : null}
            {/* 状态码编辑不能放进 <label>：WebKit 会把点击转发给第一个按钮，一次删掉两个状态码。 */}
            <div className="gw-setting-row gw-setting-row-stacked">
              <div>
                <strong>{t("重试的 HTTP 状态码")}</strong>
                <small>{t("命中时退避重发。默认不选；常用 429（限流）和 5xx（服务端故障）。")}</small>
              </div>
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
                <button
                  className="gw-text-button"
                  onClick={() => setRetryStatusCodes([...form.protocolProxyAutoRetryStatusCodes, ...COMMON_RETRY_STATUS_CODES])}
                  type="button"
                >
                  {t("填入常用")}
                </button>
              </div>
            </div>
          </>
        ) : null}
      </div>
    </div>
  );
}

function ProxySettingsScreen({
  dirty,
  form,
  relayEnvironment,
  onFormChange,
  onOpenGatewaySettings,
  actions,
}: {
  dirty: boolean;
  form: BackendSettings;
  relayEnvironment: RelayEnvironmentResult | null;
  onFormChange: (value: BackendSettings) => void;
  onOpenGatewaySettings: () => void;
  actions: Actions;
}) {
  // Codex 被新版网关接管时，旧版专用的设置不起作用，隐藏。
  const takenOverByGateway = form.activeRelayId === CODEX_GATEWAY_RELAY_ID;
  const proxyProfiles = useMemo(
    () => (form.relayProfiles ?? []).filter((profile) => profile.relayMode !== "aggregate"),
    [form.relayProfiles],
  );
  const proxyProfileKey = proxyProfiles.map((profile) => profile.id).join("|");
  const [proxyUrls, setProxyUrls] = useState<Record<string, string>>({});
  // 已保存的值：输入与它一致时「保存」置灰，十几行供应商里一眼看出哪行改过。
  const [savedProxyUrls, setSavedProxyUrls] = useState<Record<string, string>>({});
  const [proxyConfigured, setProxyConfigured] = useState<Record<string, boolean>>({});
  const [proxyBusyId, setProxyBusyId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const ids = proxyProfileKey ? proxyProfileKey.split("|") : [];
    void Promise.all(
      ids.map(async (id) => {
        try {
          const result = await invoke<{ status: string; proxyUrl?: string; configured?: boolean }>(
            "load_relay_proxy",
            { profileId: id },
          );
          return { id, url: result.status === "ok" ? result.proxyUrl ?? "" : "", configured: result.configured === true };
        } catch {
          return { id, url: "", configured: false };
        }
      }),
    ).then((entries) => {
      if (cancelled) return;
      const urls: Record<string, string> = {};
      const configured: Record<string, boolean> = {};
      entries.forEach((entry) => {
        urls[entry.id] = entry.url;
        configured[entry.id] = entry.configured;
      });
      setProxyUrls(urls);
      setSavedProxyUrls(urls);
      setProxyConfigured(configured);
    });
    return () => {
      cancelled = true;
    };
  }, [proxyProfileKey]);

  const saveProviderProxy = async (profile: RelayProfile) => {
    const profileId = profile.id;
    setProxyBusyId(profileId);
    try {
      const result = await invoke<{
        status: string;
        message: string;
        proxyUrl?: string;
        configured?: boolean;
      }>("set_relay_proxy", { profileId, proxyUrl: proxyUrls[profileId] ?? "" });
      if (result.status === "ok") {
        setProxyUrls((current) => ({ ...current, [profileId]: result.proxyUrl ?? "" }));
        setSavedProxyUrls((current) => ({ ...current, [profileId]: result.proxyUrl ?? "" }));
        setProxyConfigured((current) => ({ ...current, [profileId]: result.configured === true }));
      }
      await actions.showMessage(t("供应商独立代理"), result.message, result.status === "ok" ? "ok" : "failed");
    } catch (error) {
      await actions.showMessage(t("供应商独立代理"), String(error), "failed");
    } finally {
      setProxyBusyId(null);
    }
  };


  return (
    <div className="settings-page proxy-settings-page">
      <SettingsAutoSave dirty={dirty} form={form} actions={actions} />
      <Panel>
        <CardHead
          title={t("本地服务与请求避震已移到网关设置")}
          detail={t("本地服务状态、请求体上限和请求避震对新旧两套同时生效，统一在「网关 → 网关设置」里改。")}
        />
        <CardContent className="settings-content">
          <Button onClick={onOpenGatewaySettings} size="sm" variant="secondary">
            {t("打开网关设置")}
            <ArrowRight aria-hidden="true" className="h-3.5 w-3.5" />
          </Button>
        </CardContent>
      </Panel>

      {takenOverByGateway ? (
        <p className="relay-switch-off-note" role="status">
          {t("Codex 正由 Codex++ 网关接管，旧版代理设置不起作用，已隐藏。断开并还原后恢复。")}
        </p>
      ) : (
      <>
      <Panel>
        <SwitchPanelHead
          checked={form.protocolProxyMapUnlistedModels}
          detail={t("把 Codex 后台任务的固定模型名改写为当前供应商的默认模型")}
          label={t("改写目录外模型名")}
          onChange={(checked) => onFormChange({ ...form, protocolProxyMapUnlistedModels: checked })}
          title={t("目录外模型映射（旧版）")}
        />
        <CardContent className="settings-content setting-rows">
          <p className="setting-footnote" title={t("Codex 的自动审查（codex-auto-review）与标题/摘要等后台任务固定使用 gpt-* 等内部模型名，不经选模；第三方上游没有这些名字，原样转发只会报「模型不存在」并可能反复消耗额度。开启后，不在当前供应商模型列表里的请求模型会改写为该供应商的默认模型再转发（#1183 / #1446 / #1457）；官方登录与聚合供应商不受影响，已配置「自动审批模型」时 codex-auto-review 仍按该配置改写。默认关闭：开启后手填但未写进模型列表的模型也会被改写，请先把要用的模型都加进列表。")}>
            {t("自动审查、标题摘要等后台任务固定使用 gpt-* 内部模型名，第三方上游没有这些名字会报「模型不存在」。开启后，不在当前供应商模型列表里的模型会改写为该供应商的默认模型；官方登录与聚合供应商不受影响。")}
          </p>
          {form.protocolProxyMapUnlistedModels ? (
            <p className="setting-warning">{t("注意：手填但没写进模型列表的模型也会被改写，请先把要用的模型都加进列表。")}</p>
          ) : null}
          <p className="setting-footnote">{t("接入网关的 Agent 不用这一项：网关会把供应商池里没有的模型改写成该 Agent 的主模型。")}</p>
        </CardContent>
      </Panel>

      <Panel>
        <CardHead
          title={t("供应商独立代理（旧版供应商）")}
          detail={t("为旧版的单个供应商单独指定 HTTP 代理；留空表示沿用系统代理。网关供应商的代理在供应商池里编辑供应商时设置。")}
        />
        <CardContent className="settings-content">
          {proxyProfiles.length ? (
            <div className="proxy-provider-list">
              {proxyProfiles.map((profile) => (
                <div className="proxy-provider-row" key={profile.id}>
                  <div className="proxy-provider-meta">
                    <strong>{profile.name || t("未命名供应商")}</strong>
                    <small>
                      {relayProtocolLabel(profile.protocol)} ·{" "}
                      {proxyConfigured[profile.id] ? t("已配置独立代理") : t("未配置独立代理")}
                    </small>
                  </div>
                  <Input
                    aria-label={tf("{0} 的独立代理", [profile.name || t("未命名供应商")])}
                    placeholder={t("例如 http://127.0.0.1:7890，留空表示使用系统代理")}
                    value={proxyUrls[profile.id] ?? ""}
                    onChange={(event) => {
                      // 先取值再进 updater：updater 延迟执行时 currentTarget 已为 null，会抛错白屏（#2705）
                      const value = event.currentTarget.value;
                      setProxyUrls((current) => ({ ...current, [profile.id]: value }));
                    }}
                  />
                  <Button
                    disabled={
                      proxyBusyId === profile.id ||
                      (proxyUrls[profile.id] ?? "").trim() === (savedProxyUrls[profile.id] ?? "").trim()
                    }
                    onClick={() => void saveProviderProxy(profile)}
                    variant="secondary"
                  >
                    {proxyBusyId === profile.id ? t("保存中…") : t("保存")}
                  </Button>
                </div>
              ))}
            </div>
          ) : (
            <p className="field-hint">{t("还没有可配置独立代理的供应商。")}</p>
          )}
          <p className="field-hint">
            {t("保存后需重新应用该供应商并重启 Codex，独立代理才会作用于会话请求。")}
          </p>
        </CardContent>
      </Panel>

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
  const time = date.toLocaleTimeString("zh-CN", { hour12: false });
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
  if (entry.autoRetries > 0) return tf("自动重试 ×{0}", [String(entry.autoRetries)]);
  if (entry.attempts > 1) return tf("切换 ×{0}", [String(entry.attempts)]);
  return "-";
}

/** 请求来自哪个 Agent：网关记录带 agent 字段，Codex 直连代理的记录没有。 */
function requestLogAgent(entry: RequestLogEntry): string {
  return entry.agent || "codex";
}

function RequestLogScreen({ initialAgent = "all", onConsumeInitialAgent }: { initialAgent?: "all" | "codex" | "claude" | "grok"; onConsumeInitialAgent?: () => void }) {
  const [entries, setEntries] = useState<RequestLogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<"all" | "running" | "success" | "failed" | "retried">("all");
  const [sessionFilter, setSessionFilter] = useState("all");
  const [agentFilter, setAgentFilter] = useState<"all" | "codex" | "claude" | "grok">(initialAgent);
  // 初始过滤只用一次：之后从侧栏进来仍是「全部」。
  useEffect(() => { if (initialAgent !== "all") onConsumeInitialAgent?.(); }, []); // eslint-disable-line react-hooks/exhaustive-deps
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

  const load = async (silent = false) => {
    if (!silent) setLoading(true);
    try {
      const result = await invoke<RequestLogsResult>("load_request_logs", { limit: 300 });
      if (result.status === "ok") {
        setEntries(result.entries ?? []);
        setError("");
      } else {
        setError(result.message || t("读取请求日志失败。"));
      }
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  useEffect(() => {
    if (!autoRefresh) return;
    const timer = window.setInterval(() => {
      if (!document.hidden) void load(true);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [autoRefresh]);

  const clearLogs = async () => {
    if (!(await appConfirm(t("清空全部请求日志？此操作不可撤销。"), { title: t("清空请求日志"), confirmText: t("清空"), danger: true }))) return;
    setClearing(true);
    try {
      const result = await invoke<{ status: Status; message: string }>("clear_request_logs");
      if (result.status !== "ok") setError(result.message);
      await load(true);
    } catch (cause) {
      setError(String(cause));
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

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return entries.filter((entry) => {
      const running = entry.state === "running";
      if (filter === "running" && !running) return false;
      if (filter === "success" && (running || !entry.success)) return false;
      if (filter === "failed" && (running || entry.success)) return false;
      if (filter === "retried" && entry.autoRetries === 0 && entry.attempts <= 1) return false;
      if (agentFilter !== "all" && requestLogAgent(entry) !== agentFilter) return false;
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
        entry.relayName,
        entry.path,
        entry.threadId ?? "",
        entry.error ?? "",
        entry.wireApi,
      ]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
  }, [entries, filter, query, sessionFilter, agentFilter]);

  return (
    <Panel className="request-log-panel">
      <CardContent className="request-log-content">
        <div className="request-log-toolbar">
          <div className="request-log-search">
            <Search className="h-4 w-4" aria-hidden="true" />
            <Input
              aria-label={t("搜索模型 / 供应商 / 路径 / 错误")}
              placeholder={t("搜索模型 / 供应商 / 路径 / 错误")}
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
          </div>
          <AppSelect
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
          />
          <AppSelect
            ariaLabel={t("状态筛选")}
            className="request-log-select"
            menuWidth={140}
            onChange={(next) => setFilter(next)}
            options={[
              { value: "all", label: t("全部") },
              { value: "running", label: t("进行中") },
              { value: "success", label: t("仅成功") },
              { value: "failed", label: t("仅失败") },
              { value: "retried", label: t("含重试") },
            ] as AppSelectOption<typeof filter>[]}
            size="sm"
            value={filter}
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
          <label className="check-row request-log-auto">
            <input
              checked={autoRefresh}
              onChange={(event) => setAutoRefresh(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>{t("自动刷新")}</span>
          </label>
          <Button variant="secondary" onClick={() => void load()}>
            <RefreshCw className="h-4 w-4" />
            {t("刷新")}
          </Button>
          <Button disabled={clearing || entries.length === 0} variant="secondary" onClick={() => void clearLogs()}>
            <Trash2 className="h-4 w-4" />
            {t("清空")}
          </Button>
        </div>

        {error ? (
          <p className="field-hint request-log-error" role="alert">{error}</p>
        ) : null}

        {loading && entries.length === 0 ? (
          <div className="empty">{t("正在读取请求日志…")}</div>
        ) : filtered.length ? (
          <div className="request-log-table" role="table" aria-label={t("请求日志")}>
            <div className="request-log-row request-log-head" role="row">
              <span>{t("时间")}</span>
              <span>{t("状态")}</span>
              <span>{t("来源")}</span>
              <span>{t("模型")}</span>
              <span>{t("重试")}</span>
              <span>{t("输入")}</span>
              <span>{t("输出")}</span>
              <span>{t("缓存")}</span>
              <span>{t("耗时")}</span>
            </div>
            {filtered.map((entry) => (
              <Fragment key={entry.id}>
                <button
                  aria-expanded={expanded === entry.id}
                  className={`request-log-row request-log-entry ${entry.state === "running" ? "running" : entry.success ? "ok" : "bad"}`}
                  onClick={() => setExpanded(expanded === entry.id ? null : entry.id)}
                  role="row"
                  type="button"
                >
                  <span>{formatRequestLogTime(entry.startedAtMs || entry.timestampMs)}</span>
                  {entry.state === "running" ? (
                    <span className="request-log-status running">
                      {t("运行中")}
                    </span>
                  ) : (
                    <span
                      className={`request-log-status ${entry.success ? "ok" : "bad"}`}
                      title={entry.error ?? ""}
                    >
                      {entry.statusCode || "ERR"}
                      {!entry.success ? (
                        <small className="request-log-stream-failed">
                          {entry.stream ? t("流失败") : t("失败")}
                        </small>
                      ) : null}
                    </span>
                  )}
                  <span className="request-log-ellipsis request-log-source" title={`${agentName(requestLogAgent(entry))} · ${maskAccountName(entry.relayName)}`}>
                    <AgentGlyph agent={requestLogAgent(entry)} />
                    {maskAccountName(entry.relayName) || "-"}
                  </span>
                  <span className="request-log-ellipsis" title={entry.upstreamModel ? `${entry.model} → ${entry.upstreamModel}` : entry.model}>
                    {entry.model || "-"}
                    {entry.upstreamModel && entry.upstreamModel !== entry.model ? (
                      <small> → {entry.upstreamModel}</small>
                    ) : null}
                  </span>
                  <span
                    className={entry.autoRetries > 0 ? "request-log-retry" : "request-log-muted"}
                    title={(entry.retryReasons ?? []).join("、")}
                  >
                    {requestLogRetryLabel(entry)}
                  </span>
                  <span className="request-log-muted" title={entry.state === "running" ? t("token 用量由上游在请求完成时返回，流式过程中无法准确统计") : ""}>
                    {entry.state === "running" ? t("流式中") : formatRequestLogTokens(entry.usage?.inputTokens)}
                  </span>
                  <span
                    className="request-log-muted"
                    title={entry.state === "running"
                      ? t("按流式增量估算，完成后会更新为准确值")
                      : ""}
                  >
                    {entry.state === "running"
                      ? entry.estimatedOutputTokens
                        ? `~${formatRequestLogTokens(entry.estimatedOutputTokens)}`
                        : t("流式中")
                      : formatRequestLogTokens(entry.usage?.outputTokens)}
                  </span>
                  <span>
                    {entry.state === "running" ? (
                      <span className="request-log-muted">{t("流式中")}</span>
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
                  <span className="request-log-duration">
                    {formatRequestLogDuration(
                      entry.state === "running"
                        ? Math.max(0, nowMs - entry.startedAtMs)
                        : entry.durationMs,
                    )}
                    {entry.state === "running" ? (
                      <small>{t("进行中")}</small>
                    ) : entry.firstTokenMs != null ? (
                      <small>{tf("首字 {0}", [formatRequestLogDuration(entry.firstTokenMs)])}</small>
                    ) : null}
                  </span>
                </button>
                {expanded === entry.id ? (
                  <div className="request-log-detail">
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
                      <div><dt>{t("上游模型")}</dt><dd>{entry.upstreamModel || "-"}</dd></div>
                      <div><dt>{t("首字耗时")}</dt><dd>{entry.firstTokenMs == null ? (entry.state === "running" ? t("进行中") : "-") : formatRequestLogDuration(entry.firstTokenMs)}</dd></div>
                      <div><dt>{t("尝试次数")}</dt><dd>{entry.attempts || "-"}</dd></div>
                      <div><dt>{t("自动重试")}</dt><dd>{entry.autoRetries}</dd></div>
                      <div><dt>{t("重试原因")}</dt><dd>{(entry.retryReasons ?? []).join("、") || "-"}</dd></div>
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
                ) : null}
              </Fragment>
            ))}
          </div>
        ) : (
          <div className="empty">
            {entries.length
              ? t("没有匹配当前筛选条件的请求。")
              : t("还没有请求日志。在 Codex 或已接入网关的 Agent 里发起一次对话后即可看到。")}
          </div>
        )}
      </CardContent>
    </Panel>
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
  const allPassed = Boolean(result) && checks.every((check) => check.passed);

  return (
    <Panel>
      {/* 「重新检测」放在标题栏右侧，与其它卡片的页面级操作同位，不再单独占一行。 */}
      <CardHeader className="panel-head relay-environment-head">
        <CardTitle>{t("中转站环境配置检测")}</CardTitle>
        <div className="relay-environment-head-side">
          <CardDescription>
            {result ? (allPassed ? t("三项检测全部通过") : t("检测到需要处理的环境问题")) : t("正在读取本机环境")}
          </CardDescription>
          <Button onClick={() => void actions.refreshRelayEnvironment()} size="sm" variant="outline">
            <RefreshCw className="h-4 w-4" />
            {t("重新检测")}
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        <div className="relay-environment-checks">
          {checks.map((check) => (
            <div className={`relay-environment-check ${result ? (check.passed ? "ok" : "failed") : "pending"}`} key={check.id}>
              <div className="relay-environment-check-icon">
                {result ? (check.passed ? <CheckCircle2 className="h-5 w-5" /> : <ShieldAlert className="h-5 w-5" />) : <RefreshCw className="h-5 w-5" />}
              </div>
              <div className="relay-environment-check-copy">
                <strong>{check.title}</strong>
                <span>{check.detail}</span>
              </div>
              <Badge status={result ? (check.passed ? "ok" : "failed") : "not_checked"} />
            </div>
          ))}
        </div>
      </CardContent>
    </Panel>
  );
}

/** Codex 接入网关时，网关在直连模式的供应商列表里放的托管供应商（core 的 `CODEX_GATEWAY_RELAY_ID`）。 */
const CODEX_GATEWAY_RELAY_ID = "codex-plus-gateway";

function codexGatewayModelCount(profile: RelayProfile | undefined): number {
  return (profile?.modelList ?? "").split("\n").filter((line) => line.trim()).length;
}

/** 托管的网关供应商带 openai 会话身份：Codex 保留 ChatGPT 官方登录，请求经本地代理进网关。 */
function codexGatewayKeepsLogin(profile: RelayProfile | undefined): boolean {
  return /^\s*model_provider\s*=\s*["']openai["']/m.test(profile?.configContents ?? "");
}

/**
 * 旧版 Codex 供应商配置页顶部的提示。
 * - Codex 还指向托管供应商、网关里却没有它的接入记录（降级后又升级、gateway.json 换过或坏了）：
 *   给「切回直连」，否则页面一直显示「被网关接管」却断不开；
 * - Codex 正经网关接入：显示状态，可去管理或直接断开并还原；
 * - 断开过但托管供应商还在：提示点它的「使用」即可重新接入；
 * - 其余：建议改用新版网关的供应商池（这里的配置照旧生效）。
 */
function LegacyRelayGatewayHint({
  form,
  onOpenGateway,
  onManageAgents,
  onDisconnect,
}: {
  form: BackendSettings;
  onOpenGateway: () => void;
  onManageAgents: () => void;
  onDisconnect: () => Promise<void>;
}) {
  const [disconnecting, setDisconnecting] = useState(false);
  const managed = form.relayProfiles.find((profile) => profile.id === CODEX_GATEWAY_RELAY_ID);
  const onGateway = form.activeRelayId === CODEX_GATEWAY_RELAY_ID;
  // 网关里有没有开着的 Codex 接入记录；读不出 gateway.json 也算没有。
  const [bound, setBound] = useState<boolean | null>(null);
  useEffect(() => {
    if (!onGateway) return;
    let cancelled = false;
    invoke<GatewayResult>("load_gateway")
      .then((result) => {
        if (!cancelled) setBound(result.status === "ok" && result.gateway?.bindings?.codex?.enabled === true);
      })
      .catch(() => {
        if (!cancelled) setBound(false);
      });
    return () => {
      cancelled = true;
    };
  }, [onGateway, form.relayProfiles]);
  const disconnect = () => {
    setDisconnecting(true);
    void onDisconnect().finally(() => setDisconnecting(false));
  };
  if (onGateway && (!managed || bound === false)) {
    return (
      <div className="legacy-gateway-hint" data-state="orphan" role="status">
        <Waypoints aria-hidden="true" className="legacy-gateway-hint-icon" />
        <div className="legacy-gateway-hint-text">
          <strong>{t("Codex 还指向网关托管的供应商")}</strong>
          <span>{t("网关里没有 Codex 的接入记录（可能降级后又升级过，或网关状态文件损坏），请求可能连不上。点「切回直连」切回接入前或列表里第一个直连供应商。")}</span>
        </div>
        <Button disabled={disconnecting} onClick={disconnect} size="sm">
          {disconnecting ? t("切换中…") : t("切回直连")}
        </Button>
      </div>
    );
  }
  if (managed && onGateway) {
    const count = codexGatewayModelCount(managed);
    return (
      <div className="legacy-gateway-hint" data-state="connected" role="status">
        <Waypoints aria-hidden="true" className="legacy-gateway-hint-icon" />
        <div className="legacy-gateway-hint-text">
          <strong>
            {t("Codex 当前经 Codex++ 网关（新版）")}
            {codexGatewayKeepsLogin(managed) ? <span className="legacy-gateway-hint-badge">{t("保留 ChatGPT 登录")}</span> : null}
          </strong>
          <span>
            {count
              ? tf("用网关供应商池里的 {0} 个模型。接入期间旧版供应商列表已隐藏；要改用它们，先点「断开并还原」恢复接入前的配置。", [count])
              : t("用网关供应商池里的模型。接入期间旧版供应商列表已隐藏；要改用它们，先点「断开并还原」恢复接入前的配置。")}
          </span>
        </div>
        <Button onClick={onManageAgents} size="sm" variant="secondary">
          {t("管理接入")}
          <ArrowRight aria-hidden="true" className="h-3.5 w-3.5" />
        </Button>
        <Button disabled={disconnecting} onClick={disconnect} size="sm" variant="outline">
          {disconnecting ? t("断开中…") : t("断开并还原")}
        </Button>
      </div>
    );
  }
  if (managed) {
    return (
      <div className="legacy-gateway-hint" role="note">
        <Waypoints aria-hidden="true" className="legacy-gateway-hint-icon" />
        <div className="legacy-gateway-hint-text">
          <strong>{t("Codex 已从网关切回直连")}</strong>
          <span>{t("想回到新版网关，在下面「Codex++ 网关」那一项点「使用」，或到「Agent 接入」里打开 Codex。")}</span>
        </div>
        <Button onClick={onManageAgents} size="sm" variant="secondary">
          {t("打开 Agent 接入")}
          <ArrowRight aria-hidden="true" className="h-3.5 w-3.5" />
        </Button>
      </div>
    );
  }
  return (
    <div className="legacy-gateway-hint" role="note">
      <Waypoints aria-hidden="true" className="legacy-gateway-hint-icon" />
      <div className="legacy-gateway-hint-text">
        <strong>{t("建议改用新版网关")}</strong>
        <span>{t("在网关的「供应商池」统一管理供应商，多个 Agent 共用，支持多 Key 轮换、按模型路由、故障转移与用量统计。这里是旧版的 Codex 直连配置，仍可继续使用。")}</span>
      </div>
      <Button onClick={onOpenGateway} size="sm" variant="secondary">
        {t("打开供应商池")}
        <ArrowRight aria-hidden="true" className="h-3.5 w-3.5" />
      </Button>
    </div>
  );
}

function RelayScreen({
  settings,
  relayFiles,
  envConflicts,
  ccsProviders,
  form,
  actions,
}: {
  settings: SettingsResult | null;
  relayFiles: RelayFilesResult | null;
  envConflicts: EnvConflictsResult | null;
  ccsProviders: CcsProvidersResult | null;
  form: BackendSettings;
  actions: Actions;
}) {
  const normalized = normalizeSettings(form);
  const takenOverByGateway = normalized.activeRelayId === CODEX_GATEWAY_RELAY_ID;
  const [detailProfileId, setDetailProfileId] = useState<string | null>(null);
  const [newProfileDraft, setNewProfileDraft] = useState<RelayProfile | null>(null);
  const [providerCatalogOpen, setProviderCatalogOpen] = useState(false);
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
  const editRelayProfile = async (profileId: string) => {
    setNewProfileDraft(null);
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

  return (
    <div className="mg-page relay-management-page">
      <PageHeader
        icon={<KeyRound className="h-5 w-5" />}
        title={t("供应商列表")}
        actions={takenOverByGateway ? null : (
          <>
            {/* 总开关很少改动：收成标题栏里的小开关，完整说明放在悬停提示里；关闭时下方另有提示条。 */}
            {/* Codex 被新版网关接管时，旧版的切换、导入、添加都不起作用，整组隐藏；先「断开并还原」。 */}
            <label
              className="switch-row compact relay-master-switch relay-master-switch-compact"
              title={t("关闭后本工具不会在手动切换时写入 Codex 的 config.toml / auth.json；启动 Codex 时始终不会自动改这些文件。")}
            >
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
            <div className="third-party-import" ref={thirdPartyImportTriggerRef}>
              <Button
                onClick={openThirdPartyImport}
                variant="secondary"
              >
                <Download className="h-4 w-4" />
                {t("从第三方导入")}
              </Button>
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
            <Button
              onClick={() => { setNewProfileDraft(null); setDetailProfileId(null); setProviderCatalogOpen(true); }}
            >
              <Plus className="h-4 w-4" />
              {t("添加供应商")}
            </Button>
          </>
        )}
      />
      <div className="mg-content">
        <Panel className="relay-management-panel">
          <CardContent>
          <EnvConflictNotice envConflicts={envConflicts} actions={actions} />
          {!normalized.relayProfilesEnabled ? (
            <p className="relay-switch-off-note" role="status">
              {t("供应商配置切换已关闭：Codex++ 不会写入 Codex 的 config.toml / auth.json，供应商列表已隐藏。需要切换时打开右上角的「配置切换」。")}
            </p>
          ) : null}
          {/* Codex 被新版网关接管：旧版列表整块隐藏，断开并还原后恢复（状态与断开按钮在页面顶部）。 */}
          {normalized.relayProfilesEnabled && takenOverByGateway ? (
            <p className="relay-switch-off-note" role="status">
              {t("Codex 正由 Codex++ 网关接管，旧版供应商列表已隐藏。点上方「断开并还原」（或「切回直连」）后恢复。")}
            </p>
          ) : null}
          {/* 总开关关闭时供应商切换不会写入 Codex：分页和列表都没有意义，整块隐藏，只留上方提示条。 */}
          {normalized.relayProfilesEnabled && !takenOverByGateway ? (
            <>
            {/* 分页与「新建」按钮同一行：按钮靠右，不再单独占一行把列表往下推。 */}
            <div className="provider-view-bar">
              <div className="provider-view-tabs" role="tablist" aria-label={t("供应商配置模式")}>
                <button aria-selected={providerView === "direct"} className={providerView === "direct" ? "active" : ""} onClick={() => selectProviderView("direct")} role="tab" title={t("直接访问 Responses API 供应商：只写入 Base URL 与 Key，不做协议转换。")} type="button">
                  <Cable className="h-4 w-4" />
                  <span>{t("直连")}</span>
                  <CircleHelp className="provider-view-help" size={13} />
                </button>
                <button aria-selected={providerView === "modelAggregation"} className={providerView === "modelAggregation" ? "active" : ""} onClick={() => selectProviderView("modelAggregation")} role="tab" title={t("把成员模型合并到 Codex 模型列表，带供应商前缀的模型固定发送到对应成员。")} type="button">
                  <Layers3 className="h-4 w-4" />
                  <span>{t("模型聚合")}</span>
                  <CircleHelp className="provider-view-help" size={13} />
                </button>
                <button aria-selected={providerView === "routing"} className={providerView === "routing" ? "active" : ""} onClick={() => selectProviderView("routing")} role="tab" title={t("供应商只支持 Chat Completions（或配置了模型路由）：由本地协议代理转换后转发。")} type="button">
                  <GitBranch className="h-4 w-4" />
                  <span>{t("需协议转换")}</span>
                  <CircleHelp className="provider-view-help" size={13} />
                </button>
                <button aria-selected={providerView === "failover"} className={providerView === "failover" ? "active" : ""} onClick={() => selectProviderView("failover")} role="tab" title={t("多个供应商组成请求池，支持失败切换、会话轮转、请求轮转与权重；成员可以是 API 供应商或官方登录账号。")} type="button">
                  <ArrowRightLeft className="h-4 w-4" />
                  <span>{t("故障转移")}</span>
                  <CircleHelp className="provider-view-help" size={13} />
                </button>
              </div>
              {(providerView === "failover" || providerView === "modelAggregation") && visibleProfiles.length ? (
                <div className="provider-aggregate-toolbar">
                  <Button
                    onClick={() => createNewAggregateProfile(providerView === "failover" ? "provider" : "model")}
                    size="sm"
                    variant="secondary"
                  >
                    <Plus className="h-4 w-4" />
                    {providerView === "failover" ? t("新建故障转移") : t("新建模型聚合")}
                  </Button>
                </div>
              ) : null}
            </div>
              {providerView === "direct" && !visibleProfiles.length ? (
                <div className="provider-aggregate-empty">
                  <strong>{t("这个分页还没有直连供应商")}</strong>
                  <span>{t("直连分页只显示 Responses 直连且未配置模型路由的供应商；其他供应商在其余分页，配置不会丢失。")}</span>
                  <Button onClick={() => { setNewProfileDraft(null); setDetailProfileId(null); setProviderCatalogOpen(true); }}>{t("添加供应商")}</Button>
                </div>
              ) : null}
              {providerView === "routing" && !visibleProfiles.length ? (
                <div className="provider-aggregate-empty">
                  <strong>{t("这个分页还没有需协议转换的供应商")}</strong>
                  <span>{t("Chat Completions 协议或配置了模型路由的供应商会显示在这里；其他供应商在其余分页。")}</span>
                  <Button onClick={() => { setNewProfileDraft(null); setDetailProfileId(null); setProviderCatalogOpen(true); }}>{t("添加供应商")}</Button>
                </div>
              ) : null}
              {providerView === "failover" && !visibleProfiles.length ? (
                <div className="provider-aggregate-empty">
                  <strong>{t("还没有故障转移配置")}</strong>
                  <span>{t("在编辑页勾选成员供应商，并选择失败切换、轮转或权重策略。")}</span>
                  <Button onClick={() => createNewAggregateProfile("provider")}>{t("创建故障转移配置")}</Button>
                </div>
              ) : null}
              {providerView === "modelAggregation" && !visibleProfiles.length ? (
                <div className="provider-aggregate-empty">
                  <strong>{t("还没有模型聚合配置")}</strong>
                  <span>{t("把成员模型合并到 Codex 列表，选中模型会固定发送到对应供应商。")}</span>
                  <Button onClick={() => createNewAggregateProfile("model")}>{t("创建模型聚合配置")}</Button>
                </div>
              ) : null}
              <RelayProfileList
                form={normalized}
                visibleProfiles={visibleProfiles}
                onEdit={(profileId) => void editRelayProfile(profileId)}
                onFormChange={saveRelaySettings}
                onSwitchProfile={switchRelayProfileFromList}
                disabled={actions.relaySwitching}
                actions={actions}
              />
            </>
          ) : null}
          </CardContent>
        </Panel>
      </div>
    </div>
  );
}

function multiLaunchSupported(profile: RelayProfile): boolean {
  return !isAggregateRelayProfile(profile)
    && (profile.protocol === "responses" || profile.protocol === "chatCompletions")
    && !(profile.modelRoutes ?? []).some((route) => route.model.trim() || route.targetRelayId.trim());
}

function CodexMultiLaunchScreen({
  form,
  actions,
}: {
  form: BackendSettings;
  actions: Actions;
}) {
  const normalized = normalizeSettings(form);
  return (
    <div className="mg-page relay-management-page">
      <PageHeader
        icon={<Layers3 className="h-5 w-5" />}
        title={t("Codex多开")}
        actions={(
          <Button onClick={() => void actions.refreshSettings(true)} variant="secondary">
            <RefreshCw className="h-4 w-4" />
            {t("刷新供应商")}
          </Button>
        )}
      />
      <div className="mg-content">
        <Panel className="relay-management-panel multi-launch-running-panel">
          <CardHeader>
            <div className="multi-launch-running-header">
              <div>
                <CardTitle>{t("当前运行实例")}</CardTitle>
                <CardDescription>{t("这里列出主 Codex 和已经启动的多开实例，可单独重启或结束。")}</CardDescription>
              </div>
              <Button
                disabled={actions.multiLaunchInstancesLoading}
                onClick={() => void actions.refreshMultiLaunchInstances(false)}
                variant="secondary"
                size="sm"
              >
                <RefreshCw className={`h-4 w-4 ${actions.multiLaunchInstancesLoading ? "animate-spin" : ""}`} />
                {t("刷新")}
              </Button>
            </div>
          </CardHeader>
          <CardContent>
            {actions.multiLaunchInstances.length ? (
              <div className="multi-launch-instance-list">
                {actions.multiLaunchInstances.map((instance) => {
                  const status = multiLaunchInstanceStatus(instance);
                  const running = multiLaunchInstanceRunning(instance);
                  const pending = actions.multiLaunchActionPending === instance.instanceId;
                  const label = formatMultiLaunchInstanceLabel(instance, { main: t("主 Codex"), fallback: t("Codex 实例") });
                  // 启动失败的原因必须让用户看得见：Windows 上多开曾经完全不吭声。
                  const failure = multiLaunchInstanceFailure(instance);
                  const detail = [
                    instance.isMain ? t("主进程") : instance.instanceId,
                    typeof instance.pid === "number" ? `PID ${instance.pid}` : "",
                    typeof instance.debugPort === "number" ? `CDP ${instance.debugPort}` : "",
                    typeof instance.helperPort === "number" ? `API ${instance.helperPort}` : "",
                    failure ? `${t("启动失败")}：${failure}` : "",
                  ].filter(Boolean).join(" · ");
                  return (
                    <div className="multi-launch-instance-card" data-instance-id={instance.instanceId} key={instance.instanceId}>
                      <span className="relay-index" title={label}>{providerInitial(label)}</span>
                      <span className="relay-summary">
                        <strong>{label}</strong>
                        <small data-failed={failure ? "true" : undefined}>{detail || t("未提供进程详情")}</small>
                      </span>
                      <UiBadge className={`multi-launch-instance-status ${statusClass(status)}`} variant="secondary">{statusLabel(status)}</UiBadge>
                      <span className="relay-card-actions">
                        <Button
                          disabled={pending}
                          onClick={() => void actions.restartMultiLaunchInstance(instance)}
                          size="sm"
                          title={t("重启此 Codex 实例")}
                        >
                          <RotateCcw className="h-4 w-4" />
                          {pending ? t("处理中") : t("重启")}
                        </Button>
                        <Button
                          disabled={!running || pending}
                          onClick={() => void actions.stopMultiLaunchInstance(instance)}
                          size="sm"
                          title={running ? t("结束此 Codex 实例") : t("实例当前未运行")}
                          variant="secondary"
                        >
                          <PowerOff className="h-4 w-4" />
                          {t("结束")}
                        </Button>
                      </span>
                    </div>
                  );
                })}
              </div>
            ) : (
              <div className="empty">{actions.multiLaunchInstancesLoading ? t("正在读取运行实例…") : t("当前没有已发现的 Codex 实例。")}</div>
            )}
          </CardContent>
        </Panel>
        <Panel className="relay-management-panel">
          <CardHeader>
            <CardTitle>{t("选择供应商身份启动 Codex")}</CardTitle>
            <CardDescription>{t("每个身份使用独立的配置、会话和用户数据目录，可与其他单一 Responses 或 Chat Completions 供应商同时运行。")}</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="relay-profile-list">
              {normalized.relayProfiles.map((profile) => {
                const aggregate = isAggregateRelayProfile(profile);
                const unsupported = !multiLaunchSupported(profile);
                const pending = actions.multiLaunchPending === profile.id;
                return (
                  <div className="relay-profile-card multi-launch-card" data-relay-profile-id={profile.id} key={profile.id}>
                    <span className="relay-index" title={profile.name || t("未命名供应商")}>{providerInitial(profile.name)}</span>
                    <span className="relay-summary">
                      <strong>{profile.name || t("未命名供应商")}</strong>
                      <small>{relayProfileModeLabel(profile)} · {relayProtocolLabel(profile.protocol)} · {relayProfileConfigBrief(profile)}</small>
                    </span>
                    <span className="relay-card-actions">
                      <Button
                        disabled={unsupported || pending}
                        onClick={() => void actions.launchMultiInstance(profile)}
                        title={aggregate
                          ? t("聚合供应商暂不支持独立多开")
                          : unsupported
                            ? t("仅支持单一 Responses 或 Chat Completions 供应商，且不能配置模型路由")
                            : t("启动此供应商的独立 Codex 实例")}
                        size="sm"
                      >
                        <Rocket className="h-4 w-4" />
                        {pending ? t("启动中") : t("启动")}
                      </Button>
                    </span>
                  </div>
                );
              })}
            </div>
            {!normalized.relayProfiles.length ? <div className="empty">{t("暂无供应商，请先添加供应商配置。")}</div> : null}
          </CardContent>
        </Panel>
      </div>
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
  const filteredPresets = PRESETS.filter((preset) => {
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

function envConflictSourceLabel(source: string): string {
  if (source === "process") return t("当前进程");
  if (source === "user") return t("用户环境");
  return source || t("环境变量");
}

function agentPetValidationMessage(issue: AgentPetIssue | null): string | null {
  if (issue === "customUrl") return t("自定义图片地址无效，请填写 https:// 开头的图片地址。");
  return null;
}

type DictationLocalModelState = {
  phase: string;
  modelReady: boolean;
  completedBytes?: number;
  totalBytes: number;
  message?: string;
};

type GlobalDictationHotkeyState = {
  enabled: boolean;
  hotkey: string;
};
// 后端失败分支的 payload 是 `{}`：只在确实带有热键字段时才当作新状态。
function hotkeyStateFrom(result: Partial<GlobalDictationHotkeyState> | null | undefined): GlobalDictationHotkeyState | null {
  if (!result || typeof result.hotkey !== "string") return null;
  return { enabled: result.enabled === true, hotkey: result.hotkey };
}

function DictationSettingsPanel({ form, onFormChange }: {
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
}) {
  const [dictationPresetSelection, setDictationPresetSelection] = useState<DictationPreset>(() => dictationPreset(form.dictation));
  useEffect(() => setDictationPresetSelection(dictationPreset(form.dictation)), [form.dictation.baseUrl]);
  const [localModelState, setLocalModelState] = useState<DictationLocalModelState | null>(null);
  const [localModelDownloading, setLocalModelDownloading] = useState(false);
  const [localModelError, setLocalModelError] = useState("");
  const [globalHotkey, setGlobalHotkey] = useState<GlobalDictationHotkeyState>({ enabled: false, hotkey: "Ctrl+Shift+Space" });
  const [globalHotkeyBusy, setGlobalHotkeyBusy] = useState(false);
  useEffect(() => {
    void invoke<CommandResult<GlobalDictationHotkeyState>>("load_global_dictation_hotkey")
      .then((result) => { const next = hotkeyStateFrom(result); if (next) setGlobalHotkey(next); })
      .catch(() => {});
  }, []);
  useEffect(() => {
    if (form.dictation.provider !== "sensevoice") return;
    let active = true;
    const refresh = () => invoke<DictationLocalModelState>("dictation_local_model_status")
      .then((state) => { if (active) { setLocalModelState(state); } })
      .catch(() => { if (active) setLocalModelError(t("无法读取本地模型状态，请重试。")); });
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 1000);
    return () => { active = false; window.clearInterval(timer); };
  }, [form.dictation.provider]);
  const downloadLocalModel = async () => {
    setLocalModelDownloading(true);
    setLocalModelError("");
    try {
      const result = await invoke<CommandResult<DictationLocalModelState>>("download_dictation_local_model");
      if (result.status !== "ok") setLocalModelError(result.message);
      else setLocalModelState(result);
    } catch {
      setLocalModelError(t("本地模型下载失败，请重试。"));
    } finally {
      setLocalModelDownloading(false);
    }
  };
  const downloading = localModelDownloading || localModelState?.phase === "downloading";
  const saveGlobalHotkey = async () => {
    setGlobalHotkeyBusy(true);
    try {
      const result = await invoke<CommandResult<GlobalDictationHotkeyState>>("set_global_dictation_hotkey", globalHotkey);
      // 校验/保存失败时后端 payload 为空对象，不能用它覆盖用户刚输入的热键。
      const next = hotkeyStateFrom(result);
      if (next) setGlobalHotkey(next);
      await appAlert(result.message, { title: t("全局听写快捷键") });
    } catch (error) {
      await appAlert(String(error), { title: t("全局听写快捷键") });
    } finally {
      setGlobalHotkeyBusy(false);
    }
  };
  const updateDictation = (patch: Partial<DictationSettings>) => onFormChange({
    ...form,
    dictation: { ...form.dictation, ...patch },
  });
  const dictationIssue = dictationSettingsValidationMessage(dictationSettingsIssue(form.dictation));
  return (
    <Panel className="enhance-service-panel">
      <CardHead title={t("语音输入")} detail={t("独立的语音转文字服务，与代码模型和供应商配置无关。")} />
      <CardContent className="settings-content">
        <div className="settings-block enhance-service-block" id={ENHANCEMENT_SECTION_IDS.dictation}>
          <FeatureToggle
            title={t("启用语音输入")}
            detail={t("停止录音后，可将转写文字插入或发送到当前会话。")}
            disabled={!form.enhancementsEnabled}
            checked={form.dictation.enabled}
            onChange={(enabled) => updateDictation({ enabled })}
          />
          <label className="switch-row compact">
            <input
              checked={globalHotkey.enabled}
              disabled={globalHotkeyBusy || !form.enhancementsEnabled}
              onChange={(event) => { const enabled = event.currentTarget.checked; setGlobalHotkey((current) => ({ ...current, enabled })); }}
              type="checkbox"
            />
            <span><strong>{t("全局听写热键")}</strong><small>{t("Windows 首版：按一次开始录音，再按一次停止并插入文字。")}</small></span>
            <ToggleVisual />
          </label>
          <div className="form-row">
            <Field label={t("全局热键") }>
              <Input value={globalHotkey.hotkey} onChange={(event) => { const hotkey = event.currentTarget.value; setGlobalHotkey((current) => ({ ...current, hotkey })); }} placeholder="Ctrl+Shift+Space" />
            </Field>
            <Button disabled={globalHotkeyBusy} onClick={() => void saveGlobalHotkey()} type="button" variant="secondary">{t("保存全局热键")}</Button>
          </div>
          <p className="field-hint">{t("热键冲突会在保存时明确报告；macOS 首版暂不注册系统级热键。")}</p>
          <div className="form-row">
            <Field label={t("语音服务预设")}>
              <AppSelect
                value={dictationPresetSelection}
                onChange={(value) => {
                  setDictationPresetSelection(value);
                  onFormChange({ ...form, dictation: applyDictationPreset(form.dictation, value) });
                }}
                options={[
                  { value: "sensevoice", label: t("本地 SenseVoice（离线）") },
                  { value: "groq", label: "Groq" },
                  { value: "openai", label: t("OpenAI 兼容") },
                  { value: "local", label: t("本地服务") },
                  { value: "custom", label: t("自定义") },
                ]}
              />
            </Field>
            <Field label={t("语音模型")}>
              <Input
                value={form.dictation.model}
                onChange={(event) => updateDictation({ model: event.currentTarget.value })}
                placeholder="whisper-large-v3-turbo"
                spellCheck={false}
              />
            </Field>
          </div>
          {form.dictation.provider === "sensevoice" ? <>
            <p className="field-hint">{t("SenseVoiceSmall 在本机 CPU 上运行。请手动下载约 229 MiB 模型，下载并校验完成后才可录音转写；音频不会离开本机。")}</p>
            <div className="form-row">
              <Button type="button" disabled={downloading} onClick={() => { void downloadLocalModel(); }}>
                <Download className="h-4 w-4" />
                {downloading ? t("正在下载并校验…") : localModelState?.modelReady ? t("校验或重新下载模型") : t("下载本地模型")}
              </Button>
              <span className="field-hint" role="status">
                {downloading
                  ? `${Math.min(100, Math.floor((localModelState?.completedBytes || 0) / (localModelState?.totalBytes || 239549735) * 100))}%`
                  : localModelState?.modelReady ? t("本地模型已下载") : t("本地模型尚未下载")}
              </span>
            </div>
            {localModelError || localModelState?.phase === "failed" ? <p className="field-hint" role="alert">{localModelError || localModelState?.message}</p> : null}
          </> : <>
            <Field label="Base URL">
              <Input
                value={form.dictation.baseUrl}
                onChange={(event) => updateDictation({ baseUrl: event.currentTarget.value })}
                placeholder="https://api.groq.com/openai/v1"
                spellCheck={false}
              />
            </Field>
            <p className="field-hint">{t("填写服务的 API 基础地址，转写请求会发送到 /audio/transcriptions。可修改预设地址和模型。")}</p>
            <Field label="API Key">
              <Input
                type="password"
                autoComplete="off"
                value={form.dictation.apiKey}
                onChange={(event) => updateDictation({ apiKey: event.currentTarget.value })}
              />
            </Field>
            <p className="field-hint">{t("可填写 API Key 或环境变量名称。本地服务不要求认证时可留空。")}</p>
          </>}
          <details className="stepwise-advanced enhance-service-advanced">
            <summary>{t("高级参数")}</summary>
            <Field label={t("API Key 环境变量")}>
              <Input
                value={form.dictation.apiKeyEnv}
                onChange={(event) => updateDictation({ apiKeyEnv: event.currentTarget.value })}
                placeholder="GROQ_API_KEY"
                spellCheck={false}
              />
            </Field>
            <div className="form-row">
              <Field label={t("语音语言")}>
                <Input
                  value={form.dictation.language}
                  onChange={(event) => updateDictation({ language: event.currentTarget.value })}
                  placeholder={t("留空自动识别，例如 zh 或 en")}
                  spellCheck={false}
                />
              </Field>
              <Field label={t("转写超时（秒）")}>
                <Input
                  type="number"
                  min={1}
                  max={600}
                  value={form.dictation.timeoutSeconds}
                  onChange={(event) => updateDictation({ timeoutSeconds: clampNumber(Number(event.currentTarget.value), 1, 600) })}
                />
              </Field>
            </div>
          </details>
          {dictationIssue ? <p className="field-hint" role="alert">{dictationIssue}</p> : null}
          <div className="hint-line enhance-service-note">
            <Info className="h-4 w-4" />
            <span>{form.dictation.provider === "sensevoice"
              ? t("下载模型并保存配置后，可在 Codex 输入框录音，停止后在本机转写并插入文字。")
              : t("开启后 Codex 输入框底部会显示麦克风图标，停止录音后可插入或发送文字。录音会发送到上面填写的服务地址，无需重启。")}</span>
          </div>
        </div>
      </CardContent>
    </Panel>
  );
}

function StepwiseSettingsPanel({ form, onFormChange, actions }: {
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const group = enhancementGroup("stepwise");
  return (
    <Panel className="enhance-service-panel">
      <CardHead title={t(group.title)} detail={t(group.description ?? "")} />
      <CardContent className="settings-content">
        <div className="settings-block stepwise-settings-block" id={ENHANCEMENT_SECTION_IDS.stepwise}>
          <div className="enhance-service-toggles">
            {managerGroupItems(group.id, enhancementPlatform).map((item) => (
              <EnhancementSettingControl key={item.key} item={item} form={form} onFormChange={onFormChange} masterEnabled={form.enhancementsEnabled} />
            ))}
          </div>
          <div className="stepwise-settings-section">{t("连接")}</div>
          <div className="form-row">
            <Field label="Base URL">
              <Input
                value={form.codexAppStepwiseBaseUrl}
                onChange={(event) => onFormChange({ ...form, codexAppStepwiseBaseUrl: event.currentTarget.value })}
                placeholder="https://api.example.com/v1"
              />
            </Field>
            <Field label="Model">
              <Input
                value={form.codexAppStepwiseModel}
                onChange={(event) => onFormChange({ ...form, codexAppStepwiseModel: event.currentTarget.value })}
                placeholder={t("例如 gpt-5.4-mini")}
              />
            </Field>
          </div>
          <div className="form-row">
            <Field label={t("协议")}>
              <AppSelect
                value={form.codexAppStepwiseProtocol}
                onChange={(value) => onFormChange({ ...form, codexAppStepwiseProtocol: value })}
                options={[
                  { value: "auto", label: t("自动兼容") },
                  { value: "chat_completions", label: "Chat Completions" },
                  { value: "responses", label: "Responses API" },
                  { value: "anthropic_messages", label: "Anthropic Messages" },
                ]}
              />
            </Field>
            <Field label={t("模式")}>
              <AppSelect
                value={form.codexAppStepwiseGenerationMode}
                onChange={(value) => onFormChange({ ...form, codexAppStepwiseGenerationMode: value })}
                options={[
                  { value: "auto", label: t("自动生成") },
                  { value: "manual", label: t("手动刷新") },
                ]}
              />
            </Field>
          </div>
          <Field label="API Key">
            <Input
              type="password"
              value={form.codexAppStepwiseApiKey}
              onChange={(event) => onFormChange({ ...form, codexAppStepwiseApiKey: event.currentTarget.value })}
            />
          </Field>
          <details className="stepwise-advanced">
            <summary>{t("高级参数")}</summary>
            <div className="form-row">
              <Field label={t("API Key 环境变量")}>
                <Input
                  value={form.codexAppStepwiseApiKeyEnv}
                  onChange={(event) => onFormChange({ ...form, codexAppStepwiseApiKeyEnv: event.currentTarget.value })}
                />
              </Field>
              <Field label={t("最多建议数")}>
                <Input
                  max={6}
                  min={0}
                  type="number"
                  value={form.codexAppStepwiseMaxItems}
                  onChange={(event) =>
                    onFormChange({ ...form, codexAppStepwiseMaxItems: clampNumber(Number(event.currentTarget.value), 0, 6) })
                  }
                />
              </Field>
            </div>
            <div className="form-row">
              <Field label={t("超时毫秒")}>
                <Input
                  min={1000}
                  type="number"
                  value={form.codexAppStepwiseTimeoutMs}
                  onChange={(event) =>
                    onFormChange({ ...form, codexAppStepwiseTimeoutMs: clampNumber(Number(event.currentTarget.value), 1000, 60000) })
                  }
                />
              </Field>
              <Field label={t("最大输入字符")}>
                <Input
                  min={1000}
                  type="number"
                  value={form.codexAppStepwiseMaxInputChars}
                  onChange={(event) =>
                    onFormChange({ ...form, codexAppStepwiseMaxInputChars: clampNumber(Number(event.currentTarget.value), 1000, 24000) })
                  }
                />
              </Field>
            </div>
            <Field label={t("最大输出 tokens")}>
              <Input
                min={100}
                type="number"
                value={form.codexAppStepwiseMaxOutputTokens}
                onChange={(event) =>
                  onFormChange({ ...form, codexAppStepwiseMaxOutputTokens: clampNumber(Number(event.currentTarget.value), 100, 4000) })
                }
              />
            </Field>
          </details>
          <div className="toolbar stepwise-settings-actions">
            <Button variant="secondary" onClick={() => void actions.testStepwiseSettings(form)}>{t("测试连接")}</Button>
          </div>
        </div>
      </CardContent>
    </Panel>
  );
}

/** 增强页改动后多久自动保存：开关点完马上存，文本框停止输入后再存。 */
const ENHANCE_AUTO_SAVE_DELAY_MS = 500;

type AutoSaveState =
  | { state: "idle" | "pending" | "saving" | "saved" }
  | { state: "invalid" | "failed"; message: string; retry: () => void };

/// 增强页的自动保存。
///
/// - 防抖：同一轮连续改动只存一次；
/// - 同一时间只有一个保存在途，期间的改动等它回来后再存（App 会同步版本号并触发下一轮）；
/// - 离开页面时还有没存的改动就立即补存一次，不丢草稿。
function useEnhanceAutoSave(
  dirty: boolean,
  form: BackendSettings,
  save: (settings: BackendSettings) => Promise<AutoSaveOutcome>,
): AutoSaveState {
  const [status, setStatus] = useState<AutoSaveState>({ state: "idle" });
  const savingRef = useRef(false);
  const latest = useRef({ dirty, form, save });
  latest.current = { dirty, form, save };

  const flush = useCallback(async () => {
    if (savingRef.current) return;
    const { dirty: pending, form: snapshot, save: persist } = latest.current;
    if (!pending) return;
    savingRef.current = true;
    setStatus({ state: "saving" });
    const outcome = await persist(snapshot);
    savingRef.current = false;
    setStatus(outcome.status === "saved"
      ? { state: "saved" }
      : { state: outcome.status, message: outcome.message, retry: () => void flush() });
  }, []);

  useEffect(() => {
    if (!dirty) return;
    setStatus((current) => (current.state === "saving" || current.state === "invalid" ? current : { state: "pending" }));
    const timer = window.setTimeout(() => void flush(), ENHANCE_AUTO_SAVE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [dirty, form, flush]);

  useEffect(() => () => {
    if (latest.current.dirty && !savingRef.current) void latest.current.save(latest.current.form);
  }, []);

  return status;
}

function AutoSaveStatus({ state }: { state: AutoSaveState }) {
  if (state.state === "invalid" || state.state === "failed") {
    return (
      <>
        <span className="is-error" role="alert" title={state.message}>
          {state.state === "invalid" ? tf("未保存：{0}", [t(state.message)]) : tf("自动保存失败：{0}", [t(state.message)])}
        </span>
        {state.state === "failed" ? (
          <Button onClick={state.retry} size="sm" variant="secondary">
            <RefreshCw className="h-4 w-4" />
            {t("重试")}
          </Button>
        ) : null}
      </>
    );
  }
  const label = state.state === "saving" ? t("正在保存…")
    : state.state === "pending" ? t("即将自动保存…")
      : t("改动会自动保存");
  return (
    <span className={state.state === "saved" || state.state === "idle" ? "" : "is-dirty"} role="status">
      {state.state === "saved" ? <CheckCircle2 className="h-3.5 w-3.5" /> : null}
      {state.state === "saved" ? t("已自动保存") : label}
    </span>
  );
}

/// 设置表单的自动保存：改动后稍等片刻写入，替代原来的「保存设置」按钮。
/// 做成组件而不是在各页面里直接调用 hook，方便在列表、面板等位置按需插入。
function SettingsAutoSave({ dirty, form, actions, className = "" }: {
  dirty: boolean;
  form: BackendSettings;
  actions: Actions;
  className?: string;
}) {
  const state = useEnhanceAutoSave(dirty, form, actions.autoSaveSettings);
  return (
    <div className={`enhance-save-actions settings-autosave ${className}`}>
      <AutoSaveStatus state={state} />
    </div>
  );
}

function EnhanceScreen({
  dirty,
  activeTab,
  onTabChange,
  form,
  onFormChange,
  actions,
}: {
  dirty: boolean;
  activeTab: EnhancementTab;
  onTabChange: (tab: EnhancementTab) => void;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const setEnhanceFlag = (key: keyof BackendSettings, value: boolean) => onFormChange({ ...form, [key]: value });
  const autoSave = useEnhanceAutoSave(dirty, form, actions.autoSaveSettings);
  const masterEnabled = form.enhancementsEnabled;
  const masterItem = enhancementItem("enhancementsEnabled");
  // 开关的标题、说明、分组都来自设置项清单；这里只保留一处定制：
  // Windows 原生浏览器兼容开启前要确认，并在开关后面跟一块补丁状态。
  const renderSetting = (item: EnhancementSettingItem) => {
    if (item.key === "codexAppNativeBrowserRequireIdentification") {
      return (
        <Fragment key={item.key}>
          <EnhancementSettingControl
            item={item}
            form={form}
            onFormChange={onFormChange}
            masterEnabled={masterEnabled}
            onToggle={(value) => void (async () => {
              if (value && !(await appConfirm(nativeBrowserConsent, { title: t(item.title), confirmText: t("开启") }))) return;
              setEnhanceFlag("codexAppNativeBrowserRequireIdentification", value);
            })()}
          />
          <NativeBrowserStatusView />
        </Fragment>
      );
    }
    return <EnhancementSettingControl key={item.key} item={item} form={form} onFormChange={onFormChange} masterEnabled={masterEnabled} />;
  };
  const tabs: Array<{ value: EnhancementTab; label: string }> = [
    { value: "general", label: t("常用增强") },
    { value: "dictation", label: t("语音输入") },
    { value: "stepwise", label: t(enhancementGroup("stepwise").title) },
  ];
  const selectTab = (tab: EnhancementTab) => {
    onTabChange(tab);
    document.getElementById("enhance-page-controls")?.scrollIntoView({ block: "start" });
  };
  // 「常用增强」页里的分组，显示在左侧目录的页签下面，点了滚到对应分组。
  // 分组按清单顺序列出，独立模式排最后；锚点 id 由清单分组 id 生成，保持稳定。
  const generalSections = [...managerGeneralGroups(), managerStandaloneGroup()]
    .map((group) => ({ id: enhancementGroupAnchorId(group), label: t(group.title) }));
  return (
    <div className="settings-page enhance-page sectioned-page">
      <PageSectionNav label={t("增强分类")} sections={activeTab === "general" ? generalSections : []}>
        <div className="enhance-tabs" role="tablist" aria-label={t("Codex增强")} aria-orientation="vertical">
          {tabs.map((tab, index) => (
            <button
              key={tab.value}
              id={`enhance-tab-${tab.value}`}
              type="button"
              role="tab"
              aria-selected={activeTab === tab.value}
              aria-controls={`enhance-pane-${tab.value}`}
              tabIndex={activeTab === tab.value ? 0 : -1}
              onClick={() => selectTab(tab.value)}
              onKeyDown={(event) => {
                const forward = event.key === "ArrowRight" || event.key === "ArrowDown";
                const backward = event.key === "ArrowLeft" || event.key === "ArrowUp";
                const nextIndex = forward ? (index + 1) % tabs.length
                  : backward ? (index + tabs.length - 1) % tabs.length
                    : event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : null;
                if (nextIndex === null) return;
                event.preventDefault();
                const nextTab = tabs[nextIndex].value;
                selectTab(nextTab);
                document.getElementById(`enhance-tab-${nextTab}`)?.focus();
              }}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </PageSectionNav>
      <div className="sectioned-body">
      {/* 总开关一条细横幅，开关紧挨着说明；保存状态在横幅右侧。 */}
      <div className="enhance-master-bar" id="enhance-page-controls">
        <label className="enhance-master-switch">
          <input
            checked={form.enhancementsEnabled}
            onChange={(event) => onFormChange({ ...form, enhancementsEnabled: event.currentTarget.checked })}
            type="checkbox"
          />
          <ToggleVisual />
          <span>
            <strong>{t(masterItem.title)}</strong>
            <small>{t(masterItem.description)}</small>
          </span>
        </label>
        <div className="enhance-save-actions">
          <AutoSaveStatus state={autoSave} />
        </div>
      </div>
      <div className="enhance-tab-panel" id="enhance-pane-general" role="tabpanel" aria-labelledby="enhance-tab-general" hidden={activeTab !== "general"}>
        <Panel className="enhance-panel">
          <CardContent className="enhance-content">
            <div className="enhance-feature-groups">
              {/* 关闭增强时这组是唯一仍会生效的补丁，因此提到最前面；开启增强时它排在最后。 */}
              {masterEnabled ? null : (
                <StandaloneModeGroup form={form} onFormChange={onFormChange} masterEnabled={masterEnabled} />
              )}
              {managerGeneralGroups().map((group) => (
                <FeatureGroup key={group.id} id={enhancementGroupAnchorId(group)} title={t(group.title)} detail={t(group.description ?? "")}>
                  {managerGroupItems(group.id, enhancementPlatform).map(renderSetting)}
                </FeatureGroup>
              ))}
              {masterEnabled ? (
                <StandaloneModeGroup form={form} onFormChange={onFormChange} masterEnabled={masterEnabled} />
              ) : null}
            </div>
            <div className="hint-line enhance-footer-hint">
              <Info className="h-4 w-4" />
              <span>{t("如果使用官方模式或官方混入 API 模式，通常不需要开启插件市场解锁。")}</span>
            </div>
          </CardContent>
        </Panel>
      </div>
      <div className="enhance-tab-panel" id="enhance-pane-dictation" role="tabpanel" aria-labelledby="enhance-tab-dictation" hidden={activeTab !== "dictation"}>
        <DictationSettingsPanel form={form} onFormChange={onFormChange} />
      </div>
      <div className="enhance-tab-panel" id="enhance-pane-stepwise" role="tabpanel" aria-labelledby="enhance-tab-stepwise" hidden={activeTab !== "stepwise"}>
        <StepwiseSettingsPanel form={form} onFormChange={onFormChange} actions={actions} />
      </div>
      </div>
    </div>
  );
}

// 增强总开关关闭后仍生效的独立补丁。不受总开关禁用，所以不放进 FeatureGroup 的
// masterEnabled 门控里；位置在该页「常用增强」标签内，关闭增强时提到最前面。
function StandaloneModeGroup({
  form,
  onFormChange,
  masterEnabled,
}: {
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  masterEnabled: boolean;
}) {
  const group = managerStandaloneGroup();
  return (
    <FeatureGroup id={enhancementGroupAnchorId(group)} title={t(group.title)} detail={t(group.description ?? "")}>
      {managerGroupItems(group.id, enhancementPlatform).map((item) => (
        <EnhancementSettingControl key={item.key} item={item} form={form} onFormChange={onFormChange} masterEnabled={masterEnabled} />
      ))}
    </FeatureGroup>
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
          <small>{detail}</small>
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

function DreamSkinScreen({
  form,
  launchPending,
  library,
  libraryLoading,
  market,
  marketLoading,
  community,
  communityLoading,
  installOperation,
  draft,
  dirty,
  pendingRestart,
  selectedTheme,
  status,
  verification,
  onFormChange,
  onDraftChange,
  actions,
}: {
  form: BackendSettings;
  launchPending: boolean;
  library: DreamSkinThemeLibrary | null;
  libraryLoading: boolean;
  market: DreamSkinMarketResult | null;
  marketLoading: boolean;
  community: DreamSkinCommunityResult | null;
  communityLoading: boolean;
  installOperation: DreamSkinInstallOperation | null;
  draft: DreamSkinThemeDraft | null;
  dirty: boolean;
  pendingRestart: PendingDreamSkinRestart | null;
  selectedTheme: string;
  status: DreamSkinRuntimeResult | null;
  verification: DreamSkinVerificationResult | null;
  onFormChange: (value: BackendSettings) => void;
  onDraftChange: (value: DreamSkinThemeDraft | null) => void;
  actions: Actions;
}) {
  const [themeView, setThemeView] = useState<"market" | "community" | "local">("local");
  const [localQuery, setLocalQuery] = useState("");
  const companionInputRef = useRef<HTMLInputElement>(null);
  const [companionError, setCompanionError] = useState("");
  const masterEnabled = form.enhancementsEnabled;
  const theme = draft?.config ?? defaultDreamSkinTheme();
  const themeColors = theme.colors ?? defaultDreamSkinColors();
  const customImagePath = draft?.imagePath.trim() ?? "";
  const previewUrl = customImagePath
    ? convertFileSrc(customImagePath)
    : isWindowsPlatform
      ? dreamSkinWindowsPreviewUrl
      : dreamSkinMacPreviewUrl;
  const selectedItem = library?.themes.find((item) => item.key === selectedTheme) ?? null;
  const savedThemeSelected = selectedItem?.kind === "stored";
  const updateTheme = (next: DreamSkinThemeConfig) => {
    if (draft) onDraftChange({ ...draft, config: next });
  };
  const updateThemeText = (
    key: "id" | "name" | "brandSubtitle" | "tagline" | "projectPrefix" | "projectLabel" | "statusText" | "quote",
    value: string,
  ) => updateTheme({ ...theme, [key]: value });
  const updateThemeColor = (key: keyof DreamSkinColors, value: string) => {
    updateTheme({ ...theme, colors: { ...themeColors, [key]: value } });
  };
  const themeAppearance = theme.appearance === "light" || theme.appearance === "dark"
    ? theme.appearance
    : "auto";
  const windowsAccent = typeof theme.palette?.accent === "string" ? theme.palette.accent : "";
  const updateWindowsAccent = (value: string) => {
    const palette = { ...(theme.palette ?? {}) };
    if (value.trim()) palette.accent = value;
    else delete palette.accent;
    const next: DreamSkinThemeConfig = { ...theme, palette };
    if (!Object.keys(palette).length) delete next.palette;
    updateTheme(next);
  };
  const companion = theme.companion;
  const companionDataUrl = typeof companion?.dataUrl === "string" ? companion.dataUrl : "";
  const companionEnabled = Boolean(companionDataUrl) && companion?.enabled !== false;
  const updateCompanion = (patch: Partial<NonNullable<DreamSkinThemeConfig["companion"]>>) => {
    const nextCompanion = {
      dataUrl: companionDataUrl,
      enabled: companion?.enabled ?? true,
      width: companion?.width ?? 96,
      side: companion?.side ?? "right",
      offsetX: companion?.offsetX ?? 0,
      offsetY: companion?.offsetY ?? 4,
      ...patch,
    };
    updateTheme({ ...theme, companion: nextCompanion });
  };
  const clearCompanion = () => {
    const next = { ...theme };
    delete next.companion;
    setCompanionError("");
    updateTheme(next);
  };
  const chooseCompanion = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;
    if (!dreamSkinCompanionMimeTypes.has(file.type)) {
      setCompanionError(t("仅支持 PNG、JPEG、WebP 或 GIF 图片"));
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = typeof reader.result === "string" ? reader.result : "";
      if (!dataUrl || dataUrl.length > dreamSkinCompanionDataUrlLimit) {
        setCompanionError(t("图片过大，请选择 180 KB 以内的图片"));
        return;
      }
      setCompanionError("");
      updateCompanion({ dataUrl });
    };
    reader.onerror = () => setCompanionError(t("读取图片失败，请重新选择"));
    reader.readAsDataURL(file);
  };
  const currentItem = library?.themes.find((item) => pendingRestart
    ? item.key === pendingRestart.currentThemeKey
    : item.active);
  const currentThemeName = pendingRestart?.currentThemeName || currentItem?.name || t("未记录");
  const currentThemeLabel = status?.liveApplied
    ? t("当前运行")
    : pendingRestart ? t("此前配置") : t("当前配置");
  const stateLabel = !masterEnabled
    ? t("增强总开关已关闭")
    : !form.codexAppDreamSkinEnabled
      ? t("皮肤未启用")
      : form.codexAppDreamSkinPaused || status?.paused
        ? t("已恢复原始外观")
        : status ? dreamSkinStateLabel(status.state) : t("等待状态检查");
  const stateTone = !masterEnabled || !form.codexAppDreamSkinEnabled || form.codexAppDreamSkinPaused || status?.paused
    ? "disabled"
    : status?.state === "pass" ? "pass" : status?.state === "fail" ? "fail" : status ? "warning" : "unchecked";
  const localThemes = (library?.themes ?? []).filter((item) =>
    !localQuery.trim() || item.name.toLocaleLowerCase().includes(localQuery.trim().toLocaleLowerCase()),
  );
  // 数据未就绪或正在刷新时，tab 计数显示转圈而不是 0，避免被误读成空列表。
  const libraryPending = libraryLoading || !library;
  const marketPending = marketLoading || !market;
  const communityPending = communityLoading || !community;
  const libraryCount = library?.themes.length ?? 0;
  const marketCount = market?.themes.length ?? 0;
  const communityCount = community?.items.length ?? 0;
  const runtimeChecks = status?.checks ?? [];
  const verificationChecks = verification?.checks ?? [];

  return (
    <div className="dream-skin-page">
      <Panel className="dream-skin-panel dream-skin-overview">
        <CardContent className="dream-skin-overview-content">
          <div className="dream-skin-overview-heading">
            <div className="dream-skin-current-copy">
              <small>{currentThemeLabel}</small>
              <strong title={currentThemeName}>{currentThemeName}</strong>
              <span className={`dream-skin-status-pill is-${stateTone}`} role="status">{stateLabel}</span>
            </div>
            <div className="dream-skin-overview-controls">
            <label className="switch-row compact dream-skin-enable-switch">
              <input
                checked={form.codexAppDreamSkinEnabled}
                disabled={!masterEnabled}
                onChange={(event) => onFormChange({
                  ...form,
                  codexAppDreamSkinEnabled: event.currentTarget.checked,
                  codexAppDreamSkinPaused: false,
                })}
                type="checkbox"
              />
              <span>
                <strong>{t("启用 Codex 皮肤")}</strong>
              </span>
              <ToggleVisual />
            </label>
              <Button aria-label={t("刷新状态")} onClick={() => void actions.refreshDreamSkinStatus()} size="icon" title={t("刷新状态")} type="button" variant="ghost">
                <RefreshCw className="h-4 w-4" />
              </Button>
            </div>
          </div>
          <div className="dream-skin-selected-summary">
            <img alt={t("Dream Skin 图片预览")} src={previewUrl} />
            <div className="dream-skin-selected-copy">
              <small>{t("正在编辑")}</small>
              <strong title={theme.name}>{theme.name || t("未命名主题")}</strong>
              <span className={dirty ? "is-dirty" : ""}>{dirty ? t("有未保存修改") : savedThemeSelected ? t("已保存主题") : draft?.builtin ? t("内置主题") : t("当前未保存主题")}</span>
            </div>
            <div className="dream-skin-selection-actions">
              <Button disabled={!draft} onClick={() => void actions.saveDreamSkinTheme()} type="button" variant="secondary">
                <Save className="h-4 w-4" />
                {draft?.builtin || selectedItem?.kind === "activeUnsaved" ? t("保存为新主题") : t("保存主题")}
              </Button>
              <Button disabled={!masterEnabled || !draft} onClick={() => void actions.activateDreamSkinTheme()} title={t("保存并应用主题；需要重启时只会标记为待应用")} type="button">
                <Play className="h-4 w-4" />
                {pendingRestart ? t("更新待应用") : t("应用所选主题")}
              </Button>
              <Button onClick={() => void actions.restoreDreamSkin()} type="button" variant="outline">
                <RotateCcw className="h-4 w-4" />
                {t("恢复 Codex 外观")}
              </Button>
            </div>
          </div>
          {!masterEnabled ? (
            <div className="hint-line"><Info className="h-4 w-4" /><span>{t("请先在 Codex增强 页面开启总开关。")}</span></div>
          ) : null}
          {pendingRestart ? (
            <div className="dream-skin-pending-state" role="status">
              <Rocket className="h-5 w-5" aria-hidden="true" />
              <div>
                <strong>{t("待应用主题")}：{pendingRestart.pendingThemeName}</strong>
                <small>
                  {currentThemeLabel}：{pendingRestart.currentThemeName}。{t("配置已保存，可以继续浏览和编辑，稍后重启即可生效。")}
                </small>
              </div>
              <Button disabled={launchPending} onClick={() => void actions.restart()}>
                <Rocket className="h-4 w-4" />
                {t("重启并应用")}
              </Button>
            </div>
          ) : null}
        </CardContent>
      </Panel>
      <Panel className="dream-skin-panel dream-skin-browser">
        <CardContent>
          <div className="dream-skin-browser-toolbar">
            <div aria-label={t("主题视图")} className="dream-skin-view-tabs" role="tablist">
              <button
                aria-selected={themeView === "local"}
                className={themeView === "local" ? "is-active" : ""}
                onClick={() => setThemeView("local")}
                role="tab"
                type="button"
              >
                <Palette className="h-4 w-4" />
                {t("我的主题")}
                <span className={libraryPending ? "is-loading" : ""} title={libraryPending ? t("加载中…") : undefined}>
                  {libraryPending ? <Loader2 aria-hidden="true" className="h-3 w-3 animate-spin" /> : libraryCount}
                </span>
              </button>
              <button
                aria-selected={themeView === "community"}
                className={themeView === "community" ? "is-active" : ""}
                onClick={() => setThemeView("community")}
                role="tab"
                type="button"
              >
                <Github className="h-4 w-4" />
                {t("DreamSkin 社区")}
                <span className={communityPending ? "is-loading" : ""} title={communityPending ? t("加载中…") : undefined}>
                  {communityPending ? <Loader2 aria-hidden="true" className="h-3 w-3 animate-spin" /> : communityCount}
                </span>
              </button>
              <button
                aria-selected={themeView === "market"}
                className={themeView === "market" ? "is-active" : ""}
                onClick={() => setThemeView("market")}
                role="tab"
                type="button"
              >
                <Store className="h-4 w-4" />
                {t("主题市场")}
                <span className={marketPending ? "is-loading" : ""} title={marketPending ? t("加载中…") : undefined}>
                  {marketPending ? <Loader2 aria-hidden="true" className="h-3 w-3 animate-spin" /> : marketCount}
                </span>
              </button>
            </div>
            {themeView === "local" ? (
              <Toolbar>
                <Button disabled={Boolean(installOperation)} variant="outline" onClick={() => void actions.importDreamSkinThemePackage()}>
                  {installOperation?.source === "import"
                    ? <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
                    : <PackageOpen className="h-4 w-4" />}
                  {installOperation?.source === "import" ? t("导入中…") : t("导入主题包")}
                </Button>
                <Button variant="secondary" onClick={() => void actions.createDreamSkinTheme()}>
                  <ImagePlus className="h-4 w-4" />
                  {t("从图片创建")}
                </Button>
              </Toolbar>
            ) : null}
          </div>

          {installOperation ? (
            <DreamSkinInstallProgress label={dreamSkinInstallOperationLabel(installOperation)} />
          ) : null}

          {themeView === "community" ? (
            <DreamSkinCommunitySection
              community={community}
              communityLoading={communityLoading}
              installOperation={installOperation}
              actions={actions}
              onInstalled={() => setThemeView("local")}
            />
          ) : themeView === "market" ? (
            <section className="dream-skin-market">
              <div className="dream-skin-library-head">
                <div>
                  <strong>{t("社区主题")}</strong>
                  <small>
                    {market?.updatedAt
                      ? tf("清单更新于 {0}，安装后会保存到“我的主题”。", [market.updatedAt])
                      : t("从 CodexPlusPlus-Themes 仓库加载可安装主题。")}
                  </small>
                </div>
                <Toolbar>
                  <Button onClick={() => void actions.refreshDreamSkinMarket()} variant="secondary">
                    <RefreshCw className="h-4 w-4" />
                    {t("刷新市场")}
                  </Button>
                  <Button onClick={() => void actions.openExternalUrl(market?.repositoryUrl || "https://github.com/BigPizzaV3/CodexPlusPlus-Themes")} variant="outline">
                    <Github className="h-4 w-4" />
                    {t("投稿主题")}
                  </Button>
                </Toolbar>
              </div>
              {market?.cached || market?.warning ? (
                <div className="dream-skin-market-warning">
                  <Info className="h-4 w-4" />
                  <span>{market.warning || t("远程仓库暂不可用，当前显示本地缓存。")}</span>
                </div>
              ) : null}
              {market?.themes.length ? (
                <div className="dream-skin-market-grid">
                  {market.themes.map((item) => (
                    <DreamSkinMarketCard
                      actions={actions}
                      installOperation={installOperation}
                      key={item.id}
                      onInstalled={() => setThemeView("local")}
                      theme={item}
                    />
                  ))}
                </div>
              ) : (
                <div className="empty">
                  {market?.status === "failed" ? market.message : (
                    <span className="dream-skin-loading" role="status">
                      {marketPending ? <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" /> : null}
                      {t("正在加载主题市场…")}
                    </span>
                  )}
                </div>
              )}
            </section>
          ) : (
            <>
          <section className="dream-skin-theme-library">
            <div className="dream-skin-library-head">
              <div>
                <strong>{t("我的主题")}</strong>
                <small>
                  {pendingRestart
                    ? t("选择其他卡片可继续调整待应用主题；当前界面不会自动重启。")
                    : t("选择卡片只会载入草稿；需要完整切换时会保存为待应用主题。")}
                </small>
              </div>
              <div className="dream-skin-local-search">
                <Search aria-hidden="true" className="h-4 w-4" />
                <Input aria-label={t("搜索我的主题")} onChange={(event) => setLocalQuery(event.currentTarget.value)} placeholder={t("搜索主题名称")} type="search" value={localQuery} />
              </div>
            </div>
            <div className="dream-skin-theme-list">
              {localThemes.map((item) => {
                const cardPreview = item.previewPath
                  ? convertFileSrc(item.previewPath)
                  : isWindowsPlatform
                    ? dreamSkinWindowsPreviewUrl
                    : dreamSkinMacPreviewUrl;
                const cardDirty = item.key === selectedTheme && dirty;
                const currentRunning = pendingRestart
                  ? pendingRestart.currentThemeKey === item.key
                  : item.active;
                const pendingApplication = pendingRestart?.pendingThemeKey === item.key;
                return (
                  <article
                    className={`dream-skin-theme-card${item.key === selectedTheme ? " is-selected" : ""}${currentRunning ? " is-current" : ""}${pendingApplication ? " is-pending" : ""}`}
                    key={item.key}
                  >
                    <button
                      aria-pressed={item.key === selectedTheme}
                      className="dream-skin-theme-select"
                      onClick={() => actions.selectDreamSkinTheme(item)}
                      type="button"
                    >
                      <span className="dream-skin-theme-image">
                        <img alt={item.name} loading="lazy" src={cardPreview} />
                        {currentRunning || pendingApplication ? (
                          <span className="dream-skin-theme-badges">
                            {currentRunning ? <b>{currentThemeLabel}</b> : null}
                            {pendingApplication ? <b className="is-pending">{t("待应用")}</b> : null}
                          </span>
                        ) : null}
                      </span>
                      <span className="dream-skin-theme-copy">
                        <strong title={item.name}>{item.name}</strong>
                        <small>
                          {item.builtin
                            ? t("内置主题")
                            : item.kind === "activeUnsaved"
                              ? t("当前未保存主题")
                              : t("用户主题")}
                        </small>
                      </span>
                      {item.modified || cardDirty ? <em>{t("已修改")}</em> : null}
                    </button>
                    {item.kind === "stored" ? (
                      <details className="dream-skin-theme-menu" data-popover-menu="">
                        <summary aria-label={tf("主题操作：{0}", [item.name])} title={t("主题操作")}><MoreHorizontal className="h-4 w-4" /></summary>
                        <div>
                          <button onClick={() => void actions.renameDreamSkinTheme(item)} type="button">
                            <Edit3 className="h-4 w-4" />
                            {t("重命名")}
                          </button>
                          <button disabled={item.active || currentRunning} onClick={() => void actions.deleteDreamSkinTheme(item)} type="button">
                            <Trash2 className="h-4 w-4" />
                            {t("删除")}
                          </button>
                        </div>
                      </details>
                    ) : null}
                  </article>
                );
              })}
            </div>
            {!library ? (
              <p className="empty">
                <span className="dream-skin-loading" role="status">
                  <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
                  {t("正在加载主题库…")}
                </span>
              </p>
            ) : null}
            {library && !localThemes.length ? (
              <div className="dream-skin-local-empty">
                <Search aria-hidden="true" className="h-5 w-5" />
                <strong>{t("没有匹配的主题")}</strong>
                <Button onClick={() => setLocalQuery("")} type="button" variant="outline">{t("清除搜索")}</Button>
              </div>
            ) : null}
          </section>

          <details className="dream-skin-customizer">
            <summary>
              <span className="dream-skin-customizer-title">
                <Settings className="h-4 w-4" />
                <span>
                  <strong>{t("自定义主题")}</strong>
                  <small>{t("图片、文字和配色等高级编辑项")}</small>
                </span>
              </span>
              <em className={dirty ? "is-dirty" : ""}>{dirty ? t("有未保存修改") : t("按需展开")}</em>
            </summary>
            <div className="dream-skin-customizer-content">
              <div className="dream-skin-customizer-actions">
                <Button variant="secondary" onClick={() => void actions.createDreamSkinTheme()}>
                  <ImagePlus className="h-4 w-4" />
                  {t("从图片创建")}
                </Button>
              </div>

              <div className="dream-skin-platform-note">
                <Info className="h-4 w-4" />
                <span>
                  {isWindowsPlatform
                    ? t("Windows 使用亮暗模式、图片取色和可选强调色；完整色板仅在 macOS 生效。")
                    : t("macOS 会应用主题中的图片、文字和颜色配置。")}
                </span>
              </div>

              <div className="dream-skin-editor-layout">
                <div className="dream-skin-media-editor">
                  <div
                    className="dream-skin-preview"
                    style={isWindowsPlatform ? undefined : { backgroundColor: themeColors.background }}
                  >
                    <img alt={t("Dream Skin 图片预览")} src={previewUrl} />
                    <span style={isWindowsPlatform ? undefined : { backgroundColor: themeColors.panel, color: themeColors.text }}>
                      <strong>{theme.name}</strong>
                      <small style={isWindowsPlatform ? undefined : { color: themeColors.muted }}>{customImagePath ? t("自定义托管图片") : t("目标项目默认图片")}</small>
                    </span>
                  </div>
                  <RelayFold className="relay-config-section dream-skin-image-info" title={t("图片信息")}>
                    <Field label={t("托管图片路径")}>
                      <Input
                        readOnly
                        placeholder={t("使用目标项目默认图片")}
                        value={draft?.imagePath ?? ""}
                      />
                    </Field>
                  </RelayFold>
                  <Toolbar>
                    <Button variant="secondary" onClick={() => void actions.chooseDreamSkinImagePath()}>
                      <Camera className="h-4 w-4" />
                      {t("导入图片")}
                    </Button>
                    <Button
                      disabled={!customImagePath}
                      variant="outline"
                      onClick={() => void actions.resetDreamSkinImage()}
                    >
                      <RotateCcw className="h-4 w-4" />
                      {t("恢复默认图片")}
                    </Button>
                  </Toolbar>
                </div>

                <div className="dream-skin-edit-sections">
                  <details className="dream-skin-edit-section">
                    <summary><ChevronDown aria-hidden="true" className="dream-skin-edit-chevron h-4 w-4" /><strong>{t("输入框旁照片")}</strong></summary>
                    <div className="dream-skin-edit-section-body">
                      <div className="dream-skin-companion-controls">
                        <div className="dream-skin-companion-heading">
                          <div>
                            <small>{t("为主题选择一张显示在 Codex 输入框旁的自定义照片")}</small>
                          </div>
                          {companionDataUrl ? (
                            <img alt={t("输入框旁照片预览")} src={companionDataUrl} />
                          ) : null}
                        </div>
                        <input
                          accept="image/png,image/jpeg,image/webp,image/gif"
                          className="sr-only"
                          onChange={chooseCompanion}
                          ref={companionInputRef}
                          type="file"
                        />
                        <Toolbar>
                          <Button onClick={() => companionInputRef.current?.click()} type="button" variant="secondary">
                            <Camera className="h-4 w-4" />
                            {companionDataUrl ? t("更换照片") : t("选择照片")}
                          </Button>
                          <Button disabled={!companionDataUrl} onClick={clearCompanion} type="button" variant="outline">
                            <Trash2 className="h-4 w-4" />
                            {t("清除照片")}
                          </Button>
                        </Toolbar>
                        {companionError ? <small className="dream-skin-companion-error">{companionError}</small> : null}
                        <div className="dream-skin-companion-fields">
                          <label className="switch-row compact">
                            <input
                              checked={companionEnabled}
                              disabled={!companionDataUrl}
                              onChange={(event) => updateCompanion({ enabled: event.currentTarget.checked })}
                              type="checkbox"
                            />
                            <span>
                              <strong>{t("显示在输入框旁")}</strong>
                              <small>{t("应用主题后显示在输入框的左侧或右侧")}</small>
                            </span>
                            <ToggleVisual />
                          </label>
                          <Field label={t("照片宽度") }>
                            <Input
                              disabled={!companionDataUrl}
                              inputMode="numeric"
                              max={160}
                              min={48}
                              type="number"
                              value={companion?.width ?? 96}
                              onChange={(event) => updateCompanion({ width: Math.max(48, Math.min(160, Number(event.currentTarget.value) || 96)) })}
                            />
                          </Field>
                          <Field label={t("显示位置") }>
                            <AppSelect
                              disabled={!companionDataUrl}
                              value={companion?.side ?? "right"}
                              onChange={(value) => updateCompanion({ side: value })}
                              options={[
                                { value: "auto", label: t("自动") },
                                { value: "left", label: t("左侧") },
                                { value: "right", label: t("右侧") },
                              ]}
                            />
                          </Field>
                          <Field label={t("水平偏移") }>
                            <Input
                              disabled={!companionDataUrl}
                              inputMode="numeric"
                              max={48}
                              min={-48}
                              type="number"
                              value={companion?.offsetX ?? 0}
                              onChange={(event) => updateCompanion({ offsetX: Math.max(-48, Math.min(48, Number(event.currentTarget.value) || 0)) })}
                            />
                          </Field>
                          <Field label={t("垂直偏移") }>
                            <Input
                              disabled={!companionDataUrl}
                              inputMode="numeric"
                              max={160}
                              min={-160}
                              type="number"
                              value={companion?.offsetY ?? 4}
                              onChange={(event) => updateCompanion({ offsetY: Math.max(-160, Math.min(160, Number(event.currentTarget.value) || 0)) })}
                            />
                          </Field>
                        </div>
                      </div>

                    </div>
                  </details>
                  <details className="dream-skin-edit-section">
                    <summary><ChevronDown aria-hidden="true" className="dream-skin-edit-chevron h-4 w-4" /><strong>{t("界面文字")}</strong></summary>
                    <div className="dream-skin-edit-section-body">
                      <div className="dream-skin-text-grid">
                        <Field label={t("主题 ID")}><Input readOnly={draft?.builtin || savedThemeSelected} value={theme.id} onChange={(event) => updateThemeText("id", event.currentTarget.value)} /></Field>
                        <Field label={t("主题名称")}><Input value={theme.name} onChange={(event) => updateThemeText("name", event.currentTarget.value)} /></Field>
                        <Field label={t("品牌副标题")}><Input value={theme.brandSubtitle} onChange={(event) => updateThemeText("brandSubtitle", event.currentTarget.value)} /></Field>
                        <Field label={t("主题标语")}><Input value={theme.tagline} onChange={(event) => updateThemeText("tagline", event.currentTarget.value)} /></Field>
                        <Field label={t("项目前缀")}><Input value={theme.projectPrefix} onChange={(event) => updateThemeText("projectPrefix", event.currentTarget.value)} /></Field>
                        <Field label={t("项目按钮文字")}><Input value={theme.projectLabel} onChange={(event) => updateThemeText("projectLabel", event.currentTarget.value)} /></Field>
                        <Field label={t("状态文字")}><Input value={theme.statusText} onChange={(event) => updateThemeText("statusText", event.currentTarget.value)} /></Field>
                        <Field label={t("引用文字")}><Input value={theme.quote} onChange={(event) => updateThemeText("quote", event.currentTarget.value)} /></Field>
                      </div>
                    </div>
                  </details>
                  <details className="dream-skin-edit-section">
                    <summary><ChevronDown aria-hidden="true" className="dream-skin-edit-chevron h-4 w-4" /><strong>{t("主题配色")}</strong></summary>
                    <div className="dream-skin-edit-section-body">
                      {isWindowsPlatform ? (
                        <div className="dream-skin-windows-theme-controls">
                          <Field label={t("外观模式")}>
                            <div aria-label={t("外观模式")} className="segmented dream-skin-appearance-options" role="group">
                              {([
                                ["auto", t("自动")],
                                ["light", t("亮色")],
                                ["dark", t("暗色")],
                              ] as const).map(([value, label]) => (
                                <button
                                  aria-pressed={themeAppearance === value}
                                  className={themeAppearance === value ? "active" : ""}
                                  key={value}
                                  onClick={() => updateTheme({ ...theme, appearance: value })}
                                  type="button"
                                >
                                  {label}
                                </button>
                              ))}
                            </div>
                          </Field>
                          <div className="dream-skin-windows-accent">
                            <DreamSkinColorField
                              label={t("强调色")}
                              value={windowsAccent}
                              onChange={updateWindowsAccent}
                            />
                            <Button
                              disabled={!windowsAccent.trim()}
                              onClick={() => updateWindowsAccent("")}
                              size="sm"
                              variant="outline"
                            >
                              <RotateCcw className="h-4 w-4" />
                              {t("跟随图片配色")}
                            </Button>
                          </div>
                          <small className="dream-skin-windows-theme-note">
                            {t("亮暗模式直接控制 Codex 外观；强调色留空时自动从主题图片提取。")}
                          </small>
                        </div>
                      ) : (
                        <div className="dream-skin-colors">
                          {dreamSkinColorFields().map(([key, label]) => (
                            <DreamSkinColorField
                              key={key}
                              label={label}
                              value={String(themeColors[key])}
                              onChange={(value) => updateThemeColor(key, value)}
                            />
                          ))}
                        </div>
                      )}
                    </div>
                  </details>
                </div>
              </div>
              <div className="dream-skin-editor-footer">
              <Toolbar>
                <Button disabled={!draft} onClick={() => void actions.saveDreamSkinTheme()}>
                  <Save className="h-4 w-4" />
                  {draft?.builtin || selectedItem?.kind === "activeUnsaved" ? t("保存为新主题") : t("保存主题")}
                </Button>
                <Button variant="outline" onClick={() => void actions.resetDreamSkinTheme()}>
                  <RotateCcw className="h-4 w-4" />
                  {isWindowsPlatform ? t("恢复 Codex 默认配色") : t("恢复 Dream Skin 默认主题")}
                </Button>
              </Toolbar>
              </div>
            </div>
          </details>
            </>
          )}
        </CardContent>
      </Panel>
      <RelayFold className="dream-skin-panel dream-skin-diagnostics-fold" title={t("诊断与验证")}>
        <p className="relay-fold-description">{t("配置保存在 Codex++，实时操作通过本机回环 CDP 执行")}。{t("检查官方应用身份、CDP renderer、目标样式和页面布局")}</p>
          <div className="dream-skin-diagnostics-grid">
            <DreamSkinCheckList title={t("运行诊断")} checks={runtimeChecks} emptyText={t("刷新状态后显示运行诊断。")}/>
            <DreamSkinCheckList title={t("最近实机验证")} checks={verificationChecks} emptyText={t("运行实机验证后显示页面检查结果。")}/>
          </div>
          {verification ? (
            <div className="dream-skin-verification-meta">
              <span><small>{t("注入版本")}</small><code>{verification.version || t("未检测到")}</code></span>
              <span><small>{t("截图路径")}</small><code>{verification.screenshotPath || t("未保存截图")}</code></span>
            </div>
          ) : null}
          <Toolbar>
            <Button variant="secondary" onClick={() => void actions.refreshDreamSkinStatus()}>
              <RefreshCw className="h-4 w-4" />
              {t("刷新诊断")}
            </Button>
            <Button onClick={() => void actions.verifyDreamSkin()}>
              <ShieldCheck className="h-4 w-4" />
              {t("实机验证")}
            </Button>
            <Button variant="outline" onClick={() => void actions.saveDreamSkinScreenshot()}>
              <Camera className="h-4 w-4" />
              {t("保存截图")}
            </Button>
          </Toolbar>
      </RelayFold>
      <details className="dream-skin-about">
        <summary><ChevronDown aria-hidden="true" className="dream-skin-about-chevron h-4 w-4" /><strong>{t("来源与授权说明")}</strong></summary>
        <div>
          <p>{t("项目来源：Fei-Away/Codex-Dream-Skin · 原作者 Fei-Away · MIT License · 第三方图片需自行确认授权")}</p>
          <p>{t("应用会保存当前图片与主题配置；恢复原始外观不会删除主题。")}</p>
        </div>
      </details>
    </div>
  );
}

function DreamSkinColorField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <Field className="dream-skin-color-field" label={label}>
      <span className="dream-skin-color-control">
        <input
          aria-label={label}
          type="color"
          value={dreamSkinPickerColor(value)}
          onChange={(event) => onChange(event.currentTarget.value.toUpperCase())}
        />
        <Input value={value} onChange={(event) => onChange(event.currentTarget.value)} />
      </span>
    </Field>
  );
}

function DreamSkinMarketCard({
  theme,
  actions,
  installOperation,
  onInstalled,
}: {
  theme: DreamSkinMarketTheme;
  actions: Actions;
  installOperation: DreamSkinInstallOperation | null;
  onInstalled: () => void;
}) {
  const installing = installOperation?.source === "market" && installOperation.id === theme.id;
  const installBlocked = Boolean(installOperation);
  const status = theme.updateAvailable
    ? t("可更新")
    : theme.installed
      ? theme.installedVersion
        ? tf("已安装 {0}", [theme.installedVersion])
        : t("已安装")
      : t("未安装");
  return (
    <article className="dream-skin-market-card">
      <div className="dream-skin-market-preview">
        <img
          alt={theme.name}
          loading="lazy"
          onError={(event) => {
            event.currentTarget.onerror = null;
            event.currentTarget.src = isWindowsPlatform ? dreamSkinWindowsPreviewUrl : dreamSkinMacPreviewUrl;
          }}
          src={theme.previewUrl}
        />
        <UiBadge variant={theme.updateAvailable ? "default" : theme.installed ? "secondary" : "outline"}>{status}</UiBadge>
      </div>
      <div className="dream-skin-market-copy">
        <div className="dream-skin-market-title">
          <strong title={theme.name}>{theme.name}</strong>
          <span>v{theme.version}</span>
        </div>
        <small>{tf("作者：{0} · {1}", [theme.author, theme.license])}</small>
        <p>{theme.description || t("暂无主题说明。")}</p>
        <div className="dream-skin-market-tags">
          {theme.tags.map((tag) => <span key={tag}>{tag}</span>)}
        </div>
      </div>
      <div className="dream-skin-market-actions">
        <Button
          aria-busy={installing}
          disabled={installBlocked}
          onClick={async () => {
            if (await actions.installDreamSkinMarketTheme(theme)) onInstalled();
          }}
          size="sm"
        >
          {installing
            ? <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
            : <Download className="h-4 w-4" />}
          {installing ? t("安装中…") : theme.updateAvailable ? t("更新") : theme.installed ? t("重新安装") : t("安装")}
        </Button>
        <Button onClick={() => void actions.openExternalUrl(theme.sourceUrl)} size="sm" variant="outline">
          <ExternalLink className="h-4 w-4" />
          {t("来源")}
        </Button>
        {installing ? <DreamSkinInstallProgress /> : null}
      </div>
    </article>
  );
}

function DreamSkinCommunitySection({
  community,
  communityLoading,
  installOperation,
  actions,
  onInstalled,
}: {
  community: DreamSkinCommunityResult | null;
  communityLoading: boolean;
  installOperation: DreamSkinInstallOperation | null;
  actions: Actions;
  onInstalled: () => void;
}) {
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<"latest" | "popular" | "name">("latest");
  const items = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    const filtered = (community?.items ?? []).filter((item) => {
      if (!normalized) return true;
      return [item.name, item.authorDisplayName, item.themeId, item.license]
        .some((value) => value.toLowerCase().includes(normalized));
    });
    return [...filtered].sort((left, right) => {
      if (sort === "popular") return right.downloadCount - left.downloadCount;
      if (sort === "name") return left.name.localeCompare(right.name, "zh-CN");
      return right.reviewedAt.localeCompare(left.reviewedAt);
    });
  }, [community?.items, query, sort]);

  return (
    <section className="dream-skin-community">
      <div className="dream-skin-library-head">
        <div>
          <strong>{t("DreamSkin 社区主题")}</strong>
          <small>
            {community?.total
              ? tf("来自 DreamSkin.cc 的已审核主题，共 {0} 套；安装前仍会在本机再次校验。", [String(community.total)])
              : t("从 DreamSkin.cc 加载已审核主题包。")}
          </small>
        </div>
        <Toolbar>
          <Button onClick={() => void actions.refreshDreamSkinCommunity()} variant="secondary">
            <RefreshCw className="h-4 w-4" />
            {t("刷新社区")}
          </Button>
          <Button onClick={() => void actions.openExternalUrl("https://dreamskin.cc/gallery")} variant="outline">
            <ExternalLink className="h-4 w-4" />
            {t("在线主题库")}
          </Button>
          <Button onClick={() => void actions.openExternalUrl("https://dreamskin.cc/studio")} variant="outline">
            <Palette className="h-4 w-4" />
            {t("在线 Studio")}
          </Button>
        </Toolbar>
      </div>
      {community?.warning ? (
        <div className="dream-skin-market-warning">
          <Info className="h-4 w-4" />
          <span>{community.warning}</span>
        </div>
      ) : null}
      <div className="dream-skin-community-controls">
        <Input
          aria-label={t("搜索社区主题")}
          onChange={(event) => setQuery(event.currentTarget.value)}
          placeholder={t("搜索主题名称、作者或许可证")}
          value={query}
        />
        <AppSelect
          onChange={(value) => setSort(value as typeof sort)}
          options={[
            { value: "latest", label: t("最新审核") },
            { value: "popular", label: t("下载最多") },
            { value: "name", label: t("名称排序") },
          ]}
          title={t("社区主题排序")}
          value={sort}
        />
      </div>
      {items.length ? (
        <div className="dream-skin-community-grid">
          {items.map((item) => (
            <DreamSkinCommunityCard
              actions={actions}
              installOperation={installOperation}
              key={item.id}
              onInstalled={onInstalled}
              theme={item}
            />
          ))}
        </div>
      ) : (
        <div className="empty">
          {!community || communityLoading ? (
            <span className="dream-skin-loading" role="status">
              <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
              {t("正在加载 DreamSkin 社区…")}
            </span>
          ) : community.status === "failed"
            ? community.message
            : query.trim()
              ? t("没有匹配的社区主题。")
              : t("DreamSkin 社区暂时没有可用主题。")}
        </div>
      )}
    </section>
  );
}

function DreamSkinCommunityCard({
  theme,
  actions,
  installOperation,
  onInstalled,
}: {
  theme: DreamSkinCommunityTheme;
  actions: Actions;
  installOperation: DreamSkinInstallOperation | null;
  onInstalled: () => void;
}) {
  const installing = installOperation?.source === "community" && installOperation.id === theme.id;
  const installBlocked = Boolean(installOperation);
  const status = theme.updateAvailable
    ? t("可更新")
    : theme.installed
      ? tf("已安装 {0}", [theme.installedVersion])
      : t("未安装");
  const packageSize = theme.packageBytes >= 1024 * 1024
    ? `${(theme.packageBytes / 1024 / 1024).toFixed(1)} MiB`
    : `${Math.ceil(theme.packageBytes / 1024)} KiB`;
  return (
    <article className="dream-skin-community-card">
      <div className="dream-skin-community-preview">
        <img
          alt={theme.name}
          loading="lazy"
          onError={(event) => {
            event.currentTarget.onerror = null;
            event.currentTarget.src = isWindowsPlatform ? dreamSkinWindowsPreviewUrl : dreamSkinMacPreviewUrl;
          }}
          src={theme.previewUrl}
        />
        <UiBadge variant={theme.updateAvailable ? "default" : theme.installed ? "secondary" : "outline"}>{status}</UiBadge>
      </div>
      <div className="dream-skin-community-copy">
        <div className="dream-skin-market-title">
          <strong title={theme.name}>{theme.name}</strong>
          <span>v{theme.version}</span>
        </div>
        <small>{tf("作者：{0} · {1} · {2} 次下载", [theme.authorDisplayName, theme.license, String(theme.downloadCount)])}</small>
        <small>{tf("主题包：{0}", [packageSize])}</small>
      </div>
      <div className="dream-skin-community-actions">
        <Button
          aria-busy={installing}
          disabled={installBlocked || !theme.applyCompatible}
          onClick={async () => {
            if (await actions.installDreamSkinCommunityTheme(theme)) onInstalled();
          }}
          size="sm"
          title={theme.applyCompatible ? t("下载、校验并安装主题包") : t("此主题仅支持在线预览或下载")}
        >
          {installing
            ? <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
            : <Download className="h-4 w-4" />}
          {installing ? t("安装中…") : theme.updateAvailable ? t("更新") : theme.installed ? t("重新安装") : t("安装")}
        </Button>
        <Button onClick={() => void actions.openExternalUrl(`https://dreamskin.cc/preview?themeVersion=${encodeURIComponent(theme.id)}`)} size="sm" variant="outline">
          <Eye className="h-4 w-4" />
          {t("预览")}
        </Button>
        {installing ? <DreamSkinInstallProgress /> : null}
      </div>
    </article>
  );
}

function dreamSkinInstallOperationLabel(operation: DreamSkinInstallOperation): string {
  if (operation.source === "import") return t("正在导入并校验本地主题包…");
  return tf("正在下载、校验并安装“{0}”…", [operation.name]);
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

function DreamSkinCheckList({ title, checks, emptyText }: { title: string; checks: DreamSkinCheck[]; emptyText: string }) {
  return (
    <section className="dream-skin-check-section">
      <strong>{title}</strong>
      <div className="dream-skin-check-list">
        {checks.length ? checks.map((check) => (
          <div className={`dream-skin-check is-${check.level}`} key={`${title}-${check.id}`}>
            {dreamSkinCheckIcon(check.level)}
            <span>
              <strong>{check.label}</strong>
              <small>{check.message}</small>
            </span>
            <b>{dreamSkinCheckLevelLabel(check.level)}</b>
          </div>
        )) : <p className="empty">{emptyText}</p>}
      </div>
    </section>
  );
}

function dreamSkinColorFields(): Array<[keyof DreamSkinColors, string]> {
  return [
    ["background", t("背景色")],
    ["panel", t("面板色")],
    ["panelAlt", t("次级面板色")],
    ["accent", t("强调色")],
    ["accentAlt", t("次级强调色")],
    ["secondary", t("辅助色")],
    ["highlight", t("高亮色")],
    ["text", t("文字色")],
    ["muted", t("弱化文字色")],
    ["line", t("边线色")],
  ];
}

function dreamSkinPickerColor(value: string): string {
  const color = value.trim();
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(color);
  if (hex) {
    const digits = hex[1];
    return digits.length === 3
      ? `#${digits.split("").map((part) => `${part}${part}`).join("")}`
      : `#${digits.slice(0, 6)}`;
  }
  const rgb = /^rgba?\(\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)/i.exec(color);
  if (!rgb) return "#808080";
  const channel = (raw: string) => Math.max(0, Math.min(255, Math.round(Number(raw)))).toString(16).padStart(2, "0");
  return `#${channel(rgb[1])}${channel(rgb[2])}${channel(rgb[3])}`;
}

function dreamSkinCheckIcon(level: "pass" | "warning" | "fail") {
  if (level === "pass") return <CheckCircle2 aria-hidden="true" className="h-4 w-4" />;
  if (level === "fail") return <ShieldAlert aria-hidden="true" className="h-4 w-4" />;
  return <Info aria-hidden="true" className="h-4 w-4" />;
}

function dreamSkinCheckLevelLabel(level: "pass" | "warning" | "fail"): string {
  if (level === "pass") return t("通过");
  if (level === "fail") return t("失败");
  return t("警告");
}

function dreamSkinStateLabel(state: "pass" | "warning" | "fail" | "not_running"): string {
  if (state === "pass") return t("已应用并通过检查");
  if (state === "warning") return t("需要处理");
  if (state === "fail") return t("验证失败");
  return t("Codex 未运行或不可连接");
}

function UserScriptsScreen({ settings, market, actions }: { settings: SettingsResult | null; market: ScriptMarketResult | null; actions: Actions }) {
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
  const scripts = inventory?.scripts ?? [];
  const marketScripts = market?.market.scripts ?? [];
  const [marketSearch, setMarketSearch] = useState("");
  const [marketView, setMarketView] = useState<"grid" | "list">("grid");
  // 市场脚本 / 本地脚本放在同一块里切换，不再上下堆三张卡片。
  const [pane, setPane] = useState<"market" | "local">("market");
  const filteredMarketScripts = useMemo(() => {
    const query = marketSearch.trim().toLocaleLowerCase();
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
  }, [marketSearch, marketScripts]);
  const installedCount = marketScripts.filter((script) => script.installed).length;
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
  const localEnabled = inventory?.enabled !== false;
  return (
    <div className="tp-page user-scripts-page">
      <TopbarActions>
        <Button onClick={() => void actions.openExternalUrl(SCRIPT_MARKET_REPOSITORY_URL)} size="sm" variant="ghost">
          <ExternalLink />
          {t("投稿")}
        </Button>
        <Button onClick={() => void reload()} disabled={reloading} size="sm" variant="secondary" title={t("应用本地脚本及开关；旧脚本可能需要刷新 Codex 页面")}>
          <RefreshCw className={reloading ? "animate-spin" : ""} />
          {t("热重载脚本")}
        </Button>
        <Button onClick={() => void (async () => { await actions.refreshScriptMarket(); await actions.refreshCurrent(); })()} size="sm" variant="secondary">
          <RefreshCw />
          {t("刷新")}
        </Button>
      </TopbarActions>
      {/* 状态条：两个整体开关 + 一行摘要，代替原来四行「键：值」的统计卡。 */}
      <div className="user-scripts-strip">
        <label className="relay-bare-switch user-scripts-switch" title={t("整体开关不改变单个脚本的启停设置")}>
          <input
            aria-label={t("启用本地脚本")}
            checked={localEnabled}
            disabled={!inventory}
            onChange={() => void actions.setUserScriptsEnabled(!localEnabled)}
            type="checkbox"
          />
          <ToggleVisual />
          <span>{t("启用本地脚本")}</span>
        </label>
        <label className="relay-bare-switch user-scripts-switch" title={t("脚本导致黑屏或卡死时，下一次启动先跳过脚本；脚本启停设置不会改变")}>
          <input
            aria-label={t("下次启动跳过脚本")}
            checked={Boolean(inventory?.skip_next_start)}
            disabled={!inventory}
            onChange={() => void actions.setUserScriptsSkipNextStart(!inventory?.skip_next_start)}
            type="checkbox"
          />
          <ToggleVisual />
          <span>{t("下次启动跳过脚本")}</span>
        </label>
        <span className="user-scripts-summary">
          {tf("{0} 个市场脚本，已安装 {1} 个", [marketScripts.length, installedCount])}
          {market?.market.updatedAt ? ` · ${tf("清单更新于 {0}", [formatMarketUpdatedAt(market.market.updatedAt)])}` : market ? ` · ${market.market.message}` : ` · ${t("尚未刷新")}`}
        </span>
      </div>
      <div className="script-market-toolbar">
        <div className="mg-segmented user-scripts-panes" role="tablist" aria-label={t("脚本")}>
          <button aria-selected={pane === "market"} data-active={pane === "market"} onClick={() => setPane("market")} role="tab" type="button">
            {t("市场脚本")}<span>{marketScripts.length}</span>
          </button>
          <button aria-selected={pane === "local"} data-active={pane === "local"} onClick={() => setPane("local")} role="tab" type="button">
            {t("本地脚本")}<span>{scripts.length}</span>
          </button>
        </div>
        {pane === "market" ? <>
          <div className="script-market-search">
            <Search className="h-4 w-4" />
            <Input
              aria-label={t("搜索市场脚本")}
              onChange={(event) => setMarketSearch(event.currentTarget.value)}
              placeholder={t("搜索名称、作者、描述或标签")}
              value={marketSearch}
            />
          </div>
          <div className="script-market-view-toggle" role="group" aria-label={t("脚本排版")}>
            <Button aria-pressed={marketView === "grid"} onClick={() => setMarketView("grid")} size="sm" variant={marketView === "grid" ? "secondary" : "ghost"}>
              <LayoutGrid className="h-4 w-4" />
              {t("板块")}
            </Button>
            <Button aria-pressed={marketView === "list"} onClick={() => setMarketView("list")} size="sm" variant={marketView === "list" ? "secondary" : "ghost"}>
              <List className="h-4 w-4" />
              {t("列表")}
            </Button>
          </div>
        </> : <span className="user-scripts-summary">{t("内置、手动和市场安装脚本；可在这里启停或删除用户脚本")}</span>}
      </div>
      {pane === "market" ? (
        marketScripts.length ? (
          filteredMarketScripts.length ? (
            <div className={marketView === "list" ? "script-market-list" : "script-market-grid"}>
              {filteredMarketScripts.map((script) => (
                <MarketScriptCard
                  actions={actions}
                  installedKey={installedKeyByMarketId.get(script.id)}
                  key={script.id}
                  script={script}
                  view={marketView}
                />
              ))}
            </div>
          ) : (
            <div className="empty">{t("没有匹配的市场脚本。")}</div>
          )
        ) : (
          <div className="empty">{market?.status === "failed" ? market.message : t("点击刷新市场加载远程脚本。")}</div>
        )
      ) : (
        <Panel>
          <div className="table">
            {scripts.length ? scripts.map((script) => <ScriptRow key={script.key} script={script} actions={actions} />) : <div className="empty">{t("未发现用户脚本。")}</div>}
          </div>
        </Panel>
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
      <span className="session-maintenance-auto-state" data-enabled={form.providerSyncEnabled}>
        {form.providerSyncEnabled ? t("自动修复已开启") : t("自动修复已关闭")}
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
          <div className="session-repair-tools">
            <Field className="session-sync-target" label={t("同步目标")}>
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
            </Field>

            <label className="switch-row compact session-auto-repair">
              <input
                checked={form.providerSyncEnabled}
                onChange={(event) => onFormChange({ ...form, providerSyncEnabled: event.currentTarget.checked })}
                type="checkbox"
              />
              <span>
                <strong>{t(providerSyncItem.title)}</strong>
                <small>{t(providerSyncItem.description)}</small>
              </span>
              <ToggleVisual />
            </label>

            <label className="switch-row compact session-auto-repair">
              <input
                checked={Boolean(form.threadModelFollowSwitchEnabled)}
                onChange={(event) => onFormChange({ ...form, threadModelFollowSwitchEnabled: event.currentTarget.checked })}
                type="checkbox"
              />
              <span>
                <strong>{t("切换模型后旧会话跟随新默认")}</strong>
                <small>{t("切换供应商或默认模型后，下次启动前把仍停在旧默认模型的会话改为新默认；手动选过模型的会话不变。保存设置后生效。")}</small>
              </span>
              <ToggleVisual />
            </label>

            <div className="session-repair-actions">
              <Button
                disabled={providerSyncProgress.active || sessionIndexRepairActive || !canRepairProviderSessions}
                onClick={() => void actions.syncProvidersNow()}
                variant="outline"
              >
                <Wrench className="h-4 w-4" />
                {providerSyncProgress.active ? t("正在修复…") : t("修复历史会话")}
              </Button>
              <Button
                disabled={sessionIndexRepairActive || providerSyncProgress.active}
                onClick={() => void actions.repairSessionIndex()}
                variant="outline"
              >
                <Wrench className="h-4 w-4" />
                {sessionIndexRepairActive ? t("正在检查索引…") : t("修复会话索引")}
              </Button>
            </div>
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
              <strong>{t("最近一次会话索引修复报告")}</strong>
              <p>{t("最后检查：")}{sessionIndexRepairReport.checkedAtMs ? formatTime(sessionIndexRepairReport.checkedAtMs) : t("旧版报告未记录时间")}</p>
              <p>
                {t("读取文件")} {sessionIndexRepairReport.scannedFiles} · {t("复用缓存")} {sessionIndexRepairReport.cachedFiles} · {t("耗时")} {(sessionIndexRepairReport.elapsedMs / 1000).toFixed(1)} s
              </p>
              <p>
                {t("恢复消息")} {sessionIndexRepairReport.repairedItems} · {t("已存在")} {sessionIndexRepairReport.alreadyPresent} · {t("短暂等待")} {sessionIndexRepairReport.deferredItems ?? 0} · {t("需核查")} {sessionIndexRepairReport.skippedItems}
              </p>
              <small>{t("仅恢复有本地原文且可确认位置的消息；已打开的会话可能需要重新打开才能显示。")}</small>
              <p><small>{t("自动检查需要从此界面启动 Codex，并保持 Codex++ 运行，且自动修复开关已开启并保存；每次检查完成后间隔 30 分钟复查。此页面每 15 秒刷新报告，不会单独启动修复；再次检查不保证恢复。")}</small></p>
              <p><small>{t("短暂等待最长 30 分钟；原文和记录文件都已超过 24 小时未更新的项目直接转入需核查。缺少对应轮次或结束状态，当前证据不足以安全补回；后续检查仍会核验。")}</small></p>
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

function SessionsScreen({
  settings,
  form,
  sessions,
  providerSyncProgress,
  sessionIndexRepairActive,
  sessionIndexRepairReport,
  sessionIndexRepairReportError,
  providerSyncTargets,
  selectedProviderSyncTarget,
  onFormChange,
  actions,
}: {
  settings: SettingsResult | null;
  form: BackendSettings;
  sessions: LocalSessionsResult | null;
  providerSyncProgress: ProviderSyncProgress;
  sessionIndexRepairActive: boolean;
  sessionIndexRepairReport: SessionIndexRepairReport | null;
  sessionIndexRepairReportError: string | null;
  providerSyncTargets: ProviderSyncTargetsResult | null;
  selectedProviderSyncTarget: string;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const items = sessions?.sessions ?? [];
  const pageOffset = sessions?.offset ?? 0;
  const pageSize = sessions?.limit ?? 50;
  const currentPage = Math.floor(pageOffset / pageSize) + 1;
  const hasPreviousPage = pageOffset > 0;
  const hasNextPage = sessions?.hasMore === true;
  const activeCount = items.filter((item) => !item.archived).length;
  const archivedCount = items.length - activeCount;
  const totalCount = sessions?.totalCount ?? items.length;
  const [selectedSessionIds, setSelectedSessionIds] = useState<Set<string>>(() => new Set());
  const [selectionMode, setSelectionMode] = useState(false);
  const [bulkDeleting, setBulkDeleting] = useState(false);
  const selectedSessions = useMemo(() => items.filter((session) => selectedSessionIds.has(session.id)), [items, selectedSessionIds]);
  const selectedCount = selectedSessions.length;
  const allSelected = items.length > 0 && selectedCount === items.length;
  const providerTargets = providerSyncTargets?.targets ?? [];
  const selectedProviderTarget = providerTargets.find(
    (target) => target.id === selectedProviderSyncTarget,
  );
  const canRepairProviderSessions = selectedProviderTarget
    ? isProviderSyncTargetSelectable(selectedProviderTarget)
    : false;

  useEffect(() => {
    const itemIds = new Set(items.map((session) => session.id));
    setSelectedSessionIds((current) => {
      const next = new Set(Array.from(current).filter((id) => itemIds.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [items]);

  const toggleSessionSelection = (sessionId: string, checked: boolean) => {
    setSelectedSessionIds((current) => {
      const next = new Set(current);
      if (checked) {
        next.add(sessionId);
      } else {
        next.delete(sessionId);
      }
      return next;
    });
  };

  const selectAllSessions = () => {
    setSelectionMode(true);
    setSelectedSessionIds(new Set(items.map((session) => session.id)));
  };

  const clearSelectedSessions = () => setSelectedSessionIds(new Set());

  const deleteSelectedSessions = async () => {
    if (!selectionMode) {
      setSelectionMode(true);
      return;
    }
    setBulkDeleting(true);
    try {
      await actions.deleteLocalSessions(selectedSessions);
    } finally {
      setBulkDeleting(false);
    }
  };

  return (
    <>
      <Panel className="sessions-overview-panel">
        <CardHead title={t("会话管理")} detail={t("读取 Codex 本地 SQLite 会话库，会删除数据库记录和对应 rollout 文件")} />
        <CardContent className="sessions-overview-content">
          <div className="session-summary-bar">
            <div>
              <span>{t("会话总数")}</span>
              <strong>{tf("{0} 个", [totalCount])}</strong>
            </div>
            <div>
              <span>{t("当前页会话")}</span>
              <strong>{tf("{0} 个", [items.length])}</strong>
            </div>
            <div>
              <span>{t("当前页未归档")}</span>
              <strong>{tf("{0} 个", [activeCount])}</strong>
            </div>
            <div>
              <span>{t("当前页已归档")}</span>
              <strong>{tf("{0} 个", [archivedCount])}</strong>
            </div>
            <div className="session-summary-path">
              <span>{t("数据库")}</span>
              <code>{sessions?.dbPath ?? "~/.codex/sqlite/*.db"}</code>
            </div>
          </div>

          <SessionRepairPanel dirty={false} form={form} providerSyncProgress={providerSyncProgress} sessionIndexRepairActive={sessionIndexRepairActive} sessionIndexRepairReport={sessionIndexRepairReport} sessionIndexRepairReportError={sessionIndexRepairReportError} providerSyncTargets={providerSyncTargets} selectedProviderSyncTarget={selectedProviderSyncTarget} onFormChange={onFormChange} actions={actions} />
        </CardContent>
      </Panel>
      <Panel className="sessions-list-panel">
        <CardHead
          title={t("本地会话")}
          detail={sessions ? tf("第 {0} 页，每页最多 {1} 条，按更新时间倒序显示", [currentPage, pageSize]) : t("点击刷新会话读取本地数据库")}
        />
        <CardContent className="session-list-content">
          {items.length ? (
            <>
              <div className="session-list-toolbar">
                <span className="session-selection-summary">{t("已选择")} {selectedCount} / {items.length} {t("个会话")}</span>
                <div className="session-selection-actions">
                  <Button disabled={allSelected || bulkDeleting} onClick={selectAllSessions} size="sm" variant="outline">
                    {t("全选当前列表")}
                  </Button>
                  <Button disabled={!selectedCount || bulkDeleting} onClick={clearSelectedSessions} size="sm" variant="outline">
                    {t("清空选择")}
                  </Button>
                  <Button disabled={(selectionMode && !selectedCount) || bulkDeleting} onClick={() => void deleteSelectedSessions()} size="sm" variant="outline">
                    {selectionMode ? <Trash2 className="h-4 w-4" /> : null}
                    {selectionMode ? (bulkDeleting ? t("正在删除…") : t("删除已选")) : t("多选")}
                  </Button>
                </div>
              </div>
              <div className="session-list">
                {items.map((session) => {
                  const selected = selectedSessionIds.has(session.id);
                  return (
                    <div className="session-row" data-selection-mode={selectionMode} data-selected={selected} key={session.id}>
                      {selectionMode ? (
                        <label className="session-select" title={t("选择会话")}>
                          <input
                            aria-label={tf("选择会话 {0}", [session.title || session.id])}
                            checked={selected}
                            onChange={(event) => toggleSessionSelection(session.id, event.currentTarget.checked)}
                            type="checkbox"
                          />
                        </label>
                      ) : null}
                      <div className="session-main">
                        <strong>{session.title || t("未命名会话")}</strong>
                        <span>{session.id}</span>
                        <small>{session.cwd || t("未记录项目路径")}</small>
                      </div>
                      <div className="session-meta">
                        <Badge status={session.archived ? "archived" : "ok"} />
                        <span>{session.modelProvider || t("provider 未记录")}</span>
                        <span>{formatTime(session.updatedAtMs ?? 0)}</span>
                      </div>
                      <Button className="session-delete-button" variant="outline" onClick={() => void actions.deleteLocalSession(session)}>
                        <Trash2 className="h-4 w-4" />
                        {t("删除")}
                      </Button>
                    </div>
                  );
                })}
              </div>
              <div className="session-pagination">
                <Button
                  aria-label={t("上一页")}
                  disabled={!hasPreviousPage || bulkDeleting}
                  onClick={() => void actions.refreshLocalSessions(true, Math.max(0, pageOffset - pageSize))}
                  size="icon"
                  title={t("上一页")}
                  variant="outline"
                >
                  <ArrowLeft className="h-4 w-4" />
                </Button>
                <span>{tf("第 {0} 页", [currentPage])}</span>
                <Button
                  aria-label={t("下一页")}
                  disabled={!hasNextPage || bulkDeleting}
                  onClick={() => void actions.refreshLocalSessions(true, pageOffset + pageSize)}
                  size="icon"
                  title={t("下一页")}
                  variant="outline"
                >
                  <ArrowRight className="h-4 w-4" />
                </Button>
              </div>
            </>
          ) : (
            <div className="empty">{t("未读取到本地会话，或当前 SQLite 会话库不存在。")}</div>
          )}
        </CardContent>
      </Panel>
    </>
  );
}

/// 推荐内容页：列出广告源里的全部推荐（含 sponsor 与 normal）。
///
/// 概览页置顶位独立配置；展示多条时也从推荐池去重，避免重复出现。
function RecommendationsScreen({ ads, actions }: { ads: AdsResult | null; actions: Actions }) {
  const items = (ads?.ads ?? []).filter((ad) => !isExpiredAd(ad));
  // 置顶位排在最前，并把推荐池里指向同一家的那条去掉 —— 广告源里同一条赞助商
  // 常常同时出现在 top_ad 和 ads 里（两个 id、同一个落地页），只比 id 去不掉。
  const topAds = (ads?.topAds?.length ? ads.topAds : ads?.topAd ? [ads.topAd] : []).filter((ad) => !isExpiredAd(ad));
  const topIdentities = new Set(topAds.map(adIdentity));
  const pool = items.filter((ad) => !topIdentities.has(adIdentity(ad)));
  const ordered = [...topAds, ...pool];
  // 只展示赞助商推荐；普通推荐不再显示。
  const sponsors = ordered.filter((ad) => ad.type === "sponsor");
  return (
    <div className="tp-page rec-page">
      <TopbarActions>
        <Button onClick={() => void actions.refreshAds()} size="sm" variant="secondary">
          <RefreshCw />
          {t("刷新推荐")}
        </Button>
      </TopbarActions>
      {!ads ? <p className="rec-empty">{t("尚未加载推荐内容")}</p> : null}
      {ads ? (
        <section className="tp-page">
          <div className="tp-section-head"><h3>{t("赞助商推荐")}</h3><span>{tf("{0} 条", [sponsors.length])}</span></div>
          <AdGrid actions={actions} ads={sponsors} empty={t("暂无赞助商推荐。")} />
        </section>
      ) : null}
      <p className="tp-foot">{t("与 Codex 内插件菜单使用同一个远端广告源")}</p>
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
}: {
  overview: OverviewResult | null;
  launchPending: boolean;
  watcher: WatcherResult | null;
  autoTakeover: AutoTakeoverResult | null;
  settings: SettingsResult | null;
  launchForm: { appPath: string; debugPort: string; helperPort: string };
  onLaunchFormChange: (next: { appPath: string; debugPort: string; helperPort: string }) => void;
  actions: Actions;
}) {
  const savedCodexAppPath = settings?.settings.codexAppPath ?? "";
  return (
    <>
      <Panel id="codex-settings-check">
        <CardHead title={t("检查与修复")} detail={t("检查 Codex++ 入口、Codex 应用和开机启动状态")} />
        <CardContent>
          <div className="status-table">
            <StatusRow title={t("Codex 应用")} status={overview?.codex_app.status} path={overview?.codex_app.path} />
            <StatusRow title={t("Codex++ 应用入口")} status={overview?.management_shortcut.status} path={overview?.management_shortcut.path} />
            <StatusRow title={t("开机打开 Codex++")} status={watcher?.enabled ? "ok" : "disabled"} path={watcher?.disabled_flag} />
            <StatusRow
              title={t("自动接管官方 App")}
              status={settings?.settings.codexAppAutoTakeover && autoTakeover?.running ? "ok" : "disabled"}
              path={autoTakeover?.lastError ?? autoTakeover?.lastReason ?? null}
            />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.checkHealth()}>{t("检查")}</Button>
            <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>{t("修复快捷方式")}</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel id="codex-settings-startup">
        <CardHead title={t("开机打开 Codex++")} detail={t("登录系统后打开 Codex++ 界面，可选择自动启动 Codex")} />
        <CardContent>
          <label className="switch-row">
            <input
              checked={Boolean(watcher?.enabled)}
              disabled={!watcher}
              onChange={(event) => void actions.setWatcherEnabled(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("登录时自动打开")}</strong>
              <small>{t("开启后注册系统登录项，关闭时移除；是否自动启动 Codex 由下面的独立开关决定。")}</small>
            </span>
            <ToggleVisual />
          </label>
          <label className="switch-row">
            <input
              checked={Boolean(watcher?.start_codex_on_open)}
              disabled={!watcher}
              onChange={(event) => void actions.setWatcherStartCodexOnOpen(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("打开 Codex++ 时自动启动 Codex")}</strong>
              <small>{t("仅手动打开 Codex++ 时生效；已有 Codex 会先恢复，不会重复启动。")}</small>
            </span>
            <ToggleVisual />
          </label>
          <label className="switch-row">
            <input
              checked={Boolean(watcher?.start_codex)}
              disabled={!watcher}
              onChange={(event) => void actions.setWatcherStartCodex(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("登录时自动启动 Codex")}</strong>
              <small>{t("仅登录启动 Codex++ 时自动启动 Codex；手动打开 Codex++ 仍不会自动启动。")}</small>
            </span>
            <ToggleVisual />
          </label>
          <label className="switch-row">
            <input
              checked={Boolean(watcher?.start_minimized)}
              disabled={!watcher}
              onChange={(event) => void actions.setWatcherStartMinimized(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("启动时自动最小化")}</strong>
              <small>{t("仅登录启动时隐藏到托盘；手动打开 Codex++ 仍显示主窗口。")}</small>
            </span>
            <ToggleVisual />
          </label>
        </CardContent>
      </Panel>
      <Panel id="codex-settings-takeover">
        <CardHead title={t("自动接管官方 App")} detail={t("你自己双击启动官方 App 时，Codex++ 自动接管并注入增强")} />
        <CardContent>
          <label className="switch-row">
            <input
              checked={Boolean(settings?.settings.codexAppAutoTakeover)}
              onChange={(event) => void actions.setAutoTakeover(event.currentTarget.checked)}
              type="checkbox"
            />
            <span>
              <strong>{t("开启自动接管")}</strong>
              <small>{t("开启后，你自己启动官方 App 时 Codex++ 会自动接管。官方 App 已在运行且没有调试端口时，会先退出再带参数重开（可能中断正在进行的操作）。")}</small>
            </span>
            <ToggleVisual />
          </label>
          {settings?.settings.codexAppAutoTakeover ? (
            <>
              <div className="status-table">
                <StatusRow
                  title={t("接管状态")}
                  status={autoTakeover?.state === "degraded" ? "failed" : autoTakeover?.running ? "ok" : "disabled"}
                  path={autoTakeover?.state === "degraded" ? t("已降级") : autoTakeover?.running ? t("监听中") : t("未运行")}
                />
                <StatusRow title={t("官方 App")} status={autoTakeover?.appRunning ? "ok" : "disabled"} path={autoTakeover?.appRunning ? t("运行中") : t("未运行")} />
                <StatusRow
                  title={t("调试端口")}
                  status={autoTakeover?.actualDebugPort ? "ok" : "not_checked"}
                  path={autoTakeover?.actualDebugPort ? String(autoTakeover.actualDebugPort) : null}
                />
              </div>
              {autoTakeover?.pausedInstance ? <p className="hint">{t("本次已跳过自动接管，重启官方 App 后恢复。")}</p> : null}
              {autoTakeover?.lastError ? <p className="hint">{autoTakeover.lastError}</p> : null}
              {autoTakeover?.lastTakeoverAtMs ? (
                <p className="hint">{tf("上次接管：{0}", [new Date(autoTakeover.lastTakeoverAtMs).toLocaleString()])}</p>
              ) : null}
              {autoTakeover?.platformSupported ? null : <p className="hint">{t("自动接管目前只支持 macOS。")}</p>}
              <p className="hint">{t("仅在 Codex++ 正在运行时生效；需要重启官方 App 时会自动重启。")}</p>
              <Toolbar>
                <Button variant="secondary" onClick={() => void actions.takeOverNow()}>{t("立即接管")}</Button>
                <Button variant="secondary" onClick={() => void actions.pauseAutoTakeover()}>{t("本次不接管")}</Button>
                <Button variant="secondary" onClick={() => void actions.refreshAutoTakeover()}>{t("刷新状态")}</Button>
              </Toolbar>
            </>
          ) : (
            <p className="hint">{t("仅在 Codex++ 正在运行时生效；建议同时开启上面的「开机打开 Codex++」。")}</p>
          )}
        </CardContent>
      </Panel>
      <Panel id="codex-settings-app-path">
        <CardHead title={t("Codex 应用路径")} detail={t("免安装版或解包版只需要选择一次，之后从界面启动会自动复用")} />
        <CardContent>
          <div className="status-table">
            <StatusRow title={t("保存路径")} status={savedCodexAppPath ? "ok" : "not_checked"} path={savedCodexAppPath || null} />
            <StatusRow title={t("当前识别")} status={overview?.codex_app.status} path={overview?.codex_app.path} />
          </div>
          <Field label={t("保存的应用路径")}>
            <Input
              value={settings?.settings.codexAppPath ?? ""}
              placeholder={t("选择 Codex.exe、Codex.app、app 目录或解包目录")}
              readOnly
            />
          </Field>
          <Toolbar>
            <Button onClick={() => void actions.chooseCodexAppPath("folder")}>{t("选择应用目录")}</Button>
            <Button variant="secondary" onClick={() => void actions.chooseCodexAppPath("file")}>{t("选择 Codex.exe")}</Button>
            <Button variant="secondary" onClick={() => void actions.clearCodexAppPath()}>{t("清除保存路径")}</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel id="codex-settings-manual-launch">
        <CardHead title={t("手动启动")} detail={t("应用路径留空时使用已保存路径；没有保存路径时使用自动探测")} />
        <CardContent>
          <Field label={t("应用路径覆盖")}>
            <Input
              value={launchForm.appPath}
              onChange={(event) => onLaunchFormChange({ ...launchForm, appPath: event.currentTarget.value })}
              placeholder={savedCodexAppPath || t("例如 C:\\Program Files\\WindowsApps\\OpenAI.Codex...\\app")}
            />
          </Field>
          <div className="form-row">
            <Field label={t("Debug 端口")}>
              <Input
                value={launchForm.debugPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, debugPort: event.currentTarget.value })}
              />
            </Field>
            <Field label={t("Helper 端口")}>
              <Input
                value={launchForm.helperPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, helperPort: event.currentTarget.value })}
              />
            </Field>
          </div>
          <Toolbar>
            <Button disabled={launchPending} onClick={() => void actions.launch()}>{t("启动 Codex")}</Button>
            <Button variant="secondary" onClick={() => void actions.saveManualCodexAppPath()}>
              {t("保存为默认路径")}
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function AboutScreen({
  overview,
  update,
  updateInstallProgress,
  logs,
  diagnostics,
  actions,
}: {
  overview: OverviewResult | null;
  update: UpdateResult | null;
  updateInstallProgress: TaskProgress;
  logs: LogsResult | null;
  diagnostics: DiagnosticsResult | null;
  actions: Actions;
}) {
  return (
    <>
      <Panel id="app-settings-update">
        <CardHead title={t("版本与更新")} detail={t("从 GitHub Release 下载安装包更新")} />
        <CardContent>
          {/* 一行摘要 + 操作；发布说明在下面，下载进度只在下载时出现。 */}
          <div className="app-update-summary">
            <div className="app-update-copy">
              <strong>Codex++ {overview?.current_version ?? update?.currentVersion ?? "-"}</strong>
              <span data-tone={update?.updateAvailable ? "new" : undefined}>
                {update?.updateAvailable && update.latestVersion
                  ? tf("有新版本 {0}", [update.latestVersion])
                  : update?.latestVersion
                    ? t("已是最新版本")
                    : t("尚未检查更新")}
                {update?.updateAvailable && update.assetName ? ` · ${update.assetName}` : ""}
              </span>
            </div>
            <div className="app-update-actions">
              <Button onClick={() => void actions.checkUpdate()} variant={update?.updateAvailable ? "secondary" : "default"}>{t("检查更新")}</Button>
              {update?.updateAvailable ? (
                <Button disabled={updateInstallProgress.active} onClick={() => void actions.performUpdate()}>
                  {updateInstallProgress.active ? t("正在下载安装包…") : t("下载并运行安装包")}
                </Button>
              ) : null}
            </div>
          </div>
          {update?.updateAvailable && update.releaseSummary ? (
            <details className="app-update-notes">
              <summary>{tf("{0} 更新说明", [update.latestVersion ?? ""])}</summary>
              <ReleaseNotes markdown={update.releaseSummary} onOpenUrl={(url) => void actions.openExternalUrl(url)} />
            </details>
          ) : null}
          <TaskProgressBox completedTitle={t("上次更新结果")} progress={updateInstallProgress} title={t("安装包更新进度")} />
        </CardContent>
      </Panel>
      <Panel id="app-settings-about">
        <CardHead title={t("关于 Codex++")} detail={t("本地 Codex 增强、管理工具和安装包维护")} />
        <CardContent className="app-about">
          <dl className="app-about-facts">
            <div><dt>{t("Codex 版本")}</dt><dd>{overview?.codex_version ?? t("未检测到")}</dd></div>
            <div>
              <dt>{t("项目地址")}</dt>
              <dd>
                <button className="app-about-link" onClick={() => void actions.openExternalUrl("https://github.com/BigPizzaV3/CodexPlusPlus")} type="button">
                  github.com/BigPizzaV3/CodexPlusPlus
                  <ExternalLink aria-hidden="true" className="h-3 w-3" />
                </button>
              </dd>
            </div>
          </dl>
          <div className="app-about-links">
            <Button onClick={() => void actions.openExternalUrl("https://github.com/BigPizzaV3/CodexPlusPlus/issues")} size="sm" variant="secondary">
              <ExternalLink className="h-3.5 w-3.5" />
              {t("反馈问题")}
            </Button>
            <Button onClick={() => void actions.openExternalUrl("https://discord.gg/y96kX7A76v")} size="sm" variant="secondary">
              <MessageCircle className="h-3.5 w-3.5" />
              Discord
            </Button>
            <Button onClick={() => void actions.openExternalUrl("https://t.me/CodexPlusPlus")} size="sm" variant="secondary">
              <MessageCircle className="h-3.5 w-3.5" />
              Telegram
            </Button>
          </div>
        </CardContent>
      </Panel>
      <div className="app-settings-anchor" id="app-settings-logs"><LogsPanel logs={logs} actions={actions} /></div>
      <div className="app-settings-anchor" id="app-settings-diagnostics"><DiagnosticsPanel diagnostics={diagnostics} actions={actions} /></div>
    </>
  );
}

/// Agent 专属设置页。
///
/// 入口在侧栏各 Agent 自己的菜单里，只放跟随该 Agent 的选项：
/// - **Codex**：图片覆盖层、额外启动参数
/// - **Grok / Claude**：供应商与模型在各自页面里管理，这里给出跳转
///
/// 应用级选项（主题、配置备份、配置文件路径）在最左侧齿轮的「设置」里，
/// 见 [GlobalSettingsScreen]。语音输入和 Stepwise 的完整配置在 Codex增强 页面管理。
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
  return (
    <div className="settings-page">
      <SettingsAutoSave dirty={dirty} form={form} actions={actions} />
      <>
        <Panel id="codex-settings-overlay">
          <SwitchPanelHead
            checked={form.codexAppImageOverlayEnabled}
            detail={t("在当前 Codex 会话上叠加一张背景图")}
            label={t("启用 Codex 图片覆盖层")}
            onChange={(checked) => onFormChange({ ...form, codexAppImageOverlayEnabled: checked })}
            title={t("Codex 图片覆盖层")}
          />
          <CardContent className="settings-content setting-rows">
            {form.codexAppImageOverlayEnabled ? (
              <>
                <SettingRow title={t("覆盖图片")} description={t("支持 png / jpg / webp / gif / bmp。")}>
                  <div className="setting-inline">
                    <Input
                      value={form.codexAppImageOverlayPath}
                      onChange={(event) => onFormChange({ ...form, codexAppImageOverlayPath: event.currentTarget.value })}
                      placeholder={t("选择 png / jpg / webp / gif / bmp")}
                    />
                    <Button variant="secondary" onClick={() => void actions.chooseImageOverlayPath()}>
                      {t("选择图片")}
                    </Button>
                  </div>
                </SettingRow>
                <SettingRow title={t("透明度")} description={t("数值越大，背景图越明显。")}>
                  <div className="setting-inline setting-range">
                    <Input
                      min={1}
                      max={100}
                      type="range"
                      value={form.codexAppImageOverlayOpacity}
                      onChange={(event) =>
                        onFormChange({
                          ...form,
                          codexAppImageOverlayOpacity: clampNumber(Number(event.currentTarget.value), 1, 100),
                        })
                      }
                    />
                    <span className="setting-range-value">{form.codexAppImageOverlayOpacity}%</span>
                  </div>
                </SettingRow>
                <SettingRow title={t("背景适配方式")} description={t("图片和窗口比例不一致时如何铺放。")}>
                  <AppSelect
                    value={form.codexAppImageOverlayFitMode}
                    onChange={(value) =>
                      onFormChange({
                        ...form,
                        codexAppImageOverlayFitMode: value,
                      })
                    }
                    options={[
                      { value: "fill", label: t("填充") },
                      { value: "fit", label: t("适应") },
                      { value: "stretch", label: t("拉伸") },
                      { value: "tile", label: t("平铺") },
                      { value: "center", label: t("居中") },
                    ]}
                  />
                </SettingRow>
                <SettingRow title={t("恢复默认")} description={t("清除图片，并把透明度、适配方式恢复为默认值。")}>
                  <Button variant="secondary" onClick={() => void actions.resetImageOverlaySettings()}>
                    {t("重置背景")}
                  </Button>
                </SettingRow>
              </>
            ) : (
              <p className="setting-footnote">{t("已关闭：Codex 会话不叠加背景图。")}</p>
            )}
          </CardContent>
        </Panel>

        <Panel id="codex-settings-launch-args">
          <CardHead title={t("Codex 启动参数")} detail={t("启动 Codex App 时追加到默认 CDP 参数后。留空则保持默认启动行为。")} />
          <CardContent className="settings-content setting-rows">
            <SettingRow
              stacked
              title={t("额外参数")}
              description={t("每行一个参数，例如 --force_high_performance_gpu。不需要填写 open 或 --args。")}
            >
              <Textarea
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
            </SettingRow>
          </CardContent>
        </Panel>
      </>
    </div>
  );
}

/// 托盘快捷面板开关。值仍存在 gateway.json 的 `trayPanel`（老版本在网关设置里改过的选择照样有效），
/// 由后端命令读写，后端同时更新托盘行为和内存缓存。
function useTrayPanelSetting() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let cancelled = false;
    void invoke<boolean>("tray_panel_enabled")
      .then((value) => { if (!cancelled) setEnabled(value !== false); })
      .catch(() => { if (!cancelled) setEnabled(true); });
    return () => { cancelled = true; };
  }, []);
  const toggle = async () => {
    if (enabled === null || saving) return;
    setSaving(true);
    setError("");
    try {
      setEnabled(await invoke<boolean>("set_tray_panel_enabled", { enabled: !enabled }));
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
  dirty,
  form,
  settings,
  theme,
  actions,
  onOpenAgentSettings,
  onOpenTour,
}: {
  onOpenTour: () => void;
  dirty: boolean;
  form: BackendSettings;
  settings: SettingsResult | null;
  theme: Theme;
  actions: Actions;
  onOpenAgentSettings: () => void;
}) {
  const settingsPath = settings?.settings_path ?? "";
  const trayPanel = useTrayPanelSetting();
  // 每一项都是同一种行：左边标题 + 说明，右边操作。路径只在「配置文件路径」里出现一次。
  return (
    <div className="settings-page global-settings-page">
      <SettingsAutoSave dirty={dirty} form={form} actions={actions} />
      <Panel id="app-settings-general">
        <CardHead title={t("基础设置")} detail="" />
        <CardContent className="global-settings-list">
          <div className="global-setting-row">
            <div className="global-setting-copy">
              <strong>{t("界面主题")}</strong>
              <span>{t("当前为")}{theme === "dark" ? t("深色") : t("浅色")}{t("模式。")}</span>
            </div>
            <div className="global-setting-actions">
              <Button size="sm" variant="secondary" onClick={actions.toggleTheme}>{t("切换主题")}</Button>
            </div>
          </div>
          <div className="global-setting-row">
            <div className="global-setting-copy">
              <strong>{t("点击托盘图标弹出快捷面板")}</strong>
              <span>{trayPanel.error || t("面板里能看 Codex 状态、一键启动或重启、切换供应商、看用量，网关用户还能给各 Agent 换模型。右键仍是原来的菜单，双击打开主窗口；关掉恢复原来的点击行为。")}</span>
            </div>
            <div className="global-setting-actions">
              <Switch disabled={trayPanel.enabled === null || trayPanel.saving} label={t("点击托盘图标弹出快捷面板")} on={trayPanel.enabled ?? true} onToggle={() => void trayPanel.toggle()} />
            </div>
          </div>
          <div className="global-setting-row">
            <div className="global-setting-copy">
              <strong>{t("新手引导")}</strong>
              <span>{t("一步步认识网关、供应商池、Agent 接入和各个入口。")}</span>
            </div>
            <div className="global-setting-actions">
              <Button size="sm" variant="secondary" onClick={onOpenTour}>{t("重新打开引导")}</Button>
            </div>
          </div>
          <div className="global-setting-row">
            <div className="global-setting-copy">
              <strong>{t("Codex 专属设置")}</strong>
              <span>{t("图片覆盖层、启动参数等只作用于 Codex 的选项，在 Codex 菜单的「设置」里。")}</span>
            </div>
            <div className="global-setting-actions">
              <Button size="sm" variant="secondary" onClick={onOpenAgentSettings}>{t("打开 Codex 设置")}</Button>
            </div>
          </div>
        </CardContent>
      </Panel>
      <Panel id="app-settings-config">
        <CardHead title={t("配置文件")} detail={t("settings.json 的备份、迁移与存放位置")} />
        <CardContent className="global-settings-list">
          <div className="global-setting-row">
            <div className="global-setting-copy">
              <strong>{t("配置备份与迁移")}</strong>
              <span>{t("导出文件包含供应商 API Key 等敏感信息；导入只会读入草稿，检查后仍需点击保存设置。")}</span>
            </div>
            <div className="global-setting-actions">
              <Button size="sm" variant="secondary" onClick={() => void actions.exportSettings()}>{t("导出完整配置")}</Button>
              <Button size="sm" variant="secondary" onClick={() => void actions.importSettings()}>{t("导入配置草稿")}</Button>
            </div>
          </div>
          <div className="global-setting-row">
            <div className="global-setting-copy">
              <strong>{t("配置文件路径")}</strong>
              <code className="global-setting-path" title={settingsPath}>{settingsPath || "—"}</code>
              <span>{t("只迁移 settings.json；日志、备份、Skills 和运行状态仍留在原状态目录。更换后建议重新打开管理器。")}</span>
            </div>
            <div className="global-setting-actions">
              <Button size="sm" variant="secondary" onClick={() => void actions.chooseSettingsPath()}>{t("选择路径")}</Button>
              <Button size="sm" variant="secondary" onClick={() => void actions.resetSettingsPath()}>{t("恢复默认路径")}</Button>
            </div>
          </div>
        </CardContent>
      </Panel>
    </div>
  );
}

function LogsPanel({ logs, actions }: { logs: LogsResult | null; actions: Actions }) {
  const lines = splitLogLines(logs?.text ?? "");
  const logDetail = logs
    ? logs.truncated
      ? tf("日志大小 {0}，仅显示末尾 {1} 行", [formatBytes(logs.fileSize), logs.lines])
      : tf("日志大小 {0}", [formatBytes(logs.fileSize)])
    : "";
  return (
    <Panel>
      <CardHead title={t("最近日志")} detail={logs?.path ?? ""} />
      <CardContent>
        {logDetail ? <p className="field-hint">{logDetail}</p> : null}
        <div className="log-lines">
          {lines.length ? (
            lines.map((line, index) => (
              <div className="log-line" key={`${index}-${line.slice(0, 12)}`}>
                <span>{index + 1}</span>
                <code>{line || " "}</code>
              </div>
            ))
          ) : (
            <div className="empty">{t("暂无日志。")}</div>
          )}
        </div>
        <Toolbar>
          <Button onClick={() => void actions.refreshLogs()} size="sm" variant="secondary">{t("刷新")}</Button>
          <Button size="sm" variant="secondary" onClick={() => void actions.clearLogs()}>
            {t("清理日志")}
          </Button>
          <Button size="sm" variant="secondary" onClick={() => void actions.copyLogs()}>
            {t("复制")}
          </Button>
        </Toolbar>
      </CardContent>
    </Panel>
  );
}

function DiagnosticsPanel({ diagnostics, actions }: { diagnostics: DiagnosticsResult | null; actions: Actions }) {
  return (
    <Panel>
      <CardHead title={t("诊断报告")} detail={t("包含版本、路径、设置和平台信息")} />
      <CardContent>
        <Textarea className="log-view tall" readOnly value={diagnostics?.report ?? t("尚未生成诊断报告。")} />
        <Toolbar>
          <Button onClick={() => void actions.refreshDiagnostics()} size="sm" variant="secondary">{t("重新生成")}</Button>
          <Button size="sm" variant="secondary" onClick={() => void actions.copyDiagnostics()}>
            {t("复制报告")}
          </Button>
        </Toolbar>
      </CardContent>
    </Panel>
  );
}

function RelayProfileList({
  form,
  visibleProfiles = form.relayProfiles,
  onFormChange,
  onEdit,
  onSwitchProfile,
  disabled = false,
  actions,
}: {
  form: BackendSettings;
  visibleProfiles?: RelayProfile[];
  onFormChange: (value: BackendSettings) => void;
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
    if (next !== form) onFormChange(next);
  };
  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
      <SortableContext items={visibleProfiles.map((profile) => profile.id)} strategy={verticalListSortingStrategy}>
        <div className="relay-profile-list">
          {visibleProfiles.map((profile, index) => (
            <SortableRelayProfileCard
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
  actions,
}: {
  form: BackendSettings;
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
  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
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
        {gatewayManaged ? null : (
        <span className="relay-card-extra">
          <Button
            disabled={!multiLaunchSupported(profile) || actions.multiLaunchPending === profile.id}
            onClick={(event) => {
              event.stopPropagation();
              if (!multiLaunchSupported(profile)) return;
              void actions.launchMultiInstance(profile);
            }}
            size="icon"
            title={isAggregateRelayProfile(profile)
              ? t("聚合供应商暂不支持独立多开")
              : multiLaunchSupported(profile)
                ? t("启动独立 Codex 实例")
                : t("仅支持单一 Responses 或 Chat Completions 供应商，且不能配置模型路由")}
            variant="ghost"
          >
            <Rocket className="h-4 w-4" />
          </Button>
          <Button
            disabled={isAggregateRelayProfile(profile)}
            onClick={(event) => {
              event.stopPropagation();
              if (isAggregateRelayProfile(profile)) return;
              void actions.testRelayProfile(profile);
            }}
            size="icon"
            title={isAggregateRelayProfile(profile) ? t("聚合供应商会在真实对话中轮转成员，请测试成员供应商") : t("发送 hi 测试")}
            variant="ghost"
          >
            <TestTube className="h-4 w-4" />
          </Button>
          <Button
            onClick={(event) => {
              event.stopPropagation();
              onEdit(profile.id);
            }}
            size="icon"
            title={t("编辑")}
            variant="ghost"
          >
            <Edit3 className="h-4 w-4" />
          </Button>
          <Button
            onClick={(event) => {
              event.stopPropagation();
              onFormChange(duplicateRelayProfile(form, profile.id));
            }}
            size="icon"
            title={t("复制")}
            variant="ghost"
          >
            <Copy className="h-4 w-4" />
          </Button>
          <Button
            disabled={form.relayProfiles.length <= 1}
            onClick={(event) => {
              event.stopPropagation();
              const deleteIssue = relayProfileDeleteIssue(form, profile.id);
              if (deleteIssue) {
                void actions.showMessage(t("删除供应商"), deleteIssue, "failed");
                return;
              }
              onFormChange(removeRelayProfile(form, profile.id));
            }}
            size="icon"
            title={t("删除供应商")}
            variant="ghost"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
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

function MarketScriptCard({
  script,
  actions,
  view = "grid",
  installedKey,
}: {
  script: ScriptMarketItem;
  actions: Actions;
  view?: "grid" | "list";
  /// 已安装市场脚本在本地清单里的 key（`user:market-*.js`）。后端按 id 生成文件名，
  /// 前端不重算，直接取自 user_scripts 清单；为空表示未安装或不可删除。
  installedKey?: string;
}) {
  // 「未安装」是默认状态：满屏同一个徽标只是噪音，只给已安装 / 可更新打标。
  const status = script.updateAvailable ? t("可更新") : script.installed ? tf("已安装 {0}", [script.installedVersion]) : null;
  // 标签最多露 3 个，其余收成「+N」，否则卡片被撑成高低不齐的两三行。
  const visibleTags = script.tags.slice(0, 3);
  const hiddenTags = script.tags.slice(3);
  const isGitHubHomepage = script.homepage ? isGitHubRepositoryHomepage(script.homepage) : false;
  const githubSupportLabel = isGitHubHomepage ? tf("在 GitHub 上支持作者：{0}", [script.name]) : undefined;
  return (
    <div className="script-market-card" data-view={view}>
      <div className="script-market-title">
        <div>
          <strong>{script.name}</strong>
          <span>{script.author || t("未知作者")} · v{script.version}</span>
        </div>
        {status ? <UiBadge className="script-market-status" variant={script.updateAvailable ? "default" : "secondary"}>{status}</UiBadge> : null}
      </div>
      <p className="script-market-description">{script.description || t("暂无描述。")}</p>
      {/* 没有标签就不渲染空容器：空容器会让被拉伸的卡片在描述下露出半行（见 .script-market-card 样式注释）。 */}
      {script.tags.length ? <div className="script-market-tags">
        {visibleTags.map((tag) => (
          <span className="script-market-tag" key={tag}>{tag}</span>
        ))}
        {hiddenTags.length ? (
          <span className="script-market-tag script-market-tag-more" title={hiddenTags.join("、")}>+{hiddenTags.length}</span>
        ) : null}
      </div> : null}
      <div className="script-market-actions">
        <Button onClick={() => void actions.installMarketScript(script.id)} size="sm">
          <Download className="h-4 w-4" />
          {script.updateAvailable ? t("更新") : script.installed ? t("重新安装") : t("安装")}
        </Button>
        {script.homepage ? (
          <Button
            aria-label={githubSupportLabel}
            onClick={() => void actions.openExternalUrl(script.homepage)}
            size="sm"
            title={githubSupportLabel}
            variant="secondary"
          >
            {isGitHubHomepage ? (
              <>
                <Star className="h-4 w-4" />
                Star
                <ExternalLink className="h-3 w-3" />
              </>
            ) : (
              <>
                <ExternalLink className="h-4 w-4" />
                {t("主页")}
              </>
            )}
          </Button>
        ) : null}
        {installedKey ? (
          <Button onClick={() => void actions.deleteUserScript(installedKey)} size="sm" variant="outline">
            <Trash2 className="h-4 w-4" />
            {t("卸载")}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function RelayProfileDetail({
  profile,
  relayFiles,
  form,
  isNew = false,
  initialAggregateMode = null,
  onBack,
  onFormChange,
  onSaved,
  actions,
}: {
  profile: RelayProfile;
  relayFiles: RelayFilesResult | null;
  form: BackendSettings;
  isNew?: boolean;
  initialAggregateMode?: AggregateMode | null;
  onBack: () => void;
  onFormChange: (value: BackendSettings) => Promise<BackendSettings | null>;
  onSaved?: () => void;
  actions: Actions;
}) {
  const detailBodyRef = useRef<HTMLDivElement>(null);
  const [draft, setDraft] = useState<RelayProfile>(profile);
  const [modelWindowRows, setModelWindowRows] = useState<ModelWindowRow[]>(
    modelWindowRowsFromProfile(profile.modelList, profile.modelWindows || "", profile.modelVlm, profile.modelAutoCompact),
  );
  const [doctorResult, setDoctorResult] = useState<ProviderDoctorResult | null>(null);
  const [doctorOpen, setDoctorOpen] = useState(false);
  const [doctorRunning, setDoctorRunning] = useState(false);
  const [savingDraft, setSavingDraft] = useState(false);
  const [aggregateMode, setAggregateMode] = useState<AggregateMode | null>(() => {
    if (!isAggregateRelayProfile(profile)) return null;
    if (!isNew) return profile.aggregate?.modelAggregationEnabled === true ? "model" : "provider";
    // 从「故障转移 / 模型聚合」分页进入时类型已确定，不再弹选择器。
    return initialAggregateMode ?? null;
  });
  const isActive = !isNew && profile.id === form.activeRelayId;
  const profileUsesLiveFiles = relayProfileUsesLiveFiles(profile);
  // 只有当前供应商才从 live 文件回填；非当前供应商时不能把 relayFiles 当依赖——
  // live 文件一变就会重建整份模型行，编辑中的增删被无声撤销（保存后看起来「没生效」）。
  const liveConfigContents = isActive && profileUsesLiveFiles ? relayFiles?.configContents ?? "" : "";
  const liveAuthContents = isActive && profileUsesLiveFiles ? relayFiles?.authContents ?? "" : "";
  // 行编辑守卫：编辑器里还有未保存的模型行改动时，外部变化（元数据、live 文件、别处保存设置
  // 触发的重渲染）不许把用户的增删覆盖回已保存列表。
  const unsavedRowEditsRef = useRef(false);
  const lastDetailProfileIdRef = useRef(profile.id);
  useEffect(() => {
    const useLiveFiles = isActive && profileUsesLiveFiles && relayFiles;
    const liveDraft = isAggregateRelayProfile(profile)
      ? normalizeAggregateRelayProfile(profile, form)
      : deriveRelayProfileFromFiles(
          useLiveFiles
            ? {
              ...profile,
              configContents: relayFiles.configContents,
              authContents: relayAuthForLiveDraft(profile, relayFiles.authContents),
            }
            : profile,
        );
    const storedApiKey = useLiveFiles ? profile.apiKey.trim() : "";
    const nextDraft = useLiveFiles && !isAggregateRelayProfile(liveDraft)
      ? applyRelayProfilePatchToFiles(liveDraft, { apiKey: storedApiKey })
      : liveDraft;
    setDraft(nextDraft);
    const switchedProfile = lastDetailProfileIdRef.current !== profile.id;
    lastDetailProfileIdRef.current = profile.id;
    if (switchedProfile || !unsavedRowEditsRef.current) {
      setModelWindowRows(modelWindowRowsFromProfile(nextDraft.modelList, nextDraft.modelWindows || "", nextDraft.modelVlm, nextDraft.modelAutoCompact));
    }
    // 依赖里刻意不放 profile.modelMetadata / 原始 relayFiles：元数据改动是本编辑器自己触发的
    // （删行会清该模型元数据），把它当依赖会让「删行 → 清元数据 → 重建行」把删掉的行加回来。
  }, [profile.id, profile.modelList, profile.modelWindows, profile.modelAutoCompact, profile.modelVlm, profileUsesLiveFiles, isActive, isNew, liveConfigContents, liveAuthContents]);
  const validationSettings = relaySettingsWithDraft(form, profile.id, draft, isNew);
  const validationError = relaySessionProviderValidation(draft)
    ?? (isAggregateRelayProfile(draft)
      ? aggregateRelayProfileValidation(draft)
      : relayModelRoutesSettingsValidation(validationSettings));
  const modelRowsError = modelWindowRowsValidationMessage(modelWindowRowsValidationError(modelWindowRows));
  const customHeadersError = relayHeadersValidationMessage(profile.customHeaders || []);
  const draftWithModelRows = () => {
    const serializedRows = serializeModelWindowRows(modelWindowRows);
    const validSlugs = serializedRows.modelList.split("\n").map((slug) => slug.trim()).filter(Boolean);
    return {
      ...draft,
      modelList: serializedRows.modelList,
      modelWindows: serializedRows.modelWindows,
      modelAutoCompact: serializedRows.modelAutoCompact,
      modelMetadata: retainModelMetadataForSlugs(draft.modelMetadata, validSlugs),
      customHeaders: serializeRelayHeaders(draft.customHeaders || []),
      modelVlm: serializedRows.modelVlm,
    };
  };
  const currentModelState = draftWithModelRows();
  const persistedModelState = {
    modelList: profile.modelList || "",
    modelWindows: profile.modelWindows || "",
    modelAutoCompact: profile.modelAutoCompact || "",
    modelMetadata: profile.modelMetadata || "",
    modelVlm: profile.modelVlm || "",
  };
  const hasUnsavedModelChanges = JSON.stringify({
    modelList: currentModelState.modelList,
    modelWindows: currentModelState.modelWindows,
    modelAutoCompact: currentModelState.modelAutoCompact,
    modelMetadata: currentModelState.modelMetadata,
    modelVlm: currentModelState.modelVlm,
  }) !== JSON.stringify(persistedModelState);
  // 交给上面的行编辑守卫：有未保存行改动时，外部变化不覆盖模型行。
  unsavedRowEditsRef.current = hasUnsavedModelChanges;
  const saveDraft = async () => {
    if (savingDraft) return;
    // 配置有误不再静默 return：把原因弹出来（按钮也不再因此禁用，见头部那一行提示）。
    if (isNew && isAggregateRelayProfile(draft) && aggregateMode === null) {
      void actions.showMessage(t("保存供应商"), t("先选择聚合类型（模型聚合 / 故障转移）再保存。"), "failed");
      return;
    }
    const blockingIssue = validationError || modelRowsError;
    if (blockingIssue) {
      void actions.showMessage(t("保存供应商"), blockingIssue, "failed");
      return;
    }
    setSavingDraft(true);
    try {
      const draftWithWindows = draftWithModelRows();
      const normalizedDraft = isAggregateRelayProfile(draftWithWindows) ? normalizeAggregateRelayProfile(draftWithWindows, form) : deriveRelayProfileFromFiles(draftWithWindows);
      const next = normalizeSettings(isNew
        ? addRelayProfile(form, normalizedDraft)
        : updateRelayProfile(form, profile.id, normalizedDraft));
      const settingsValidationError = relaySettingsValidation(next);
      if (settingsValidationError) return;
      const activeLiveBaseUrl = codexBaseUrlFromConfig(
        relayFiles?.configContents ?? profile.configContents,
      );
      const requiresRestart = isActive && modelRouteSaveRequiresRestart(
        normalizeSettings(form),
        next,
        activeLiveBaseUrl,
      );
      if (requiresRestart && !(await appConfirm(t("此请求设置需要启动本地协议代理。保存后将立即重启 Codex，使设置安全生效。是否继续？"), { title: t("需要重启 Codex"), confirmText: t("保存并重启") }))) {
        return;
      }
      const savedSettings = await onFormChange(next);
      if (!savedSettings) return;
      if (requiresRestart) {
        const restarted = await actions.restart(true);
        if (!restarted) return;
        onSaved?.();
        return;
      }
      const savedProfile = savedSettings.relayProfiles.find((candidate) => candidate.id === normalizedDraft.id)
        ?? normalizedDraft;
      if (isActive && savedSettings.relayProfilesEnabled && relayProfileUsesLiveFiles(savedProfile)) {
        await actions.switchRelayProfile(savedSettings, savedSettings.activeRelayId);
      }
      onSaved?.();
    } finally {
      setSavingDraft(false);
    }
  };
  const switchDraft = () => {
    if (isNew || validationError || modelRowsError) return;
    const draftWithWindows = draftWithModelRows();
    const normalizedDraft = isAggregateRelayProfile(draftWithWindows) ? normalizeAggregateRelayProfile(draftWithWindows, form) : deriveRelayProfileFromFiles(draftWithWindows);
    const previousActiveRelayId = form.activeRelayId;
    const next = syncLegacyRelayFields({
      ...form,
      relayProfiles: form.relayProfiles.map((item) => (item.id === profile.id ? normalizedDraft : item)),
      activeRelayId: profile.id,
    });
    void actions.switchRelayProfile(next, previousActiveRelayId);
  };
  const runProviderDoctor = async () => {
    setDoctorOpen(true);
    setDoctorRunning(true);
    setDoctorResult(null);
    const draftWithWindows = draftWithModelRows();
    const result = await actions.diagnoseRelayProfile(deriveRelayProfileFromFiles(draftWithWindows));
    setDoctorResult(result);
    setDoctorRunning(false);
  };
  const aggregateProfile = isAggregateRelayProfile(draft);
  const aggregateModeRequired = isNew && aggregateProfile && aggregateMode === null;
  const selectAggregateMode = (mode: AggregateMode) => {
    if (!aggregateProfile) return;
    setAggregateMode(mode);
    setDraft(aggregateProfileForMode(draft, form, mode));
  };
  const showDoctor = !aggregateProfile && (draft.relayMode !== "official" || draft.officialMixApiKey);
  const detailStatus = aggregateProfile
    ? aggregateModeRequired
      ? t("先选择类型")
      : draft.aggregate?.modelAggregationEnabled === true
        ? t("模型聚合：模型固定绑定供应商")
        : t("故障转移：按策略分配请求")
    : relayProfileEditorStatus(draft, form, isNew);
  return (
    <div className="relay-detail-page" key={profile.id}>
      <div className="relay-detail-header">
        <div className="relay-editor-heading">
          <Button aria-label={t("返回列表")} onClick={() => void (async () => {
            if (hasUnsavedModelChanges && !(await appConfirm(t("有未保存的模型修改，确定放弃吗？"), { title: t("放弃修改"), confirmText: t("放弃"), danger: true }))) return;
            onBack();
          })()} size="icon" title={t("返回列表")} type="button" variant="ghost">
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <div className="relay-editor-heading-copy">
            <strong>{draft.name || (aggregateProfile ? t("未命名聚合供应商") : t("未命名供应商"))}</strong>
            <span>{hasUnsavedModelChanges ? `${detailStatus} · ${t("有未保存修改")}` : detailStatus}</span>
            {validationError || modelRowsError ? (
              // 就地写出「为什么存不了」：按钮不再禁用，这里也给人一眼能看到的原因。
              <span className="relay-editor-validation" role="alert">
                {validationError || modelRowsError}
              </span>
            ) : null}
          </div>
        </div>
        <div className="relay-editor-actions">
          {showDoctor ? (
            <Button disabled={doctorRunning} onClick={() => void runProviderDoctor()} type="button" variant="secondary">
              <Stethoscope className="h-4 w-4" />
              {doctorRunning ? t("诊断中") : t("诊断供应商")}
            </Button>
          ) : null}
          {aggregateProfile ? (
            <UiBadge variant="secondary">{aggregateModeRequired ? t("聚合")
              : draft.aggregate?.modelAggregationEnabled === true ? t("模型聚合") : t("故障转移")}</UiBadge>
          ) : isNew ? null : (
            <Button
              disabled={actions.relaySwitching}
              onClick={switchDraft}
              title={!form.relayProfilesEnabled ? t("供应商配置总开关已关闭，点击后按提示开启") : actions.relaySwitching ? t("供应商切换中") : undefined}
              variant={draft.id === form.activeRelayId ? "secondary" : "default"}
            >
              {actions.relaySwitching ? t("切换中") : draft.id === form.activeRelayId ? t("使用中") : t("设为当前")}
            </Button>
          )}
          <Button
            // 配置有误时也让人点：点一下把原因弹出来，比一个点不动的按钮清楚。
            disabled={savingDraft}
            onClick={() => void saveDraft()}
            title={aggregateModeRequired ? t("先选择类型") : validationError || modelRowsError || t("保存")}
            type="button"
          >
            <Save className="h-4 w-4" />
            {savingDraft ? t("保存中") : t("保存供应商")}
          </Button>
        </div>
      </div>
      {!aggregateModeRequired && !isAggregateRelayProfile(draft) ? (
        // 编辑页很长：标题栏下给一排分区跳转，点哪个就展开该分区并滚过去。
        <nav className="relay-section-nav" aria-label={t("跳转到分区")}>
          {relaySectionLinks().map((link) => (
            <button key={link.id} onClick={() => jumpToRelaySection(detailBodyRef.current, link.id)} type="button">
              {link.label}
            </button>
          ))}
        </nav>
      ) : null}
      <div className="relay-detail-body" ref={detailBodyRef}>
        {aggregateModeRequired ? (
          <AggregateModeChooser onSelect={selectAggregateMode} />
        ) : (
          <RelayProfileEditor
            profile={draft}
            form={form}
            isNew={isNew}
            onProfileChange={setDraft}
            actions={actions}
            modelWindowRows={modelWindowRows}
            setModelWindowRows={setModelWindowRows}
          />
        )}
        {isAggregateRelayProfile(draft) ? null : (
        <RelayFileEditors
          contextProfile={profile}
          profile={draft}
          form={form}
          isActive={isActive}
          profileId={profile.id}
          onFormChange={onFormChange}
          onProfileChange={setDraft}
          actions={actions}
        />
        )}
      </div>
      {doctorOpen ? (
        <ProviderDoctorModal
          result={doctorResult}
          running={doctorRunning}
          onClose={() => {
            if (!doctorRunning) setDoctorOpen(false);
          }}
        />
      ) : null}
    </div>
  );
}

function ContextScreen({
  form,
  liveEntries,
  relayFiles,
  onFormChange,
  actions,
}: {
  form: BackendSettings;
  liveEntries: CodexContextEntries | null;
  relayFiles: RelayFilesResult | null;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  return (
    <Panel fill>
      <CardHead title={t("Codex MCP&插件")} detail={t("独立管理 Codex 的 MCP 服务器与插件；切换任意供应商都会带上。")} />
      <CardContent>
        <RelayContextManager
          form={normalizeSettings(form)}
          liveEntries={liveEntries}
          relayFiles={relayFiles}
          onFormChange={onFormChange}
          actions={actions}
        />
      </CardContent>
    </Panel>
  );
}

type SortableModelWindowEntryProps = {
  id: string;
  children: (sortable: ReturnType<typeof useSortable>) => ReactNode;
};

function SortableModelWindowEntry({ id, children }: SortableModelWindowEntryProps) {
  const sortable = useSortable({ id });
  const style: CSSProperties = {
    transform: CSS.Transform.toString(sortable.transform),
    transition: sortable.transition,
  };

  return (
    <div
      className={`relay-model-entry ${sortable.isDragging ? "dragging" : ""}`}
      ref={sortable.setNodeRef}
      style={style}
    >
      {children(sortable)}
    </div>
  );
}

/// 编辑页分区导航的条目。id 与 RelayFold 的 sectionId 一致；`basic` 是不可折叠的基础设置。
function relaySectionLinks() {
  return [
    { id: "basic", label: t("基础") },
    { id: "models", label: t("模型") },
    { id: "requests", label: t("请求") },
    { id: "advanced", label: t("更多选项") },
    { id: "config-preview", label: t("配置预览") },
  ];
}

/// 跳到编辑页某个分区：折叠着就先展开（会触发 RelayFold 记住展开状态），再滚到分区顶部。
function jumpToRelaySection(root: HTMLElement | null, id: string) {
  if (!root) return;
  const target = id === "basic"
    ? root.querySelector<HTMLElement>(".relay-basic-section")
    : root.querySelector<HTMLElement>(`details[data-section="${id}"]`);
  if (!target) return;
  if (target instanceof HTMLDetailsElement && !target.open) target.open = true;
  requestAnimationFrame(() => target.scrollIntoView({ behavior: "smooth", block: "start" }));
}

function RelayFold({ title, children, className = "", bodyClassName = "", issue, sectionId, defaultOpen = false }: {
  title: string;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
  issue?: string | null;
  sectionId?: string;
  defaultOpen?: boolean;
}) {
  const preferenceKey = sectionId ? `codex-plus-relay-fold:${sectionId}` : null;
  const [open, setOpen] = useState(() => {
    if (issue) return true;
    try {
      const stored = preferenceKey ? window.localStorage.getItem(preferenceKey) : null;
      if (stored === "true" || stored === "false") return stored === "true";
    } catch {
      // 无法使用本地存储时仍可展开、编辑并保存供应商草稿。
    }
    return defaultOpen;
  });
  const previousIssue = useRef(issue);
  useEffect(() => {
    // 同一项错误更新文字时不抢回用户刚收起的面板，新出现的错误才展开。
    if (issue && !previousIssue.current) setOpen(true);
    previousIssue.current = issue;
  }, [issue]);
  return (
    <details className={`relay-fold ${className}`.trim()} data-section={sectionId} open={open} onToggle={(event) => {
      const next = event.currentTarget.open;
      if (next === open) return;
      setOpen(next);
      if (preferenceKey) {
        try { window.localStorage.setItem(preferenceKey, String(next)); } catch { /* 偏好不可写不影响编辑。 */ }
      }
    }}>
      <summary className="relay-fold-summary">
        <ChevronDown aria-hidden="true" className="relay-fold-chevron h-4 w-4" />
        <strong>{title}</strong>
        {issue ? <span className="relay-fold-issue" title={issue}>{t("需要检查")}</span> : null}
      </summary>
      <div className={`relay-fold-body ${bodyClassName}`.trim()}>{children}</div>
    </details>
  );
}

// 模型行级内置元数据索引：窗口/压缩用于回填，「思考等级」列用作内置初值。
type BuiltinMetadataIndexEntry = {
  source: string;
  context_window: unknown;
  auto_compact_token_limit: unknown;
  supported_reasoning_levels?: unknown;
};

function RelayProfileEditor({
  profile,
  form,
  isNew = false,
  onProfileChange,
  actions,
  modelWindowRows,
  setModelWindowRows,
}: {
  profile: RelayProfile;
  form: BackendSettings;
  isNew?: boolean;
  onProfileChange: (value: RelayProfile) => void;
  actions: Actions;
  modelWindowRows: ModelWindowRow[];
  setModelWindowRows: (value: ModelWindowRow[]) => void;
}) {
  const [vlmTestOpen, setVlmTestOpen] = useState(false);
  const [relayProxyUrl, setRelayProxyUrl] = useState("");
  const [relayProxyBusy, setRelayProxyBusy] = useState(false);
  const relayProxyProfileRef = useRef(profile);
  relayProxyProfileRef.current = profile;
  const relayProxyRevisionRef = useRef(0);
  const [stabilizePromptPrefix, setStabilizePromptPrefix] = useState(false);
  // 「高级兼容选项」分组：null = 用户没手动开合过，按是否有开启项决定。
  const [compatOpen, setCompatOpen] = useState<boolean | null>(null);
  const compatEnabledCount = [
    profile.standardOpenaiProtocol,
    stabilizePromptPrefix,
    profile.webSearchHistoryCompat,
    profile.localCompactionPromptAtEnd === true,
    profile.trustUpstreamCatalogLite === true,
    profile.strictResponsesReasoning === true,
  ].filter(Boolean).length;
  const [stabilizePromptPrefixBusy, setStabilizePromptPrefixBusy] = useState(false);
  const useCommonConfig = profile.useCommonConfig !== false;
  useEffect(() => {
    let cancelled = false;
    const revision = ++relayProxyRevisionRef.current;
    setRelayProxyUrl("");
    setIndependentProxyConfigured(profile.id, false);
    void invoke<{ status: string; proxyUrl?: string; configured?: boolean }>("load_relay_proxy", { profileId: profile.id })
      .then((result) => {
        if (!cancelled && revision === relayProxyRevisionRef.current && result.status === "ok") {
          setIndependentProxyConfigured(profile.id, result.configured ?? Boolean(result.proxyUrl?.trim()));
          setRelayProxyUrl(result.proxyUrl || "");
        }
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [profile.id]);
  useEffect(() => {
    let cancelled = false;
    setStabilizePromptPrefix(false);
    void invoke<{ stabilizePromptPrefix?: boolean }>("load_relay_feature", { profileId: profile.id })
      .then((result) => { if (!cancelled) setStabilizePromptPrefix(result.stabilizePromptPrefix === true); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [profile.id]);
  const saveRelayProxy = async () => {
    const profileId = profile.id;
    ++relayProxyRevisionRef.current;
    setRelayProxyBusy(true);
    try {
      const result = await invoke<{ status: string; message: string; configured?: boolean }>("set_relay_proxy", {
        profileId,
        proxyUrl: relayProxyUrl,
      });
      if (result.status === "ok") {
        setIndependentProxyConfigured(profileId, result.configured ?? Boolean(relayProxyUrl.trim()));
        const current = relayProxyProfileRef.current;
        if (current.id === profileId) {
          // 保存响应返回时从最新草稿重算，保留期间编辑的其它字段。
          onProfileChange(applyRelayProfilePatchToFiles(current, { baseUrl: current.baseUrl }, { allowGenerateFiles: isNew }));
        }
      }
      await actions.showMessage(t("供应商独立代理"), result.message, result.status === "ok" ? "ok" : "failed");
    } catch (error) {
      await actions.showMessage(t("供应商独立代理"), String(error), "failed");
    } finally {
      setRelayProxyBusy(false);
    }
  };
  const saveStabilizePromptPrefix = async (enabled: boolean) => {
    setStabilizePromptPrefix(enabled);
    setStabilizePromptPrefixBusy(true);
    try {
      const result = await invoke<{ status: string; message: string }>("set_relay_feature", {
        profileId: profile.id,
        stabilizePromptPrefix: enabled,
      });
      await actions.showMessage(t("前缀稳定层"), result.message, result.status === "ok" ? "ok" : "failed");
    } catch (error) {
      setStabilizePromptPrefix(!enabled);
      await actions.showMessage(t("前缀稳定层"), String(error), "failed");
    } finally {
      setStabilizePromptPrefixBusy(false);
    }
  };
  const [activeImportDraft, setActiveImportDraft] = useState<ActiveImportDraft | null>(null);
  const [metadataImportError, setMetadataImportError] = useState("");
  const [builtinMatch, setBuiltinMatch] = useState<BuiltinModelMetadataMatch | null>(null);
  const [builtinMatchSlug, setBuiltinMatchSlug] = useState("");
  const [builtinQueryState, setBuiltinQueryState] = useState<BuiltinMetadataQueryState | null>(null);
  const [importPrefillSource, setImportPrefillSource] = useState<"builtin" | "existing" | null>(null);
  const [builtinIndex, setBuiltinIndex] = useState<Map<string, BuiltinMetadataIndexEntry>>(new Map());
  const modelSensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  // 面板内置查询的请求代数：begin/rematch 是命令式调用（没有 effect cleanup
  // 的 cancelled 通道），响应返回时代数不匹配即丢弃全部 setState——防止迟到
  // 响应把已取消的面板重新打开，或覆盖用户改名后的新查询结果。
  const builtinQuerySeqRef = useRef(0);
  const queryBuiltinCommand = async <T,>(command: string, args?: Record<string, unknown>): Promise<T | null> => {
    try { return await invoke<T>(command, args); } catch (error) {
      // 索引只影响行级标记与便利回填（失败是假阴性、不产错误数据），可诊断即可。
      console.warn(`[Codex++] ${command} 查询失败`, error);
      return null;
    }
  };
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const result = await queryBuiltinCommand<{ entries: Array<BuiltinMetadataIndexEntry & { slug: string }> }>("builtin_model_metadata_index");
      if (cancelled || !result?.entries) return;
      const map = new Map<string, BuiltinMetadataIndexEntry>();
      for (const entry of result.entries) {
        map.set(modelMetadataKey(entry.slug), {
          source: entry.source,
          context_window: entry.context_window,
          auto_compact_token_limit: entry.auto_compact_token_limit,
          supported_reasoning_levels: entry.supported_reasoning_levels,
        });
      }
      setBuiltinIndex(map);
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // 导入区打开期间模型名被修改：标签与内置预填实时跟随新名字（覆盖
  // 「创建模型后改名」「编辑模型名」场景，而不是沿用旧名字的匹配结果）。
  // 面板身份只用 draft.index；行已删除时回退打开面板时的行名兜底。
  const activeImportSlug = activeImportDraft
    ? modelWindowRows[activeImportDraft.index]?.model.trim() ?? activeImportDraft.originalSlug
    : "";
  const metadataImportTarget = activeImportDraft;
  const metadataImportDocument = activeImportDraft?.document ?? "";
  const metadataImportPreview = activeImportDraft?.preview ?? null;
  const setMetadataImportDocument = (document: string) =>
    setActiveImportDraft((draft) => draft ? updateActiveImportDraft(draft, { document }) : draft);
  const setMetadataImportPreview = (preview: ImportedModelMetadata | null) =>
    setActiveImportDraft((draft) => draft ? updateActiveImportDraft(draft, { preview }) : draft);
  useEffect(() => {
    if (!metadataImportTarget || !activeImportSlug || activeImportSlug === builtinMatchSlug) return;
    let cancelled = false;
    void (async () => {
      let match: BuiltinModelMetadataMatch | null = null;
      try {
        const result = await invoke<BuiltinModelMetadataMatch>("query_builtin_model_metadata", { slug: activeImportSlug });
        // 迟到响应守卫：改名后旧查询一律丢弃（含错误态），否则旧 error 会把
        // 当前正确匹配的徽标误藏成回退/错误态（cancelled 只由 cleanup 置位，
        // cleanup 跑过必有新 run 接手，不存在「最后一次响应被误丢」）。
        if (cancelled) return;
        const state = builtinMetadataQueryState(result);
        setBuiltinQueryState(state);
        match = state.status === "error" ? null : state.value;
      } catch (error) {
        if (cancelled) return;
        const state = builtinMetadataQueryState(null, error);
        setBuiltinQueryState(state);
        setMetadataImportError(state.status === "error" ? state.error : "");
      }
      if (cancelled) return;
      setBuiltinMatch(match);
      setBuiltinMatchSlug(activeImportSlug);
      // 内置预填态（用户尚未编辑）跟随新名字重新预填；已编辑/自有内容不动。
      if (importPrefillSource === "builtin" && match?.matched && match.entry) {
        const document = builtinEntryToImportDocument(match.entry, modelSlugFromRowName(activeImportSlug));
        // 文档写的是后端返回的规范 slug，匹配时也要用规范 slug（剥掉 [1M] 后缀），
        // 否则带后缀的行名永远匹配不到，面板一打开就报「找不到 slug」。
        const preview = parseModelMetadataDocument(document, modelSlugFromRowName(activeImportSlug));
        setMetadataImportDocument(document);
        setMetadataImportError("");
        setMetadataImportPreview(preview.ok ? preview.value : null);
        if (preview.ok && preview.value.contextWindow && !suffixWindowString(activeImportSlug)) {
          updateModelWindowRow(metadataImportTarget.index, { window: preview.value.contextWindow });
        }
      }
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeImportSlug, builtinMatchSlug, metadataImportTarget, importPrefillSource]);

  const modelSlugOriginsRef = useRef(modelWindowRows.map((row) => row.model.trim()));
  useEffect(() => {
    modelSlugOriginsRef.current = modelWindowRows.map((row) => row.model.trim());
  }, [profile.id, profile.modelList]);
  const importedModelMetadata = useMemo(
    () => parseModelMetadataMap(profile.modelMetadata),
    [profile.modelMetadata],
  );
  // 当前内置匹配条目的「全字段」基线：用来判断面板里的内容是否只是内置的复刻。
  // 与预填走同一管道（同行名、同 slug 文本），并保留窗口/压缩字段——窗口/
  // 压缩偏离内置值同样是用户编辑，保存应落自定义；写 map 仍走白名单过滤。
  const builtinMetadata = useMemo(() => {
    if (!builtinMatch?.matched || !builtinMatch.entry) return null;
    const rowName = activeImportSlug || builtinMatchSlug || builtinMatch.entry.slug;
    const canonical = modelSlugFromRowName(rowName);
    const parsed = parseModelMetadataDocument(builtinEntryToImportDocument(builtinMatch.entry, canonical), canonical);
    return parsed.ok ? parsed.value.documentEntry : null;
  }, [builtinMatch, activeImportSlug, builtinMatchSlug]);
  // VLM/Strip 对 Chat Completions 与 Responses 协议均可用(注入块类型已按协议适配)。
  const vlmUnsupportedProtocol = false;
  if (isAggregateRelayProfile(profile)) {
    return (
      <AggregateRelayProfileEditor
        profile={profile}
        form={form}
        onProfileChange={onProfileChange}
      />
    );
  }

  const editorMode = relayEditorMode(profile);
  const showApiFields = editorMode !== "official";
  const sessionProvider = relaySessionProvider(profile);
  const canUseOpenAiSessionProvider = profile.relayMode !== "official" || profile.officialMixApiKey;
  const updateDraft = (patch: Partial<RelayProfile>) => {
    onProfileChange(applyRelayProfilePatchToFiles(profile, patch, { allowGenerateFiles: isNew }));
  };
  const normalizedApiKeys = normalizeRelayApiKeys(profile);
  const updateApiKeys = (apiKeys: RelayApiKey[], activeApiKeyId = normalizedApiKeys.activeApiKeyId) => {
    const selected = apiKeys.find((entry) => entry.id === activeApiKeyId) || apiKeys[0];
    updateDraft({ apiKeys, activeApiKeyId: selected?.id || "", apiKey: selected?.apiKey || "" });
  };
  const selectEditorMode = (mode: RelayEditorMode) => {
    if (mode === "official") {
      updateDraft({ relayMode: "official", officialMixApiKey: false });
      return;
    }
    if (mode === "mixedApi") {
      updateDraft({ relayMode: "official", officialMixApiKey: true });
      return;
    }
    updateDraft({ relayMode: "pureApi", officialMixApiKey: false });
  };
  const modelRoutes = normalizeRelayModelRoutes(profile.modelRoutes);
  const modelRouteTargets = form.relayProfiles.filter(
    (candidate) => candidate.id !== profile.id && !isAggregateRelayProfile(candidate) && candidate.protocol === "responses",
  );
  const updateModelRoute = (index: number, patch: Partial<RelayModelRoute>) => {
    updateDraft({
      modelRoutes: modelRoutes.map((route, routeIndex) => (routeIndex === index ? { ...route, ...patch } : route)),
    });
  };
  const commitModelMetadata = (modelMetadata: string) => {
    updateDraft({ modelMetadata });
  };
  const updateModelWindowRow = (index: number, patch: Partial<ModelWindowRow>) => {
    setModelWindowRows(
      modelWindowRows.map((row, rowIndex) => (rowIndex === index ? { ...row, ...patch } : row)),
    );
  };
  // 表头「全部设为」：十几行模型的压缩比例 / 图片处理通常一样，不必逐行改。
  const updateAllModelWindowRows = (patch: Partial<ModelWindowRow>) => {
    setModelWindowRows(modelWindowRows.map((row) => ({ ...row, ...patch })));
  };
  // 内置命中 → 回填空窗口/压缩列的统一入口：上游获取、手动提交行名、打开导入
  // 面板都走这一条规则，不再只有「重新匹配」才回填（裁决见 builtinRowBackfillValue）。
  const backfillRowFromBuiltin = (index: number, slug: string) => {
    const row = modelWindowRows[index];
    if (!row) return;
    const patch = builtinRowBackfillValue(slug, row.window, row.autoCompact, builtinIndex.get(modelMetadataKey(slug)));
    if (patch.window || patch.autoCompact) {
      updateModelWindowRow(index, {
        ...(patch.window ? { window: patch.window } : {}),
        ...(patch.autoCompact ? { autoCompact: patch.autoCompact } : {}),
      });
    }
  };
  const resolvePendingModelSlugRenames = (
    rows: ModelWindowRow[],
    origins: string[],
    modelMetadata: string,
  ) => {
    const slugs = rows.map((row) => row.model.trim()).filter(Boolean);
    if (new Set(slugs).size !== slugs.length) return { modelMetadata, origins };
    const nextOrigins = rows.map((row, index) => row.model.trim() || origins[index] || "");
    return {
      modelMetadata: remapModelMetadataSlugs(
        modelMetadata,
        rows.map((row, index) => ({
          previousSlug: origins[index] || "",
          nextSlug: row.model,
        })),
      ),
      origins: nextOrigins,
    };
  };
  const commitModelSlug = (index: number) => {
    const nextSlug = modelWindowRows[index]?.model.trim() ?? "";
    if (!nextSlug) return;
    // 面板身份只用 index（activeImportSlug 实时跟随输入框），改名只需在这里
    // 迁移 metadata map 的 key；draft 不再持有 slug 副本，避免双轨漂移。
    const resolved = resolvePendingModelSlugRenames(
      modelWindowRows,
      modelSlugOriginsRef.current,
      profile.modelMetadata,
    );
    modelSlugOriginsRef.current = resolved.origins;
    if (resolved.modelMetadata !== profile.modelMetadata) commitModelMetadata(resolved.modelMetadata);
    // 手动新增/改名提交即同步：内置命中且窗口/压缩列为空时立即回填
    backfillRowFromBuiltin(index, nextSlug);
  };
  const closeModelMetadataImport = () => {
    // 作废所有在飞的命令式内置查询（begin/rematch）：面板已关，迟到响应
    // 不得再把面板重新打开或写入陈旧匹配状态。
    builtinQuerySeqRef.current += 1;
    setActiveImportDraft(null);
    setMetadataImportError("");
  };
  const handleModelRowsDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const activeIndex = Number(String(active.id).replace("model-row-", ""));
    const overIndex = Number(String(over.id).replace("model-row-", ""));
    if (!Number.isInteger(activeIndex) || !Number.isInteger(overIndex)) return;
    const nextRows = reorderModelWindowRows(modelWindowRows, activeIndex, overIndex);
    if (nextRows === modelWindowRows) return;
    // 导入面板的身份按行索引保存；排序时关闭它，避免面板跟着错误的行移动。
    if (activeImportDraft) closeModelMetadataImport();
    const nextOrigins = [...modelSlugOriginsRef.current];
    const [movedOrigin] = nextOrigins.splice(activeIndex, 1);
    nextOrigins.splice(overIndex, 0, movedOrigin);
    modelSlugOriginsRef.current = nextOrigins;
    setModelWindowRows(nextRows);
  };
  const cancelModelMetadataImport = () => {
    if (activeImportDraft) {
      const cancelled = cancelActiveImportDraft(activeImportDraft);
      updateModelWindowRow(activeImportDraft.index, {
        window: cancelled.rowPatch.window,
        autoCompact: cancelled.rowPatch.autoCompact,
      });
    }
    closeModelMetadataImport();
  };
  const beginModelMetadataImport = async (index: number, slug: string) => {
    // 配置可能还挂在「改名尚未提交」的旧 key 下（resolvePendingModelSlugRenames
    // 的 previousSlug 同源），按行解析而不是按实时名硬查。
    const existingKey = resolveModelMetadataRowKey(importedModelMetadata, {
      current: slug,
      origin: modelSlugOriginsRef.current[index],
    });
    const existingMetadata = existingKey ? importedModelMetadata[existingKey] : undefined;
    const existingDocument = existingMetadata
      ? serializeModelMetadataDocument(
          // 文档 slug 必须与解析目标（剥后缀的行名）一致，否则带 [1M] 的行
          // 现有配置文档永远匹配不上，保存键被解析失败锁死。
          modelSlugFromRowName(slug),
          existingMetadata,
          modelWindowRows[index]?.window ?? "",
          modelWindowRows[index]?.autoCompact ?? "",
        )
      : "";
    // 无论有无已导入配置都查询内置匹配（标签需要准确的匹配状态）；
    // 无已导入配置且命中内置时，把内置条目预填为可编辑底稿（预填 ≠ 导入）。
    let match: BuiltinModelMetadataMatch | null = null;
    let metadataQueryError = "";
    if (slug.trim()) {
      const querySeq = ++builtinQuerySeqRef.current;
      try {
        const result = await invoke<BuiltinModelMetadataMatch>("query_builtin_model_metadata", { slug });
        // 代数守卫：await 期间面板被取消或被另一次 begin/rematch 抢占
        // （seq 已变）时，本次响应整体丢弃——迟到响应不得重开已关闭的面板。
        if (querySeq !== builtinQuerySeqRef.current) return;
        const state = builtinMetadataQueryState(result);
        setBuiltinQueryState(state);
        match = state.status === "error" ? null : state.value;
      } catch (error) {
        if (querySeq !== builtinQuerySeqRef.current) return;
        const state = builtinMetadataQueryState(null, error);
        setBuiltinQueryState(state);
        metadataQueryError = state.status === "error" ? state.error : "";
      }
    }
    let document = existingDocument;
    if (!document && match?.matched && match.entry) {
      document = builtinEntryToImportDocument(match.entry, modelSlugFromRowName(slug));
      setImportPrefillSource("builtin");
      // 打开面板即同步：内置条目的窗口/压缩回填空列（后缀与已填值不覆盖，
      // 与上游获取/手动提交同一规则）
      const backfill = builtinRowBackfillValue(
        slug,
        modelWindowRows[index]?.window ?? "",
        modelWindowRows[index]?.autoCompact ?? "",
        match.entry,
      );
      if (backfill.window || backfill.autoCompact) {
        updateModelWindowRow(index, {
          ...(backfill.window ? { window: backfill.window } : {}),
          ...(backfill.autoCompact ? { autoCompact: backfill.autoCompact } : {}),
        });
      }
    } else {
      setImportPrefillSource(existingMetadata ? "existing" : null);
    }
    setBuiltinMatch(match);
    setBuiltinMatchSlug(slug);
    const existingPreview = document
      ? parseModelMetadataDocument(document, modelSlugFromRowName(slug))
      : null;
    setActiveImportDraft(createActiveImportDraft({
      index,
      rowName: slug,
      window: modelWindowRows[index]?.window ?? "",
      autoCompact: modelWindowRows[index]?.autoCompact ?? "",
      document,
      preview: existingPreview?.ok ? existingPreview.value : null,
    }));
    setMetadataImportError(metadataQueryError);
  };
  const applyModelMetadataImport = () => {
    if (!metadataImportTarget) return;
    // 空文档是合法保存输入（清除 → 保存 = 放弃自定义回退内置）；解析失败才无内容可存。
    const documentBlank = !metadataImportDocument.trim();
    if (!metadataImportPreview && !documentBlank) return;
    // 保存跟当前行名（改名后保存写回新 slug，不存旧名）
    const slug = modelWindowRows[metadataImportTarget.index]?.model.trim()
      || metadataImportPreview?.slug || "";
    const key = modelMetadataKey(slug);
    const decision = importSaveDecision({
      parseOk: true,
      documentBlank,
      imported: Boolean(importedModelMetadata[key]),
      // 内容与内置全字段一致时目标态就是「用内置」，不写自定义覆盖——
      // 否则「重新匹配后保存」会把内置数据复制成一份自定义配置。
      matchesBuiltin: metadataImportPreview && !(importPrefillSource === "existing"
        && modelFastSupportMode(metadataImportPreview.metadata) !== "inherit")
        ? metadataMatchesBuiltin(metadataImportPreview.documentEntry, builtinMetadata)
        : false,
    });
    if (!decision.needsSave) {
      closeModelMetadataImport();
      return;
    }
    if (decision.effect === "builtin") {
      commitModelMetadata(clearModelMetadataForSlug(profile.modelMetadata, slug));
      closeModelMetadataImport();
      return;
    }
    if (!metadataImportPreview) {
      closeModelMetadataImport();
      return;
    }
    commitModelMetadata(replaceModelMetadataForSlug(
      profile.modelMetadata,
      slug,
      metadataImportPreview.metadata,
    ));
    // 保存的窗口/压缩写回与实时同步规则一致：解析有值才写，null 不写，
    // 不再 fallback 回导入前的旧值（保存把用户清空的窗口又写回旧值）。
    const row = modelWindowRows[metadataImportTarget.index];
    if (row) {
      const patch = importDocumentSyncPatch(row, metadataImportPreview);
      if (suffixWindowString(row.model)) delete patch.window;
      if (patch.window !== undefined || patch.autoCompact !== undefined) {
        updateModelWindowRow(metadataImportTarget.index, patch);
      }
    }
    closeModelMetadataImport();
  };
  // 「重新匹配」：按当前模型名重查内置元数据并重填下方内容（含实时写回行窗口）。
  const rematchBuiltinImport = async (slug: string) => {
    setImportPrefillSource("builtin");
    if (!slug.trim()) return;
    const querySeq = ++builtinQuerySeqRef.current;
    let match: BuiltinModelMetadataMatch | null = null;
    try {
      const result = await invoke<BuiltinModelMetadataMatch>("query_builtin_model_metadata", { slug });
      // 代数守卫：连点重新匹配或面板已关闭时，旧响应整体丢弃。
      if (querySeq !== builtinQuerySeqRef.current) return;
      const state = builtinMetadataQueryState(result);
      setBuiltinQueryState(state);
      match = state.status === "error" ? null : state.value;
    } catch (error) {
      if (querySeq !== builtinQuerySeqRef.current) return;
      const state = builtinMetadataQueryState(null, error);
      setBuiltinQueryState(state);
      setMetadataImportError(state.status === "error" ? state.error : "");
    }
    setBuiltinMatch(match);
    // 未命中也要同步 slug：否则跟随 effect 的「slug 未变」早退会挡住后续查询，
    // 用户把名字改对后按钮状态不更新。
    setBuiltinMatchSlug(slug);
    if (!match?.matched || !match.entry) return;
    // 重新匹配只替换当前 draft；窗口后缀是用户显式意图，不能被内置值覆盖。
    const document = builtinEntryToImportDocument(match.entry, modelSlugFromRowName(slug));
    const preview = parseModelMetadataDocument(document, modelSlugFromRowName(slug));
    if (activeImportDraft) {
      setActiveImportDraft(rematchActiveImportDraft(
        activeImportDraft, document, preview.ok ? preview.value : null,
      ));
    }
    setMetadataImportError("");
    if (metadataImportTarget && preview.ok && preview.value.contextWindow
      && !suffixWindowString(slug)) {
      updateModelWindowRow(metadataImportTarget.index, { window: preview.value.contextWindow });
    }
  };
  // 「清除」：只清空面板文档（draft 内容），面板保持打开——不摘已保存配置、
  // 不回滚窗口/压缩列、不关闭面板。回到内置走 清除 → 重新匹配 → 保存（内容=
  // 内置 → 目标态内置）；粘贴最新供应商元数据也以清空后的面板为起点。
  const clearImportDocument = () => {
    setActiveImportDraft((draft) => draft
      ? updateActiveImportDraft(draft, { document: "", preview: null })
      : draft);
    // 内容已被用户接管：后续改名不再自动回填内置预填
    setImportPrefillSource(null);
    setMetadataImportError("");
  };
  const removeModelWindowRow = (index: number) => {
    const removedSlug = modelWindowRows[index]?.model.trim() || modelSlugOriginsRef.current[index] || "";
    // 「配置模型」不能被删掉：保存时后端会把 profile.model 合并回 model_list
    // （relay_config::merge_model_into_model_list），删了也会被加回来、看起来像"保存没生效"。
    const configuredSlug = parseModelSuffix(profile.model || "").slug.trim();
    if (removedSlug && configuredSlug && removedSlug === configuredSlug) {
      void actions.showMessage(
        t("配置模型不能从列表里删除"),
        tf("「{0}」是当前「配置模型」：Codex 会始终把它发布在模型列表里，删掉保存后也会被加回来。要减少模型，请先把上方「配置模型」改成要保留的那个，再删这一行。", [removedSlug]),
      );
      return;
    }
    const nextRows = modelWindowRows.filter((_, rowIndex) => rowIndex !== index);
    const nextOrigins = modelSlugOriginsRef.current.filter((_, rowIndex) => rowIndex !== index);
    const resolved = resolvePendingModelSlugRenames(nextRows, nextOrigins, profile.modelMetadata);
    modelSlugOriginsRef.current = resolved.origins;
    // 删到没有模型就保持空列表，不再补一行空行。
    setModelWindowRows(nextRows);
    const slugStillPresent = nextRows.some((row) => row.model.trim() === removedSlug)
      || resolved.origins.includes(removedSlug);
    const nextMetadata = removedSlug && !slugStillPresent
      ? clearModelMetadataForSlug(resolved.modelMetadata, removedSlug)
      : resolved.modelMetadata;
    if (nextMetadata !== profile.modelMetadata) commitModelMetadata(nextMetadata);
    if (metadataImportTarget?.index === index) {
      closeModelMetadataImport();
    } else if (metadataImportTarget && metadataImportTarget.index > index) {
      setActiveImportDraft({ ...activeImportDraft, index: activeImportDraft.index - 1 });
    }
  };
  const addModelWindowRows = (rows: ModelWindowRow[]) => {
    const merged = mergeModelWindowRows(modelWindowRows, rows);
    modelSlugOriginsRef.current = merged.map((row) => {
      const currentIndex = modelWindowRows.findIndex((current) => current.model.trim() === row.model.trim());
      return currentIndex >= 0 ? modelSlugOriginsRef.current[currentIndex] || row.model.trim() : row.model.trim();
    });
    // 上游获取的行窗口/压缩列一律为空：内置命中时立即回填（同一回填裁决）
    setModelWindowRows(merged.map((row) => {
      if (row.window.trim() && row.autoCompact.trim()) return row;
      const entry = builtinIndex.get(modelMetadataKey(row.model));
      const patch = builtinRowBackfillValue(row.model, row.window, row.autoCompact, entry);
      return patch.window || patch.autoCompact
        ? {
            ...row,
            ...(patch.window ? { window: patch.window } : {}),
            ...(patch.autoCompact ? { autoCompact: patch.autoCompact } : {}),
          }
        : row;
    }));
  };
  const appendEmptyModelRow = () => {
    modelSlugOriginsRef.current = [...modelSlugOriginsRef.current, ""];
    setModelWindowRows([...modelWindowRows, { model: "", window: "", autoCompact: "", imageHandling: "" }]);
  };
  const modelRowsError = modelWindowRowsValidationMessage(modelWindowRowsValidationError(modelWindowRows));
  const customHeadersError = relayHeadersValidationMessage(profile.customHeaders || []);
  const localizeMetadataSourceTag = (tag: ReturnType<typeof metadataSourceTags>[number]) => {
    if (tag.kind === "match") {
      return {
        text: tf("匹配：{0}", [tag.source]),
        title: tf("内置元数据：{0}", [tag.source]),
      };
    }
    if (tag.kind === "fallback") {
      return {
        text: tf("回退：{0}", [tag.source]),
        title: tf("无内置元数据，生成时回退 {0} 官方模板", [tag.source]),
      };
    }
    return {
      text: t("自定义"),
      title: tag.source
        ? tf("已导入自定义元数据，生成时覆盖内置（{0}）", [tag.source])
        : t("已导入自定义元数据，生成时以该配置为准"),
    };
  };
  return (
    <div className="relay-profile-editor">
      <div className="relay-fields">
        <section className="relay-config-section relay-basic-section">
          <div className="relay-config-section-head">
            <div>
              <strong>{t("基础设置")}</strong>
              <span>{t("先选择接入方式，相关字段会立即显示在这里。")}</span>
            </div>
          </div>
          <div className="relay-basic-fields">
            <Field className="relay-field-name" label={t("名称")}>
              <Input
                value={profile.name}
                onChange={(event) => updateDraft({ name: event.currentTarget.value })}
              />
            </Field>
          </div>
          <div className="relay-mode-block">
            <div className="relay-mode-block-head">
              <div>
                <strong>{t("接入方式")}</strong>
                <span>{t("官方登录 + API 会保留官方账号，同时使用下方 API 连接；不需要再到其他面板勾选。")}</span>
              </div>
            </div>
            <div className="relay-mode-options" role="radiogroup" aria-label={t("接入方式")}>
              {([
                {
                  mode: "official" as const,
                  icon: ShieldCheck,
                  title: t("官方登录"),
                  description: t("使用 ChatGPT 官方账号，不写入 API 凭据。"),
                },
                {
                  mode: "mixedApi" as const,
                  icon: Cable,
                  title: t("官方登录 + API"),
                  description: t("保留官方登录，同时使用 API Base URL 与 Key。"),
                },
                {
                  mode: "pureApi" as const,
                  icon: KeyRound,
                  title: t("纯 API"),
                  description: t("只使用 API 凭据，不依赖官方登录。"),
                },
              ]).map(({ mode, icon: Icon, title, description }) => (
                <button
                  aria-checked={editorMode === mode}
                  className={`relay-mode-option ${editorMode === mode ? "active" : ""}`}
                  key={mode}
                  onClick={() => selectEditorMode(mode)}
                  role="radio"
                  type="button"
                >
                  <span className="relay-mode-option-icon"><Icon className="h-4 w-4" /></span>
                  <span className="relay-mode-option-copy">
                    <strong>{title}</strong>
                    <small>{description}</small>
                  </span>
                  <span className="relay-mode-option-indicator" aria-hidden="true" />
                </button>
              ))}
            </div>
            {showApiFields ? (
              <div className="relay-inline-connection">
                <div className="relay-inline-connection-head">
                  <div>
                    <strong>{t("API 连接")}</strong>
                    <span>
                      {editorMode === "mixedApi"
                        ? t("官方账号负责登录与会话，API 连接负责实际请求。")
                        : t("保存后会把这组凭据用于当前供应商的请求。")}
                    </span>
                  </div>
                  <UiBadge variant={profile.baseUrl.trim() && profile.apiKey.trim() ? "secondary" : "outline"}>
                    {profile.baseUrl.trim() && profile.apiKey.trim() ? t("已填写") : t("待填写")}
                  </UiBadge>
                </div>
                <div className="relay-api-fields">
                  <div className="relay-api-connection-fields">
                    <Field className="relay-field-base-url" label="Base URL">
                      <Input
                        value={profile.baseUrl}
                        onChange={(event) => updateDraft({ baseUrl: event.currentTarget.value })}
                        placeholder={t("填写供应商的 Base URL")}
                      />
                    </Field>
                    <Field className="relay-field-protocol" label={t("上游协议")}>
                      <AppSelect
                        ariaLabel={t("上游协议")}
                        value={profile.protocol}
                        onChange={(protocol) => updateDraft({ protocol })}
                        options={[
                          { value: "responses", label: "Responses API" },
                          { value: "chatCompletions", label: "Chat Completions" },
                          { value: "anthropicMessages", label: "Anthropic Messages" },
                        ]}
                      />
                    </Field>
                  </div>
                  <Field className="relay-field-key" label="API Keys">
                    <div className="relay-api-key-list">
                      {normalizedApiKeys.apiKeys.map((entry) => {
                        const active = entry.id === normalizedApiKeys.activeApiKeyId;
                        return (
                          <div className={`relay-api-key-row ${active ? "active" : ""}`} key={entry.id}>
                            <button
                              aria-label={t("设为当前 Key")}
                              aria-pressed={active}
                              className="relay-api-key-select"
                              onClick={() => updateApiKeys(normalizedApiKeys.apiKeys, entry.id)}
                              title={t("设为当前 Key")}
                              type="button"
                            >
                              <CheckCircle2 className="h-4 w-4" />
                            </button>
                            <Input
                              aria-label={t("Key 名称")}
                              value={entry.name}
                              onChange={(event) => updateApiKeys(normalizedApiKeys.apiKeys.map((key) =>
                                key.id === entry.id ? { ...key, name: event.currentTarget.value } : key))}
                            />
                            <Input
                              aria-label={tf("{0} 的 API Key", [entry.name])}
                              type="password"
                              value={entry.apiKey}
                              onChange={(event) => updateApiKeys(normalizedApiKeys.apiKeys.map((key) =>
                                key.id === entry.id ? { ...key, apiKey: event.currentTarget.value } : key))}
                              placeholder={t("输入供应商的 API Key")}
                            />
                            {apiKeyPreview(entry.apiKey) ? (
                              <span className="relay-api-key-preview" title={t("Key 首尾预览")}>
                                {apiKeyPreview(entry.apiKey)}
                              </span>
                            ) : null}
                            <Button
                              aria-label={t("删除 Key")}
                              disabled={normalizedApiKeys.apiKeys.length === 1}
                              onClick={() => updateApiKeys(normalizedApiKeys.apiKeys.filter((key) => key.id !== entry.id))}
                              size="icon"
                              title={t("删除 Key")}
                              type="button"
                              variant="ghost"
                            >
                              <Trash2 className="h-4 w-4" />
                            </Button>
                          </div>
                        );
                      })}
                      <Button onClick={() => {
                        const key = createRelayApiKey(normalizedApiKeys.apiKeys);
                        updateApiKeys([...normalizedApiKeys.apiKeys, key]);
                      }} size="sm" type="button" variant="secondary">
                        <Plus className="h-4 w-4" />
                        {t("添加 Key")}
                      </Button>
                    </div>
                  </Field>
                  {profile.protocol === "anthropicMessages" ? (
                    <>
                      <Field className="relay-field-anthropic-auth" label={t("认证字段")}>
                        <AppSelect
                          ariaLabel={t("认证字段")}
                          value={profile.anthropicAuthHeader || "x-api-key"}
                          onChange={(value) => updateDraft({ anthropicAuthHeader: value })}
                          options={[
                            { value: "x-api-key", label: t("x-api-key（Anthropic 原生）") },
                            { value: "authorization", label: t("Authorization: Bearer") },
                          ]}
                        />
                      </Field>
                      <Field className="relay-field-anthropic-max-tokens" label={t("最大输出 tokens")}>
                        <Input
                          inputMode="numeric"
                          value={profile.anthropicMaxTokens ? String(profile.anthropicMaxTokens) : ""}
                          placeholder="8192"
                          onChange={(event) =>
                            updateDraft({ anthropicMaxTokens: normalizeAnthropicMaxTokens(Number(event.currentTarget.value)) })
                          }
                        />
                      </Field>
                      <Field className="relay-field-anthropic-thinking" label={t("思考模式")}>
                        <AppSelect
                          ariaLabel={t("思考模式")}
                          value={profile.anthropicThinkingMode || "auto"}
                          onChange={(value) =>
                            updateDraft({ anthropicThinkingMode: value === "auto" ? "" : value })
                          }
                          options={[
                            { value: "auto", label: t("自动（新代模型透传等级）") },
                            { value: "adaptive", label: t("等级透传（adaptive）") },
                            { value: "budget", label: t("经典预算（budget_tokens）") },
                            { value: "off", label: t("关闭思考") },
                          ]}
                        />
                      </Field>
                      <label className="switch-row compact relay-switch-row relay-field-anthropic-cache">
                        <input
                          checked={profile.anthropicPromptCache !== false}
                          onChange={(event) =>
                            updateDraft({ anthropicPromptCache: event.currentTarget.checked })
                          }
                          type="checkbox"
                        />
                        <span>
                          <strong>{t("提示缓存")}</strong>
                          <small>
                            {t("给系统提示、工具定义和最新一轮对话打 5 分钟缓存断点（最多 4 个），长对话不会每轮全价重发。")}
                          </small>
                        </span>
                        <ToggleVisual />
                      </label>
                      <label className="switch-row compact relay-switch-row relay-field-anthropic-impersonate">
                        <input
                          checked={profile.anthropicImpersonateClaudeCode === true}
                          onChange={(event) =>
                            updateDraft({ anthropicImpersonateClaudeCode: event.currentTarget.checked })
                          }
                          type="checkbox"
                        />
                        <span>
                          <strong>{t("模拟 Claude Code 客户端")}</strong>
                          <small>
                            {t("伪装 User-Agent、anthropic-beta、x-app，并在系统提示首行注入 Claude Code 身份。仅在网关限制「只能通过 Claude Code 使用」时开启。")}
                          </small>
                        </span>
                        <ToggleVisual />
                      </label>
                      <div className="hint-line relay-protocol-hint">
                        <MessageCircle className="h-4 w-4" />
                        <span>
                          {t("思考档位跟随 Codex 的 reasoning effort：自动模式下新代模型按等级透传（adaptive + effort），旧代模型折算成 thinking 预算；模型声明 1M 窗口时会自动附上 1m 上下文 beta 头。")}
                        </span>
                      </div>
                    </>
                  ) : null}
                </div>
              </div>
            ) : (
              <div className="relay-mode-note">
                <ShieldCheck className="h-4 w-4" />
                <span>{t("当前使用官方账号登录；切换到“官方登录 + API”或“纯 API”后，API 连接字段会在这里展开。")}</span>
              </div>
            )}
          </div>
        </section>
        <RelayFold
          className="relay-config-section relay-model-settings"
          sectionId="models"
          defaultOpen
          issue={modelRowsError || relayModelRoutesSettingsValidation(relaySettingsWithDraft(form, profile.id, profile, isNew))}
          title={t("模型配置")}
        >
          <div className="relay-options-grid">
            <Field className="relay-field-config-model" label={t("配置模型")}>
              <Input
                value={profile.model}
                onChange={(event) => updateDraft({ model: event.currentTarget.value })}
                placeholder={t("例如 deepseek-v4-pro")}
              />
              <p className="field-hint">
                {t("默认启动 Codex 时使用的模型名，请勿带后缀；上下文窗口请在下方「模型列表」中按模型单独配置。")}
              </p>
            </Field>
            {/* 官方登录 profile 同样维护模型列表：按模型聚合与官方模型目录拉取都依赖它。 */}
            <section className="relay-config-section relay-field-model-list">
                <div className="relay-config-section-head">
                  <div>
                    <strong>{t("模型列表")}</strong>
                    <span>
                      {t("每行一个模型；上下文窗口可填")} <code>1M</code>{t("、")}<code>200K</code> {t("或")} <code>1000000</code>{t("，留空表示使用 Codex 默认长度。")}
                    </span>
                  </div>
                  <div className="relay-model-list-tools">
                    <Button
                      onClick={appendEmptyModelRow}
                      size="sm"
                      type="button"
                      variant="secondary"
                    >
                      <Plus className="h-4 w-4" />
                      {t("添加模型")}
                    </Button>
                    <Button
                      onClick={async () => {
                        const serializedRows = serializeModelWindowRows(modelWindowRows);
                        const models = await actions.fetchRelayProfileModels({
                          ...profile,
                          modelList: serializedRows.modelList,
                          modelWindows: serializedRows.modelWindows,
                          modelAutoCompact: serializedRows.modelAutoCompact,
                        });
                        if (models?.length) {
                          addModelWindowRows(models.map((model) => ({ model, window: "", autoCompact: "", imageHandling: "" })));
                        }
                      }}
                      size="sm"
                      type="button"
                      variant="secondary"
                    >
                      <Download className="h-4 w-4" />
                      {t("从上游获取")}
                    </Button>
                    <Button
                      disabled={!modelWindowRows.some((row) => row.model.trim())}
                      onClick={() => setModelWindowRows([])}
                      size="sm"
                      title={t("清空模型")}
                      type="button"
                      variant="outline"
                    >
                      <Trash2 className="h-4 w-4" />
                      {t("清空模型")}
                    </Button>
                  </div>
                </div>
                <div className="relay-model-row-editor">
                  {modelWindowRows.length ? <div className="relay-model-row relay-model-row-head">
                    <span aria-hidden="true" />
                    <span>{t("模型名称")}</span>
                    <span>{t("上下文窗口")}</span>
                    <span className="relay-model-head-bulk">
                      {t("自动压缩")}
                      <AppSelect
                        ariaLabel={t("全部模型的自动压缩")}
                        className="relay-model-bulk-select"
                        menuWidth={140}
                        onChange={(next) => {
                          if (next) updateAllModelWindowRows({ autoCompact: next });
                        }}
                        options={autoCompactOptions("").map((value) => ({ value, label: value }))}
                        placeholder={t("全部设为…")}
                        size="sm"
                        title={t("把所有模型设为同一个自动压缩比例")}
                        value=""
                      />
                    </span>
                    <span className="relay-model-head-bulk">
                      {t("图片处理方式")}
                      <AppSelect
                        ariaLabel={t("全部模型的图片处理方式")}
                        className="relay-model-bulk-select"
                        disabled={vlmUnsupportedProtocol}
                        menuWidth={160}
                        onChange={(next) => {
                          if (next) updateAllModelWindowRows({ imageHandling: next as ModelWindowRow["imageHandling"] });
                        }}
                        options={[
                          { value: "send-as-is", label: t("原样发送图片") },
                          { value: "strip", label: t("移除图片") },
                          { value: "vlm", label: t("视觉辅助分析") },
                        ]}
                        placeholder={t("全部设为…")}
                        size="sm"
                        title={t("把所有模型设为同一种图片处理方式")}
                        value=""
                      />
                    </span>
                    <span>{t("思考等级")}</span>
                    <span>{t("模型配置")}</span>
                    <span aria-hidden="true" />
                  </div> : null}
                  <DndContext
                    sensors={modelSensors}
                    collisionDetection={closestCenter}
                    onDragEnd={handleModelRowsDragEnd}
                  >
                    <SortableContext
                      items={modelWindowRows.map((_, index) => `model-row-${index}`)}
                      strategy={verticalListSortingStrategy}
                    >
                      {modelWindowRows.map((row, index) => {
                        const slug = row.model.trim();
                        // 面板身份只用 index：slug 双轨（draft 副本 vs 实时输入）曾导致
                        // 改名时面板整体卸载、blur 后重挂抢焦点。
                        const importing = metadataImportTarget?.index === index;
                        // 配置可能还挂在「改名尚未提交」的旧 key 下，按行解析而不是按实时名硬查。
                        const rowMetadataKey = resolveModelMetadataRowKey(importedModelMetadata, {
                          current: slug,
                          origin: modelSlugOriginsRef.current[index],
                        });
                        const imported = rowMetadataKey !== null;
                        // 思考等级列：自定义声明优先，其次内置初值；写回沿用到
                        // 改名前尚未提交的 key，blur 迁移时不会丢这次编辑。
                        const reasoning = resolveModelReasoningLevels(
                          rowMetadataKey ? importedModelMetadata[rowMetadataKey] : undefined,
                          builtinIndex.get(modelMetadataKey(slug))?.supported_reasoning_levels,
                        );
                        // 按钮可用性与状态行都从这一个纯函数出（见 model-metadata.ts）。
                        // 面板关闭时不创建导入控件，避免用一个面板级状态为所有行派生按钮状态。
                        const importControls = importing
                          ? importPanelControls({
                              slug,
                              document: metadataImportDocument,
                              imported,
                              // 空文档（清除后）不算解析失败：保存键保持可用，
                              // 走「放弃自定义回退内置」的保存路径。
                              parseOk: !metadataImportDocument.trim() || Boolean(metadataImportPreview),
                              matched: builtinQueryState?.status === "error" ? false : Boolean(builtinMatch?.matched),
                              // 面板内容与内置条目全字段等价：保存的目标态就是「用内置」。
                              matchesBuiltin: metadataImportPreview
                                ? metadataMatchesBuiltin(metadataImportPreview.documentEntry, builtinMetadata)
                                : false,
                            })
                          : null;

                        return (
                      <SortableModelWindowEntry id={`model-row-${index}`} key={index}>
                        {(sortable) => (
                          <>
                        <div className="relay-model-row">
                          <button
                            aria-label={t("拖动排序")}
                            className="relay-model-drag"
                            title={t("拖动排序")}
                            type="button"
                            {...sortable.attributes}
                            {...sortable.listeners}
                          >
                            <GripVertical className="h-4 w-4" />
                          </button>
                          <div className="relay-model-cell relay-model-cell-name">
                            <span className="relay-model-cell-label">{t("模型名称")}</span>
                            <Input
                              aria-label={t("模型名称")}
                              value={row.model}
                              onChange={(event) => updateModelWindowRow(index, { model: event.currentTarget.value })}
                              onBlur={() => commitModelSlug(index)}
                              placeholder="deepseek/deepseek-v4-flash"
                            />
                          </div>
                          <div className="relay-model-cell relay-model-cell-window">
                            <span className="relay-model-cell-label">{t("上下文窗口")}</span>
                            <Input
                              aria-label={t("上下文窗口")}
                              value={row.window}
                              onChange={(event) => {
                                const window = event.currentTarget.value;
                                updateModelWindowRow(index, { window });
                                // 导入面板尚未粘贴 JSON 时，只编辑模型行；不要把空文档同步失败显示成错误。
                                if (!importing || !metadataImportDocument.trim() || !metadataImportPreview) return;
                                const synchronized = synchronizeModelMetadataDocumentLimitsPreview(
                                  metadataImportDocument,
                                  slug,
                                  window,
                                  metadataImportPreview?.autoCompactCalculationPercent
                                    ?? metadataImportPreview?.autoCompactPercent
                                    ?? row.autoCompact,
                                );
                                if (!synchronized) {
                                  setMetadataImportPreview(null);
                                  setMetadataImportError(t("上下文窗口与自动压缩值无效，无法同步模型配置。"));
                                  return;
                                }
                                setMetadataImportDocument(synchronized.document);
                                setMetadataImportPreview(synchronized.preview);
                                setMetadataImportError("");
                              }}
                              placeholder={t("留空使用默认值")}
                            />
                          </div>
                          <div className="relay-model-cell relay-model-cell-compact">
                            <span className="relay-model-cell-label">{t("自动压缩")}</span>
                            <AppSelect
                              ariaLabel={t("自动压缩")}
                              className="text-xs"
                              value={normalizeAutoCompactPercent(row.autoCompact) || DEFAULT_AUTO_COMPACT_PERCENT}
                              onChange={(value) => {
                                updateModelWindowRow(index, { autoCompact: value });
                                // 导入面板尚未粘贴 JSON 时，只编辑模型行；不要把空文档同步失败显示成错误。
                                if (!importing || !metadataImportDocument.trim() || !metadataImportPreview) return;
                                const synchronized = synchronizeModelMetadataDocumentLimitsPreview(
                                  metadataImportDocument,
                                  slug,
                                  row.window,
                                  value,
                                );
                                if (!synchronized) {
                                  setMetadataImportPreview(null);
                                  setMetadataImportError(t("上下文窗口与自动压缩值无效，无法同步模型配置。"));
                                  return;
                                }
                                setMetadataImportDocument(synchronized.document);
                                setMetadataImportPreview(synchronized.preview);
                                setMetadataImportError("");
                              }}
                              options={autoCompactOptions(row.autoCompact).map((value) => ({ value, label: value }))}
                              title={t("自动压缩留空时沿用 Codex 默认行为；填写百分比后会按该模型的上下文窗口重新计算阈值。")}
                            />
                          </div>
                          <div className="relay-model-cell relay-model-cell-image">
                            <span className="relay-model-cell-label">{t("图片处理方式")}</span>
                            <AppSelect
                              ariaLabel={t("图片处理方式")}
                              className="text-xs"
                              value={row.imageHandling}
                              disabled={vlmUnsupportedProtocol}
                              onChange={(value) => updateModelWindowRow(index, { imageHandling: value })}
                              options={[
                                { value: "", label: t("纯文本模型请配置此项"), disabled: true },
                                { value: "send-as-is", label: t("原样发送图片"), title: t("多模态模型直接接收图片,不经过任何处理") },
                                { value: "strip", label: t("移除图片"), title: t("删掉图片只发文字,避免纯文本模型报错(模型看不到图)") },
                                { value: "vlm", label: t("视觉辅助分析"), title: t("图片先由视觉辅助模型(Qwen)转成文字描述,纯文本模型也能\"看图\"") },
                              ]}
                              title={vlmUnsupportedProtocol ? t("VLM 仅支持 Chat Completions 协议和聚合模式") : t("多模态模型（支持图片输入的模型）请保持 send-as-is。")}
                            />
                          </div>
                          <div className="relay-model-cell relay-model-cell-reasoning">
                            <span className="relay-model-cell-label">{t("思考等级")}</span>
                            <ModelReasoningLevelsCell
                              levels={reasoning.levels}
                              parameterMode={reasoningParameterMode(rowMetadataKey ? importedModelMetadata[rowMetadataKey] : undefined)}
                              onParameterModeChange={(mode) => commitModelMetadata(setModelReasoningParameterMode(profile.modelMetadata, rowMetadataKey ?? slug, mode))}
                              inherited={!reasoning.overridden}
                              defaultLevel={reasoning.defaultLevel}
                              onChange={(levels, defaultLevel) => commitModelMetadata(
                                setModelReasoningLevelsForSlug(
                                  profile.modelMetadata,
                                  rowMetadataKey ?? slug,
                                  levels,
                                  defaultLevel,
                                ),
                              )}
                            />
                          </div>
                          <Button
                            className={`relay-model-import-button${imported ? " relay-model-import-custom" : builtinIndex.has(modelMetadataKey(slug)) ? " relay-model-import-builtin" : ""}`}
                            aria-expanded={importing}
                            disabled={!slug}
                            onClick={() => (importing ? cancelModelMetadataImport() : beginModelMetadataImport(index, slug))}
                            size="icon"
                            title={t("模型配置：元数据与 Fast 支持")}
                            type="button"
                            variant="ghost"
                          >
                            <FileCode2 className="h-4 w-4" />
                          </Button>
                          <Button
                            className="relay-model-delete"
                            aria-label={t("删除模型")}
                            onClick={() => removeModelWindowRow(index)}
                            size="icon"
                            title={t("删除模型")}
                            type="button"
                            variant="ghost"
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                        {importing ? (
                          <section className="relay-model-import-workbench">
                            <Field label={t("Fast（priority）支持")}>
                              <AppSelect
                                ariaLabel={t("Fast（priority）支持")}
                                value={importPrefillSource === "builtin" ? "inherit" : modelFastSupportMode(metadataImportPreview?.metadata)}
                                onChange={(value) => {
                                  const changed = synchronizeModelMetadataFastSupport(metadataImportDocument, slug, value as ModelFastSupportMode);
                                  if (!changed) {
                                    setMetadataImportError(t("请先修正当前模型的 JSON 配置，再设置 Fast 支持。"));
                                    return;
                                  }
                                  setMetadataImportDocument(changed.document);
                                  setMetadataImportPreview(changed.preview);
                                  setImportPrefillSource("existing");
                                  setMetadataImportError("");
                                }}
                                options={[
                                  { value: "inherit", label: t("继承模型默认能力") },
                                  { value: "supported", label: t("明确支持 priority") },
                                  { value: "unsupported", label: t("不支持 priority") },
                                ]}
                              />
                              <p className="field-hint">{t("按当前供应商逐模型声明。仅在上游支持 service_tier=priority 时选择支持；继承会清除自定义声明，取消不会保存修改。")}</p>
                            </Field>
                            <Field label={t("Responses Lite 支持")}>
                              <AppSelect value={metadataImportPreview?.metadata.use_responses_lite === true ? "supported" : metadataImportPreview?.metadata.use_responses_lite === false ? "unsupported" : "inherit"}
                                options={[{ value: "inherit", label: t("继承模型默认能力") }, { value: "supported", label: t("明确支持 Lite") }, { value: "unsupported", label: t("标准 Responses") }]}
                                onChange={(value) => {
                                  const changed = synchronizeModelMetadataLiteSupport(metadataImportDocument, slug, value as "inherit" | "supported" | "unsupported");
                                  if (!changed) { setMetadataImportError(t("请先修正当前模型的 JSON 配置。")); return; }
                                  setMetadataImportDocument(changed.document); setMetadataImportPreview(changed.preview); setImportPrefillSource("existing"); setMetadataImportError("");
                                }} />
                              <p className="field-hint">{t("保存此模型后还需保存并使用供应商；只为已核实支持 Lite 的上游选择支持。")}</p>
                            </Field>
                            <Textarea
                              autoFocus
                              value={metadataImportDocument}
                              onChange={(event) => {
                                const document = event.currentTarget.value;
                                setImportPrefillSource("existing");
                                setMetadataImportDocument(document);
                                setMetadataImportError("");
                                const parsed = parseModelMetadataDocument(document, slug);
                                if (!parsed.ok) {
                                  setMetadataImportPreview(null);
                                  setMetadataImportError(t(parsed.error));
                                  return;
                                }
                                setMetadataImportPreview(parsed.value);
                                // 实时替换：JSON 里的窗口/压缩比即时写回模型行输入框（所见即所得）。
                                if (metadataImportTarget) {
                                  const row = modelWindowRows[metadataImportTarget.index];
                                  if (row) {
                                    const patch = importDocumentSyncPatch(row, parsed.value);
                                    if (suffixWindowString(row.model)) delete patch.window;
                                    if (patch.window !== undefined || patch.autoCompact !== undefined) {
                                      updateModelWindowRow(metadataImportTarget.index, patch);
                                    }
                                  }
                                }
                              }}
                              placeholder={t("需要补充供应商模型信息时填写；不填则使用 Codex++ 默认配置（自动压缩 90%、图片原样发送）。从供应商的 models.json 或 model.json 复制，支持多个模型。")}
                              rows={7}
                            />
                            {metadataImportError ? <div className="relay-model-metadata-import-error" role="alert">{metadataImportError}</div> : null}
                            {metadataImportPreview?.contextWindow && row.window.trim() && metadataImportPreview.contextWindow !== row.window.trim() ? (
                              <div className="relay-model-metadata-import-warning" role="status">
                                {tf("文档里的上下文窗口是 {0}，当前列填的是 {1}，已保留当前值；要用文档的值请先清空窗口。", [metadataImportPreview.contextWindow, row.window.trim()])}
                              </div>
                            ) : null}
                            {metadataImportPreview?.ignoredFields.length ? (
                              <div className="relay-model-metadata-import-warning" role="status">
                                {tf("以下字段由 Codex++ 计算或维护，导入不会覆盖：{0}", [metadataImportPreview.ignoredFields.join(", ")])}
                              </div>
                            ) : null}
                            <div className="relay-model-metadata-import-actions">
                              <div className="relay-model-import-copy">
                                <strong>{slug}</strong>
                                {metadataSourceTags({
                                  slug,
                                  imported,
                                  builtinMatch: builtinQueryState?.status === "error" ? null : builtinMatch,
                                  builtinIndexSlug: builtinIndex.get(modelMetadataKey(slug)),
                                  // 回退模板名从后端 fallback 字段实时取，不写死
                                  fallbackSlug: builtinMatch?.fallback?.slug,
                                }).map((tag) => (
                                  (() => {
                                    const localized = localizeMetadataSourceTag(tag);
                                    return (
                                      <span
                                        key={tag.kind}
                                        className={`relay-model-source-badge relay-model-source-${tag.tone}`}
                                        title={localized.title}
                                      >
                                        {localized.text}
                                      </span>
                                    );
                                  })()
                                ))}
                              </div>
                              <div className="relay-model-metadata-import-flow">
                                {/* 四个按钮恒定渲染：只用置灰表达可用性，不再随状态出现/消失 */}
                                <Button
                                  disabled={importControls?.rematch.disabled ?? true}
                                  onClick={() => void rematchBuiltinImport(slug)}
                                  size="sm"
                                  title={t(importControls?.rematch.title ?? "重新匹配")}
                                  type="button"
                                  variant="ghost"
                                >
                                  {t("重新匹配")}
                                </Button>
                                <Button
                                  disabled={importControls?.clear.disabled ?? true}
                                  onClick={() => clearImportDocument()}
                                  size="sm"
                                  title={t(importControls?.clear.title ?? "清除")}
                                  type="button"
                                  variant="ghost"
                                >
                                  {t("清除")}
                                </Button>
                                <Button
                                  disabled={importControls?.cancel.disabled ?? true}
                                  onClick={cancelModelMetadataImport}
                                  size="sm"
                                  title={t(importControls?.cancel.title ?? "取消")}
                                  type="button"
                                  variant="ghost"
                                >
                                  {t("取消")}
                                </Button>
                                <Button
                                  disabled={importControls?.save.disabled ?? true}
                                  onClick={applyModelMetadataImport}
                                  size="sm"
                                  type="button"
                                  title={t(importControls?.save.title ?? "保存此模型")}
                                >
                                  {t(importControls?.save.label ?? "保存此模型")}
                                </Button>
                              </div>
                            </div>
                          </section>
                        ) : null}
                          </>
                        )}
                      </SortableModelWindowEntry>
                        );
                      })}
                    </SortableContext>
                  </DndContext>
                </div>
                {modelRowsError ? <div className="relay-model-metadata-import-error" role="alert">{modelRowsError}</div> : null}
                <p className="field-hint">
                  {t("自动压缩留空时沿用 Codex 默认行为；填写百分比后会按该模型的上下文窗口重新计算阈值。")}
                  {" "}{t("需要自定义 Fast 白名单时，打开对应模型的配置按钮，设置 Fast（priority）支持并保存此模型与供应商。")}
                </p>
            </section>
            {showApiFields ? (
              <section className="relay-config-section relay-field-model-routes">
                <div className="relay-config-section-head">
                  <div>
                    <strong>{t("单模型协议转换")}</strong>
                    <span>{t("仅在当前供应商启用时生效；精确匹配模型名并使用目标供应商的 URL 与 Key。目标必须是 Responses API，且需要从 Codex++ 启动。")}</span>
                  </div>
                  <div className="relay-model-list-tools">
                    <Button
                      disabled={modelRouteTargets.length === 0}
                      onClick={() => updateDraft({ modelRoutes: [...modelRoutes, { model: "", targetRelayId: "", targetModel: "" }] })}
                      size="sm"
                      title={modelRouteTargets.length === 0 ? t("请先创建一个 Responses API 目标供应商") : t("添加协议转换")}
                      type="button"
                      variant="secondary"
                    >
                      <Plus className="h-4 w-4" />
                      {t("添加协议转换")}
                    </Button>
                  </div>
                </div>
                <div className="relay-model-route-editor">
                  {modelRoutes.length ? (
                    <div className="relay-model-route-row relay-model-route-head">
                      <span>{t("匹配模型")}</span>
                      <span>{t("目标供应商")}</span>
                      <span>{t("目标模型（可选）")}</span>
                    </div>
                  ) : null}
                  {modelRoutes.map((route, index) => (
                    <div className="relay-model-route-row" key={`model-route-${index}`}>
                      <Input
                        value={route.model}
                        onChange={(event) => updateModelRoute(index, { model: event.currentTarget.value })}
                        placeholder={t("例：gpt-5.6-luna")}
                      />
                      <AppSelect
                        value={route.targetRelayId}
                        onChange={(targetRelayId) => updateModelRoute(index, { targetRelayId })}
                        options={[
                          { value: "", label: t("选择 Responses 供应商"), disabled: true },
                          ...modelRouteTargets.map((candidate) => ({ value: candidate.id, label: candidate.name || candidate.id })),
                        ]}
                      />
                      <Input
                        value={route.targetModel}
                        onChange={(event) => updateModelRoute(index, { targetModel: event.currentTarget.value })}
                        placeholder={t("留空保持原模型名")}
                      />
                      <Button
                        aria-label={t("删除协议转换")}
                        onClick={() => updateDraft({ modelRoutes: modelRoutes.filter((_, routeIndex) => routeIndex !== index) })}
                        size="icon"
                        title={t("删除协议转换")}
                        type="button"
                        variant="ghost"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  ))}
                </div>
              </section>
            ) : null}
            {showApiFields && modelWindowRows.some((row) => row.imageHandling === "vlm") ? (
              <div className="relay-vlm-section">
                <div className="relay-vlm-section-header">{t("Vision Analysis Provider")}</div>
                <Field className="relay-field-vlm-api-key" label={t("VLM API Key")}>
                  <Input
                    type="password"
                    value={profile.vlmApiKey}
                    onChange={(event) => updateDraft({ vlmApiKey: event.currentTarget.value })}
                    placeholder="sk-..."
                  />
                </Field>
                <Field className="relay-field-vlm-model" label={t("VLM Model")}>
                  <Input
                    value={profile.vlmModel}
                    onChange={(event) => updateDraft({ vlmModel: event.currentTarget.value })}
                    placeholder="qwen-vl-plus"
                  />
                </Field>
                <Field className="relay-field-vlm-base-url" label={t("VLM Base URL")}>
                  <Input
                    value={profile.vlmBaseUrl}
                    onChange={(event) => updateDraft({ vlmBaseUrl: event.currentTarget.value })}
                    placeholder="https://dashscope.aliyuncs.com/compatible-mode/v1"
                  />
                </Field>
                <p className="field-hint">
                  {t("若开启 VLM analysis，请确认 VLM 配置项完整且服务可用。")}
                  <br />
                  {t("仅在 Chat Completion 和聚合模式生效。")}
                </p>
                {modelWindowRows.some((row) => row.imageHandling === "vlm") && (!profile.vlmApiKey || !profile.vlmModel || !profile.vlmBaseUrl) ? (
                  <p className="field-hint warn">{t("VLM 配置不完整：API Key、Model 和 Base URL 为必填项，否则 VLM 不会生效。")}</p>
                ) : null}
                <div className="vlm-test-entry">
                  <Button
                    onClick={() => setVlmTestOpen((v) => !v)}
                    size="sm"
                    type="button"
                    variant="secondary"
                  >
                    {vlmTestOpen ? t("收起测试面板") : t("测试 VLM")}
                  </Button>
                </div>
                {vlmTestOpen ? <VlmTestPanel profile={profile} onClose={() => setVlmTestOpen(false)} /> : null}
              </div>
            ) : null}
          </div>
        </RelayFold>
        {showApiFields ? (
          <RelayFold
            className="relay-config-section relay-request-settings"
            defaultOpen
            sectionId="requests"
            issue={customHeadersError || relaySessionProviderValidation(profile)}
            title={t("请求设置")}
          >
          <div className="relay-options-grid">
            {showApiFields ? (
              <Field className="relay-field-session-provider" label={t("Codex 会话身份")}>
                <AppSelect
                  value={sessionProvider}
                  onChange={(value) => updateDraft({ sessionProvider: value })}
                  options={[
                    { value: "custom", label: t("Custom（默认）") },
                    {
                      value: "openai",
                      label: t("OpenAI（兼容 ChatGPT Remote）"),
                      disabled: !canUseOpenAiSessionProvider || profile.protocol !== "responses",
                    },
                  ]}
                />
                <p className="field-hint">
                  {profile.protocol !== "responses"
                    ? t("OpenAI 会话身份需要 Responses API；Chat Completions 不支持远程压缩。")
                    : canUseOpenAiSessionProvider
                      ? t("选择 OpenAI 后，Codex Remote 会把当前会话识别为 ChatGPT 会话；中转仍使用 custom 表。")
                      : t("官方登录未混入 API 时不写入会话 provider")}
                </p>
              </Field>
            ) : null}
            {showApiFields ? (
              <Field className="relay-field-user-agent" label="User-Agent">
                <Input
                  value={profile.userAgent}
                  onChange={(event) => updateDraft({ userAgent: event.currentTarget.value })}
                  placeholder={t("留空使用默认值")}
                />
              </Field>
            ) : null}
            {showApiFields ? (
              <Field className="relay-field-auto-review-model" label={t("自动审批模型")}>
                <Input
                  value={profile.autoReviewModel ?? ""}
                  onChange={(event) => updateDraft({ autoReviewModel: event.currentTarget.value })}
                  placeholder={t("留空则原样转发 codex-auto-review")}
                />
              </Field>
            ) : null}
            {showApiFields ? (
              <Field className="relay-field-proxy" label={t("供应商独立代理")}>
                <div className="relay-inline-field">
                  <Input
                    value={relayProxyUrl}
                    onChange={(event) => setRelayProxyUrl(event.currentTarget.value)}
                    placeholder="http://127.0.0.1:7890"
                  />
                  <Button disabled={relayProxyBusy} onClick={() => void saveRelayProxy()} size="sm" type="button" variant="secondary">
                    {t("保存")}
                  </Button>
                </div>
                <span className="hint-line">{t("留空表示使用系统代理；仅影响当前供应商的模型拉取、测试和请求。")}</span>
              </Field>
            ) : null}
            {showApiFields ? (
              // 五个兼容性开关默认都关，且只在上游明确需要时才开：收成一组，摘要上写开了几项。
              <details
                className="relay-compat-group"
                open={compatOpen ?? compatEnabledCount > 0}
                onToggle={(event) => setCompatOpen(event.currentTarget.open)}
              >
                <summary>
                  <ChevronDown aria-hidden="true" className="relay-fold-chevron h-4 w-4" />
                  <strong>{t("高级兼容选项")}</strong>
                  <small>{compatEnabledCount ? tf("已开启 {0} 项", [compatEnabledCount]) : t("全部关闭；只在上游明确需要时开启")}</small>
                </summary>
                <div className="relay-compat-body">
                {showApiFields ? (
                  <label
                    className={`switch-row compact relay-switch-row relay-field-standard${profile.protocol === "chatCompletions" ? "" : " is-disabled"}`}
                    title={profile.protocol === "chatCompletions" ? undefined : t("仅在上游协议为 Chat Completions 时可用。Responses API 会原样转发。")}
                  >
                    <input
                      checked={profile.standardOpenaiProtocol}
                      disabled={profile.protocol !== "chatCompletions"}
                      onChange={(event) =>
                        updateDraft({ standardOpenaiProtocol: event.currentTarget.checked })
                      }
                      type="checkbox"
                    />
                    <span>
                      <strong>{t("纯标准协议")}</strong>
                      <small>
                        {t("强制走标准 OpenAI 协议，不注入厂商私有 reasoning 参数。面向只认标准 OpenAI 字段、拒绝厂商私有参数的第三方网关。")}
                        {" "}
                        {t("若网关提示 thinking type: adaptive 无效，可启用此项后重试；这会停用厂商私有推理参数。")}
                      </small>
                    </span>
                    <ToggleVisual />
                  </label>
                ) : null}
                {showApiFields ? (
                  <label className="switch-row compact relay-switch-row">
                    <input
                      checked={stabilizePromptPrefix}
                      disabled={stabilizePromptPrefixBusy}
                      onChange={(event) => void saveStabilizePromptPrefix(event.currentTarget.checked)}
                      type="checkbox"
                    />
                    <span><strong>{t("前缀稳定层")}</strong><small>{t("按供应商稳定可选历史块的顺序并移除成对 image_resize_notice；默认关闭，适合已确认上游支持的缓存场景。")}</small></span>
                    <ToggleVisual />
                  </label>
                ) : null}
                {showApiFields ? (
                  <label
                    className={`switch-row compact relay-switch-row relay-field-search-history${profile.protocol === "responses" ? "" : " is-disabled"}`}
                    title={profile.protocol === "responses" ? undefined : t("仅限 Responses API。")}
                  >
                    <input
                      checked={profile.webSearchHistoryCompat}
                      disabled={profile.protocol !== "responses"}
                      onChange={(event) =>
                        updateDraft({ webSearchHistoryCompat: event.currentTarget.checked })
                      }
                      type="checkbox"
                    />
                    <span>
                      <strong>{t("搜索历史压缩兼容（实验）")}</strong>
                      <small>response protection is unavailable</small>
                    </span>
                    <ToggleVisual />
                  </label>
                ) : null}
                {showApiFields && profile.relayMode !== "aggregate" && profile.protocol !== "anthropicMessages" ? (
                  <label className="switch-row compact relay-switch-row relay-field-local-compaction">
                    <input
                      checked={profile.localCompactionPromptAtEnd === true}
                      onChange={(event) => updateDraft({ localCompactionPromptAtEnd: event.currentTarget.checked })}
                      type="checkbox"
                    />
                    <span>
                      <strong>{t("本地压缩缓存复用（实验）")}</strong>
                      <small>{t("仅支持旧版直连供应商，暂不支持 2.0 网关。实测可改善压缩请求的缓存命中概率，但不保证每次命中，也不保证后置压缩提示不影响摘要质量。缓存按 Key 隔离，切换或轮换 Key 可能降低命中率。未命中供应商缓存时，补回的工具声明会增加输入 token 开销；上游若忽略禁用工具调用要求，可能返回空摘要。默认关闭。")}</small>
                    </span>
                    <ToggleVisual />
                  </label>
                ) : null}
                {showApiFields && profile.protocol === "responses" ? <>
                  <label className="switch-row compact relay-switch-row">
                    <input type="checkbox" checked={profile.trustUpstreamCatalogLite === true} onChange={(event) => updateDraft({ trustUpstreamCatalogLite: event.currentTarget.checked })} />
                    <span><strong>{t("保留外部目录的 Lite 声明")}</strong><small>{t("仅在已确认上游支持时启用；保留每个模型的显式声明，不会为全部模型开启 Lite。")}</small></span><ToggleVisual />
                  </label>
                  <label className="switch-row compact relay-switch-row">
                    <input type="checkbox" checked={profile.strictResponsesReasoning === true} onChange={(event) => updateDraft({ strictResponsesReasoning: event.currentTarget.checked })} />
                    <span><strong>{t("严格 Responses 推理历史兼容")}</strong><small>{t("仅当目标上游接受当前账号的加密历史时启用。混合推理只发送可识别的加密信封；占位或无效密文会明确报错。本地记录不变。")}</small></span><ToggleVisual />
                  </label>
                </> : null}
                </div>
              </details>
            ) : null}
            {showApiFields ? (
              <Field className="relay-field-custom-headers" label={t("自定义请求头")}>
                <div className="relay-custom-headers">
                  {(profile.customHeaders || []).map((row, index) => (
                    <div className="relay-custom-header-row" key={`custom-header-${index}`}>
                      <Input
                        aria-label={t("请求头名称")}
                        value={row.key}
                        onChange={(event) => {
                          const next = (profile.customHeaders || []).slice();
                          next[index] = { ...next[index], key: event.currentTarget.value };
                          updateDraft({ customHeaders: next });
                        }}
                        placeholder="X-Tenant"
                      />
                      <Input
                        aria-label={t("请求头值")}
                        value={row.value}
                        onChange={(event) => {
                          const next = (profile.customHeaders || []).slice();
                          next[index] = { ...next[index], value: event.currentTarget.value };
                          updateDraft({ customHeaders: next });
                        }}
                        placeholder={t("请求头值")}
                      />
                      <Button
                        aria-label={t("删除这一项")}
                        onClick={() =>
                          updateDraft({
                            customHeaders: (profile.customHeaders || []).filter((_, i) => i !== index),
                          })
                        }
                        size="sm"
                        type="button"
                        variant="secondary"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  ))}
                  <div className="relay-custom-headers-actions">
                    <Button
                      onClick={() =>
                        updateDraft({
                          customHeaders: [...(profile.customHeaders || []), { key: "", value: "" }],
                        })
                      }
                      size="sm"
                      type="button"
                      variant="secondary"
                    >
                      <Plus className="h-4 w-4" />
                      {t("添加请求头")}
                    </Button>
                  </div>
                  <span className="hint-line">
                    {t("自定义请求头会同时用于测试连接、模型列表与实际代理请求。")}
                  </span>
                  <span className="hint-line">
                    {t("Host、Content-Length 等传输头由协议层掌控，不能覆盖；配置 Authorization 时以它为准，不再注入 API Key。")}
                  </span>
                  {customHeadersError ? (
                    <span className="hint-line relay-custom-headers-error">{customHeadersError}</span>
                  ) : null}
                </div>
              </Field>
            ) : null}
            {showApiFields && profile.protocol !== "responses" ? (
              <div className="hint-line relay-protocol-hint">
                <MessageCircle className="h-4 w-4" />
                <span>
                  {profile.protocol === "anthropicMessages"
                    ? t("此上游说 Anthropic Messages（/v1/messages）：本地 127.0.0.1:57321 会把请求转成 Anthropic 协议发出，再转回 Responses，需要从 Codex++ 启动 Codex。")
                    : t("此上游会通过本地 127.0.0.1:57321 转成 Responses API，需要从 Codex++ 启动 Codex。")}
                </span>
              </div>
            ) : null}
            <div className="hint-line relay-protocol-hint">
              <ShieldCheck className="h-4 w-4" />
              <span>{relayProfileModeHelp(profile)}</span>
            </div>
          </div>
          </RelayFold>
        ) : null}
      </div>
      <div className="relay-bottom-options">
        <RelayFold className="relay-advanced-block" defaultOpen sectionId="advanced" title={t("更多选项")}>
          <p className="relay-fold-description">
            {t("包含上下文大小与压缩阈值；留空即沿用全局默认值。")}
          </p>
          <div className="relay-advanced-fields">
            <Field className="relay-field-context-window" label={t("上下文大小")}>
              <Input
                inputMode="numeric"
                value={profile.contextWindow}
                onChange={(event) => updateDraft({ contextWindow: normalizeTokenCountInput(event.currentTarget.value) })}
                placeholder={t("留空不改写，例如 200000")}
              />
            </Field>
            <Field className="relay-field-auto-compact" label={t("压缩上下文大小")}>
              <Input
                inputMode="numeric"
                value={profile.autoCompactLimit}
                onChange={(event) => updateDraft({ autoCompactLimit: normalizeTokenCountInput(event.currentTarget.value) })}
                placeholder={t("留空不改写，例如 160000")}
              />
            </Field>
          </div>
        </RelayFold>
      </div>
    </div>
  );
}

type VlmTestState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done"; result: TestVlmResult };

type TestVlmResult = {
  status: string;
  message: string;
  vlmStatus: string;
  httpCode: number | null;
  durationMs: number;
  error: string | null;
  description: string | null;
  model: string;
  rawRequest: string | null;
  rawResponse: string | null;
};

const VLM_TEST_MAX_IMAGE_BYTES = 10 * 1024 * 1024;

/// 方向 C：选图即测 + 排障增强（spec §4）。
/// 选完文件自动开跑；失败时给通俗诊断 + 复制错误 + 原始请求/响应折叠。
function VlmTestPanel({
  profile,
  onClose,
}: {
  profile: Pick<RelayProfile, "vlmApiKey" | "vlmModel" | "vlmBaseUrl">;
  onClose: () => void;
}) {
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [state, setState] = useState<VlmTestState>({ kind: "idle" });
  const [showRaw, setShowRaw] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const tr = (zh: string, params?: string[]) => (params ? tf(zh, params) : t(zh));

  const runTest = async (url: string) => {
    setState({ kind: "running" });
    setLocalError(null);
    try {
      const res = await invoke<TestVlmResult>("test_vlm", {
        request: {
          apiKey: profile.vlmApiKey,
          model: profile.vlmModel,
          baseUrl: profile.vlmBaseUrl,
          imageDataUrl: url,
        },
      });
      setState({ kind: "done", result: res });
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      setState({
        kind: "done",
        result: {
          status: "failed",
          message: msg,
          vlmStatus: "client_error",
          httpCode: null,
          durationMs: 0,
          error: msg,
          description: null,
          model: profile.vlmModel,
          rawRequest: null,
          rawResponse: null,
        },
      });
    }
  };

  // 选图即测：选完文件自动发起，无需二次点击（spec §4）。
  const onFile = (file: File | undefined) => {
    if (!file) return;
    if (!file.type.startsWith("image/")) {
      setLocalError(t("请选择图片文件。"));
      return;
    }
    if (file.size > VLM_TEST_MAX_IMAGE_BYTES) {
      setLocalError(t("图片超过 10MB，请换一张较小的图片。"));
      return;
    }
    const reader = new FileReader();
    reader.onerror = () => setLocalError(t("读取图片失败，请重新选择"));
    reader.onload = () => {
      const url = typeof reader.result === "string" ? reader.result : null;
      if (url) {
        setDataUrl(url);
        void runTest(url);
      }
    };
    reader.readAsDataURL(file);
  };

  // 复制完整排障信息（诊断 + 原始报文，无 API Key），可直接贴 issue（spec §3）。
  const copyError = async () => {
    if (state.kind !== "done") return;
    const r = state.result;
    const text = [
      vlmTestTranslation(r.vlmStatus, r.httpCode ?? undefined, r.durationMs, tr),
      `model: ${r.model}`,
      r.httpCode != null ? `HTTP ${r.httpCode}` : null,
      r.error ? `error: ${r.error}` : null,
      r.rawRequest ? `--- request ---\n${r.rawRequest}` : null,
      r.rawResponse ? `--- response ---\n${r.rawResponse}` : null,
    ]
      .filter((x) => x !== null)
      .join("\n");
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      setLocalError(t("复制失败，请从报文中手动复制。"));
    }
  };

  const done = state.kind === "done" ? state.result : null;
  const running = state.kind === "running";
  const formReady = !!profile.vlmApiKey && !!profile.vlmModel && !!profile.vlmBaseUrl;
  const canRun = !!dataUrl && formReady;

  return (
    <div className="vlm-test-panel">
      <div className="modal-head">
        <div>
          <h2>{t("测试 VLM")}</h2>
          <p className="modal-message">
            {t("选一张图片立即验证当前 VLM 配置（使用表单当前值，无需保存）。")}
          </p>
        </div>
        <button className="toast-close" aria-label={t("关闭窗口")} onClick={onClose} type="button">×</button>
      </div>

      <div className="vlm-test-upload">
        <input
          ref={fileInputRef}
          accept="image/*"
          onChange={(e) => {
            onFile(e.currentTarget.files?.[0]);
            e.currentTarget.value = "";
          }}
          type="file"
          style={{ display: "none" }}
        />
        <Button disabled={running || !formReady} onClick={() => fileInputRef.current?.click()} size="sm" type="button" variant="secondary">
          {dataUrl ? t("换图并测试") : t("选择图片并测试")}
        </Button>
        {dataUrl ? <img alt={t("图片预览")} className="vlm-test-preview" src={dataUrl} /> : null}
        {running ? (
          <p className="vlm-test-running">
            <span className="vlm-test-spinner" aria-hidden="true" />
            {t("正在调用 VLM…")}
          </p>
        ) : null}
      </div>

      {localError ? <p className="field-hint warn">{localError}</p> : null}

      {done ? (
        <div className="vlm-test-result">
          <p className="vlm-test-summary" role="status">
            {vlmTestTranslation(done.vlmStatus, done.httpCode ?? undefined, done.durationMs, tr)}
          </p>
          {done.description ? (
            <pre className="vlm-test-description">{done.description}</pre>
          ) : null}
          {done.vlmStatus !== "ok" ? (
            <button className="vlm-test-detail-toggle" onClick={() => void copyError()} type="button">
              {t("复制错误")}
            </button>
          ) : null}
          <button
            className="vlm-test-detail-toggle"
            aria-expanded={showRaw}
            onClick={() => setShowRaw((v) => !v)}
            type="button"
          >
            {showRaw ? t("隐藏原始报文") : t("显示原始报文")}
          </button>
          {showRaw ? (
            <div className="vlm-test-raw">
              <div className="label">{t("原始请求")}</div>
              <pre className="vlm-test-description">{done.rawRequest ?? "-"}</pre>
              <div className="label">{t("原始响应")}</div>
              <pre className="vlm-test-description">{done.rawResponse ?? "-"}</pre>
            </div>
          ) : null}
        </div>
      ) : null}

      <Toolbar>
        {dataUrl ? (
          <Button disabled={!canRun || running} onClick={() => dataUrl && void runTest(dataUrl)} type="button">
            {t("重测")}
          </Button>
        ) : null}
        <Button onClick={onClose} type="button" variant="secondary">
          {t("收起")}
        </Button>
      </Toolbar>
    </div>
  );
}

function AggregateModeChooser({ onSelect }: { onSelect: (mode: AggregateMode) => void }) {
  return (
    <div className="aggregate-mode-chooser">
      <div className="aggregate-mode-chooser-copy">
        <h2>{t("先选择类型")}</h2>
        <p>{t("故障转移与模型聚合彼此独立；选择后只显示对应的成员和路由表单。")}</p>
      </div>
      <div className="aggregate-mode-choice-grid">
        <button className="mode-option aggregate-mode-choice" onClick={() => onSelect("provider")} type="button">
          <strong>{t("故障转移")}</strong>
          <span>{t("多个供应商组成请求池，支持失败切换、会话轮转、请求轮转和权重。")}</span>
        </button>
        <button className="mode-option aggregate-mode-choice" onClick={() => onSelect("model")} type="button">
          <strong>{t("模型聚合")}</strong>
          <span>{t("把成员模型合并到 Codex 列表，选择模型后固定发送到对应供应商。")}</span>
        </button>
      </div>
    </div>
  );
}

function AggregateRelayProfileEditor({
  profile,
  form,
  onProfileChange,
}: {
  profile: RelayProfile;
  form: BackendSettings;
  onProfileChange: (value: RelayProfile) => void;
}) {
  const candidates = aggregateMemberCandidates(form.relayProfiles, profile.id);
  const aggregate = normalizeAggregateConfig(profile.aggregate, candidates);
  const modelMode = aggregate.modelAggregationEnabled === true;
  const memberIds = new Set(aggregate.members.map((member) => member.profileId));
  const sessionProvider = normalizeRelaySessionProvider(profile.sessionProvider);
  const updateAggregate = (nextAggregate: RelayAggregateConfig) => {
    onProfileChange(normalizeAggregateRelayProfile({ ...profile, aggregate: nextAggregate }, form));
  };
  const toggleMember = (profileId: string, checked: boolean) => {
    const members = checked
      ? [...aggregate.members, { profileId, weight: 1 }]
      : aggregate.members.filter((member) => member.profileId !== profileId);
    updateAggregate({ ...aggregate, members });
  };
  const updateWeight = (profileId: string, weight: number) => {
    updateAggregate({
      ...aggregate,
      members: aggregate.members.map((member) =>
        member.profileId === profileId ? { ...member, weight: clampAggregateWeight(weight) } : member,
      ),
    });
  };
  const totalWeight = aggregate.members.reduce((total, member) => total + clampAggregateWeight(member.weight), 0);
  // 只有"纯官方登录"成员才有 ChatGPT 凭据可同步；带 URL/Key 的官方混入走 API 通道。
  const defaultMemberCandidate = candidates.find((candidate) => candidate.id === aggregate.defaultRelayId);
  const defaultMemberIsOfficialLogin = !!defaultMemberCandidate
    && defaultMemberCandidate.relayMode === "official"
    && !defaultMemberCandidate.officialMixApiKey;
  const routes = aggregate.routes ?? [];
  const routeTargetOptions = aggregate.members
    .map((member) => {
      const candidate = candidates.find((item) => item.id === member.profileId);
      return { value: member.profileId, label: candidate?.name || t("未命名供应商") };
    })
    .filter((option) => option.value.trim() !== "");
  const updateRoute = (index: number, patch: Partial<RelayAggregateRoute>) => {
    updateAggregate({
      ...aggregate,
      routes: routes.map((route, routeIndex) => (routeIndex === index ? { ...route, ...patch } : route)),
    });
  };
  const removeRoute = (index: number) => {
    updateAggregate({ ...aggregate, routes: routes.filter((_, routeIndex) => routeIndex !== index) });
  };
  const addRoute = () => {
    updateAggregate({
      ...aggregate,
      routes: [...routes, { pattern: "", profileId: aggregate.members[0]?.profileId ?? "", priority: 0 }],
    });
  };
  // 模型聚合的最终效果：默认成员的模型以裸名发布，其余成员带 cpx- 前缀。勾选时就告诉用户 Codex 会看到什么。
  const memberModelCount = (candidate: RelayProfile) => new Set([
    ...candidate.modelList.split(/[\r\n,]+/).map((line) => line.trim()).filter(Boolean),
    ...(candidate.model.trim() ? [candidate.model.trim()] : []),
  ]).size;
  const selectedCandidates = candidates.filter((candidate) => memberIds.has(candidate.id));
  const defaultCandidate = selectedCandidates.find((candidate) => candidate.id === aggregate.defaultRelayId);
  const bareModelCount = defaultCandidate ? memberModelCount(defaultCandidate) : 0;
  const prefixedModelCount = selectedCandidates
    .filter((candidate) => candidate.id !== defaultCandidate?.id)
    .reduce((sum, candidate) => sum + memberModelCount(candidate), 0);

  return (
    <div className="relay-profile-editor aggregate-editor" data-aggregate-mode={modelMode ? "model" : "provider"}>
      <div className="relay-fields aggregate-fields">
        <Field className="relay-field-name" label={t("名称")}>
          <Input
            value={profile.name}
            onChange={(event) => onProfileChange({ ...profile, name: event.currentTarget.value })}
            placeholder={t("例如 主力聚合池")}
          />
        </Field>
        {!modelMode ? (
          <Field className="aggregate-strategy-field" label={t("聚合策略")}>
            <AppSelect
              value={aggregate.strategy}
              onChange={(value) => updateAggregate({ ...aggregate, strategy: value })}
              options={aggregateStrategyOptions.map((option) => ({ value: option.value, label: option.label }))}
            />
          </Field>
        ) : null}
        <Field className="relay-field-session-provider" label={t("Codex 会话身份")}>
          <AppSelect
            value={sessionProvider}
            onChange={(value) => onProfileChange(normalizeAggregateRelayProfile({ ...profile, sessionProvider: value }, form))}
            options={[
              { value: "custom", label: t("Custom（默认）") },
              { value: "openai", label: t("OpenAI（兼容 ChatGPT Remote）") },
            ]}
          />
          <p className="field-hint">
            {modelMode
              ? t("模型聚合请求仍由本地 Responses 代理转发；OpenAI 身份用于让 ChatGPT Remote 识别会话。")
              : t("聚合请求仍由本地 Responses 代理轮转成员；OpenAI 身份用于让 ChatGPT Remote 识别会话。")}
          </p>
        </Field>
      </div>
      {!modelMode ? (
        <div className="aggregate-strategy-grid">
          {aggregateStrategyOptions.map((option) => (
            <button
              className={`mode-option aggregate-strategy-option ${aggregate.strategy === option.value ? "active" : ""}`}
              key={option.value}
              onClick={() => updateAggregate({ ...aggregate, strategy: option.value })}
              type="button"
            >
              <strong>{option.label}</strong>
              <span>{option.description}</span>
            </button>
          ))}
        </div>
      ) : (
        null
      )}
      <div className="aggregate-members">
        <div className="aggregate-members-head">
          <div>
            <strong>{t("成员供应商")}</strong>
            <span>
              {modelMode
                ? t("默认成员的模型以裸名发布，其他成员带 cpx- 前缀；悬停已勾选的成员可「设为默认」。本模式不使用轮转策略或通配符路由。")
                : t("可勾选 API 供应商或已完成 ChatGPT 登录的官方账号；聚合供应商不会作为成员。")}
            </span>
            {modelMode && selectedCandidates.length ? (
              <span className="aggregate-model-summary">
                {defaultCandidate
                  ? tf("Codex 将看到 {0} 个裸名模型（来自 {1}）+ {2} 个 cpx- 前缀模型", [bareModelCount, defaultCandidate.name || t("未命名供应商"), prefixedModelCount])
                  : tf("Codex 将看到 {0} 个 cpx- 前缀模型；还没指定默认成员，不带前缀的请求会报错", [prefixedModelCount])}
              </span>
            ) : null}
          </div>
          <UiBadge variant="outline">{aggregate.members.length} / {candidates.length}</UiBadge>
        </div>
        {candidates.length ? (
          <div className="aggregate-member-list">
            {candidates.map((candidate) => {
              const member = aggregate.members.find((item) => item.profileId === candidate.id);
              const checked = memberIds.has(candidate.id);
              const isDefaultMember = !!aggregate.defaultRelayId && aggregate.defaultRelayId === candidate.id;
              // 只显示模型数量 + 行内「设为默认」：整串模型名会把每张卡片撑成不同高度。
              // 完整名单放 title；「默认」和勾选在同一行，不再需要下方单独的下拉。
              const memberModelNames = [...new Set([
                ...candidate.modelList.split(/[\r\n,]+/).map((line) => line.trim()).filter(Boolean),
                ...(candidate.model.trim() ? [candidate.model.trim()] : []),
              ])];
              return (
                <label className={`aggregate-member-row ${checked ? "selected" : ""}`} key={candidate.id}>
                  <input
                    checked={checked}
                    onChange={(event) => toggleMember(candidate.id, event.currentTarget.checked)}
                    type="checkbox"
                  />
                  <span className="aggregate-member-summary">
                    {/* 与供应商列表一致：接入方式做成标签，第二行只放地址。 */}
                    <span className="aggregate-member-title">
                      <strong>{candidate.name || t("未命名供应商")}</strong>
                      <span className="relay-tag" data-tone={relayProfileTagTone(candidate)}>{relayProfileModeLabel(candidate)}</span>
                      {candidate.protocol !== "responses" ? (
                        <span className="relay-tag" data-tone="protocol">{relayProtocolShortLabel(candidate.protocol)}</span>
                      ) : null}
                    </span>
                    <small>
                      {isOfficialLoginMemberProfile(candidate)
                        ? t("使用已登录的 ChatGPT 账号凭据")
                        : relayProfileConfigBrief(candidate)}
                    </small>
                  </span>
                  {modelMode ? (
                    <span className="aggregate-member-models">
                      <span
                        className="aggregate-member-models-count"
                        title={memberModelNames.length ? memberModelNames.join("\n") : undefined}
                      >
                        {memberModelNames.length
                          ? tf("{0} 个模型", [memberModelNames.length])
                          : (isOfficialLoginMemberProfile(candidate)
                            ? t("请填写该官方账号的模型列表")
                            : t("未填写模型"))}
                      </span>
                      {checked ? (isDefaultMember ? (
                        <UiBadge className="aggregate-member-default-badge" variant="secondary">
                          {t("默认")}
                        </UiBadge>
                      ) : (
                        <Button
                          className="aggregate-member-default-action"
                          onClick={(event) => {
                            // 行本身是 <label>：不拦默认行为会顺带切换勾选状态。
                            event.preventDefault();
                            event.stopPropagation();
                            updateAggregate({ ...aggregate, defaultRelayId: candidate.id });
                          }}
                          size="sm"
                          title={t("不带 cpx- 前缀的请求（旧会话、自动压缩带的模型名）会原样交给它")}
                          type="button"
                          variant="ghost"
                        >
                          {t("设为默认")}
                        </Button>
                      )) : null}
                    </span>
                  ) : (
                    <span className="aggregate-weight-box">
                      <span>{t("权重")}</span>
                      <Input
                        disabled={!checked}
                        min={1}
                        onChange={(event) => updateWeight(candidate.id, Number.parseInt(event.currentTarget.value, 10))}
                        type="number"
                        value={String(member?.weight ?? 1)}
                      />
                    </span>
                  )}
                </label>
              );
            })}
          </div>
        ) : (
          <div className="empty">{t("先添加至少 1 个可用的 API 供应商或官方登录账号，再创建聚合供应商。")}</div>
        )}
      </div>
      {modelMode ? (
        <div className="aggregate-default-provider aggregate-default-hint">
          <span>
            {aggregate.defaultRelayId
              ? tf("默认供应商：{0}。不带 cpx- 前缀的请求（旧会话、自动压缩带的模型名等）会原样交给它；带 cpx- 前缀但解析不出来的 ID 一律报错，不回落。", [
                candidates.find((candidate) => candidate.id === aggregate.defaultRelayId)?.name || aggregate.defaultRelayId,
              ])
              : t("还没指定默认供应商：在上方成员行尾点「设为默认」。不带 cpx- 前缀的请求会原样交给它，没设就报错。")}
          </span>
          {defaultMemberIsOfficialLogin ? (
            <label className="aggregate-default-login-sync">
              <input
                checked={aggregate.syncDefaultMemberLogin === true}
                onChange={(event) => updateAggregate({
                  ...aggregate,
                  syncDefaultMemberLogin: event.currentTarget.checked,
                })}
                type="checkbox"
              />
              <span>
                <strong>{t("把默认成员的官方登录同步到 Codex")}</strong>
                <small>{t("开启后切换到本聚合时，会把默认成员的 ChatGPT 凭据一并写进 Codex（代理 Key 保留），账号在 Codex 里就是登录态。默认成员不是官方登录时无效。")}</small>
              </span>
            </label>
          ) : (
            <span className="aggregate-default-login-note">
              {t("当前默认成员不是官方登录账号，无法把登录同步到 Codex；只有「官方登录」类型的成员才有 ChatGPT 凭据。")}
            </span>
          )}
        </div>
      ) : null}
      {!modelMode ? <div className="aggregate-routes">
        <div className="aggregate-routes-head">
          <div>
            <strong>{t("路由规则")}</strong>
            <span>{t("按模型名自动路由到指定成员；仅支持 * 通配符，chat/completions 协议不走路由。")}</span>
          </div>
          <UiBadge variant="outline">{routes.length}</UiBadge>
        </div>
        {routes.length ? (
          <div className="aggregate-route-list">
            {routes.map((route, index) => (
              <div className="aggregate-route-row" key={index}>
                <Input
                  onChange={(event) => updateRoute(index, { pattern: event.currentTarget.value })}
                  placeholder={t("例如 deepseek-*")}
                  value={route.pattern}
                />
                <AppSelect
                  onChange={(value) => updateRoute(index, { profileId: value })}
                  options={routeTargetOptions}
                  value={route.profileId}
                />
                {!routeTargetOptions.some((option) => option.value === route.profileId) ? (
                  <span className="aggregate-route-target-error">{t("路由目标必须是已勾选的聚合成员，请先在成员供应商中勾选。")}</span>
                ) : null}
                <div className="aggregate-route-priority">
                  <span>{t("优先级")}</span>
                  <Input
                    min={0}
                    onChange={(event) =>
                      updateRoute(index, { priority: clampAggregateRoutePriority(Number.parseInt(event.currentTarget.value, 10)) })
                    }
                    type="number"
                    value={String(route.priority)}
                  />
                </div>
                <button
                  className="aggregate-route-remove"
                  onClick={() => removeRoute(index)}
                  title={t("删除规则")}
                  type="button"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </div>
            ))}
          </div>
        ) : (
          <div className="empty">{t("暂无路由规则，未匹配的模型会按聚合策略选择成员。")}</div>
        )}
        <div>
          <Button disabled={!aggregate.members.length} onClick={addRoute} size="sm" variant="secondary">
            <Plus className="h-4 w-4" />
            {t("添加规则")}
          </Button>
        </div>
      </div> : null}
      {!modelMode ? (
        <>
          <div className="relay-grid compact aggregate-preview">
            <Metric label={t("策略")} value={aggregateStrategyLabel(aggregate.strategy)} />
            <Metric label={t("成员数量")} value={tf("{0} 个", [aggregate.members.length])} />
            <Metric label={t("总权重")} value={`${totalWeight}`} />
            <Metric label={t("序列化字段")} value="aggregate.strategy / aggregate.members / aggregate.routes" />
          </div>
          <div className="hint-line relay-protocol-hint">
            <ShieldCheck className="h-4 w-4" />
            <span>{aggregateStrategyHelp(aggregate.strategy)}</span>
          </div>
        </>
      ) : null}
    </div>
  );
}

function RelayContextManager({
  form,
  liveEntries,
  relayFiles,
  onFormChange,
  actions,
}: {
  form: BackendSettings;
  liveEntries: CodexContextEntries | null;
  relayFiles: RelayFilesResult | null;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const entries = contextEntriesWithLiveEntries(form, liveEntries);
  const [activeKind, setActiveKind] = useState<ContextKind>("mcp");
  const [editor, setEditor] = useState<{ kind: ContextKind; entry?: CodexContextEntry } | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [skillInventory, setSkillInventory] = useState<SkillInventoryResult | null>(null);
  const refreshSkillInventory = async () => {
    try {
      setSkillInventory(await invoke<SkillInventoryResult>("list_installed_skills"));
    } catch (error) {
      setSkillInventory({ status: "failed", message: stringifyError(error), skills: [], codexSkillsDir: "" });
      await actions.showMessage("Skills", stringifyError(error), "failed");
    }
  };
  useEffect(() => { void refreshSkillInventory(); }, []);
  const visibleEntries = contextEntriesByKind(entries, activeKind);
  const label = contextKindLabel(activeKind);

  const syncContextEntries = async (next: BackendSettings) => {
    const syncResult = await actions.syncLiveContextEntries(next, true);
    if (!syncResult || !isSuccessStatus(syncResult.status)) return false;
    await actions.refreshRelayFiles();
    return true;
  };

  const saveEntry = async (kind: ContextKind, id: string, tomlBody: string) => {
    const next = await actions.upsertContextEntry(form, kind, id, tomlBody);
    if (!next) return;
    onFormChange(next);
    if (!(await syncContextEntries(next))) return;
    setEditor(null);
  };

  const toggleContextEntryEnabled = async (entry: CodexContextEntry) => {
    const nextBody = setContextEntryEnabled(entry.tomlBody, !entry.enabled);
    const next = await actions.upsertContextEntry(form, entry.kind, entry.id, nextBody);
    if (!next) return;
    onFormChange(next);
    await syncContextEntries(next);
  };

  const deleteEntry = async (entry: CodexContextEntry) => {
    const next = await actions.deleteContextEntry(form, entry.kind, entry.id);
    if (!next) return;
    onFormChange(next);
    await syncContextEntries(next);
  };

  const importJson = async (json: string) => {
    const next = await actions.importMcpServersJson(form, json);
    if (!next) return;
    onFormChange(next);
    if (!(await syncContextEntries(next))) return;
    setImportOpen(false);
  };

  return (
    <div className="relay-context-panel">
      <div className="relay-context-head">
        <div>
          <strong>{t("Codex MCP&插件")}</strong>
          <span>{t("MCP 与插件作为全局配置独立管理，切换任意供应商都会合并。")}</span>
        </div>
        <div className="relay-context-head-actions">
          {activeKind === "mcp" ? (
            <Button
              onClick={() => {
                setEditor(null);
                setImportOpen(true);
              }}
              size="sm"
              variant="secondary"
            >
              <Download className="h-4 w-4" />
              {t("导入 JSON")}
            </Button>
          ) : null}
          {activeKind === "skill" ? (
            <Button onClick={() => void refreshSkillInventory()} size="sm" variant="secondary"><RefreshCw className="h-4 w-4" />{t("刷新列表")}</Button>
          ) : <Button onClick={() => setEditor({ kind: activeKind })} size="sm" variant="secondary">
            <Plus className="h-4 w-4" />
            {t("新增")}{label}
          </Button>}
        </div>
      </div>
      <div className="segmented">
        {contextKindOptions.map((option) => (
          <button
            className={activeKind === option.kind ? "active" : ""}
            key={option.kind}
            onClick={() => setActiveKind(option.kind)}
            type="button"
          >
            <span>{option.label}</span>
            <small>{option.kind === "skill" ? (skillInventory ? (isSuccessStatus(skillInventory.status) ? skillInventory.skills.length : "?") : "…") : contextEntriesByKind(entries, option.kind).length}</small>
          </button>
        ))}
      </div>
      <div className="relay-context-summary">
        {activeKind === "skill" ? <>{t("本地与内置 Skills 由 Codex 从目录发现。")}{skillInventory ? <span className="break-all"> {skillInventory.codexSkillsDir}</span> : null}</> : <>{t("当前共有")} {visibleEntries.length} {t("个")}{label}{t("；这些条目独立于供应商保存，会写入所有供应商切换后的 config.toml。")}</>}
      </div>
      <div className="relay-context-list">
        {activeKind === "skill" ? (
          !skillInventory ? <div className="empty">{t("正在读取 Skills…")}</div>
          : !isSuccessStatus(skillInventory.status) ? <div className="empty">{skillInventory.message}</div>
          : skillInventory.skills.length ? skillInventory.skills.map((skill) => (
            <div className="relay-context-row" key={`local-skill-${skill.id}`}>
              <div><strong className="context-title">{skill.name || skill.id}</strong><div className="text-xs">{skill.description}</div></div>
              <span>{skill.bundled ? t("内置") : t("本地")}{!skill.enabled ? ` · ${t("已停用")}` : ""}</span>
            </div>
          )) : <div className="empty">{t("Skills 目录中暂无技能。")}</div>
        ) : visibleEntries.length ? (
          visibleEntries.map((entry) => (
            <div className="relay-context-row" key={`${entry.kind}-${entry.id}`}>
              <strong className="context-title">{entry.title || entry.id}</strong>
              <div className="relay-context-actions">
                <button
                  aria-checked={entry.enabled}
                  aria-label={`contextEnabledSwitch-${entry.kind}-${entry.id}`}
                  className={`context-enabled-switch ${entry.enabled ? "active" : ""}`}
                  onClick={() => void toggleContextEntryEnabled(entry)}
                  role="switch"
                  title={entry.enabled ? t("禁用此扩展项") : t("启用此扩展项")}
                  type="button"
                >
                  <span className="context-switch-track" aria-hidden="true">
                    <span className="context-switch-thumb" />
                  </span>
                </button>
                <Button onClick={() => setEditor({ kind: entry.kind, entry })} size="icon" title={t("编辑扩展项")} variant="ghost">
                  <Edit3 className="h-4 w-4" />
                </Button>
                <Button
                  className="relay-context-delete"
                  onClick={() => void deleteEntry(entry)}
                  size="icon"
                  title={t("删除扩展项")}
                  variant="ghost"
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ))
        ) : (
          <div className="empty">{t("暂无")}{label}{t("，可以从通用配置文件或这里新增。")}</div>
        )}
      </div>
      {importOpen ? (
        <McpJsonImporter actions={actions} onCancel={() => setImportOpen(false)} onImport={importJson} />
      ) : null}
      {editor ? (
        <ContextEntryEditor
          entry={editor.entry}
          kind={editor.kind}
          onCancel={() => setEditor(null)}
          onSave={(kind, id, tomlBody) => void saveEntry(kind, id, tomlBody)}
        />
      ) : null}
    </div>
  );
}

function McpJsonImporter({
  actions,
  onCancel,
  onImport,
}: {
  actions: Actions;
  onCancel: () => void;
  onImport: (json: string) => void;
}) {
  const [json, setJson] = useState("");
  const [preview, setPreview] = useState<McpImportPreviewResult | null>(null);

  const runPreview = async () => {
    const result = await actions.previewMcpServersJson(json);
    setPreview(result && isSuccessStatus(result.status) ? result : null);
  };

  return (
    <div aria-modal="true" className="modal-backdrop" role="dialog">
      <div className="modal-card context-modal">
        <div className="modal-head">
          <div>
            <h2>{t("导入 MCP JSON")}</h2>
            <p className="modal-message">{t("支持 mcpServers / servers 包裹，也支持直接粘贴单个服务器配置。")}</p>
          </div>
          <button aria-label={t("关闭窗口")} className="toast-close" onClick={onCancel} type="button">×</button>
        </div>
        <Field label={t("MCP 配置 JSON")}>
          <Textarea
            className="context-editor-textarea"
            value={json}
            onChange={(event) => {
              setJson(event.currentTarget.value);
              setPreview(null);
            }}
            placeholder={'{\n  "mcpServers": {\n    "context7": {\n      "command": "npx",\n      "args": ["-y", "@upstash/context7-mcp"]\n    }\n  }\n}'}
            spellCheck={false}
          />
        </Field>
        {preview ? (
          <div className="relay-context-summary">
            <div>{tf("将导入 {0} 个：{1}", [preview.entries.length, preview.entries.map((item) => item.id).join("、")])}</div>
            {preview.warnings.map((warning) => <div key={warning}>{warning}</div>)}
          </div>
        ) : null}
        <Toolbar>
          <Button disabled={!json.trim()} onClick={() => void runPreview()} size="sm" variant="secondary">{t("预览")}</Button>
          <Button disabled={!preview} onClick={() => onImport(json)} size="sm">
            <Download className="h-4 w-4" />
            {t("确认导入")}
          </Button>
          <Button onClick={onCancel} size="sm" variant="secondary">{t("取消")}</Button>
        </Toolbar>
      </div>
    </div>
  );
}

function ContextEntryEditor({
  kind,
  entry,
  onCancel,
  onSave,
}: {
  kind: ContextKind;
  entry?: CodexContextEntry;
  onCancel: () => void;
  onSave: (kind: ContextKind, id: string, tomlBody: string) => void;
}) {
  const [draftKind, setDraftKind] = useState<ContextKind>(entry?.kind ?? kind);
  const [id, setId] = useState(entry?.id ?? "");
  const [tomlBody, setTomlBody] = useState(entry?.tomlBody ?? "");
  const canSave = id.trim().length > 0;

  return (
    <div className="context-editor">
      <div className="context-editor-fields">
        <Field label={t("类型")}>
          <AppSelect
            disabled={!!entry}
            value={draftKind}
            onChange={(value) => setDraftKind(value)}
            options={contextKindOptions.map((option) => ({ value: option.kind, label: option.label }))}
          />
        </Field>
        <Field label="ID">
          <Input
            disabled={!!entry}
            value={id}
            onChange={(event) => setId(event.currentTarget.value.trim())}
            placeholder={t("例如 context7")}
          />
        </Field>
      </div>
      <Field label={t("TOML 配置体")}>
        <Textarea
          className="context-editor-textarea"
          value={tomlBody}
          onChange={(event) => setTomlBody(event.currentTarget.value)}
          placeholder={t("只填写表头下面的内容，例如：\ncommand = \"npx\"\nargs = [\"-y\", \"@upstash/context7-mcp\"]")}
          spellCheck={false}
        />
      </Field>
      <Toolbar>
        <Button disabled={!canSave} onClick={() => onSave(draftKind, id.trim(), tomlBody)} size="sm">
          <Save className="h-4 w-4" />
          {t("保存扩展项")}
        </Button>
        <Button onClick={onCancel} size="sm" variant="secondary">{t("取消")}</Button>
      </Toolbar>
    </div>
  );
}

function SyncedTextarea({
  value,
  onValueChange,
  className,
}: {
  value: string;
  onValueChange: (value: string) => void;
  className?: string;
}) {
  const [localValue, setLocalValue] = useState(value);
  const isFocusedRef = useRef(false);
  const latestExternalValueRef = useRef(value);

  useEffect(() => {
    latestExternalValueRef.current = value;
    if (!isFocusedRef.current) {
      setLocalValue(value);
    }
  }, [value]);

  return (
    <Textarea
      className={className}
      value={localValue}
      onBlur={() => {
        isFocusedRef.current = false;
        setLocalValue(latestExternalValueRef.current);
      }}
      onChange={(event) => {
        const next = event.currentTarget.value;
        setLocalValue(next);
        onValueChange(next);
      }}
      onFocus={() => {
        isFocusedRef.current = true;
      }}
      spellCheck={false}
    />
  );
}

function RelayFileEditors({
  contextProfile,
  profile,
  form,
  isActive,
  profileId,
  onFormChange,
  onProfileChange,
  actions,
}: {
  contextProfile: RelayProfile;
  profile: RelayProfile;
  form: BackendSettings;
  isActive: boolean;
  profileId: string;
  onFormChange: (value: BackendSettings) => void;
  onProfileChange: (value: RelayProfile) => void;
  actions: Actions;
}) {
  const configPreview = effectiveRelayConfigPreview(profile, form, contextProfile);
  const entries = contextEntriesForProfile(form, contextProfile);
  return (
    <div className="relay-file-grid">
      <RelayFold className="relay-file-panel relay-config-preview" sectionId="config-preview" title={t("config.toml 预览")}>
        <p className="relay-fold-description">
          {isActive ? t("当前供应商切换后会写入的预览；上下文开关变化会立即反映") : t("切换到此供应商时会写入的预览；上下文开关变化会立即反映")}
        </p>
        <SyncedTextarea
          className="relay-file-textarea"
          value={configPreview}
          onValueChange={(value) => {
            const withoutCommon = stripCommonConfigTextFallback(
              value,
              relayCombinedCommonConfig(form),
            );
            const configContents = stripContextEntriesFromConfig(withoutCommon, entries);
            onProfileChange(deriveRelayProfileFromFiles({
              ...profile,
              configContents,
            }, "config"));
          }}
        />
      </RelayFold>
      <RelayFold
        bodyClassName="relay-common-config-body"
        className="relay-file-panel relay-common-config-panel"
        defaultOpen
        sectionId="common-config"
        title={t("通用配置文件")}
      >
        <p className="relay-fold-description">
          {t("只保留非 MCP、插件的跨供应商配置；MCP&插件在独立页面管理。点此展开编辑。")}
        </p>
            <Button
              onClick={async () => {
                const extracted = await actions.extractRelayCommonConfig(profile.configContents || "");
                if (!extracted) return;
                const split = splitContextConfigText(extracted.commonConfigContents || "");
                if (!split.common.trim() && !split.context.trim()) {
                  await actions.showMessage(t("通用配置文件"), t("当前供应商 config.toml 里没有可提取的通用配置。"), "failed");
                  return;
                }
                const promotedProfile = {
                  ...profile,
                  configContents: extracted.profileConfigContents,
                };
                const next = syncLegacyRelayFields({
                  ...form,
                  relayCommonConfigContents: split.common,
                  relayContextConfigContents: joinTomlSectionsRootFirst([form.relayContextConfigContents || "", split.context]),
                  relayProfiles: form.relayProfiles.map((item) => (item.id === profileId ? promotedProfile : item)),
                });
                onFormChange(next);
                onProfileChange(promotedProfile);
                await actions.saveSettingsValue(next, false);
              }}
              size="sm"
              type="button"
              variant="secondary"
            >
              <Download className="h-4 w-4" />
              {t("提取当前供应商配置")}
            </Button>
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={form.relayPreserveNonproviderSettings === true}
              onChange={(event) => onFormChange({ ...form, relayPreserveNonproviderSettings: event.currentTarget.checked })} />
            {t("切换时保留缺失的项目信任设置与通知命令")}
          </label>
          <SyncedTextarea
            className="relay-file-textarea"
            value={form.relayCommonConfigContents}
            onValueChange={(value) => onFormChange({ ...form, relayCommonConfigContents: value })}
          />
      </RelayFold>
      <RelayFold className="relay-file-panel relay-auth-preview" defaultOpen sectionId="auth-preview" title="auth.json">
        <p className="relay-fold-description">
          {isActive
            ? profile.relayMode === "pureApi"
              ? t("当前使用中：保留此供应商的 auth 存档，避免 Codex 登录密钥覆盖供应商密钥")
              : t("当前使用中：打开时从 Codex 主目录的 auth.json 回填，保存后会作为此供应商 auth 存档")
            : t("切换到此供应商时会写入 Codex 主目录的 auth.json")}
        </p>
        <SyncedTextarea
          className="relay-file-textarea"
          value={profile.authContents}
          onValueChange={(value) => onProfileChange(deriveRelayProfileFromFiles({ ...profile, authContents: value }, "auth"))}
        />
      </RelayFold>
    </div>
  );
}

function ProviderDoctorModal({
  result,
  running,
  onClose,
}: {
  result: ProviderDoctorResult | null;
  running: boolean;
  onClose: () => void;
}) {
  const steps = providerDoctorSteps(result, running);
  const doneCount = steps.filter((step) => step.state === "ok" || step.state === "warning" || step.state === "failed").length;
  const progress = Math.round((doneCount / steps.length) * 100);
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card provider-doctor-modal">
        <div className="modal-head">
          <div>
            <h2>Provider Doctor</h2>
            <p>{running ? t("正在诊断供应商，请稍候。") : result?.summary ?? t("诊断已完成。")}</p>
          </div>
          <UiBadge variant={result && !isSuccessStatus(result.status) ? "outline" : "secondary"}>
            {running ? t("诊断中") : result && !isSuccessStatus(result.status) ? t("异常") : t("完成")}
          </UiBadge>
        </div>
        <div className="provider-doctor-progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress} role="progressbar">
          <div style={{ width: `${progress}%` }} />
        </div>
        <div className="provider-doctor-step-list">
          {steps.map((step) => (
            <div className={`provider-doctor-step ${step.state}`} key={step.id}>
              <span className="provider-doctor-step-icon">
                {step.state === "running" ? (
                  <RefreshCw className="h-4 w-4" />
                ) : step.state === "ok" ? (
                  <CheckCircle2 className="h-4 w-4" />
                ) : step.state === "warning" ? (
                  <ShieldAlert className="h-4 w-4" />
                ) : step.state === "failed" ? (
                  <Info className="h-4 w-4" />
                ) : (
                  <span />
                )}
              </span>
              <div>
                <strong>{step.title}</strong>
                <small>{step.detail}</small>
              </div>
            </div>
          ))}
        </div>
        {result?.recommendation ? <p className="provider-doctor-recommendation">{result.recommendation}</p> : null}
        <div className="modal-actions">
          <Button disabled={running} onClick={onClose} variant="secondary">
            {running ? t("诊断中") : t("关闭")}
          </Button>
        </div>
      </div>
    </div>
  );
}

type ProviderDoctorStepState = "pending" | "running" | "ok" | "warning" | "failed";

function providerDoctorSteps(
  result: ProviderDoctorResult | null,
  running: boolean,
): Array<{ id: string; title: string; detail: string; state: ProviderDoctorStepState }> {
  const base = [
    { id: "config", title: t("配置完整性"), pending: t("等待检查 Base URL / API Key。") },
    { id: "models", title: t("模型列表"), pending: t("等待检查 /v1/models。") },
    { id: "request", title: t("真实请求"), pending: t("等待发送一次测试请求。") },
    { id: "recommendation", title: t("处理建议"), pending: t("等待生成建议。") },
  ];
  if (!result) {
    return base.map((step, index) => ({
      id: step.id,
      title: step.title,
      detail: index === 0 && running ? t("正在检查配置完整性…") : step.pending,
      state: index === 0 && running ? "running" : "pending",
    }));
  }
  const checks = new Map(result.checks.map((check) => [check.id, check]));
  return base.map((step) => {
    if (step.id === "recommendation") {
      return {
        id: step.id,
        title: step.title,
        detail: result.recommendation || step.pending,
        state: result.status === "failed" ? "warning" : "ok",
      };
    }
    const check = checks.get(step.id);
    if (!check) {
      return {
        id: step.id,
        title: step.title,
        detail: step.id === "models" || step.id === "request" ? t("该步骤未执行。") : step.pending,
        state: "pending",
      };
    }
    return {
      id: step.id,
      title: check.title || step.title,
      detail: check.detail,
      state: check.status === "ok" ? "ok" : check.status === "warning" ? "warning" : "failed",
    };
  });
}

function FeatureItem({ title, detail, enabled }: { title: string; detail: string; enabled: boolean }) {
  return (
    <div className="feature-item">
      <div>
        <strong>{title}</strong>
        <span>{detail}</span>
      </div>
      <Badge status={enabled ? "ok" : "disabled"} />
    </div>
  );
}

/**
 * 设置类页面（增强、设置）左侧的分类目录：点一下滚到对应分组，滚动时高亮当前所在的分组。
 * `children` 放在目录最上面（增强页把「常用增强 / 语音输入 / 下一步建议」页签放在这里）。
 */
function PageSectionNav({ label, sections, children }: { label: string; sections: Array<{ id: string; label: string }>; children?: ReactNode }) {
  const [current, setCurrent] = useState(sections[0]?.id ?? "");
  const sectionKey = sections.map((section) => section.id).join("|");
  useEffect(() => {
    const nodes = sections.map((section) => document.getElementById(section.id)).filter((node): node is HTMLElement => Boolean(node));
    if (!nodes.length) return;
    const scroller = nodes[0].closest(".screen");
    const sync = () => {
      const line = (scroller?.getBoundingClientRect().top ?? 0) + 96;
      let active = nodes[0].id;
      for (const node of nodes) if (node.getBoundingClientRect().top <= line) active = node.id;
      setCurrent(active);
    };
    sync();
    scroller?.addEventListener("scroll", sync, { passive: true });
    return () => scroller?.removeEventListener("scroll", sync);
    // sections 每次渲染都是新数组，按 id 列表判断是否变化。
  }, [sectionKey]);
  return (
    <nav aria-label={label} className="page-section-nav">
      {children}
      {sections.length ? (
        <div className="page-section-nav-list">
          {sections.map((section) => (
            <button
              aria-current={current === section.id ? "true" : undefined}
              key={section.id}
              onClick={() => {
                setCurrent(section.id);
                document.getElementById(section.id)?.scrollIntoView({ block: "start", behavior: "smooth" });
              }}
              type="button"
            >
              {section.label}
            </button>
          ))}
        </div>
      ) : null}
    </nav>
  );
}

function FeatureGroup({ title, detail, children, id }: { title: string; detail: string; children: ReactNode; id?: string }) {
  return (
    <section className="feature-group" id={id}>
      <div className="feature-group-head">
        <strong>{title}</strong>
        <small>{detail}</small>
      </div>
      <div className="feature-switch-grid">{children}</div>
    </section>
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
          {experimental ? <em className="feature-tag" data-tone="experimental">{t("实验")}</em> : null}
          {needsRestart ? <em className="feature-tag">{t("需重启")}</em> : null}
        </strong>
        <small>{detail}</small>
      </span>
      <ToggleVisual />
    </label>
  );
}

/** 应用设置页（含原「关于」）左侧目录的分组，id 对应各张卡片。 */
function appSettingsSections() {
  return [
    { id: "app-settings-general", label: t("基础设置") },
    { id: "app-settings-config", label: t("配置文件") },
    { id: "app-settings-update", label: t("版本与更新") },
    { id: "app-settings-about", label: t("关于") },
    { id: "app-settings-logs", label: t("日志") },
    { id: "app-settings-diagnostics", label: t("诊断") },
  ];
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

/** 设置页（含安装维护）左侧目录的分组，id 对应各张卡片。 */
function codexSettingsSections() {
  return [
    { id: "codex-settings-overlay", label: t("图片覆盖层") },
    { id: "codex-settings-launch-args", label: t("启动参数") },
    { id: "codex-settings-check", label: t("检查与修复") },
    { id: "codex-settings-startup", label: t("开机启动") },
    { id: "codex-settings-takeover", label: t("自动接管") },
    { id: "codex-settings-app-path", label: t("应用路径") },
    { id: "codex-settings-manual-launch", label: t("手动启动") },
  ];
}

const EXPERIMENTAL_TITLE = /\s*(?:（实验性?）|\(experimental\))$/i;
const RESTART_HINT = /重启 ?Codex|重新启动 ?Codex|下次启动 ?Codex|restart(?:ing)? Codex|relaunch/i;

function ToggleVisual() {
  return (
    <span aria-hidden="true" className="toggle-switch-visual">
      <span className="toggle-switch-thumb" />
    </span>
  );
}

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

function formatProgressPercent(value: number): string {
  if (!Number.isFinite(value)) return "0.00";
  return Math.min(100, Math.max(0, value)).toFixed(2);
}

function GuideList({ items }: { items: string[] }) {
  return (
    <div className="guide-list">
      {items.map((item, index) => (
        <div className="guide-step" key={item}>
          <span>{index + 1}</span>
          <p>{item}</p>
        </div>
      ))}
    </div>
  );
}

function DreamSkinUnsavedDialog({
  onSave,
  onDiscard,
  onCancel,
}: {
  onSave: () => void;
  onDiscard: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card">
        <div className="modal-head">
          <div>
            <h2>{t("主题有未保存修改")}</h2>
            <p className="modal-message">{t("保存修改后继续，或放弃修改。")}</p>
          </div>
          <button className="toast-close" onClick={onCancel} type="button">×</button>
        </div>
        <Toolbar>
          <Button onClick={onSave}>
            <Save className="h-4 w-4" />
            {t("保存并继续")}
          </Button>
          <Button onClick={onDiscard} variant="secondary">{t("放弃修改")}</Button>
          <Button onClick={onCancel} variant="outline">{t("取消")}</Button>
        </Toolbar>
      </div>
    </div>
  );
}

function UpdateDialog({ update, progress, onClose, onMuteToday, onUpdate, onOpenUrl }: {
  update: UpdateResult;
  progress: TaskProgress;
  onClose: () => void;
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
      <p className="update-dialog-hint">{t("也可随时在“关于”页面检查和安装更新。")}</p>
      {!update.assetName || !update.assetUrl ? <p className="update-dialog-hint">{t("此版本暂未提供适用于当前系统的安装包。")}</p> : null}
      <Toolbar className="update-dialog-actions">
        <Button autoFocus disabled={progress.active || !update.assetName || !update.assetUrl} onClick={onUpdate}>
          <Download className="h-4 w-4" />
          {progress.active ? t("正在下载安装包…") : t("立即更新")}
        </Button>
        <Button variant="secondary" onClick={onClose}>{t("关闭提示")}</Button>
        <Button variant="ghost" onClick={onMuteToday}>{t("今日不再提示")}</Button>
      </Toolbar>
    </dialog>
  );
}

/**
 * 右下角的提示卡：左侧状态色条 + 图标，按成功 / 警告 / 失败 / 信息分色。
 * 鼠标停在卡片上时暂停倒计时；失败提示停留更久，并可一键复制详情。
 */
function NoticeDialog({
  notice,
  onClose,
}: {
  notice: { title: string; message: string; status?: Status };
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

function ConfirmDialog({
  confirm,
  onConfirm,
  onCancel,
}: {
  confirm: { title: string; message: string; confirmText: string; cancelText: string };
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card confirm-modal">
        <div className="modal-head">
          <div>
            <h2>{confirm.title}</h2>
          </div>
          <button className="toast-close" onClick={onCancel} type="button">×</button>
        </div>
        <div className="confirm-modal-body">
          <p className="modal-message">{confirm.message}</p>
        </div>
        <Toolbar className="confirm-modal-actions">
          <Button onClick={onConfirm}>
            <Trash2 className="h-4 w-4" />
            {confirm.confirmText}
          </Button>
          <Button onClick={onCancel} variant="secondary">{confirm.cancelText}</Button>
        </Toolbar>
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
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card provider-import-modal">
        <div className="modal-head">
          <div>
            <h2>{t("导入 Codex++ 供应商")}</h2>
            <p>{t("检测到来自网页的供应商配置导入请求，确认后会写入本机 Codex++。")}</p>
          </div>
          <button className="toast-close" onClick={onDismiss} type="button">×</button>
        </div>
        <div className="metric-list">
          <Metric label={t("名称")} value={request.name || t("未命名供应商")} />
          <Metric label="Base URL" value={request.baseUrl || t("未填写")} />
          <Metric label={t("协议")} value={providerImportWireApiLabel(request.wireApi)} />
          <Metric label={t("模式")} value={providerImportRelayModeLabel(request.relayMode)} />
          <Metric label="API Key" value={maskSecret(request.apiKey)} />
        </div>
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
  installBlocked,
  installPending,
  onConfirm,
  onDismiss,
}: {
  versionId: string;
  installBlocked: boolean;
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
          <Button aria-busy={installPending} disabled={installBlocked} onClick={onConfirm}>
            {installPending
              ? <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
              : <Download className="h-4 w-4" />}
            {installPending ? t("安装中…") : t("下载并安装")}
          </Button>
          <Button disabled={installPending} onClick={onDismiss} variant="secondary">{t("取消")}</Button>
        </Toolbar>
        {installPending ? <DreamSkinInstallProgress /> : null}
        {installBlocked && !installPending ? (
          <div className="hint-line" role="status">{t("其他主题正在安装，请等待完成后再安装此主题。")}</div>
        ) : null}
      </div>
    </div>
  );
}

function TaskProgressBox({ progress, title, completedTitle = t("上次修复结果") }: { progress: TaskProgress; title: string; completedTitle?: string }) {
  if (!progress.active && progress.percent <= 0 && !progress.failed) return null;
  return (
    <div className="provider-sync-progress task-progress" data-active={progress.active}>
      <div className="provider-sync-progress-head">
        <strong>{progress.active ? title : completedTitle}</strong>
        <span>{progress.indeterminate || progress.failed ? null : `${formatProgressPercent(progress.percent)}%`}</span>
      </div>
      {!progress.failed ? <div
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={progress.indeterminate ? undefined : progress.percent}
        className="provider-sync-progress-bar"
        data-indeterminate={progress.indeterminate ? "true" : undefined}
        role="progressbar"
      >
        <div
          className="provider-sync-progress-fill"
          style={progress.indeterminate ? undefined : { width: `${progress.percent}%` }}
        />
      </div> : null}
      <small>{progress.message}</small>
    </div>
  );
}

function Panel({ children, fill = false, className = "", id }: { children: React.ReactNode; fill?: boolean; className?: string; id?: string }) {
  return (
    <Card className={`panel ${fill ? "fill" : ""} ${className}`} id={id}>
      {children}
    </Card>
  );
}

type ClaudeLocalizationStatus = {
  supported: boolean;
  sourceApp: string;
  copyApp: string;
  sourceVersion: string;
  copyVersion: string;
  prepared: boolean;
  needsRebuild: boolean;
  running: boolean;
  nodeAvailable: boolean;
  pythonAvailable: boolean;
  shellLocale: string | null;
  catalogEntries: number;
  voiceInput: boolean;
  localization: boolean;
  computerUse: boolean;
  /** 当前平台能否真正应用 Computer Use；Windows 为 false（#2673）。 */
  computerUseSupported: boolean;
};

/**
 * Claude 增强页跨挂载的记忆：Codex++ 运行期间只在第一次进入时自动检测一次，
 * 之后再进来直接显示上次结果，需要时手动点「检测 Claude」刷新。
 *
 * 检测已在后台线程跑、带 15 秒超时，未安装 Claude 时直接早退（#2539 的死机不会再现），
 * 所以首次进入自动检测是安全的；但每次切回来都重新探测进程和运行时没有必要。
 */
const claudeLocalizationMemo: {
  status: CommandResult<ClaudeLocalizationStatus> | null;
  autoDetected: boolean;
} = { status: null, autoDetected: false };

function ClaudeLocalizationScreen() {
  const [status, setStatus] = useState<CommandResult<ClaudeLocalizationStatus> | null>(claudeLocalizationMemo.status);
  // 首次进入会自动检测：初值就标成检测中，免得第一帧先闪出「点击重试」。
  const [busy, setBusy] = useState<string | null>(claudeLocalizationMemo.autoDetected ? null : "detect");
  const [error, setError] = useState<string | null>(null);
  /** 制作增强版时宿主推送的阶段进度（0-100）。 */
  const [progress, setProgress] = useState<{ percent: number; message: string } | null>(null);
  const mounted = useRef(true);
  const pending = useRef(false);
  /**
   * 页头右上角的操作槽：「启动增强版」放在和 Codex 页「启动 Codex」相同的位置。
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
    if (result.status === "ok") setStatus(result);
    else setError(result.message);
  };
  const refresh = async () => {
    if (pending.current) return;
    pending.current = true;
    setBusy("detect"); setError(null);
    try { accept(await invoke<CommandResult<ClaudeLocalizationStatus>>("claude_localization_status")); }
    catch (cause) { if (mounted.current) setError(stringifyError(cause)); }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  useEffect(() => {
    if (claudeLocalizationMemo.autoDetected) return;
    claudeLocalizationMemo.autoDetected = true;
    void refresh();
  }, []);
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
    if (pending.current) return;
    pending.current = true;
    setBusy(action); setError(null);
    if (action === "prepare") setProgress({ percent: 0, message: t("准备开始…") });
    try {
      const result = await invoke<CommandResult<ClaudeLocalizationStatus>>("claude_localization_action", { action });
      // 失败响应里的 data 是空对象：直接覆盖状态会让下面的渲染读到不存在的字段
      // （曾经就是这样整页白屏的），所以只有成功时才更新状态。
      accept(result);
    } catch (cause) { if (mounted.current) setError(stringifyError(cause)); }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
      if (action === "prepare") setTimeout(() => { if (mounted.current) setProgress(null); }, 1500);
    }
  };
  const toggleVoiceInput = async (enabled: boolean) => {
    if (pending.current) return;
    pending.current = true;
    setBusy("voiceInput"); setError(null);
    try {
      const result = await invoke<CommandResult<ClaudeLocalizationStatus>>(
        "claude_localization_set_voice_input",
        { enabled },
      );
      // 失败响应里的 data 是空对象：直接覆盖状态会让下面的渲染读到不存在的字段
      // （曾经就是这样整页白屏的），所以只有成功时才更新状态。
      accept(result);
    } catch (cause) { if (mounted.current) setError(stringifyError(cause)); }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  const toggleComputerUse = async (enabled: boolean) => {
    if (pending.current) return;
    pending.current = true;
    setBusy("computerUse"); setError(null);
    try {
      const result = await invoke<CommandResult<ClaudeLocalizationStatus>>(
        "claude_localization_set_computer_use",
        { enabled },
      );
      accept(result);
    } catch (cause) { if (mounted.current) setError(stringifyError(cause)); }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  const toggleLocalization = async (enabled: boolean) => {
    if (pending.current) return;
    pending.current = true;
    setBusy("localization"); setError(null);
    try {
      const result = await invoke<CommandResult<ClaudeLocalizationStatus>>(
        "claude_localization_set_localization",
        { enabled },
      );
      // 失败响应里的 data 是空对象：直接覆盖状态会让下面的渲染读到不存在的字段
      // （曾经就是这样整页白屏的），所以只有成功时才更新状态。
      accept(result);
    } catch (cause) { if (mounted.current) setError(stringifyError(cause)); }
    finally {
      pending.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  const s = status;
  const canLaunch = Boolean(s?.prepared && !s.needsRebuild && s.nodeAvailable);
  const statusTone = !s ? "pending" : s.prepared ? (s.needsRebuild ? "warn" : "ok") : "idle";
  const statusText = busy === "detect" ? t("检测中…") : !s ? t("未检测") : !s.supported ? t("未检测到 Claude") : s.prepared ? (s.needsRebuild ? t("需要更新") : t("已准备")) : t("未准备");
  return (
    <div className="claude-localization-page">
      {topbarSlot
        ? createPortal(
            <Button disabled={busy !== null || !canLaunch} onClick={() => void act("launch")} title={t("启动增强版")}>
              <Rocket className="h-4 w-4" />
              {busy === "launch" ? t("启动中…") : t("启动增强版")}
            </Button>,
            topbarSlot,
          )
        : null}
      {/* 顶部通栏：所有增强共同的前置条件。状态和操作放在第一屏，不用滚动就能点到。 */}
      <Panel className="claude-l10n-hero">
        <CardContent className="claude-l10n-hero-body">
          <div className="claude-l10n-hero-head">
            <div className="claude-l10n-hero-title">
              <span className="claude-l10n-hero-icon"><PackageOpen className="h-5 w-5" /></span>
              <div>
                <strong>{t("受管增强版")}</strong>
                <span className="claude-l10n-hero-subtitle">{t("所有增强的前置条件：官方 Claude.app 与 app.asar 保持原样，功能通过副本和启动时注入生效。")}</span>
              </div>
            </div>
            <span className={`claude-l10n-status claude-l10n-status-${statusTone}`}>
              <i aria-hidden="true" />
              {statusText}
            </span>
          </div>
          {/* 三步进度：原来右栏的「上手步骤」收成一排，完成的步骤打勾。 */}
          <ol className="claude-l10n-progress-steps">
            <li data-done={s?.prepared ? "true" : "false"}>
              <span aria-hidden="true">{s?.prepared ? "✓" : "1"}</span>
              <div><strong>{t("制作增强版")}</strong><small>{t("官方 Claude.app 与 app.asar 保持原样")}</small></div>
            </li>
            <li data-done={s?.running ? "true" : "false"}>
              <span aria-hidden="true">{s?.running ? "✓" : "2"}</span>
              <div><strong>{t("启动增强版")}</strong><small>{t("右上角启动，增强在启动时注入")}</small></div>
            </li>
            <li data-done="false">
              <span aria-hidden="true">3</span>
              <div><strong>{t("开启需要的增强")}</strong><small>{t("下方各项独立开关，重启副本后应用")}</small></div>
            </li>
          </ol>
          {/* 版本与词表信息只作参考，收成一行小字。 */}
          <dl className="claude-l10n-meta">
            <div><dt>{t("官方版本")}</dt><dd title={s?.sourceVersion || undefined}>{!s ? t("未检测") : s.sourceVersion || t("未检测到")}</dd></div>
            <div><dt>{t("副本版本")}</dt><dd title={s?.copyVersion || undefined}>{s?.copyVersion || t("未准备")}</dd></div>
            <div><dt>{t("外壳语言")}</dt><dd>{s?.shellLocale || t("未设置")}</dd></div>
            <div><dt>{t("官方词条")}</dt><dd>{s?.catalogEntries != null ? `${s.catalogEntries.toLocaleString()} ${t("条")}` : t("未检测")}</dd></div>
          </dl>
          {s?.supported && !s.prepared ? (
            <p className="claude-l10n-hint">
              <Info className="h-4 w-4" />
              {t("首次制作会创建独立副本并执行完整性校验；官方应用、登录态和 app.asar 不会被改写。macOS 可能需要钥匙串授权，Windows 会使用独立副本启动。")}
            </p>
          ) : null}
          {!s ? (
            <p className="claude-l10n-hint">
              {busy === "detect"
                ? t("正在读取 Claude 安装状态，只读检测，不会修改应用。")
                : t("没有读到安装状态，点击“检测 Claude”重试；检测不会修改应用。")}
            </p>
          ) : !s.supported ? (
            <p className="claude-l10n-hint">{t("未检测到 Claude，请先安装官方应用。你可以继续使用 Codex++ 的其他功能。")}</p>
          ) : null}
          {error ? <div className="claude-l10n-error" role="alert">{error}</div> : null}
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
          {/* 制作在左，检测类的次要操作收在右边，窄时整组换行而不是剩一个按钮掉下去。启动在页头右上角。 */}
          <div className="claude-l10n-actions">
            <div className="claude-l10n-actions-primary">
            <Button disabled={busy !== null || !s?.supported} onClick={() => void act("prepare")}>
              <PackageOpen className="h-4 w-4" />
              {busy === "prepare"
                ? t("制作中…")
                : s?.needsRebuild
                  ? t("更新副本")
                  : s?.prepared
                    ? t("重新制作增强版")
                    : t("制作增强版")}
            </Button>
            </div>
            <div className="claude-l10n-actions-secondary">
              <Button disabled={busy !== null} onClick={() => void refresh()} variant="secondary">
                <RefreshCw className={`h-4 w-4 ${busy === "detect" ? "animate-spin" : ""}`} />
                {busy === "detect" ? t("检测中…") : t("检测 Claude")}
              </Button>
              <Button disabled={busy !== null || !s?.prepared} onClick={() => void act("check")} variant="secondary">
                <ShieldCheck className="h-4 w-4" />{t("检查增强版")}
              </Button>
            </div>
          </div>
        </CardContent>
      </Panel>
      {/* 增强项铺满整行、三列并排：每一项都跑在上方的受管副本上，汉化只是其中之一。 */}
      <div className="claude-l10n-column claude-l10n-features">
        <Panel>
          <CardHead
            title={t("增强功能")}
            detail={t("各自独立开关，互不影响；副本只是它们共同的前置条件，改动在重启副本后应用。")}
          />
          <CardContent className="claude-l10n-feature-grid">
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
                <small>{s?.computerUseSupported === false
                  ? t("暂仅支持 macOS：Windows 的副本构建流程不会改写 app.asar，打开这个开关不会有任何效果。")
                  : t("解锁桌面控制：订阅与灰度校验已解除，API Key（3p）模式同样可用；会改写副本的 app.asar 并关闭 Electron 的 asar 完整性校验，原件不动。首次启动增强版会提示授权辅助功能与屏幕录制。")}</small>
              </span>
              <ToggleVisual />
            </label>
            {s?.computerUseSupported !== false ? (
              s?.computerUse && claudeDesktopCuConnected ? (
                <p className="claude-l10n-hint is-warning" role="alert">
                  {t("Claude Desktop 同时接入了「Computer Use」页的 codex-plus-cu：同一个对话里会有两套电脑控制，Claude 可能混着调用、互相抢前台，同一个应用也要确认两次。建议二选一：保留这里的开关，就到「Computer Use」页撤销 Claude Desktop 的接入；或者关掉这里的开关。")}
                </p>
              ) : (
                <p className="claude-l10n-hint">
                  {t("这里的电脑控制和「Computer Use」页给 Claude Desktop 接入的 codex-plus-cu 是两套独立实现，同一个 Claude Desktop 里二选一即可，不要同时开。")}
                </p>
              )
            ) : null}
            {s?.supported && !s.prepared ? (
              <p className="claude-l10n-hint">
                {t("增强版还没制作：开关会先存下来，等你在上方制作好增强版并启动后生效。")}
              </p>
            ) : null}
          </CardContent>
        </Panel>
      </div>
    </div>
  );
}

/// 设置行：左边标题 + 一句短说明，右边控件；长说明放进悬停提示（`detail`）。
/// `stacked` 时控件放到说明下方，给状态码编辑、多行输入这类宽控件用。
function SettingRow({ title, description, detail, stacked = false, children }: {
  title: ReactNode;
  description?: ReactNode;
  detail?: string;
  stacked?: boolean;
  children?: ReactNode;
}) {
  return (
    <div className={`setting-row${stacked ? " setting-row-stacked" : ""}`} title={detail}>
      <div className="setting-row-copy">
        <strong>{title}</strong>
        {description ? <span>{description}</span> : null}
      </div>
      {children ? <div className="setting-row-control">{children}</div> : null}
    </div>
  );
}

/// 标题栏右侧带开关的卡片头：开关关闭时，卡片里的设置整块收起，只留一行提示。
function SwitchPanelHead({ title, detail, checked, onChange, label }: {
  title: string;
  detail: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
}) {
  return (
    <CardHeader className="panel-head switch-panel-head">
      <div className="switch-panel-head-copy">
        <CardTitle>{title}</CardTitle>
        <CardDescription>{detail}</CardDescription>
      </div>
      <label className="switch-row compact relay-master-switch-compact">
        <input aria-label={label} checked={checked} onChange={(event) => onChange(event.currentTarget.checked)} type="checkbox" />
        <span>{checked ? t("已开启") : t("已关闭")}</span>
        <ToggleVisual />
      </label>
    </CardHeader>
  );
}

/// 带状态图标的小卡片，与概览页健康检查同一套样式。
function HealthTile({ item }: { item: ReturnType<typeof healthItems>[number] }) {
  return (
    <div className={`health-item ${item.ok ? "ok" : "needs-fix"}`}>
      <span className={`health-icon ${statusClass(item.status)}`}><item.icon className="h-4 w-4" /></span>
      <div>
        <strong>{item.title}</strong>
        <span title={item.detail}>{item.detail}</span>
      </div>
      <Badge status={item.status} />
    </div>
  );
}

function CardHead({ title, detail }: { title: string; detail: string }) {
  return (
    <CardHeader className="panel-head">
      <CardTitle>{title}</CardTitle>
      <CardDescription>{detail}</CardDescription>
    </CardHeader>
  );
}

function Toolbar({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return <div className={`toolbar ${className}`.trim()}>{children}</div>;
}

type GrokProvidersResult = CommandResult<{
  profiles: RelayProfile[];
  activeRelayId: string;
  live: {
    grokHome: string;
    configPath: string;
    configExists: boolean;
    cliPath: string | null;
    cliInstalled: boolean;
    revision: string;
    defaultModel: string;
    modelsBaseUrl: string;
    models: Array<{ alias: string; model: string; baseUrl: string; contextWindow: number | null; apiKeyConfigured: boolean }>;
  };
  liveProfile: RelayProfile;
}>;

/**
 * Grok 分区的供应商管理。
 *
 * 映射约定是「一个供应商 = 一个 base_url」：应用到 Grok 时，这个供应商的模型
 * 列表会整体替换 `~/.grok/config.toml` 里所有受管的 `[model.*]` 表，未管理字段
 * （`[ui]`、`[models].web_search` 等）保留。所以「应用到 Grok」是需要确认的
 * 破坏性操作，这里显式二次确认。
 */
const SIDEBAR_COLLAPSED_KEY = "codex-plus-sidebar-collapsed";

function loadCollapsedGroups(): string[] {
  try {
    const value = JSON.parse(window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) ?? "[]");
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

/**
 * 唯一的侧栏：任意 Agent → Codex++ → 任意供应商。
 *
 * - 顶部是网关状态卡和 ⌘K 快速跳转；
 * - 「网关」：总览、供应商池、Agent 接入、路由、用量、网关设置；
 * - 「全局管理」：请求日志、会话、MCP、Skills，同时管理所有 Agent；
 * - 「Agent」：每个 Agent 一行（状态点表示接入方式）；它自己的页面（供应商配置、增强、皮肤……）在内容区顶部的页签里；
 * - 「工具」：服务所有 Agent、但与供应商无关的页面；
 * - 底部：版本与更新、整体设置、关于、语言和主题。
 */
/**
 * 当前 Agent 的全部页面：内容区顶部一排页签，分组（概览 / 供应商 / 增强 / 设置）之间用细线隔开。
 * 网关、全局管理、工具这些通用页面不显示；只有一个页面的 Agent 也不显示。
 */
function AgentPageTabs({ route, activeTool, onNavigate }: { route: Route; activeTool: ToolId; onNavigate: (route: Route) => void }) {
  if (globalNavigationRoutes.includes(route)) return null;
  const groups = agentTabGroups(navigationSections, routeTool, activeTool);
  if (groups.reduce((count, entry) => count + entry.routes.length, 0) < 2) return null;
  return (
    <nav aria-label={t("Agent 页面")} className="tp-tabs agent-page-tabs" role="tablist">
      {groups.map(({ group, routes: groupRoutes }, index) => (
        <Fragment key={group.id}>
          {index > 0 ? <span aria-hidden="true" className="agent-page-tabs-sep" /> : null}
          {groupRoutes.map((routeId) => {
            const item = routes.find((candidate) => candidate.id === routeId);
            if (!item) return null;
            const Icon = item.icon;
            return (
              <button aria-selected={route === routeId} data-active={route === routeId} key={routeId} onClick={() => onNavigate(routeId)} role="tab" type="button">
                <Icon aria-hidden="true" size={14} />
                {item.label}
              </button>
            );
          })}
        </Fragment>
      ))}
    </nav>
  );
}

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
  onOpenUpdate,
  codexRun,
}: {
  /** Codex 行的播放 / 停止：只有桌面应用的 Codex 有，命令行 Agent 没有常驻进程可启停。 */
  codexRun: { running: boolean; busy: boolean; onStart: () => void; onStop: () => void };
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
  onOpenUpdate: () => void;
}) {
  const gateway = useGatewayPayload(10_000);
  const [collapsed, setCollapsed] = useState<string[]>(loadCollapsedGroups);
  const toggleGroup = (key: string) => {
    setCollapsed((current) => {
      const next = current.includes(key) ? current.filter((item) => item !== key) : [...current, key];
      try {
        window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, JSON.stringify(next));
      } catch {
        // 存不了就只在本次会话里生效。
      }
      return next;
    });
  };
  const onGlobalPage = globalNavigationRoutes.includes(route);
  const languageLabel = getLanguage() === "en" ? t("切换到中文") : t("切换到英文");
  const themeLabel = theme === "dark" ? t("切换到浅色") : t("切换到深色");
  const connectedAgents = new Set((gateway?.agents ?? []).filter((agent) => agent.enabled).map((agent) => agent.agent as string));
  const gatewayState = !gateway
    ? t("读取中…")
    : gateway.helperRunning
      ? tf("网关运行中 · 端口 {0}", [gateway.port])
      : connectedAgents.size
        ? t("网关未运行")
        : t("还没有 Agent 接入");

  const renderItem = (routeId: Route) => {
    const item = routes.find((candidate) => candidate.id === routeId);
    if (!item) return null;
    const Icon = item.icon;
    const selected = route === item.id;
    return (
      <button
        aria-current={selected ? "page" : undefined}
        className={`side-item ${selected ? "active" : ""}`}
        data-tour={`nav-${item.id}`}
        key={item.id}
        onClick={() => onNavigate(item.id)}
        title={item.label}
        type="button"
      >
        <Icon aria-hidden="true" className="side-item-icon" />
        <span className="side-item-label">{item.label}</span>
        {item.badge ? <span className="side-item-badge">{t(item.badge)}</span> : null}
      </button>
    );
  };

  const groupLabel = (key: string, label: string, aside?: ReactNode) => (
    <div className="side-group-label">
      <button aria-expanded={!collapsed.includes(key)} onClick={() => toggleGroup(key)} type="button">
        <ChevronDown aria-hidden="true" className="side-group-chevron" />
        {label}
      </button>
      {aside}
    </div>
  );

  return (
    <aside className="side">
      <button className={`side-hub ${route === "hub" ? "active" : ""}`} data-tour="hub" onClick={() => onNavigate("hub")} title={t("网关总览")} type="button">
        <img alt="" draggable={false} src={APP_LOGO_URL} />
        <span className="side-hub-text">
          <strong>Codex++</strong>
          <small><i className="side-dot" data-tone={gateway?.helperRunning ? "ok" : "off"} />{gatewayState}</small>
        </span>
      </button>
      <button className="side-search" data-tour="palette" onClick={onOpenPalette} type="button">
        <Search aria-hidden="true" className="side-item-icon" />
        <span>{t("快速跳转")}</span>
        <kbd>⌘K</kbd>
      </button>

      <nav aria-label={t("主导航")} className="side-nav">
        <section className={`side-group ${collapsed.includes("gateway") ? "collapsed" : ""}`}>
          {groupLabel("gateway", t("网关"))}
          <div className="side-group-items">{gatewayNavigationRoutes.map((routeId) => renderItem(routeId))}</div>
        </section>

        <section className={`side-group ${collapsed.includes("management") ? "collapsed" : ""}`} data-tour="management">
          {groupLabel("management", t("全局管理"))}
          <div className="side-group-items">{managementNavigationRoutes.map((routeId) => renderItem(routeId))}</div>
        </section>

        <section className={`side-group ${collapsed.includes("agents") ? "collapsed" : ""}`} data-tour="agents">
          {groupLabel(
            "agents",
            "Agent",
            <button aria-label={t("接入 Agent")} className="side-group-add" onClick={() => onNavigate("gateway")} title={t("接入 Agent")} type="button">
              <Plus aria-hidden="true" />
            </button>,
          )}
          <div className="side-group-items">
            {/* 每个 Agent 一行，始终都在；点一下切到它，回到上次看的页面（没有就是落脚页）。它的页面都在内容区顶部的页签里。 */}
            {tools.map((tool) => {
              const current = tool.id === activeTool;
              const title = tool.switchable
                ? tf("{0}｜{1}｜{2} 个供应商", [tool.name, tool.homeDir || t("未配置目录"), tool.relayCount])
                : tf("{0}｜{1}｜供应商配置尚未接入", [tool.name, tool.homeDir || t("未配置目录")]);
              // 状态点表示接入方式，不表示运行状态（运行状态在顶栏）：Codex 由 Codex++ 直接管理供应商配置，
              // 其余 Agent 看是否已经接入网关。
              // 接入了网关一律绿色；Codex 没接入网关时是直连（紫色），由 Codex++ 直接管理供应商配置。
              const access = connectedAgents.has(tool.id)
                ? { tone: "ok", label: t("已经通过 Codex++ 网关接入") }
                : tool.id === "codex"
                  ? { tone: "direct", label: t("Codex 的供应商配置由 Codex++ 直接管理") }
                  : { tone: "off", label: t("未接入网关") };
              return (
                <div className="side-agent-wrap" key={tool.id}>
                  <button
                    aria-current={current && !onGlobalPage ? "true" : undefined}
                    aria-label={tool.switchable ? `${tool.name} · ${access.label}` : `${tool.name} · ${t("待接入")}`}
                    className={`side-item side-agent ${current && !onGlobalPage ? "current active" : ""}`}
                    disabled={!tool.switchable}
                    onClick={() => onOpenAgent(tool.id)}
                    title={`${title}｜${access.label}`}
                    type="button"
                  >
                    <AgentGlyph agent={tool.id} className="side-item-icon" />
                    <span className="side-item-label">{tool.name}</span>
                    <i aria-hidden="true" className="side-dot side-agent-dot" data-tone={access.tone} />
                  </button>
                  {tool.id === "codex" && tool.switchable ? (
                    <button
                      aria-label={codexRun.running ? t("停止 Codex") : t("启动 Codex")}
                      className="side-agent-run"
                      data-running={codexRun.running}
                      disabled={codexRun.busy}
                      onClick={codexRun.running ? codexRun.onStop : codexRun.onStart}
                      title={codexRun.busy ? t("处理中…") : codexRun.running ? t("停止 Codex") : t("启动 Codex")}
                      type="button"
                    >
                      {codexRun.busy ? <Loader2 aria-hidden="true" className="animate-spin" /> : codexRun.running ? <Square aria-hidden="true" /> : <Play aria-hidden="true" />}
                    </button>
                  ) : null}
                </div>
              );
            })}
          </div>
        </section>

        <section className={`side-group ${collapsed.includes("tools") ? "collapsed" : ""}`}>
          {groupLabel("tools", t("工具"))}
          <div className="side-group-items">{toolNavigationRoutes.map((routeId) => renderItem(routeId))}</div>
        </section>
      </nav>

      <footer className="side-foot">
        <span className="side-version">{version ? `v${version}` : "Codex++"}</span>
        {pendingUpdateVersion ? (
          <button
            aria-label={tf("发现新版本 {0}，点击查看更新", [pendingUpdateVersion])}
            className="side-update"
            onClick={onOpenUpdate}
            title={tf("发现新版本 {0}，点击查看更新", [pendingUpdateVersion])}
            type="button"
          >
            <ArrowUp aria-hidden="true" />
            {t("更新")}
          </button>
        ) : null}
        <span className="side-foot-spacer" />
        {(["settings"] as Route[]).map((routeId) => {
          const item = routes.find((candidate) => candidate.id === routeId);
          if (!item) return null;
          const Icon = item.icon;
          return (
            <button
              aria-current={route === routeId ? "page" : undefined}
              aria-label={item.label}
              className={`side-icon-button ${route === routeId ? "active" : ""}`}
              data-tour={routeId}
              key={routeId}
              onClick={() => onNavigate(routeId)}
              title={item.label}
              type="button"
            >
              <Icon aria-hidden="true" />
            </button>
          );
        })}
        <button aria-label={languageLabel} className="side-icon-button" onClick={() => toggleLanguage()} title={languageLabel} type="button">
          <Languages aria-hidden="true" />
        </button>
        <button aria-label={themeLabel} className="side-icon-button" onClick={onToggleTheme} title={themeLabel} type="button">
          {theme === "dark" ? <Sun aria-hidden="true" /> : <Moon aria-hidden="true" />}
        </button>
      </footer>
    </aside>
  );
}

function Field({ label, children, className = "" }: { label: string; children: React.ReactNode; className?: string }) {
  return (
    <Label className={`field ${className}`}>
      <span>{label}</span>
      {children}
    </Label>
  );
}

function StatusRow({ title, status = "unknown", path }: { title: string; status?: string; path?: string | null }) {
  return (
    <div className="status-row">
      <span>{title}</span>
      <Badge status={status} />
      <code>{path || t("未记录路径")}</code>
    </div>
  );
}

function Badge({ status }: { status: string }) {
  return <UiBadge className={statusClass(status)} variant="secondary">{statusLabel(status)}</UiBadge>;
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function ScriptRow({ script, actions }: { script: NonNullable<UserScriptInventory["scripts"]>[number]; actions: Actions }) {
  const source = script.market_id ? tf("市场 · {0}", [script.version || t("未知版本")]) : script.source === "builtin" ? t("内置") : t("用户");
  const canDelete = script.source === "user";
  return (
    <div className="table-row">
      <span>{script.name}</span>
      <span>{source}</span>
      <span>{script.enabled ? t("启用") : t("关闭")}</span>
      <span>{script.status}</span>
      <div className="script-row-actions">
        <Button onClick={() => void actions.setUserScriptEnabled(script.key, !script.enabled)} size="sm" variant="secondary">
          {script.enabled ? <PowerOff className="h-4 w-4" /> : <Power className="h-4 w-4" />}
          {script.enabled ? t("禁用") : t("启用")}
        </Button>
        {canDelete ? (
          <Button onClick={() => void actions.deleteUserScript(script.key)} size="sm" variant="outline">
            <Trash2 className="h-4 w-4" />
            {t("删除")}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function AdGrid({ ads, empty, actions }: { ads: AdItem[]; empty: string; actions: Actions }) {
  if (!ads.length) return <div className="rec-empty">{empty}</div>;
  return (
    <div className="rec-grid">
      {ads.map((ad) => (
        <button className="tp-card rec-card" key={ad.id || `${ad.type}-${ad.title}`} onClick={() => void actions.openExternalUrl(ad.url)} title={ad.url} type="button">
          {ad.image ? <img alt="" className="rec-image" loading="lazy" src={ad.image} /> : null}
          <div className="rec-body">
            <div className="rec-title">
              <strong>{formatAdTitle(ad.title)}</strong>
              <small>{adHost(ad.url)}</small>
              <ExternalLink aria-hidden="true" />
            </div>
            <p className="rec-desc" title={ad.description}>{ad.description}</p>
            {ad.highlights?.length ? (
              <div className="rec-tags">
                {ad.highlights.map((item) => (
                  <span key={item}>{item}</span>
                ))}
              </div>
            ) : null}
          </div>
        </button>
      ))}
    </div>
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

function routeTitle(route: Route) {
  return routes.find((item) => item.id === route)?.label ?? t("概览");
}

function routeSubtitle(route: Route) {
  const subtitles: Record<Route, string> = {
    overview: t("检查问题、启动与快速修复"),
    relay: t("管理 API 供应商、协议、Key 与配置文件"),
    multiLaunch: t("为不同供应商启动相互隔离的 Codex 实例"),
    claude: t("保留官方 Claude.app 原样，通过增强版为 Claude 应用增强功能"),
    relayEnvironment: t("旧版直连模式专用：目录外模型映射与供应商独立代理"),
    requestLog: t("所有 Agent 经 Codex++ 的请求：状态、模型、重试、token 用量与缓存命中"),
    sessions: t("Codex、Claude Code、Grok 的本地会话放在一起：在终端继续，Codex 会话还能删除、归档与修复"),
    context: t("同时管理 Codex、Claude Code、Grok 的 MCP 服务器：每列一个 Agent，打开即写入它的配置"),
    skills: t("从 GitHub 仓库安装 Skill，按 Agent 启用到 Codex、Claude Code 或 Grok"),
    weixin: t("通过个人微信连接本机 Codex 会话"),
    enhance: t("管理常用功能、语音输入与下一步建议。"),
    dreamSkin: t("Codex-Dream-Skin 风格主题和换图"),
    userScripts: t("内置和用户自定义脚本清单"),
    pluginMarket: t("检索插件，按需下载并安装到 Codex"),
    recommendations: t("来自 Codex++ 广告源的推荐，点卡片在浏览器里打开"),
    agentCache: t("扫描 Codex、Claude 与 Codex++ 的已知缓存目录，由你选择清理项目。"),
    agentCollaboration: t("连接 Codex、DeepSeek Harness 与 Claude Code，派发任务并查看结果。"),
    computerUse: t("复用 Codex 的 Computer Use，通过 MCP 接给 Claude Code、Claude Desktop 与 Codex。"),
    agentPet: t("在 Agent 窗口里放一个可拖动的小角色，点一下压扁回弹，带按压音效"),
    hub: t("任意 Agent 经 Codex++ 接入任意供应商：谁在用、走哪家、是否健康，一屏看全"),
    gateway: t("选择哪些 Agent 经 Codex++ 网关接入，以及它们默认用哪个模型"),
    gatewayProviders: t("所有接入的 Agent 共用的供应商池，支持多 Key 轮换"),
    gatewayRouting: t("多家都有的模型怎么分、你建的组怎么走：路由方式、会话固定、出错换谁"),
    gatewayUsage: t("按 Agent、供应商与模型统计调用、token、缓存与费用"),
    gatewaySettings: t("监听地址、Key 轮换方式与退出时的恢复策略"),
    maintenance: t("入口检查与修复、开机启动与应用路径"),
    about: t("版本信息、项目链接、GitHub Release 更新、日志与诊断"),
    settings: t("主题、配置文件、版本更新与日志诊断，不随 Agent 变化"),
    agentSettings: t("当前 Agent 的专属设置"),
  };
  return subtitles[route];
}

const contextKindOptions: Array<{ kind: ContextKind; label: string; tableName: string }> = [
  { kind: "mcp", label: "MCP", tableName: "mcp_servers" },
  { kind: "skill", label: "Skills", tableName: "skills" },
  { kind: "plugin", get label() { return t("插件"); }, tableName: "plugins" },
];

function contextKindLabel(kind: ContextKind) {
  return contextKindOptions.find((option) => option.kind === kind)?.label ?? t("扩展项");
}

function contextEntriesFromSettings(settings: BackendSettings): CodexContextEntries {
  const commonConfig = normalizeDuplicateTomlTables(settings.relayContextConfigContents || "");
  return {
    mcpServers: parseContextEntries(commonConfig, "mcp", "mcp_servers"),
    skills: parseContextEntries(commonConfig, "skill", "skills"),
    plugins: parseContextEntries(commonConfig, "plugin", "plugins"),
  };
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

function contextEntriesForProfile(settings: BackendSettings, profile: RelayProfile): CodexContextEntries {
  return filterContextEntriesBySelection(contextEntriesFromSettings(settings), profile.contextSelection);
}

function contextEntriesFromConfig(configContents: string): CodexContextEntries {
  return {
    mcpServers: parseContextEntries(configContents, "mcp", "mcp_servers"),
    skills: parseContextEntries(configContents, "skill", "skills"),
    plugins: parseContextEntries(configContents, "plugin", "plugins"),
  };
}

function mergeContextEntries(primary: CodexContextEntries, secondary: CodexContextEntries): CodexContextEntries {
  return {
    mcpServers: mergeContextEntryList(primary.mcpServers, secondary.mcpServers),
    skills: mergeContextEntryList(primary.skills, secondary.skills),
    plugins: mergeContextEntryList(primary.plugins, secondary.plugins),
  };
}

function mergeContextEntryList(primary: CodexContextEntry[], secondary: CodexContextEntry[]): CodexContextEntry[] {
  return dedupeContextEntryList([...primary, ...secondary]);
}

function dedupeContextEntryList(entries: CodexContextEntry[]): CodexContextEntry[] {
  const byId = new Map<string, CodexContextEntry>();
  for (const entry of entries) {
    byId.set(entry.id, entry);
  }
  return Array.from(byId.values());
}

function parseContextEntries(commonConfig: string, kind: ContextKind, tableName: string): CodexContextEntry[] {
  const anyHeaderPattern = /^\s*\[[^\]]+\]\s*$/;
  const entries = new Map<string, CodexContextEntry>();
  let currentId: string | null = null;
  let body: string[] = [];

  const flush = () => {
    if (!currentId) return;
    const tomlBody = ensureTrailingNewline(body.join("\n").trimEnd());
    entries.set(currentId, {
      id: currentId,
      kind,
      title: currentId,
      summary: contextEntrySummary(tomlBody),
      tomlBody,
      enabled: contextEntryEnabled(tomlBody),
    });
  };

  for (const line of commonConfig.split(/\r?\n/)) {
    const path = tomlTablePathFromLine(line);
    if (path?.[0] === tableName && path.length >= 2) {
      const id = path[1];
      if (currentId === id && path.length > 2) {
        body.push(`[${path.slice(2).map(tomlKey).join(".")}]`);
        continue;
      }
      flush();
      currentId = id;
      body = [];
      continue;
    }
    if (currentId && anyHeaderPattern.test(line)) {
      flush();
      currentId = null;
      body = [];
      continue;
    }
    if (currentId) body.push(line);
  }
  flush();

  return Array.from(entries.values());
}

function tomlTablePathFromLine(line: string): string[] | null {
  const match = /^\s*\[([^\]]+)\]\s*$/.exec(line);
  if (!match) return null;
  return parseTomlDottedPath(match[1].trim());
}

function parseTomlDottedPath(path: string): string[] | null {
  const parts: string[] = [];
  let current = "";
  let quote: '"' | "'" | null = null;
  let escaping = false;

  for (const char of path) {
    if (quote) {
      if (quote === '"' && escaping) {
        current += char;
        escaping = false;
      } else if (quote === '"' && char === "\\") {
        escaping = true;
      } else if (char === quote) {
        quote = null;
      } else {
        current += char;
      }
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (char === ".") {
      if (!current.trim()) return null;
      parts.push(current.trim());
      current = "";
      continue;
    }
    current += char;
  }

  if (quote || escaping || !current.trim()) return null;
  parts.push(current.trim());
  return parts;
}

function contextEntrySummary(tomlBody: string) {
  return tomlBody
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line && !line.startsWith("#") && !/^enabled\s*=/.test(line))
    ?.slice(0, 96) ?? "";
}

function contextEntryEnabled(tomlBody: string) {
  return !tomlBody.split(/\r?\n/).some((line) => /^\s*enabled\s*=\s*false\s*(#.*)?$/i.test(line));
}

function setContextEntryEnabled(tomlBody: string, enabled: boolean) {
  const lines = tomlBody.trimEnd().split(/\r?\n/);
  const nextValue = `enabled = ${enabled ? "true" : "false"}`;
  let replaced = false;
  const next = lines.map((line) => {
    if (/^\s*enabled\s*=/.test(line)) {
      replaced = true;
      return nextValue;
    }
    return line;
  });
  if (!replaced) next.unshift(nextValue);
  return ensureTrailingNewline(next.join("\n").trimEnd());
}

function ensureTrailingNewline(value: string) {
  return value.trim() ? `${value}\n` : "";
}

function unquoteTomlKey(key: string) {
  if (key.length >= 2 && ((key.startsWith('"') && key.endsWith('"')) || (key.startsWith("'") && key.endsWith("'")))) {
    return key.slice(1, -1);
  }
  return key;
}

function contextEntriesByKind(entries: CodexContextEntries, kind: ContextKind): CodexContextEntry[] {
  if (kind === "mcp") return dedupeContextEntryList(entries.mcpServers);
  if (kind === "skill") return dedupeContextEntryList(entries.skills);
  return dedupeContextEntryList(entries.plugins);
}

function filterContextEntriesBySelection(entries: CodexContextEntries, selection: RelayContextSelection): CodexContextEntries {
  const selected = {
    mcp: new Set(selection.mcpServers.map((id) => id.trim()).filter(Boolean)),
    skill: new Set(selection.skills.map((id) => id.trim()).filter(Boolean)),
    plugin: new Set(selection.plugins.map((id) => id.trim()).filter(Boolean)),
  };
  return {
    mcpServers: entries.mcpServers.filter((entry) => selected.mcp.has(entry.id)),
    skills: entries.skills.filter((entry) => selected.skill.has(entry.id)),
    plugins: entries.plugins.filter((entry) => selected.plugin.has(entry.id)),
  };
}

function effectiveRelayConfigPreview(profile: RelayProfile, settings: BackendSettings, contextProfile = profile): string {
  const entries = contextEntriesForProfile(settings, contextProfile);
  const isolatedConfig = stripContextEntriesFromConfig(profile.configContents, entries);
  const configWithLimits = applyContextLimitPreview(isolatedConfig, profile);
  const profileAndCommon = mergeFeaturesTableForPreview(configWithLimits, settings.relayCommonConfigContents || "");
  return joinTomlSectionsRootFirst([profileAndCommon, selectedContextConfigToml(entries)]);
}

function mergeFeaturesTableForPreview(profileConfig: string, commonConfig: string): string {
  const profile = splitFeaturesTable(profileConfig);
  const common = splitFeaturesTable(commonConfig);
  if (!profile.body && !common.body) return joinTomlSectionsRootFirst([profileConfig, commonConfig]);

  const profileKeys = new Set(tomlAssignmentKeys(profile.body));
  const commonBody = common.body
    .split(/\r?\n/)
    .filter((line) => {
      const key = tomlAssignmentKey(line);
      return !key || !profileKeys.has(key);
    })
    .join("\n");
  const mergedFeatures = ["[features]", commonBody, profile.body]
    .filter((part) => part.trim())
    .join("\n");
  return joinTomlSectionsRootFirst([
    profile.without,
    common.without,
    mergedFeatures,
  ]);
}

function splitFeaturesTable(contents: string): { without: string; body: string } {
  const lines = contents.trim().split(/\r?\n/);
  const start = lines.findIndex((line) => line.trim() === "[features]");
  if (start < 0) return { without: contents, body: "" };
  let end = lines.length;
  for (let index = start + 1; index < lines.length; index += 1) {
    if (/^\s*\[[^\]]+\]\s*$/.test(lines[index])) {
      end = index;
      break;
    }
  }
  return {
    without: [...lines.slice(0, start), ...lines.slice(end)].join("\n"),
    body: lines.slice(start + 1, end).join("\n"),
  };
}

function tomlAssignmentKey(line: string): string | undefined {
  return /^\s*([A-Za-z0-9_-]+)\s*=/.exec(line)?.[1];
}

function tomlAssignmentKeys(contents: string): string[] {
  return contents.split(/\r?\n/).map(tomlAssignmentKey).filter((key): key is string => Boolean(key));
}

function selectedContextConfigToml(entries: CodexContextEntries): string {
  const sections: string[] = [];
  for (const option of contextKindOptions) {
    for (const entry of dedupeContextEntryList(contextEntriesByKind(entries, option.kind))) {
      if (!entry.enabled) continue;
      sections.push(contextEntryToTomlSection(option.tableName, entry));
    }
  }
  return ensureTrailingNewline(sections.join("\n\n"));
}

function allContextConfigToml(entries: CodexContextEntries): string {
  const sections: string[] = [];
  for (const option of contextKindOptions) {
    for (const entry of dedupeContextEntryList(contextEntriesByKind(entries, option.kind))) {
      sections.push(contextEntryToTomlSection(option.tableName, entry));
    }
  }
  return ensureTrailingNewline(sections.join("\n\n"));
}

function contextEntryToTomlSection(tableName: string, entry: CodexContextEntry): string {
  const parentHeader = `[${tableName}.${tomlKey(entry.id)}]`;
  const body = entry.tomlBody
    .trimEnd()
    .split(/\r?\n/)
    .map((line) => relativeContextSubtableToAbsolute(line, tableName, entry.id))
    .join("\n");
  return `${parentHeader}\n${body}`;
}

function relativeContextSubtableToAbsolute(line: string, tableName: string, id: string): string {
  const match = /^\s*\[([^\]]+)\]\s*$/.exec(line);
  if (!match) return line;
  const subtable = match[1].trim();
  if (!subtable || subtable.includes(".")) return line;
  return `[${tableName}.${tomlKey(id)}.${tomlKey(subtable)}]`;
}

function syncLiveConfigContextState(liveConfigContents: string, settings: BackendSettings): string {
  const entries = contextEntriesFromSettings(settings);
  const withoutManaged = stripContextEntriesFromConfig(liveConfigContents, entries);
  return joinTomlSectionsRootFirst([withoutManaged, selectedContextConfigToml(entries)]);
}

function relayCombinedCommonConfig(settings: BackendSettings): string {
  return joinTomlSectionsRootFirst([settings.relayCommonConfigContents || "", settings.relayContextConfigContents || ""]);
}

function splitContextConfigText(configContents: string): { common: string; context: string } {
  const entries = contextEntriesFromConfig(configContents);
  return {
    common: stripContextEntriesFromConfig(configContents, entries),
    context: allContextConfigToml(entries),
  };
}

function stripContextEntriesFromConfig(configContents: string, entries: CodexContextEntries): string {
  const knownIds: Record<ContextKind, Set<string>> = {
    mcp: new Set(entries.mcpServers.map((entry) => entry.id)),
    skill: new Set(entries.skills.map((entry) => entry.id)),
    plugin: new Set(entries.plugins.map((entry) => entry.id)),
  };
  const lines = configContents.split(/\r?\n/);
  const kept: string[] = [];
  let skipping = false;

  for (const line of lines) {
    const contextHeader = contextHeaderFromLine(line);
    if (contextHeader) {
      skipping = knownIds[contextHeader.kind].has(contextHeader.id);
    } else if (/^\s*\[[^\]]+\]\s*$/.test(line)) {
      skipping = false;
    }
    if (!skipping) kept.push(line);
  }

  return ensureTrailingNewline(kept.join("\n").trimEnd());
}

function stripCommonConfigTextFallback(configContents: string, commonConfig: string): string {
  const anchors = commonConfigAnchors(commonConfig);
  if (!anchors.rootKeys.size && !anchors.tableHeaders.size) return ensureTrailingNewline(configContents.trimEnd());

  const kept: string[] = [];
  let skippingTable = false;

  for (const line of configContents.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^\[[^\]]+\]$/.test(trimmed)) {
      skippingTable = anchors.tableHeaders.has(trimmed);
      if (skippingTable) continue;
    }
    if (skippingTable) continue;
    const key = tomlRootKeyFromLine(trimmed);
    if (key && anchors.rootKeys.has(key)) continue;
    kept.push(line);
  }

  return ensureTrailingNewline(kept.join("\n").trimEnd());
}

function commonConfigAnchors(commonConfig: string): { rootKeys: Set<string>; tableHeaders: Set<string> } {
  const rootKeys = new Set<string>();
  const tableHeaders = new Set<string>();
  let inRoot = true;

  for (const line of commonConfig.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^\[[^\]]+\]$/.test(trimmed)) {
      inRoot = false;
      tableHeaders.add(trimmed);
      continue;
    }
    if (inRoot) {
      const key = tomlRootKeyFromLine(trimmed);
      if (key) rootKeys.add(key);
    }
  }

  return { rootKeys, tableHeaders };
}

function tomlRootKeyFromLine(line: string): string | null {
  if (!line || line.startsWith("#")) return null;
  const index = line.indexOf("=");
  if (index < 0) return null;
  const key = line.slice(0, index).trim();
  return key || null;
}

function contextHeaderFromLine(line: string): { kind: ContextKind; id: string } | null {
  const path = tomlTablePathFromLine(line);
  if (!path || path.length !== 2) return null;
  const option = contextKindOptions.find((item) => item.tableName === path[0]);
  return option ? { kind: option.kind, id: path[1] } : null;
}

function applyContextLimitPreview(configContents: string, profile: RelayProfile): string {
  const replacements: Array<[string, string]> = [
    ["model_context_window", profile.contextWindow],
    ["model_auto_compact_token_limit", profile.autoCompactLimit],
  ];
  let lines = configContents.split(/\r?\n/);

  for (const [key, value] of replacements) {
    const trimmed = value.trim();
    if (!trimmed) continue;
    let replaced = false;
    lines = lines.map((line) => {
      if (!replaced && new RegExp(`^\\s*${key}\\s*=`).test(line)) {
        replaced = true;
        return `${key} = ${trimmed}`;
      }
      return line;
    });
    if (!replaced) {
      const firstTable = lines.findIndex((line) => /^\s*\[[^\]]+\]\s*$/.test(line));
      const insertAt = firstTable >= 0 ? firstTable : lines.length;
      lines.splice(insertAt, 0, `${key} = ${trimmed}`);
    }
  }

  return ensureTrailingNewline(lines.join("\n").trimEnd());
}

function removeRootTomlKey(contents: string, key: string): string {
  const lines: string[] = [];
  let inRoot = true;
  const sourceLines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(sourceLines);
  for (const [index, line] of sourceLines.entries()) {
    if (structural[index] && tomlSectionName(line) !== null) inRoot = false;
    const assignment = tomlKeyAssignment(line);
    if (structural[index] && inRoot && assignment?.path.length === 1 && assignment.path[0] === key) continue;
    lines.push(line);
  }
  return ensureTrailingNewline(lines.join("\n").trimEnd());
}

function joinTomlSections(sections: string[]): string {
  return ensureTrailingNewline(
    sections
      .map((section) => section.trim())
      .filter(Boolean)
      .join("\n\n"),
  );
}

function joinTomlSectionsRootFirst(sections: string[]): string {
  const rootParts: string[] = [];
  const tableParts: string[] = [];

  for (const section of sections) {
    const { root, tables } = splitTomlRootAndTables(section);
    if (root.trim()) rootParts.push(root.trim());
    if (tables.trim()) tableParts.push(tables.trim());
  }

  return normalizeDuplicateTomlTables(joinTomlSections([...dedupeTomlRootLines(rootParts), ...tableParts]));
}

function normalizeDuplicateTomlTables(contents: string): string {
  const seenHeaders = new Set<string>();
  const kept: string[] = [];
  let skipping = false;

  for (const line of contents.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^\[[^\]]+\]$/.test(trimmed)) {
      skipping = seenHeaders.has(trimmed);
      seenHeaders.add(trimmed);
      if (skipping) continue;
    }
    if (!skipping) kept.push(line);
  }

  return ensureTrailingNewline(kept.join("\n").trimEnd());
}

function dedupeTomlRootLines(rootParts: string[]): string[] {
  const rootLines = rootParts
    .join("\n")
    .split(/\r?\n/)
    .map((line) => line.trimEnd());
  const rootSeen = new Set<string>();
  const kept: string[] = [];

  for (let index = rootLines.length - 1; index >= 0; index -= 1) {
    const line = rootLines[index];
    const key = tomlRootKeyFromLine(line.trim());
    if (key) {
      if (rootSeen.has(key)) continue;
      rootSeen.add(key);
    }
    kept.push(line);
  }

  const normalized = kept.reverse().join("\n").trim();
  return normalized ? [normalized] : [];
}

function splitTomlRootAndTables(section: string): { root: string; tables: string } {
  const lines = section.trim().split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  const firstTable = lines.findIndex((line, index) => structural[index] && tomlSectionName(line) !== null);
  if (firstTable < 0) return { root: lines.join("\n"), tables: "" };
  return {
    root: lines.slice(0, firstTable).join("\n"),
    tables: lines.slice(firstTable).join("\n"),
  };
}

function tomlKey(key: string): string {
  return /^[A-Za-z0-9_-]+$/.test(key) ? key : `"${tomlString(key)}"`;
}

function contextSelectionIds(selection: RelayContextSelection, kind: ContextKind): string[] {
  if (kind === "mcp") return selection.mcpServers;
  if (kind === "skill") return selection.skills;
  return selection.plugins;
}

function setContextSelectionId(selection: RelayContextSelection, kind: ContextKind, id: string, checked: boolean): RelayContextSelection {
  const next = {
    mcpServers: [...selection.mcpServers],
    skills: [...selection.skills],
    plugins: [...selection.plugins],
  };
  const list = contextSelectionIds(next, kind);
  const normalizedId = id.trim();
  const exists = list.includes(normalizedId);
  if (checked && normalizedId && !exists) list.push(normalizedId);
  if (!checked && exists) list.splice(list.indexOf(normalizedId), 1);
  return next;
}

function removeContextSelectionFromSettings(settings: BackendSettings, kind: ContextKind, id: string): BackendSettings {
  return {
    ...settings,
    relayProfiles: settings.relayProfiles.map((profile) => ({
      ...profile,
      contextSelection: setContextSelectionId(profile.contextSelection, kind, id, false),
    })),
  };
}

function contextSelectionForAllEntries(settings: BackendSettings): RelayContextSelection {
  const entries = contextEntriesFromSettings(settings);
  return {
    mcpServers: entries.mcpServers.map((entry) => entry.id),
    skills: entries.skills.map((entry) => entry.id),
    plugins: entries.plugins.map((entry) => entry.id),
  };
}

function relayProfileEditorStatus(profile: RelayProfile, form: BackendSettings, isNew: boolean) {
  if (isNew) return t("新建供应商需要先保存到列表");
  if (!form.relayProfilesEnabled) return t("供应商配置总开关已关闭；当前只保存配置，不写入 Codex live 文件");
  return profile.id === form.activeRelayId ? t("当前正在使用") : t("编辑后保存列表，再切换模式时会使用新配置");
}

/// 供应商头像上的两个字，规则见 provider-name.ts 的 providerAvatarText。
function providerInitial(name: string) {
  return providerAvatarText(name, t("供"));
}

function statusLabel(status: string) {
  const labels: Record<string, string> = {
    found: t("已找到"),
    missing: t("缺失"),
    installed: t("已安装"),
    ok: t("正常"),
    running: t("运行中"),
    running_degraded: t("运行中（增强等待中）"),
    degraded: t("已降级"),
    starting: t("启动中"),
    failed: t("失败"),
    archived: t("已归档"),
    accepted: t("已受理"),
    not_checked: t("未检查"),
    not_implemented: t("未实现"),
    disabled: t("已禁用"),
    stopped: t("未运行"),
    waiting: t("等待启动"),
    off: t("未启用"),
    connected: t("已连接"),
    disconnected: t("未连接"),
    unknown: t("未知"),
  };
  return labels[status] ?? status;
}

function statusClass(status: string) {
  if (["found", "installed", "ok", "running", "running_degraded", "connected"].includes(status)) return "good";
  if (status === "waiting" || status === "off") return "idle";
  if (["failed", "missing"].includes(status)) return "bad";
  return "warn";
}

function isSuccessStatus(status?: Status) {
  return status === "ok" || status === "accepted";
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

/// 概览健康检查的各项。每项带自己的图标；本地服务器与协议转换拆成两项：
/// 协议转换跑在本地服务器里，但两者状态独立（服务器在跑、转换没开是常态）。
function healthItems(overview: OverviewResult | null): Array<{ title: string; status: string; ok: boolean; detail: string; icon: LucideIcon }> {
  const conversionEnabled = overview?.runtime_health?.protocol_conversion_enabled;
  const serviceItem = (title: string, state: RuntimeServiceState | undefined, icon: LucideIcon) => ({
    title,
    status: state?.status ?? "not_checked",
    ok: ["running", "connected", "disabled"].includes(state?.status ?? ""),
    detail: state ? [state.address, t(state.message)].filter(Boolean).join(" · ") : t("等待状态检查"),
    icon,
  });
  // 协议转换没开不是问题：直连 Responses 的供应商本来就不需要，显示为中性的「未启用」。
  const conversionHealth = {
    title: t("协议转换"),
    status: typeof conversionEnabled !== "boolean" ? "not_checked" : conversionEnabled ? "ok" : "off",
    ok: typeof conversionEnabled === "boolean",
    detail: typeof conversionEnabled !== "boolean"
      ? t("等待状态检查")
      : conversionEnabled
        ? t("Chat Completions / Anthropic 请求经本地服务器转成 Responses")
        : t("当前供应商直连 Responses，不需要转换"),
    icon: ArrowRightLeft,
  };
  // 调试连接依赖 Codex 在运行：Codex 没启动时连不上是预期内的，显示为中性的「等待启动」而不是警告。
  const debuggerState = overview?.runtime_health?.debugger;
  const codexRunning = overview?.runtime_health?.codex_app?.status === "running";
  const debuggerHealth = debuggerState && debuggerState.status !== "connected" && !codexRunning
    ? {
      title: t("调试连接状态"),
      status: "waiting",
      ok: true,
      detail: [debuggerState.address, t("Codex 未运行，启动后自动连接")].filter(Boolean).join(" · "),
      icon: Bug,
    }
    : serviceItem(t("调试连接状态"), debuggerState, Bug);
  return [
    {
      title: t("Codex 应用"),
      status: overview?.codex_app.status ?? "not_checked",
      ok: overview?.codex_app.status === "found",
      detail: overview?.codex_app.path || t("尚未检查 Codex 应用路径。"),
      icon: AppWindow,
    },
    serviceItem(t("本地服务器状态"), overview?.runtime_health?.local_server, Server),
    conversionHealth,
    debuggerHealth,
  ];
}


function normalizeTypingEffect(value: unknown): TypingEffect {
  return value === "rainbow" || value === "fireworks" || value === "stars" ? value : "off";
}

function normalizeSettings(settings: BackendSettings): BackendSettings {
  const backendAggregates = new Map(
    (settings.aggregateRelayProfiles ?? []).map((aggregate) => [aggregate.id, aggregate] as const),
  );
  const splitCommon = splitContextConfigText(settings.relayCommonConfigContents || "");
  const relayCommonConfigContents = splitCommon.common;
  const relayContextConfigContents = joinTomlSectionsRootFirst([
    settings.relayContextConfigContents || "",
    splitCommon.context,
  ]);
  const defaultContextSelection = contextSelectionForAllEntries({
    ...settings,
    relayCommonConfigContents,
    relayContextConfigContents,
  });
  const profiles =
    settings.relayProfiles?.length
      ? settings.relayProfiles.map((profile) =>
          normalizeRelayProfile(hydrateAggregateRelayProfile(profile, backendAggregates.get(profile.id)), defaultContextSelection),
        )
      : [
          {
            id: settings.activeRelayId || "default",
            name: t("默认中转"),
            model: "",
            baseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
            upstreamBaseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
            apiKey: settings.relayApiKey || "",
            protocol: "responses" as RelayProtocol,
            relayMode: "official" as RelayMode,
            sessionProvider: "custom" as RelaySessionProvider,
            officialMixApiKey: false,
            hideOfficialUsageAlert: false,
            testModel: "",
            configContents: "",
            authContents: "",
            useCommonConfig: true,
            contextSelection: defaultContextSelection,
            contextSelectionInitialized: true,
            contextWindow: "",
            autoCompactLimit: "",
            modelList: "",
            modelWindows: "",
            modelAutoCompact: "",
            modelMetadata: "",
            modelVlm: "",
            vlmApiKey: "",
            vlmModel: "",
            vlmBaseUrl: "",
            userAgent: "",
            customHeaders: [],
            sub2apiEnabled: false,
            noAuth: false,
            sub2apiMultiplier: "",
            standardOpenaiProtocol: false,
            webSearchHistoryCompat: false,
          },
        ];
  const activeRelayId = profiles.some((profile) => profile.id === settings.activeRelayId)
    ? settings.activeRelayId
    : profiles[0]?.id || "default";
  return syncLegacyRelayFields({
    ...defaultSettings,
    ...settings,
    // 旧版「Codex 用量挂件」开着、还没有桌宠设置的用户，升级后桌宠保持开启。
    agentPet: normalizeAgentPetSettings(settings.agentPet, (settings as { codexAppWhaleWidgetEnabled?: unknown }).codexAppWhaleWidgetEnabled === true),
    ccsDbPath: (settings.ccsDbPath || "").trim(),
    dictation: normalizeDictationSettings(settings.dictation),
    codexAppTypingEffect: normalizeTypingEffect(settings.codexAppTypingEffect),
    relayProfilesEnabled: settings.relayProfilesEnabled !== false,
    protocolProxyAutoRetry: settings.protocolProxyAutoRetry !== false,
    // 默认关闭（opt-in），只有显式开启才生效。
    protocolProxyMapUnlistedModels: settings.protocolProxyMapUnlistedModels === true,
    protocolProxyAutoRetryMaxAttempts: clampNumber(settings.protocolProxyAutoRetryMaxAttempts ?? 5, 0, 20),
    protocolProxyAutoRetryMode:
      settings.protocolProxyAutoRetryMode === "buffered"
        ? "buffered"
        : settings.protocolProxyAutoRetryMode === "streaming-legacy"
          ? "streaming-legacy"
          : "streaming",
    protocolProxyAutoRetryCommitDelayMs: clampNumber(settings.protocolProxyAutoRetryCommitDelayMs ?? 1000, 0, 10000),
    protocolProxyAutoRetryIntervalSeconds: clampNumber(settings.protocolProxyAutoRetryIntervalSeconds ?? 2, 0, 30),
    protocolProxyAutoRetryStatusCodes: normalizeRetryStatusCodes(settings.protocolProxyAutoRetryStatusCodes),
    helperMaxRequestBodyMib: clampNumber(settings.helperMaxRequestBodyMib ?? 128, 32, 512),
    codexAppImageOverlayOpacity: clampNumber(settings.codexAppImageOverlayOpacity || 35, 1, 100),
    codexAppImageOverlayFitMode: normalizeImageOverlayFitMode(settings.codexAppImageOverlayFitMode),
    codexAppDreamSkinPaused: settings.codexAppDreamSkinPaused === true,
    codexAppDreamSkinThemeConfig: normalizeDreamSkinTheme(settings.codexAppDreamSkinThemeConfig),
    codexAppDreamSkinImagePath: (settings.codexAppDreamSkinImagePath || "").trim(),
    codexAppStepwiseProtocol: normalizeStepwiseProtocol(settings.codexAppStepwiseProtocol),
    codexAppStepwiseGenerationMode: normalizeStepwiseGenerationMode(settings.codexAppStepwiseGenerationMode),
    codexAppStepwiseMaxItems: clampNumber(settings.codexAppStepwiseMaxItems ?? 4, 0, 6),
    codexAppStepwiseMaxInputChars: clampNumber(settings.codexAppStepwiseMaxInputChars || 6000, 1000, 24000),
    codexAppStepwiseMaxOutputTokens: clampNumber(settings.codexAppStepwiseMaxOutputTokens || 500, 100, 4000),
    codexAppStepwiseTimeoutMs: clampNumber(settings.codexAppStepwiseTimeoutMs || 8000, 1000, 60000),
    relayCommonConfigContents,
    relayContextConfigContents,
    relayProfiles: profiles,
    activeRelayId,
  });
}

function dictationSettingsValidationMessage(issue: DictationSettingsIssue | null): string | null {
  if (issue === "baseUrl") return t("语音 Base URL 必须是 HTTP(S) 基础地址，不能包含账号密码、查询参数或片段。");
  if (issue === "model") return t("启用语音输入时需要填写语音模型。");
  if (issue === "apiKeyEnv") return t("语音 API Key 环境变量名称只能包含字母、数字和下划线，且不能以数字开头。");
  return null;
}

function normalizeStepwiseProtocol(value: StepwiseProtocol | undefined): StepwiseProtocol {
  return value === "auto"
    || value === "responses"
    || value === "anthropic_messages"
    ? value
    : "chat_completions";
}

function backendSettingsEqual(left: BackendSettings, right: BackendSettings): boolean {
  const { revision: _leftRevision, ...leftValues } = normalizeSettings(left);
  const { revision: _rightRevision, ...rightValues } = normalizeSettings(right);
  return JSON.stringify(leftValues) === JSON.stringify(rightValues);
}

/** 「填入常用」：限流与常见的服务端/网关瞬时错误。 */
const COMMON_RETRY_STATUS_CODES = [429, 500, 502, 503, 504];

/** 与 Rust `normalize_protocol_proxy_auto_retry_status_codes` 保持一致：400~599、去重升序、最多 20 个。 */
function normalizeRetryStatusCodes(value: unknown): number[] {
  if (!Array.isArray(value)) return [];
  const codes = value
    .map(Number)
    .filter((status) => Number.isInteger(status) && status >= 400 && status <= 599);
  return Array.from(new Set(codes)).sort((a, b) => a - b).slice(0, 20);
}

function clampNumber(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.round(value)));
}

function normalizeStepwiseGenerationMode(value: StepwiseGenerationMode | undefined): StepwiseGenerationMode {
  return value === "manual" ? "manual" : "auto";
}

function parsePort(value: string, fallback: number): number {
  const parsed = Number.parseInt(value, 10);
  return Number.isInteger(parsed) && parsed >= 1 && parsed <= 65535 ? parsed : fallback;
}

function normalizeImageOverlayFitMode(value: string | undefined): ImageOverlayFitMode {
  return value === "fill" || value === "fit" || value === "stretch" || value === "tile" || value === "center"
    ? value
    : "fit";
}

function codexExtraArgsToInput(args: string[] | undefined) {
  return (args ?? []).join("\n");
}

function inputToCodexExtraArgs(value: string) {
  return value === "" ? [] : value.split(/\r?\n/);
}

function normalizeRelayProfile(profile: RelayProfile, defaultContextSelection = emptyContextSelection()): RelayProfile {
  const legacyMixedApi = profile.relayMode === "mixedApi";
  if (profile.relayMode === "aggregate" || profile.aggregate) {
    return normalizeAggregateRelayProfile(
      {
        ...profile,
        model: profile.model || "",
        baseUrl: "",
        upstreamBaseUrl: "",
        apiKey: "",
        protocol: "responses",
        relayMode: "aggregate",
        sessionProvider: normalizeRelaySessionProvider(profile.sessionProvider),
        officialMixApiKey: false,
        hideOfficialUsageAlert: false,
        testModel: profile.testModel || "",
        configContents: "",
        authContents: "",
        useCommonConfig: profile.useCommonConfig !== false,
        contextSelection: profile.contextSelectionInitialized
          ? normalizeContextSelection(profile.contextSelection)
          : normalizeContextSelection(undefined, defaultContextSelection),
        contextSelectionInitialized: true,
        contextWindow: "",
        autoCompactLimit: "",
        modelList: "",
        modelWindows: "",
        modelAutoCompact: "",
        modelMetadata: "",
        modelRoutes: [],
        sub2apiEnabled: false,
        noAuth: false,
        sub2apiMultiplier: "",
        standardOpenaiProtocol: false,
        webSearchHistoryCompat: false,
      },
      null,
    );
  }
  const relayMode = normalizeRelayMode(profile.relayMode);
  const officialMixApiKey = profile.officialMixApiKey === true || legacyMixedApi;
  let normalized: RelayProfile = {
    ...profile,
    model: profile.model || "",
    baseUrl: profile.baseUrl || defaultSettings.relayBaseUrl,
    upstreamBaseUrl: profile.upstreamBaseUrl || profile.baseUrl || "",
    apiKey: profile.apiKey || "",
    protocol: normalizeRelayProtocol(profile.protocol),
    relayMode,
    sessionProvider: relaySessionProvider(profile),
    officialMixApiKey,
    hideOfficialUsageAlert: profile.hideOfficialUsageAlert === true,
    testModel: profile.testModel || "",
    configContents: profile.configContents || "",
    authContents: relayMode === "official" && !officialMixApiKey ? buildOfficialRelayAuthJson(profile.authContents || "") : profile.authContents || "",
    useCommonConfig: profile.useCommonConfig !== false,
    contextSelection: profile.contextSelectionInitialized
      ? normalizeContextSelection(profile.contextSelection)
      : normalizeContextSelection(undefined, defaultContextSelection),
    contextSelectionInitialized: true,
    contextWindow: profile.contextWindow || "",
    autoCompactLimit: profile.autoCompactLimit || "",
    modelList: profile.modelList || "",
    modelWindows: profile.modelWindows || "",
    modelAutoCompact: profile.modelAutoCompact || "",
    modelMetadata: profile.modelMetadata || "",
    modelRoutes: relayMode === "official" && !officialMixApiKey ? [] : normalizeRelayModelRoutes(profile.modelRoutes),
    userAgent: profile.userAgent || "",
    autoReviewModel: profile.autoReviewModel?.trim() || "",
    customHeaders: profile.customHeaders || [],
    sub2apiEnabled: profile.noAuth ? false : profile.sub2apiEnabled === true,
    sub2apiMultiplier: !profile.noAuth && profile.sub2apiEnabled === true ? profile.sub2apiMultiplier || "" : "",
    standardOpenaiProtocol: profile.standardOpenaiProtocol === true,
    webSearchHistoryCompat: profile.webSearchHistoryCompat === true,
    localCompactionPromptAtEnd: profile.localCompactionPromptAtEnd === true,
    trustUpstreamCatalogLite: profile.trustUpstreamCatalogLite === true,
    strictResponsesReasoning: profile.strictResponsesReasoning === true,
    anthropicAuthHeader: normalizeAnthropicAuthHeader(profile.anthropicAuthHeader),
    anthropicMaxTokens: normalizeAnthropicMaxTokens(profile.anthropicMaxTokens),
    anthropicPromptCache: profile.anthropicPromptCache !== false,
    anthropicImpersonateClaudeCode: profile.anthropicImpersonateClaudeCode === true,
    anthropicThinkingMode: normalizeAnthropicThinkingMode(profile.anthropicThinkingMode),
  };
  const derived = relayProfileUsesLiveFiles(normalized) ? deriveRelayProfileFromFiles(normalized) : normalized;
  return derived.noAuth || (derived.relayMode === "official" && !derived.officialMixApiKey)
    ? derived : relayProfileWithNormalizedApiKeys(derived);
}

function hydrateAggregateRelayProfile(profile: RelayProfile, aggregate: AggregateRelayProfile | undefined): RelayProfile {
  if (!aggregate) return profile;
  return {
    ...profile,
    name: profile.name || aggregate.name,
    relayMode: "aggregate",
    sessionProvider: normalizeRelaySessionProvider(aggregate.sessionProvider),
    aggregate: {
      strategy: aggregate.strategy,
      modelAggregationEnabled: aggregate.modelAggregationEnabled === true,
      members: aggregate.members.map((member) => ({
        profileId: member.relayId,
        weight: clampAggregateWeight(member.weight),
      })),
      routes: normalizeAggregateRoutes(aggregate.routes ?? []),
      defaultRelayId: aggregate.defaultRelayId ?? "",
      syncDefaultMemberLogin: aggregate.syncDefaultMemberLogin === true,
    },
  };
}

function activeRelayProfile(settings: BackendSettings): RelayProfile {
  return (
    settings.relayProfiles.find((profile) => profile.id === settings.activeRelayId) ||
    settings.relayProfiles[0] ||
    defaultSettings.relayProfiles[0]
  );
}

/** 协议取值归一化：认不出的值一律回退 Responses，避免脏数据把上游写成直连。 */
function normalizeRelayProtocol(value: RelayProtocol | undefined): RelayProtocol {
  if (value === "chatCompletions" || value === "anthropicMessages") return value;
  return "responses";
}

/** Anthropic 认证字段：只允许 `x-api-key` 与 `authorization`，缺省用前者。 */
function normalizeAnthropicAuthHeader(value: string | undefined): string {
  const normalized = (value ?? "").trim().toLowerCase();
  if (normalized === "authorization" || normalized === "bearer") return "authorization";
  if (normalized === "x-api-key" || normalized === "x_api_key" || normalized === "api_key") return "x-api-key";
  return "";
}

/** 思考模式：只认四个取值，其余（含空串）按 auto。 */
function normalizeAnthropicThinkingMode(value: string | undefined): string {
  const normalized = (value ?? "").trim().toLowerCase();
  if (normalized === "adaptive" || normalized === "budget" || normalized === "off") {
    return normalized;
  }
  return "";
}

/** Anthropic 的 `max_tokens` 兜底值：空/非法按 0（用内置 8192），上限 200000。 */
function normalizeAnthropicMaxTokens(value: number | undefined): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return 0;
  return Math.min(200000, Math.round(value));
}

/// 列表标签用的短协议名：卡片空间小，「转 Responses」留在编辑页里说明。
function relayProtocolShortLabel(protocol: RelayProtocol): string {
  if (protocol === "chatCompletions") return "Chat Completions";
  if (protocol === "anthropicMessages") return "Anthropic";
  return "Responses";
}

/// 接入方式标签的配色分组。
function relayProfileTagTone(profile: RelayProfile): string {
  if (isAggregateRelayProfile(profile)) return "aggregate";
  if (profile.relayMode === "official") return profile.officialMixApiKey ? "mixed" : "official";
  return "api";
}

function relayProtocolLabel(protocol: RelayProtocol): string {
  if (protocol === "chatCompletions") return t("Chat Completions 转 Responses");
  if (protocol === "anthropicMessages") return t("Anthropic Messages 转 Responses");
  return "Responses API";
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

function normalizeRelayMode(mode: RelayMode | undefined): RelayMode {
  if (mode === "aggregate") return mode;
  if (mode === "pureApi") return mode;
  return "official";
}

function normalizeRelaySessionProvider(value: string | undefined): RelaySessionProvider {
  return value === "openai" ? "openai" : "custom";
}

function relaySessionProviderFromConfig(contents: string): RelaySessionProvider {
  return normalizeRelaySessionProvider(rootTomlStringValue(contents, "model_provider"));
}

function relaySessionProvider(profile: Pick<RelayProfile, "configContents" | "sessionProvider">): RelaySessionProvider {
  const fromConfig = relaySessionProviderFromConfig(profile.configContents);
  return fromConfig === "openai" || profile.sessionProvider === "openai" ? "openai" : "custom";
}

function normalizeContextSelection(
  selection?: Partial<RelayContextSelection>,
  fallback: RelayContextSelection = emptyContextSelection(),
): RelayContextSelection {
  if (!selection) {
    return {
      mcpServers: [...fallback.mcpServers],
      skills: [...fallback.skills],
      plugins: [...fallback.plugins],
    };
  }
  return {
    mcpServers: Array.isArray(selection?.mcpServers) ? selection.mcpServers.map(String) : [],
    skills: Array.isArray(selection?.skills) ? selection.skills.map(String) : [],
    plugins: Array.isArray(selection?.plugins) ? selection.plugins.map(String) : [],
  };
}

function relayModeLabel(mode: RelayMode): string {
  if (mode === "aggregate") return t("聚合供应商");
  if (mode === "pureApi") return t("纯 API");
  return t("官方登录");
}

function relayEditorMode(profile: Pick<RelayProfile, "relayMode" | "officialMixApiKey">): RelayEditorMode {
  if (profile.relayMode === "pureApi") return "pureApi";
  if (profile.relayMode === "mixedApi" || profile.officialMixApiKey) return "mixedApi";
  return "official";
}

function relayProfileModeLabel(profile: Pick<RelayProfile, "relayMode" | "officialMixApiKey" | "aggregate">): string {
  if (profile.relayMode === "aggregate") {
    return profile.aggregate?.modelAggregationEnabled === true ? t("模型聚合") : t("故障转移");
  }
  return relayEditorMode(profile) === "mixedApi" ? t("官方登录 + API") : relayModeLabel(profile.relayMode);
}

function providerImportWireApiLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "chat" || normalized === "chat_completions" || normalized === "chat-completions") {
    return "Chat Completions";
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

function relayProfileConfigBrief(profile: RelayProfile): string {
  if (isAggregateRelayProfile(profile)) {
    const aggregate = normalizeAggregateConfig(profile.aggregate, []);
    // 模型聚合与轮转策略无关，卡片上不显示「失败切换」这类策略文案。
    if (aggregate.modelAggregationEnabled === true) {
      return tf("模型固定绑定 · {0} 个成员", [aggregate.members.length]);
    }
    return tf("{0} · {1} 个成员", [aggregateStrategyLabel(aggregate.strategy), aggregate.members.length]);
  }
  if (profile.relayMode === "official") return profile.officialMixApiKey ? t("混入 API Key") : t("不写 API 文件");
  return profile.baseUrl || t("未填写 URL");
}

function relayProfileModeHelp(profile: RelayProfile): string {
  if (isAggregateRelayProfile(profile)) {
    const aggregate = normalizeAggregateConfig(profile.aggregate, []);
    if (aggregate.modelAggregationEnabled) {
      return t("此聚合会把成员模型合并到 Codex 列表；带供应商前缀的模型固定发送到对应成员，普通模型仍按策略轮转。");
    }
    return t("聚合供应商只保存成员和策略配置，成员来自已有 API 供应商；切为当前后会通过本地协议代理轮转请求。");
  }
  if (profile.relayMode === "official") {
    if (profile.officialMixApiKey) {
      return t("此供应商会保留官方登录模式，并把请求混入当前 API Key；Codex增强仍使用兼容模式。");
    }
    return t("此供应商会切回官方登录模式，使用 ChatGPT 官方账号，不写入 API Key。");
  }
  if (profile.relayMode === "pureApi") {
    return t("此供应商会同时写入 config.toml 和 auth.json；API Key 也会注入到 provider bearer token。");
  }
  return t("此供应商会保留官方登录模式，并把请求混入当前 API Key；Codex增强仍使用兼容模式。");
}

function relayProfileReadinessText(profile: RelayProfile, relay: RelayResult | null): string {
  if (isAggregateRelayProfile(profile)) {
    const aggregate = normalizeAggregateConfig(profile.aggregate, []);
    return tf("聚合供应商已配置为{0}，包含 {1} 个成员；真实对话会走本地代理轮转。", [aggregateStrategyLabel(aggregate.strategy), aggregate.members.length]);
  }
  if (profile.relayMode === "official") {
    if (profile.officialMixApiKey) {
      const hasApiFields = profile.baseUrl.trim() && profile.apiKey.trim();
      if (!relay?.authenticated && !hasApiFields) return t("当前未登录官方账号，也未配置混入 API 的 Base URL / Key。");
      if (!relay?.authenticated) return t("当前未登录官方账号；官方登录混入 API Key 需要先登录官方账号。");
      if (!hasApiFields) return t("当前还没有填写混入 API 的 Base URL / Key。");
      return tf("官方登录已就绪：{0}，会混入当前 API Key。", [relay.accountLabel || t("已登录")]);
    }
    return relay?.authenticated
      ? tf("官方账号已登录：{0}。", [relay.accountLabel || relay.authSource || t("已检测")])
      : t("当前未登录官方账号；切到官方登录模式后仍需要先在 Codex/ChatGPT 登录。");
  }
  const hasFiles = profile.configContents.trim() && profile.authContents.trim();
  if (!hasFiles) return t("当前供应商还没有完整 config.toml / API Key 存档。");
  if (relay && !relay.configured) return t("纯 API 配置未完整写入：请检查此供应商是否有 OPENAI_API_KEY，且 config.toml 是否包含 model_provider / provider / base_url。");
  return t("纯 API 就绪：会同时写入 config.toml 和 auth.json。");
}

function relayProfileSwitchCommand(profile: RelayProfile): "clear_relay_injection" | "apply_relay_injection" | "apply_pure_api_injection" {
  if (isAggregateRelayProfile(profile)) return "apply_relay_injection";
  if (profile.relayMode === "pureApi") return "apply_pure_api_injection";
  if (profile.relayMode === "official" && !profile.officialMixApiKey) return "clear_relay_injection";
  if (profile.configContents.trim()) return "apply_relay_injection";
  return profile.officialMixApiKey ? "apply_relay_injection" : "clear_relay_injection";
}

function withGeneratedRelayFiles(profile: RelayProfile): RelayProfile {
  if (isAggregateRelayProfile(profile)) {
    return { ...profile, configContents: "", authContents: "", aggregate: normalizeAggregateConfig(profile.aggregate, []) };
  }
  if (profile.relayMode === "official") {
    return {
      ...profile,
      configContents: profile.officialMixApiKey
        ? buildRelayConfigToml(profile, { includeBearerToken: true, requiresOpenAiAuth: true })
        : "",
      authContents: profile.authContents || "",
    };
  }
  return {
    ...profile,
    configContents: buildRelayConfigToml(profile, { includeBearerToken: !profile.noAuth, requiresOpenAiAuth: false }),
    authContents: buildRelayAuthJson(profile),
  };
}

function buildRelayConfigToml(
  profile: Pick<RelayProfile, "id" | "model" | "baseUrl" | "upstreamBaseUrl" | "apiKey" | "protocol" | "sessionProvider" | "relayMode" | "officialMixApiKey" | "webSearchHistoryCompat" | "localCompactionPromptAtEnd" | "strictResponsesReasoning" | "modelMetadata">,
  options: { includeBearerToken: boolean; requiresOpenAiAuth?: boolean },
): string {
  const searchHistoryCompat = profile.webSearchHistoryCompat === true
    && profile.protocol === "responses"
    && profile.relayMode !== "aggregate"
    && (profile.relayMode !== "official" || profile.officialMixApiKey);
  const localCompaction = profile.localCompactionPromptAtEnd === true
    && (profile.protocol === "responses" || profile.protocol === "chatCompletions")
    && profile.relayMode !== "aggregate"
    && (profile.relayMode !== "official" || profile.officialMixApiKey);
  const baseUrl = profile.protocol !== "responses" || independentProxyEnabled(profile) || searchHistoryCompat || localCompaction || requestCompatibilityEnabled(profile)
    ? PROTOCOL_PROXY_BASE_URL
    : profile.baseUrl.trim();
  const apiKey = profile.apiKey.trim();
  const sessionProvider = normalizeRelaySessionProvider(profile.sessionProvider);
  const rootLines = [
    profile.model.trim() ? `model = "${tomlString(profile.model.trim())}"` : null,
    `model_provider = "${sessionProvider}"`,
    sessionProvider === "openai" ? `openai_base_url = "${PROTOCOL_PROXY_BASE_URL}"` : null,
    "",
  ].filter((line): line is string => line !== null);
  const requiresOpenAiAuthLine = options.requiresOpenAiAuth === undefined
    ? null
    : `requires_openai_auth = ${options.requiresOpenAiAuth ? "true" : "false"}`;
  return [
    ...rootLines,
    "[model_providers.custom]",
    'name = "custom"',
    'wire_api = "responses"',
    requiresOpenAiAuthLine,
    `base_url = "${tomlString(baseUrl)}"`,
    options.includeBearerToken && apiKey ? `experimental_bearer_token = "${tomlString(apiKey)}"` : null,
    "",
  ].filter((line): line is string => line !== null).join("\n");
}

function buildRelayAuthJson(profile: Pick<RelayProfile, "apiKey">): string {
  return `${JSON.stringify({ OPENAI_API_KEY: profile.apiKey.trim() }, null, 2)}\n`;
}

function buildOfficialRelayAuthJson(contents: string): string {
  const trimmed = contents.trim();
  if (!trimmed) return "";
  try {
    const parsed = JSON.parse(trimmed) as Record<string, unknown>;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return contents;
    const knownOAuth = String(parsed.auth_mode || "").toLowerCase() === "chatgpt" && parsed.tokens && typeof parsed.tokens === "object";
    if (!knownOAuth && Object.keys(parsed).some((key) => key !== "OPENAI_API_KEY")) return contents;
    delete parsed.OPENAI_API_KEY;
    return `${JSON.stringify(parsed, null, 2)}\n`;
  } catch {
    return contents;
  }
}

function deriveRelayProfileFromFiles(profile: RelayProfile, editedFile?: "config" | "auth"): RelayProfile {
  if (isAggregateRelayProfile(profile)) {
    return normalizeAggregateRelayProfile(profile, null);
  }
  if (editedFile === "auth") {
    try {
      JSON.parse(profile.authContents || "{}");
    } catch {
      return profile;
    }
  }
  let configContents = profile.configContents || "";
  let authContents = profile.relayMode === "official" ? buildOfficialRelayAuthJson(profile.authContents || "") : profile.authContents || "";
  const configBaseUrl = codexBaseUrlFromConfig(configContents);
  const chatUpstreamBaseUrl = rootTomlStringValue(configContents, CHAT_UPSTREAM_BASE_URL_KEY);
  const isProxyConfig = configBaseUrl === PROTOCOL_PROXY_BASE_URL;
  const upstreamBaseUrl = profile.upstreamBaseUrl || chatUpstreamBaseUrl || (configBaseUrl && !isProxyConfig ? configBaseUrl : profile.baseUrl || "");
  const configApiKey = codexExperimentalBearerTokenFromConfig(configContents);
  const apiKey = editedFile === "config" ? configApiKey
    : editedFile === "auth" && profile.relayMode === "pureApi" ? codexApiKeyFromAuth(authContents)
    : profile.relayMode === "official"
    ? configApiKey || profile.apiKey || ""
    : codexApiKeyFromAuth(authContents) || configApiKey || "";
  if (profile.relayMode === "pureApi") {
    // 旧 false/缺省模板也需要显式 provider 认证；保留用户已有的 true/false。
    configContents = profile.noAuth
      ? removeCodexExperimentalBearerToken(configContents)
      : apiKey && configContents.trim()
        ? setCodexExperimentalBearerToken(configContents, apiKey, { requiresOpenAiAuth: false })
        : configContents;
    if (editedFile && !profile.noAuth) {
      if (editedFile === "config") authContents = setAuthOpenAiApiKey(authContents, apiKey);
      if (!apiKey) configContents = removeCodexExperimentalBearerToken(configContents);
    }
  }
  const configModel = codexModelFromConfig(configContents);
  // 如果用户输入了带后缀的模型名，优先保留在界面的「配置模型」字段中；
  // config.toml 里实际写的是剥离后缀的 slug（由 applyRelayProfilePatchToFiles 处理）。
  const model = /\[.+\]$/.test(profile.model.trim()) ? profile.model.trim() : configModel;
  const derived = {
    ...profile,
    model,
    sessionProvider: relaySessionProviderFromConfig(configContents),
    baseUrl: upstreamBaseUrl,
    upstreamBaseUrl,
    apiKey,
    contextWindow: codexTopLevelIntFromConfig(configContents, "model_context_window"),
    autoCompactLimit: codexTopLevelIntFromConfig(configContents, "model_auto_compact_token_limit"),
    configContents,
    authContents,
  };
  return editedFile ? relayProfileWithUpdatedApiKey(derived, apiKey) : derived;
}

function applyRelayProfilePatchToFiles(
  profile: RelayProfile,
  patch: Partial<RelayProfile>,
  options: { allowGenerateFiles?: boolean } = {},
): RelayProfile {
  const protocol = patch.protocol ?? profile.protocol;
  const sessionProvider = "sessionProvider" in patch
    ? normalizeRelaySessionProvider(patch.sessionProvider)
    : relaySessionProvider(profile);
  const compatibleSession = sessionProviderForProtocol(sessionProvider, protocol);
  const normalizedPatch = compatibleSession === sessionProvider
    ? patch
    : { ...patch, sessionProvider: compatibleSession };
  let next: RelayProfile = { ...profile, ...normalizedPatch };
  if (isAggregateRelayProfile(next)) {
    return normalizeAggregateRelayProfile(next, null);
  }
  const shouldHaveFiles =
    next.relayMode !== "official" || next.officialMixApiKey || next.configContents.trim() || next.authContents.trim();
  const needsAuthFile = next.relayMode === "pureApi";
  if (options.allowGenerateFiles && shouldHaveFiles && (!next.configContents.trim() || (needsAuthFile && !next.authContents.trim()))) {
    next = withGeneratedRelayFiles(next);
  }

  if ("sessionProvider" in patch) {
    const sessionProvider = normalizeRelaySessionProvider(patch.sessionProvider);
    next.sessionProvider = sessionProvider;
    next.configContents = setRootTomlStringKey(next.configContents, "model_provider", sessionProvider);
    next.configContents = setManagedOpenAiBaseUrl(next.configContents, sessionProvider === "openai");
  }

  if ("model" in patch) {
    // 模型后缀（如 [1M]）仅供 CodexPlusPlus 内部使用，写入 config.toml 前需剥离，
    // 否则 codex 会按带后缀的字符串去匹配 catalog slug，导致窗口回退到默认值。
    const { slug } = parseModelSuffix(patch.model || "");
    next.configContents = setRootTomlStringKey(next.configContents, "model", slug);
  }
  if ("apiKey" in patch) {
    next = relayProfileWithUpdatedApiKey(next, patch.apiKey || "");
    if (next.relayMode === "pureApi") {
      next.authContents = setAuthOpenAiApiKey(next.authContents, patch.apiKey || "");
      next.configContents = setCodexExperimentalBearerToken(next.configContents, patch.apiKey || "", { requiresOpenAiAuth: false });
    } else {
      next.configContents = setCodexExperimentalBearerToken(next.configContents, patch.apiKey || "");
    }
  }
  if ("baseUrl" in patch) {
    next.upstreamBaseUrl = patch.baseUrl || "";
  }
  if ("upstreamBaseUrl" in patch) {
    next.baseUrl = patch.upstreamBaseUrl || "";
  }
  if ("baseUrl" in patch || "upstreamBaseUrl" in patch || "protocol" in patch || "modelRoutes" in patch || "webSearchHistoryCompat" in patch || "localCompactionPromptAtEnd" in patch || "strictResponsesReasoning" in patch || "modelMetadata" in patch) {
    const searchHistoryCompat = next.webSearchHistoryCompat === true
      && next.protocol === "responses"
      && next.relayMode !== "aggregate"
      && (next.relayMode !== "official" || next.officialMixApiKey);
    const localCompaction = next.localCompactionPromptAtEnd === true
      && (next.protocol === "responses" || next.protocol === "chatCompletions")
      && next.relayMode !== "aggregate"
      && (next.relayMode !== "official" || next.officialMixApiKey);
    const baseUrlForConfig = next.protocol !== "responses" || independentProxyEnabled(next) || normalizeRelayModelRoutes(next.modelRoutes).length > 0 || searchHistoryCompat || localCompaction || requestCompatibilityEnabled(next)
      ? PROTOCOL_PROXY_BASE_URL
      : next.upstreamBaseUrl || next.baseUrl;
    next.configContents = setCodexProviderStringKey(next.configContents, "base_url", baseUrlForConfig, {
      requiresOpenAiAuth: next.relayMode !== "pureApi",
    });
    next.configContents = removeRootTomlKey(next.configContents, CHAT_UPSTREAM_BASE_URL_KEY);
  }
  if ("contextWindow" in patch) {
    next.configContents = setRootTomlIntKey(next.configContents, "model_context_window", patch.contextWindow || "");
  }
  if ("autoCompactLimit" in patch) {
    next.configContents = setRootTomlIntKey(
      next.configContents,
      "model_auto_compact_token_limit",
      patch.autoCompactLimit || "",
    );
  }
  if ("relayMode" in patch || "officialMixApiKey" in patch) {
    if (next.relayMode === "official" && !next.officialMixApiKey) {
      next.configContents = "";
      next.authContents = buildOfficialRelayAuthJson(next.authContents);
    } else if (options.allowGenerateFiles && (!next.configContents.trim() || (next.relayMode === "pureApi" && !next.authContents.trim()))) {
      next = withGeneratedRelayFiles(next);
    }
  }
  return deriveRelayProfileFromFiles(next);
}

function codexModelFromConfig(contents: string): string {
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    if (trimmed.startsWith("[")) break;
    const match = /^model\s*=\s*(["'])(.*)\1\s*$/.exec(trimmed);
    if (match) return match[2].replace(/\\(["'\\])/g, "$1");
  }
  return "";
}

/// 解析模型后缀语法，如 deepseek-v4-flash[1M] -> { slug: "deepseek-v4-flash", window: 1000000 }
/// 非法或没有后缀时返回原串作为 slug。剥离与换算统一走 model-metadata.ts 的
/// suffixWindowString/modelSlugFromRowName，避免两处实现对「什么算合法后缀」
/// 的判断分叉。
function parseModelSuffix(raw: string): { slug: string; window?: number } {
  const window = suffixWindowString(raw);
  if (window === null) return { slug: raw.trim() };
  return { slug: modelSlugFromRowName(raw), window: Number(window) };
}

function codexBaseUrlFromConfig(contents: string): string {
  return codexProviderStringFromConfig(contents, "base_url");
}

function codexExperimentalBearerTokenFromConfig(contents: string): string {
  return codexProviderStringFromConfig(contents, "experimental_bearer_token");
}

function codexProviderStringFromConfig(contents: string, key: string): string {
  const provider = rootTomlStringValue(contents, "model_provider");
  const targetSection = codexProviderSectionName(provider || "custom");
  const fallbackSection = provider === "openai" && rootTomlStringValue(contents, "openai_base_url") === PROTOCOL_PROXY_BASE_URL
    ? codexProviderSectionName("custom") : "";
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  let currentSection = "";
  let rootValue = "";
  let fallbackValue = "";

  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const section = tomlSectionName(line);
    if (section !== null) {
      currentSection = section;
      continue;
    }
    const value = tomlStringAssignmentValue(line, key);
    if (value === null) continue;
    if (currentSection === targetSection) return value;
    if (fallbackSection && currentSection === fallbackSection) fallbackValue = value;
    if (!provider && currentSection === "") rootValue = value;
  }

  // 不借用 inactive provider 的 Key；仅托管 OpenAI 会话允许回落 custom transport。
  return fallbackValue || rootValue;
}

function codexApiKeyFromAuth(contents: string): string {
  try {
    const parsed = JSON.parse(contents || "{}") as { OPENAI_API_KEY?: unknown };
    return typeof parsed.OPENAI_API_KEY === "string" ? parsed.OPENAI_API_KEY : "";
  } catch {
    return "";
  }
}

function codexTopLevelIntFromConfig(contents: string, key: string): string {
  const topLevel = splitTomlRootAndTables(contents).root;
  const pattern = new RegExp(`^\\s*${key}\\s*=\\s*(\\d+)\\s*(?:#.*)?$`);
  const lines = topLevel.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const match = pattern.exec(line);
    if (match) return match[1];
  }
  return "";
}

function rootTomlStringValue(contents: string, key: string): string {
  const topLevel = splitTomlRootAndTables(contents).root;
  const lines = topLevel.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const value = tomlStringAssignmentValue(line, key);
    if (value !== null) return value;
  }
  return "";
}

function tomlStructureMask(lines: string[]): boolean[] {
  // 提示词里的伪表头/赋值只是字符串正文，不能参与 provider 鉴权读写。
  let multiline: string | null = null;
  return lines.map((line) => {
    const structural = multiline === null;
    for (let index = 0; index < line.length; index += 1) {
      if (multiline !== null) {
        if (multiline === '"' && line[index] === "\\") {
          index += 1;
        } else if (line[index] === multiline) {
          let end = index;
          while (line[end] === multiline) end += 1;
          if (end - index >= 3) multiline = null;
          index = end - 1;
        }
      } else if (line[index] === "#") {
        break;
      } else if (line[index] === '"' || line[index] === "'") {
        if (line[index + 1] === line[index] && line[index + 2] === line[index]) {
          multiline = line[index];
          index += 2;
        } else {
          const quoted = tomlQuotedValue(line, index);
          if (!quoted) break;
          index = quoted.end - 1;
        }
      }
    }
    return structural;
  });
}

function tomlSectionName(line: string): string | null {
  const text = line.trim();
  if (!text.startsWith("[")) return null;
  const array = text.startsWith("[[");
  const start = array ? 2 : 1;
  for (let index = start; index < text.length; index += 1) {
    if (text[index] === '"' || text[index] === "'") {
      const quoted = tomlQuotedValue(text, index);
      if (!quoted) return null;
      index = quoted.end - 1;
    } else if (text[index] === "]") {
      const end = index + (array ? 2 : 1);
      if (array && text[index + 1] !== "]") return null;
      const rest = text.slice(end).trim();
      if (rest && !rest.startsWith("#")) return null;
      const path = tomlDottedKeyPath(text.slice(start, index));
      // 数组表也是前一个表的边界，但不能与同名普通表混为一谈。
      return path ? (array ? "[]" : "") + tomlKeyPathName(path) : null;
    }
  }
  return null;
}

function tomlQuotedValue(text: string, start: number): { value: string; end: number } | null {
  const quote = text[start];
  if (quote !== '"' && quote !== "'") return null;
  let value = "";
  for (let index = start + 1; index < text.length; index += 1) {
    const character = text[index];
    if (character === quote) return { value, end: index + 1 };
    if ((character.charCodeAt(0) < 32 && character !== "\t") || character.charCodeAt(0) === 127) return null;
    if (quote === '"' && character === "\\") {
      const escape = text[++index];
      const escapes: Record<string, string> = { '"': '"', "\\": "\\", b: "\b", t: "\t", n: "\n", f: "\f", r: "\r" };
      if (Object.prototype.hasOwnProperty.call(escapes, escape)) value += escapes[escape];
      else if (escape === "u" || escape === "U") {
        const length = escape === "u" ? 4 : 8;
        const digits = text.slice(index + 1, index + 1 + length);
        if (digits.length !== length || !/^[0-9a-f]+$/i.test(digits)) return null;
        const codepoint = Number.parseInt(digits, 16);
        if (codepoint > 0x10ffff || (codepoint >= 0xd800 && codepoint <= 0xdfff)) return null;
        value += String.fromCodePoint(codepoint);
        index += length;
      } else return null;
    } else value += character;
  }
  return null;
}

function tomlDottedKeyPath(text: string): string[] | null {
  const path: string[] = [];
  let index = 0;
  while (index < text.length) {
    while (text[index] === " " || text[index] === "\t") index += 1;
    if (text[index] === '"' || text[index] === "'") {
      const quoted = tomlQuotedValue(text, index);
      if (!quoted) return null;
      path.push(quoted.value);
      index = quoted.end;
    } else {
      const bare = /^[A-Za-z0-9_-]+/.exec(text.slice(index));
      if (!bare) return null;
      path.push(bare[0]);
      index += bare[0].length;
    }
    while (text[index] === " " || text[index] === "\t") index += 1;
    if (index === text.length) return path;
    if (text[index] !== ".") return null;
    index += 1;
  }
  return null;
}

function tomlKeyPathName(path: string[]): string {
  return path.map((part) => /^[A-Za-z0-9_-]+$/.test(part) ? part : JSON.stringify(part)).join(".");
}

function codexProviderSectionName(provider: string): string {
  return tomlKeyPathName(["model_providers", provider]);
}

function tomlKeyAssignment(line: string): { path: string[]; valueStart: number } | null {
  for (let index = 0; index < line.length; index += 1) {
    if (line[index] === '"' || line[index] === "'") {
      const quoted = tomlQuotedValue(line, index);
      if (!quoted) return null;
      index = quoted.end - 1;
    } else if (line[index] === "#") return null;
    else if (line[index] === "=") {
      const path = tomlDottedKeyPath(line.slice(0, index));
      return path ? { path, valueStart: index + 1 } : null;
    }
  }
  return null;
}

function tomlInlineComment(line: string, start: number): string {
  for (let index = start; index < line.length; index += 1) {
    if (line[index] === '"' || line[index] === "'") {
      const quoted = tomlQuotedValue(line, index);
      if (!quoted) return "";
      index = quoted.end - 1;
    } else if (line[index] === "#") {
      const spacing = /[ \t]*$/.exec(line.slice(start, index))?.[0] || "";
      return spacing + line.slice(index);
    }
  }
  return "";
}

function tomlStringAssignmentValue(line: string, key: string): string | null {
  const assignment = tomlKeyAssignment(line);
  if (!assignment || assignment.path.length !== 1 || assignment.path[0] !== key) return null;
  const text = line.slice(assignment.valueStart).trimStart();
  const quoted = tomlQuotedValue(text, 0);
  if (!quoted) return null;
  const rest = text.slice(quoted.end).trimStart();
  return !rest || rest.startsWith("#") ? quoted.value : null;
}

function setAuthOpenAiApiKey(contents: string, apiKey: string): string {
  let parsed: Record<string, unknown> = {};
  try {
    const value = JSON.parse(contents || "{}");
    if (value && typeof value === "object" && !Array.isArray(value)) parsed = value as Record<string, unknown>;
  } catch {
    parsed = {};
  }
  parsed.OPENAI_API_KEY = apiKey.trim();
  return `${JSON.stringify(parsed, null, 2)}\n`;
}

function setRootTomlStringKey(contents: string, key: string, value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return removeRootTomlKey(contents, key);
  return setRootTomlLine(contents, key, `${key} = "${tomlString(trimmed)}"`);
}

function setManagedOpenAiBaseUrl(contents: string, enabled: boolean): string {
  const current = rootTomlStringValue(contents, "openai_base_url");
  if (enabled) {
    return !current || current === PROTOCOL_PROXY_BASE_URL
      ? setRootTomlStringKey(contents, "openai_base_url", PROTOCOL_PROXY_BASE_URL)
      : contents;
  }
  return current === PROTOCOL_PROXY_BASE_URL ? removeRootTomlKey(contents, "openai_base_url") : contents;
}

function setRootTomlIntKey(contents: string, key: string, value: string): string {
  const trimmed = value.replace(/[^\d]/g, "");
  if (!trimmed) return removeRootTomlKey(contents, key);
  return setRootTomlLine(contents, key, `${key} = ${trimmed}`);
}

function setRootTomlLine(contents: string, key: string, lineText: string): string {
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  const firstTable = lines.findIndex((line, index) => structural[index] && tomlSectionName(line) !== null);
  const rootEnd = firstTable >= 0 ? firstTable : lines.length;
  for (let index = 0; index < rootEnd; index += 1) {
    if (!structural[index]) continue;
    const assignment = tomlKeyAssignment(lines[index]);
    if (assignment?.path.length === 1 && assignment.path[0] === key) {
      const replacement = tomlKeyAssignment(lineText);
      lines[index] = replacement
        ? lines[index].slice(0, assignment.valueStart) + " " + lineText.slice(replacement.valueStart).trimStart() + tomlInlineComment(lines[index], assignment.valueStart)
        : lineText;
      return ensureTrailingNewline(lines.join("\n").trimEnd());
    }
  }
  const insertAt = key === "model" ? 0 : rootEnd;
  lines.splice(insertAt, 0, lineText);
  return ensureTrailingNewline(lines.join("\n").trimEnd());
}

function codexRequiresOpenAiAuthFromConfig(contents: string): boolean {
  const provider = rootTomlStringValue(contents, "model_provider");
  const targetSection = provider ? codexProviderSectionName(provider) : "";
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  let currentSection = "";
  let sawProviderSection = false;
  let rootRequiresAuth = false;

  for (const [index, line] of lines.entries()) {
    if (!structural[index]) continue;
    const section = tomlSectionName(line);
    if (section !== null) {
      currentSection = section;
      if (section.startsWith("model_providers.")) sawProviderSection = true;
      continue;
    }
    const match = /^\s*requires_openai_auth\s*=\s*(true|false)\s*(?:#.*)?$/i.exec(line);
    if (match && currentSection === "") rootRequiresAuth = match[1].toLowerCase() === "true";
    if (!match || !currentSection.startsWith("model_providers.")) continue;
    if (targetSection) {
      if (currentSection === targetSection) return match[1].toLowerCase() === "true";
      continue;
    }
    if (match[1].toLowerCase() === "true") return true;
  }

  return !sawProviderSection && rootRequiresAuth;
}

function setCodexProviderStringKey(
  contents: string,
  key: string,
  value: string,
  options: { requiresOpenAiAuth?: boolean } = {},
): string {
  const sessionProvider = rootTomlStringValue(contents, "model_provider") || "custom";
  const provider = sessionProvider === "openai" ? "custom" : sessionProvider;
  let next = contents;
  if (!rootTomlStringValue(next, "model_provider")) {
    next = setRootTomlStringKey(next, "model_provider", sessionProvider);
  }
  next = ensureCodexProviderDefaults(next, provider, { requiresOpenAiAuth: options.requiresOpenAiAuth !== false });
  return setTomlSectionStringKey(next, codexProviderSectionName(provider), key, value);
}

function setCodexExperimentalBearerToken(contents: string, apiKey: string, options: { requiresOpenAiAuth?: boolean } = {}): string {
  const trimmed = apiKey.trim();
  if (trimmed && options.requiresOpenAiAuth === false) {
    // 补纯 API 鉴权时只改目标字段，不覆盖导入模板的名称、协议或登录要求。
    const sessionProvider = rootTomlStringValue(contents, "model_provider") || "custom";
    const provider = sessionProvider === "openai" ? "custom" : sessionProvider;
    const next = rootTomlStringValue(contents, "model_provider")
      ? contents
      : setRootTomlStringKey(contents, "model_provider", sessionProvider);
    return setTomlSectionStringKey(next, codexProviderSectionName(provider), "experimental_bearer_token", trimmed);
  }
  return trimmed
    ? setCodexProviderStringKey(contents, "experimental_bearer_token", trimmed, options)
    : removeCodexExperimentalBearerToken(contents);
}

function removeCodexExperimentalBearerToken(contents: string): string {
  const sessionProvider = rootTomlStringValue(contents, "model_provider") || "custom";
  const provider = sessionProvider === "openai" ? "custom" : sessionProvider;
  return removeTomlSectionKey(contents, codexProviderSectionName(provider), "experimental_bearer_token");
}

function ensureCodexProviderDefaults(
  contents: string,
  provider: string,
  options: { requiresOpenAiAuth?: boolean } = {},
): string {
  let next = contents;
  const section = codexProviderSectionName(provider);
  // 已有非空 name 必须原样保留：name 只是展示标签，用户可能改成 "My relay"，
  // 无条件写回 provider id 会在改 Base URL 等字段时把自定义名覆盖掉（issue #2682）。
  next = setTomlSectionStringKey(next, section, "name", resolveProviderName(contents, provider));
  next = setTomlSectionStringKey(next, section, "wire_api", "responses");
  return options.requiresOpenAiAuth === false ? next : setTomlSectionBoolKey(next, section, "requires_openai_auth", true);
}

function setTomlSectionBoolKey(contents: string, sectionName: string, key: string, value: boolean): string {
  return setTomlSectionRawKey(contents, sectionName, key, value ? "true" : "false");
}

function setTomlSectionStringKey(contents: string, sectionName: string, key: string, value: string): string {
  return setTomlSectionRawKey(contents, sectionName, key, `"${tomlString(value.trim())}"`);
}

function setTomlSectionRawKey(contents: string, sectionName: string, key: string, value: string): string {
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  let sectionStart = -1;
  let sectionEnd = lines.length;
  for (let index = 0; index < lines.length; index += 1) {
    if (!structural[index]) continue;
    const section = tomlSectionName(lines[index]);
    if (section === null) continue;
    if (sectionStart >= 0) {
      sectionEnd = index;
      break;
    }
    if (section === sectionName) sectionStart = index;
  }
  if (sectionStart < 0) {
    const prefix = ensureTrailingNewline(lines.join("\n").trimEnd()).trimEnd();
    return joinTomlSections([prefix, `[${sectionName}]\n${key} = ${value}`]);
  }
  const replacement = `${key} = ${value}`;
  for (let index = sectionStart + 1; index < sectionEnd; index += 1) {
    if (!structural[index]) continue;
    const assignment = tomlKeyAssignment(lines[index]);
    if (assignment?.path.length === 1 && assignment.path[0] === key) {
      lines[index] = lines[index].slice(0, assignment.valueStart) + " " + value + tomlInlineComment(lines[index], assignment.valueStart);
      return ensureTrailingNewline(lines.join("\n").trimEnd());
    }
  }
  let insertAt = sectionEnd;
  while (insertAt > sectionStart + 1 && lines[insertAt - 1].trim() === "") insertAt -= 1;
  lines.splice(insertAt, 0, replacement);
  return ensureTrailingNewline(lines.join("\n").trimEnd());
}

function removeTomlSectionKey(contents: string, sectionName: string, key: string): string {
  const lines = contents.split(/\r?\n/);
  const structural = tomlStructureMask(lines);
  let sectionStart = -1;
  let sectionEnd = lines.length;
  for (let index = 0; index < lines.length; index += 1) {
    if (!structural[index]) continue;
    const section = tomlSectionName(lines[index]);
    if (section === null) continue;
    if (sectionStart >= 0) {
      sectionEnd = index;
      break;
    }
    if (section === sectionName) sectionStart = index;
  }
  if (sectionStart < 0) return contents;
  const next = lines.filter((line, index) => {
    if (index <= sectionStart || index >= sectionEnd || !structural[index]) return true;
    const assignment = tomlKeyAssignment(line);
    return assignment?.path.length !== 1 || assignment.path[0] !== key;
  });
  return ensureTrailingNewline(next.join("\n").trimEnd());
}

function relayProfileSwitchValidation(profile: RelayProfile, settings: BackendSettings | null = null): string | null {
  const sessionProviderError = relaySessionProviderValidation(profile);
  if (sessionProviderError) return sessionProviderError;
  if (isAggregateRelayProfile(profile)) {
    return aggregateRelayProfileValidation(profile);
  }
  const modelRouteError = relayModelRoutesValidation(profile, settings);
  if (modelRouteError) return modelRouteError;
  if (profile.relayMode === "official" && !profile.officialMixApiKey) return null;
  if (!profile.configContents.trim()) {
    return tf("供应商「{0}」缺少独立 config.toml，已停止切换，避免继续显示上一套配置文件。请先在该供应商详情里保存 config.toml。", [profile.name || profile.id]);
  }
  if (profile.relayMode !== "official" || !authJsonHasOpenAiApiKey(profile.authContents)) return null;
  return t("官方混合 API 不应在 auth.json 中保存 OPENAI_API_KEY。请清理此供应商的 auth.json 后再切换。");
}

function relayModelRoutesValidation(profile: RelayProfile, settings: BackendSettings | null): string | null {
  const issue = findRelayModelRouteIssue([profile], settings?.relayProfiles ?? [profile]);
  return relayModelRouteIssueMessage(issue);
}

function relayModelRoutesSettingsValidation(settings: BackendSettings): string | null {
  return relayModelRouteIssueMessage(
    findRelayModelRouteIssue(settings.relayProfiles, settings.relayProfiles),
  );
}

function relaySettingsValidation(settings: BackendSettings): string | null {
  for (const profile of settings.relayProfiles) {
    const sessionProviderError = relaySessionProviderValidation(profile);
    if (sessionProviderError) return sessionProviderError;
  }
  return relayModelRoutesSettingsValidation(settings);
}

function relaySessionProviderValidation(profile: RelayProfile): string | null {
  if (relaySessionProvider(profile) === "openai" && profile.protocol !== "responses") {
    return t("OpenAI 会话身份仅支持 Responses API；Chat Completions 不支持 ChatGPT Remote 的远程压缩。请切换协议或改回 Custom。");
  }
  return null;
}

function relayModelRouteIssueMessage(issue: ReturnType<typeof findRelayModelRouteIssue>): string | null {
  if (!issue) return null;
  switch (issue.kind) {
    case "incomplete":
      return t("单模型协议转换需要填写模型名称和目标供应商。");
    case "duplicate":
      return tf("模型「{0}」存在重复路由。", [issue.model]);
    case "self":
      return tf("模型「{0}」不能路由到当前供应商自身。", [issue.model]);
    case "missingTarget":
      return tf("模型「{0}」的目标供应商不存在。", [issue.model]);
    case "aggregateTarget":
      return tf("模型「{0}」不能路由到聚合供应商。", [issue.model]);
    case "targetProtocol":
      return tf("模型「{0}」的目标供应商必须使用 Responses API。", [issue.model]);
    case "targetCredentials":
      return tf("模型「{0}」的目标供应商缺少 Base URL 或 Key。", [issue.model]);
  }
}

function relaySettingsWithDraft(
  settings: BackendSettings,
  profileId: string,
  draft: RelayProfile,
  isNew: boolean,
): BackendSettings {
  const normalizedDraft = isAggregateRelayProfile(draft)
    ? normalizeAggregateRelayProfile(draft, settings)
    : deriveRelayProfileFromFiles(draft);
  return isNew
    ? addRelayProfile(settings, normalizedDraft)
    : updateRelayProfile(settings, profileId, normalizedDraft);
}

function relayProfileUsesLiveFiles(profile: RelayProfile): boolean {
  return profile.relayMode !== "official" || profile.officialMixApiKey || !!profile.configContents.trim();
}

function authJsonHasOpenAiApiKey(contents: string): boolean {
  const trimmed = contents.trim();
  if (!trimmed) return false;
  try {
    const value = JSON.parse(trimmed);
    return !!value && typeof value === "object" && typeof value.OPENAI_API_KEY === "string" && value.OPENAI_API_KEY.trim().length > 0;
  } catch {
    return /"OPENAI_API_KEY"\s*:/.test(trimmed);
  }
}

function tomlString(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

function syncLegacyRelayFields(settings: BackendSettings): BackendSettings {
  // 先规范化非聚合供应商：聚合成员的"存在性"要用**规范化后仍然在列表里**的那些 id，
  // 否则同一批 profile 里排在聚合后面的成员会被当成不存在而丢掉（顺序相关的老问题）。
  const nonAggregate = settings.relayProfiles
    .filter((profile) => !isAggregateRelayProfile(profile))
    .map((profile) => deriveRelayProfileFromFiles(profile));
  const relayProfiles = settings.relayProfiles.map((profile) =>
    isAggregateRelayProfile(profile)
      ? normalizeAggregateRelayProfile(profile, { ...settings, relayProfiles: [...nonAggregate, ...settings.relayProfiles.filter(isAggregateRelayProfile)] })
      : deriveRelayProfileFromFiles(profile),
  );
  const active = activeRelayProfile({ ...settings, relayProfiles });
  const aggregateRelayProfiles = normalizeAggregateProfilesFromRelayProfiles(relayProfiles);
  const activeAggregateRelayId = isAggregateRelayProfile(active) ? active.id : "";
  return {
    ...settings,
    relayProfiles,
    activeRelayId: active.id,
    relayBaseUrl: isAggregateRelayProfile(active) ? PROTOCOL_PROXY_BASE_URL : active.baseUrl,
    relayApiKey: active.apiKey,
    aggregateRelayProfiles,
    activeAggregateRelayId,
  };
}

function normalizeAggregateProfilesFromRelayProfiles(profiles: RelayProfile[]): AggregateRelayProfile[] {
  const candidates = profiles.filter((profile) => !isAggregateRelayProfile(profile));
  return profiles.filter(isAggregateRelayProfile).map((profile) => {
    const aggregate = normalizeAggregateConfig(profile.aggregate, candidates);
    const memberIds = new Set(aggregate.members.map((member) => member.profileId));
    return {
      id: profile.id,
      name: profile.name || t("聚合供应商"),
      sessionProvider: normalizeRelaySessionProvider(profile.sessionProvider),
      modelAggregationEnabled: aggregate.modelAggregationEnabled === true,
      strategy: aggregate.strategy,
      members: aggregate.members.map((member) => ({
        relayId: member.profileId,
        weight: clampAggregateWeight(member.weight),
      })),
      routes: normalizeAggregateRoutes(aggregate.routes ?? [], { dropEmptyPattern: true, memberIds }).map((route) => ({
        pattern: route.pattern,
        relayId: route.profileId,
        priority: route.priority,
      })),
      defaultRelayId: aggregate.defaultRelayId ?? "",
      syncDefaultMemberLogin: aggregate.syncDefaultMemberLogin === true,
    };
  });
}
function updateRelayProfile(settings: BackendSettings, id: string, patch: Partial<RelayProfile>): BackendSettings {
  if (patch.relayMode === "aggregate" || patch.aggregate) {
    return syncLegacyRelayFields({
      ...settings,
      relayProfiles: settings.relayProfiles.map((profile) =>
        profile.id === id ? normalizeAggregateRelayProfile({ ...profile, ...patch }, settings) : profile,
      ),
    });
  }
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: settings.relayProfiles.map((profile) => {
      if (profile.id !== id) return profile;
      return deriveRelayProfileFromFiles({ ...profile, ...patch });
    }),
  });
}

function createRelayProfile(settings: BackendSettings): RelayProfile {
  const id = `relay-${Date.now().toString(36)}`;
  const contextSelection = contextSelectionForAllEntries(settings);
  const next = {
    id,
    name: tf("供应商 {0}", [settings.relayProfiles.length + 1]),
    model: "",
    baseUrl: defaultSettings.relayBaseUrl,
    upstreamBaseUrl: defaultSettings.relayBaseUrl,
    apiKey: "",
    protocol: "responses" as RelayProtocol,
    relayMode: "official" as RelayMode,
    sessionProvider: "custom" as RelaySessionProvider,
    officialMixApiKey: false,
    hideOfficialUsageAlert: false,
    testModel: "",
    configContents: "",
    authContents: "",
    useCommonConfig: true,
    contextSelection,
    contextSelectionInitialized: true,
    contextWindow: "",
    autoCompactLimit: "",
    modelList: "",
    modelWindows: "",
    modelAutoCompact: "",
    modelMetadata: "",
    modelVlm: "",
    vlmApiKey: "",
    vlmModel: "",
    vlmBaseUrl: "",
    userAgent: "",
    customHeaders: [],
    sub2apiEnabled: false,
    noAuth: false,
    sub2apiMultiplier: "",
    modelRoutes: [],
    standardOpenaiProtocol: false,
    webSearchHistoryCompat: false,
  };
  return withGeneratedRelayFiles(next);
}

function createAggregateRelayProfile(settings: BackendSettings): RelayProfile {
  const id = `aggregate-${Date.now().toString(36)}`;
  const contextSelection = contextSelectionForAllEntries(settings);
  const candidates = aggregateMemberCandidates(settings.relayProfiles, id);
  return normalizeAggregateRelayProfile(
    {
      id,
      name: tf("聚合供应商 {0}", [settings.relayProfiles.filter(isAggregateRelayProfile).length + 1]),
      model: "",
      baseUrl: "",
      upstreamBaseUrl: "",
      apiKey: "",
      protocol: "responses",
      relayMode: "aggregate",
      sessionProvider: "custom",
      officialMixApiKey: false,
      hideOfficialUsageAlert: false,
      testModel: "",
      configContents: "",
      authContents: "",
      useCommonConfig: true,
      contextSelection,
      contextSelectionInitialized: true,
      contextWindow: "",
      autoCompactLimit: "",
      modelList: "",
      modelWindows: "",
      modelAutoCompact: "",
      modelMetadata: "",
      modelVlm: "",
      vlmApiKey: "",
      vlmModel: "",
      vlmBaseUrl: "",
      userAgent: "",
      customHeaders: [],
      sub2apiEnabled: false,
      noAuth: false,
      sub2apiMultiplier: "",
      modelRoutes: [],
      standardOpenaiProtocol: false,
      webSearchHistoryCompat: false,
      aggregate: {
        strategy: "failover",
        modelAggregationEnabled: false,
        members: candidates.slice(0, 1).map((profile) => ({ profileId: profile.id, weight: 1 })),
        defaultRelayId: "",
      },
    },
    settings,
  );
}

function addRelayProfile(settings: BackendSettings, profile: RelayProfile): BackendSettings {
  const nextWithFiles = isAggregateRelayProfile(profile)
    ? normalizeAggregateRelayProfile(profile, settings)
    : deriveRelayProfileFromFiles(
        profile.configContents.trim() || profile.authContents.trim() ? profile : withGeneratedRelayFiles(profile),
      );
  const activeId = settings.relayProfiles.some((item) => item.id === settings.activeRelayId)
    ? settings.activeRelayId
    : activeRelayProfile(settings).id;
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: [...settings.relayProfiles, nextWithFiles],
    activeRelayId: activeId,
  });
}

function duplicateRelayProfile(settings: BackendSettings, id: string): BackendSettings {
  const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === id);
  const source = settings.relayProfiles[sourceIndex] || activeRelayProfile(settings);
  const nextId = `relay-${Date.now().toString(36)}`;
  const next = {
    ...source,
    id: nextId,
    name: tf("{0} 副本", [source.name || t("未命名供应商")]),
  };
  const normalizedNext = isAggregateRelayProfile(next) ? normalizeAggregateRelayProfile(next, settings) : next;
  const relayProfiles = [...settings.relayProfiles];
  relayProfiles.splice(sourceIndex >= 0 ? sourceIndex + 1 : relayProfiles.length, 0, normalizedNext);
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles,
  });
}

function reorderRelayProfiles(settings: BackendSettings, sourceId: string, targetId: string): BackendSettings {
  if (sourceId === targetId) return settings;
  const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === sourceId);
  const targetIndex = settings.relayProfiles.findIndex((profile) => profile.id === targetId);
  if (sourceIndex < 0 || targetIndex < 0) return settings;
  const relayProfiles = [...settings.relayProfiles];
  const [moved] = relayProfiles.splice(sourceIndex, 1);
  relayProfiles.splice(targetIndex, 0, moved);
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles,
  });
}

function removeRelayProfile(settings: BackendSettings, id: string): BackendSettings {
  if (relayProfileDeleteIssue(settings, id)) return settings;
  const profiles = settings.relayProfiles.filter((profile) => profile.id !== id);
  const scrubbedProfiles = profiles.map((profile) =>
    isAggregateRelayProfile(profile)
      ? normalizeAggregateRelayProfile(
          {
            ...profile,
            aggregate: {
              ...normalizeAggregateConfig(profile.aggregate, [], new Set(profiles.map((item) => item.id))),
              // 用户显式删除的这家必须从成员/路由里移除：存在性判定会保留它
              // （profiles 里已经没有了，其实也会被丢掉），这里再做一次明确的过滤。
              members: normalizeAggregateConfig(profile.aggregate, [], new Set(profiles.map((item) => item.id)))
                .members.filter((member) => member.profileId !== id),
              routes: (normalizeAggregateConfig(profile.aggregate, [], new Set(profiles.map((item) => item.id))).routes ?? [])
                .filter((route) => route.profileId !== id),
            },
          },
          { ...settings, relayProfiles: profiles },
        )
      : {
          ...profile,
          modelRoutes: normalizeRelayModelRoutes(profile.modelRoutes).filter((route) => route.targetRelayId !== id),
        },
  );
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: scrubbedProfiles.length ? scrubbedProfiles : defaultSettings.relayProfiles,
    activeRelayId: settings.activeRelayId === id ? scrubbedProfiles[0]?.id || "default" : settings.activeRelayId,
  });
}

function relayProfileDeleteIssue(settings: BackendSettings, id: string): string | null {
  if (settings.activeRelayId === id) {
    return t("当前正在使用该供应商，请先切换到其他供应商后再删除。");
  }
  const existingProfileIds = new Set(settings.relayProfiles.map((profile) => profile.id));
  for (const aggregateProfile of settings.relayProfiles.filter(isAggregateRelayProfile)) {
    const aggregate = normalizeAggregateConfig(aggregateProfile.aggregate, [], existingProfileIds);
    if (!aggregate.members.some((member) => member.profileId === id)) continue;
    if (aggregate.defaultRelayId === id) {
      return tf("供应商「{0}」是聚合配置「{1}」的默认成员，请先调整聚合配置后再删除。", [
        settings.relayProfiles.find((profile) => profile.id === id)?.name || id,
        aggregateProfile.name || aggregateProfile.id,
      ]);
    }
    if (aggregate.members.length <= 1) {
      return tf("供应商「{0}」是聚合配置「{1}」的最后成员，请先调整聚合配置后再删除。", [
        settings.relayProfiles.find((profile) => profile.id === id)?.name || id,
        aggregateProfile.name || aggregateProfile.id,
      ]);
    }
  }
  return null;
}

const aggregateStrategyOptions: Array<{ value: RelayAggregateStrategy; label: string; description: string }> = [
  {
    value: "failover",
    get label() { return t("失败切换"); },
    get description() { return t("按成员顺序请求，失败后切到下一个供应商。"); },
  },
  {
    value: "conversationRoundRobin",
    get label() { return t("按对话轮转"); },
    get description() { return t("同一对话保持一个成员，不同对话依次分配。"); },
  },
  {
    value: "requestRoundRobin",
    get label() { return t("按请求轮转"); },
    get description() { return t("每次请求按成员顺序切换，适合均匀摊请求量。"); },
  },
  {
    value: "weightedRoundRobin",
    get label() { return t("权重轮转"); },
    get description() { return t("按成员权重分配请求，权重越高承担越多。"); },
  },
];

function isAggregateRelayProfile(profile: Pick<RelayProfile, "relayMode" | "aggregate">): boolean {
  return isAggregateProfile(profile);
}

type ProviderViewKind = "direct" | "routing" | "failover" | "modelAggregation";

function relayProviderView(profile: RelayProfile): ProviderViewKind {
  if (isAggregateRelayProfile(profile)) {
    return profile.aggregate?.modelAggregationEnabled === true ? "modelAggregation" : "failover";
  }
  return profile.protocol !== "responses" || normalizeRelayModelRoutes(profile.modelRoutes).length
    ? "routing"
    : "direct";
}

function normalizeAggregateRelayProfile(profile: RelayProfile, settings: BackendSettings | null): RelayProfile {
  const candidates = settings ? aggregateMemberCandidates(settings.relayProfiles, profile.id) : [];
  // 传"全部供应商 id"而不是候选：成员的存在性判定不能受"当前是否就绪"影响。
  const existingProfileIds = settings ? new Set(settings.relayProfiles.map((item) => item.id)) : null;
  const aggregate = normalizeAggregateConfig(profile.aggregate, candidates, existingProfileIds);
  return {
    ...profile,
    baseUrl: "",
    upstreamBaseUrl: "",
    apiKey: "",
    protocol: "responses",
    relayMode: "aggregate",
    sessionProvider: normalizeRelaySessionProvider(profile.sessionProvider),
    officialMixApiKey: false,
    hideOfficialUsageAlert: false,
    configContents: "",
    authContents: "",
    sub2apiEnabled: false,
    noAuth: false,
    sub2apiMultiplier: "",
    standardOpenaiProtocol: false,
    webSearchHistoryCompat: false,
    aggregate,
  };
}

// 创建时选定类型；重新创建草稿时不沿用另一类的路由、权重或测试模型。
function aggregateProfileForMode(profile: RelayProfile, settings: BackendSettings, mode: AggregateMode): RelayProfile {
  const members = (profile.aggregate?.members ?? []).map((member) => ({ profileId: member.profileId, weight: 1 }));
  // 模型聚合总要有一个默认供应商（对标 CC Switch）：沿用仍合法的旧值，否则默认第一个成员。
  const defaultRelayId = mode === "model"
    ? normalizeAggregateDefaultRelayId(
        profile.aggregate?.defaultRelayId,
        new Set(members.map((member) => member.profileId)),
      ) || members[0]?.profileId || ""
    : "";
  return normalizeAggregateRelayProfile({
    ...profile,
    testModel: "",
    aggregate: {
      strategy: "failover",
      modelAggregationEnabled: mode === "model",
      members,
      routes: [],
      defaultRelayId,
    },
  }, settings);
}

function normalizeAggregateConfig(
  aggregate: RelayAggregateConfig | null | undefined,
  candidates: RelayProfile[],
  /**
   * 当前快照里**全部**供应商 id（不只是可勾选候选）。传了才判定成员的存在性；不传
   * （分页子集、拿不到完整快照）则一律保留成员——宁可留着让用户自己取消勾选，
   * 也不能替用户删配置。
   */
  existingProfileIds: ReadonlySet<string> | null = null,
): RelayAggregateConfig {
  const candidateIds = new Set(candidates.map((profile) => profile.id));
  const seen = new Set<string>();
  const strategy: RelayAggregateStrategy =
    aggregate?.strategy && aggregateStrategyOptions.some((option) => option.value === aggregate.strategy)
      ? aggregate.strategy
      : "failover";
  const modelAggregationEnabled = aggregate?.modelAggregationEnabled === true;
  // 成员去留只看"这家供应商还在不在"：候选集合只决定 UI 能勾选什么，不能用来删掉
  // 已经在聚合里的成员（供应商编辑中缺 Key 时判定会翻转，见 normalizeAggregateMembers）。
  const members = normalizeAggregateMembers(aggregate?.members ?? [], existingProfileIds)
    .map((member) => {
      seen.add(member.profileId);
      return { profileId: member.profileId, weight: modelAggregationEnabled ? 1 : clampAggregateWeight(member.weight) };
    });
  // 路由目标同样只按"供应商还在不在"去留：路由早就配好了，目标一时不就绪（或同一批
  // settings 里排在聚合后面）不代表用户想删掉这条路由。
  const routes = modelAggregationEnabled ? [] : (aggregate?.routes ?? [])
    .filter((route) => route.pattern.trim() !== "" || route.profileId.trim() !== "")
    .filter((route) => !existingProfileIds || !route.profileId.trim() || existingProfileIds.has(route.profileId.trim()))
    .map((route) => ({
      pattern: route.pattern.trim(),
      profileId: route.profileId,
      priority: clampAggregateRoutePriority(route.priority),
    }));
  // 默认供应商必须是当前成员；成员被移出（或值不再合法）时自动清空为「未设置」。
  const defaultRelayId = normalizeAggregateDefaultRelayId(
    aggregate?.defaultRelayId,
    new Set(members.map((member) => member.profileId)),
  );
  return {
    strategy: modelAggregationEnabled ? "failover" : strategy,
    modelAggregationEnabled,
    members,
    routes,
    defaultRelayId,
    syncDefaultMemberLogin: aggregate?.syncDefaultMemberLogin === true,
  };
}

function clampAggregateWeight(value: number): number {
  if (!Number.isFinite(value)) return 1;
  return Math.max(1, Math.min(999, Math.round(value)));
}

function aggregateStrategyLabel(strategy: RelayAggregateStrategy): string {
  return aggregateStrategyOptions.find((option) => option.value === strategy)?.label ?? t("失败切换");
}

function aggregateStrategyHelp(strategy: RelayAggregateStrategy): string {
  if (strategy === "failover") return t("失败切换会保留成员顺序，优先使用第一个可用供应商。");
  if (strategy === "conversationRoundRobin") return t("按对话轮转会让同一对话尽量保持固定成员，降低上下文漂移。");
  if (strategy === "requestRoundRobin") return t("按请求轮转会逐请求切换成员，适合供应商能力接近的场景。");
  return t("权重轮转会读取每个成员的权重值，权重越高的成员获得更多请求。");
}

function aggregateRelayProfileValidation(profile: RelayProfile): string | null {
  const aggregate = normalizeAggregateConfig(profile.aggregate, []);
  if (aggregate.members.length < 1) {
    return t("聚合供应商至少需要勾选 1 个可用成员（API 供应商或官方登录账号）。");
  }
  // 模型聚合必须指定默认供应商（不带前缀的请求只有它能接）。
  if (aggregate.modelAggregationEnabled === true) {
    const defaultRelayId = (aggregate.defaultRelayId ?? "").trim();
    const defaultIsMember = !!defaultRelayId
      && aggregate.members.some((member) => member.profileId === defaultRelayId);
    if (!defaultIsMember) {
      return t("模型聚合必须指定「默认供应商」：在成员列表里点某一行的「设为默认」。不带 cpx- 前缀的请求会原样交给它，没设就报错。");
    }
  }
  const issues = validateAggregateRoutes(
    aggregate.routes ?? [],
    new Set(aggregate.members.map((member) => member.profileId)),
  );
  if (!issues) return null;
  const first = issues[0];
  if (first.code === "emptyPattern") {
    return t("路由规则的模型匹配模式不能为空。");
  }
  if (first.code === "invalidPriority") {
    return tf("路由规则「{0}」的优先级必须是大于等于 0 的整数。", [first.pattern]);
  }
  return tf("路由规则「{0}」的目标供应商必须是聚合成员，请先将其勾选为成员。", [first.pattern]);
}

function numberOrDefault(value: string, fallback: number) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function splitLogLines(text: string) {
  return text.trimEnd().split(/\r?\n/).filter((line, index, lines) => line.length > 0 || index < lines.length - 1);
}

function formatTime(value: number) {
  if (!value) return "-";
  return new Date(value).toLocaleString("zh-CN");
}

function formatDuration(startedAtMs: number): string {
  if (!startedAtMs) return "-";
  const elapsed = Date.now() - startedAtMs;
  if (elapsed < 0) return formatTime(startedAtMs);
  const mins = Math.floor(elapsed / 60000);
  if (mins < 1) return t("刚刚启动");
  if (mins < 60) return tf("已运行 {0} 分钟", [mins]);
  const hours = Math.floor(mins / 60);
  const remainMins = mins % 60;
  return tf("已运行 {0} 小时 {1} 分钟", [hours, remainMins]);
}

function stringifyError(error: unknown) {
  if (error instanceof Error) return error.message;
  return String(error);
}

function loadInitialTheme(): Theme {
  if (typeof window === "undefined") return "dark";
  return window.localStorage.getItem("codex-plus-theme") === "light" ? "light" : "dark";
}

function loadInitialRoute(): Route {
  if (typeof window === "undefined") return "overview";
  const params = new URLSearchParams(window.location.search);
  const previewRoute = params.get("previewRoute");
  if (previewRoute && routes.some((item) => item.id === previewRoute)) {
    return previewRoute as Route;
  }
  if (params.get("showUpdate") === "1" || window.location.hash === "#about") {
    return "settings";
  }
  return "overview";
}

/** 首次使用引导：没看过当前版本就自动打开；预览模式（?previewRoute=）不打开。 */
function initialTourOpen(): boolean {
  if (typeof window === "undefined") return false;
  if (new URLSearchParams(window.location.search).has("previewRoute")) return false;
  try {
    return shouldShowTour(window.localStorage.getItem(TOUR_STORAGE_KEY));
  } catch {
    return false;
  }
}

/** 「关于」已经并进「设置」：启动时要看更新，就滚到版本与更新。 */
function initialSettingsFocus(): string | null {
  if (typeof window === "undefined") return null;
  const params = new URLSearchParams(window.location.search);
  return params.get("showUpdate") === "1" || window.location.hash === "#about" ? "app-settings-update" : null;
}
