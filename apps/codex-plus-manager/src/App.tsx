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
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Bell,
  CheckCircle2,
  ClipboardList,
  Code2,
  CircleArrowUp,
  Copy,
  Database,
  Download,
  Edit3,
  GripVertical,
  Info,
  ExternalLink,
  Hammer,
  KeyRound,
  LayoutDashboard,
  Link2,
  MessageCircle,
  FileCode2,
  FileText,
  Moon,
  Network,
  Power,
  PowerOff,
  Plus,
  RefreshCw,
  Rocket,
  Save,
  Settings,
  ShieldCheck,
  Sun,
  Table2,
  TestTube,
  Trash2,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import { ProviderPresetSelector } from "@/components/ProviderPresetSelector";
import type { PresetPatch } from "@/components/ProviderPresetSelector";
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";

import { Badge as UiBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

type Status = "ok" | "failed" | "not_implemented" | "not_checked" | string;

type CommandResult<T> = T & {
  status: Status;
  message: string;
};

type PathState = {
  status: string;
  path: string | null;
};

type LaunchStatus = {
  status: string;
  message: string;
  started_at_ms: number;
  debug_port: number | null;
  helper_port: number | null;
  codex_app: string | null;
};

type OverviewResult = CommandResult<{
  codex_app: PathState;
  codex_version: string | null;
  silent_shortcut: PathState;
  management_shortcut: PathState;
  latest_launch: LaunchStatus | null;
  current_version: string;
  update_status: string;
  settings_path: string;
  logs_path: string;
}>;

type BackendSettings = {
  codexAppPath: string;
  codexExtraArgs: string[];
  providerSyncEnabled: boolean;
  providerSyncSavedProviders: string[];
  providerSyncManualProviders: string[];
  providerSyncLastSelectedProvider: string;
  relayProfilesEnabled: boolean;
  ccsLinkEnabled: boolean;
  configOwnership: ConfigOwnership;
  enhancementsEnabled: boolean;
  codexAppPluginEntryUnlock: boolean;
  codexAppPluginMarketplaceUnlock: boolean;
  codexAppForcePluginInstall: boolean;
  codexAppModelWhitelistUnlock: boolean;
  codexAppSessionDelete: boolean;
  codexAppMarkdownExport: boolean;
  codexAppProjectMove: boolean;
  codexAppConversationTimeline: boolean;
  codexAppConversationView: boolean;
  codexAppThreadScrollRestore: boolean;
  codexAppZedRemoteOpen: boolean;
  zedRemoteOpenStrategy: ZedOpenStrategy;
  zedRemoteProjectRegistryEnabled: boolean;
  zedRemoteSyncToZedSettings: boolean;
  codexAppUpstreamWorktreeCreate: boolean;
  codexAppNativeMenuPlacement: boolean;
  codexAppServiceTierControls: boolean;
  codexGoalsEnabled: boolean;
  launchMode: LaunchMode;
  relayBaseUrl: string;
  relayApiKey: string;
  jiyiLocalProxyEnabled: boolean;
  jiyiLocalUsageMeterEnabled: boolean;
  jiyiDailyTokenLimit: number;
  jiyiIdentitySyncEndpoint: string;
  jiyiIdentitySyncApiKey: string;
  jiyiManagedProxyEnabled: boolean;
  jiyiManagedProxyEndpoint: string;
  relayProfiles: RelayProfile[];
  relayCommonConfigContents: string;
  relayContextConfigContents: string;
  activeRelayId: string;
  relayTestModel: string;
  cliWrapperEnabled: boolean;
  cliWrapperBaseUrl: string;
  cliWrapperApiKey: string;
  cliWrapperApiKeyEnv: string;
};

type ZedOpenStrategy = "addToFocusedWorkspace" | "reuseWindow" | "newWindow" | "default";
type LaunchMode = "patch" | "relay";
type ConfigOwnership = "auto" | "codexPlusPlus" | "ccSwitch";

type RelayProfile = {
  id: string;
  linkedCcsProviderId: string;
  name: string;
  model: string;
  baseUrl: string;
  upstreamBaseUrl: string;
  apiKey: string;
  protocol: RelayProtocol;
  relayMode: RelayMode;
  officialMixApiKey: boolean;
  testModel: string;
  configContents: string;
  authContents: string;
  useCommonConfig: boolean;
  contextSelection: RelayContextSelection;
  contextSelectionInitialized: boolean;
  contextWindow: string;
  autoCompactLimit: string;
  modelList: string;
  userAgent: string;
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

type RelayProtocol = "responses" | "chatCompletions";
type RelayMode = "official" | "mixedApi" | "pureApi";
const PROTOCOL_PROXY_BASE_URL = "http://127.0.0.1:57321/v1";
const CHAT_UPSTREAM_BASE_URL_KEY = "codex_plus_chat_base_url";
const SCRIPT_MARKET_REPOSITORY_URL = "https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket";

const emptyContextSelection = (): RelayContextSelection => ({
  mcpServers: [],
  skills: [],
  plugins: [],
});

type UserScriptInventory = {
  enabled?: boolean;
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

type SettingsResult = CommandResult<{
  settings: BackendSettings;
  settings_path: string;
  user_scripts: UserScriptInventory;
}>;

type RelayResult = CommandResult<{
  authenticated: boolean;
  authSource: string;
  accountLabel: string | null;
  configPath: string;
  configured: boolean;
  requiresOpenaiAuth: boolean;
  hasBearerToken: boolean;
  apiKeyConfigured: boolean;
  apiKeySource: string;
  backupPath: string | null;
}>;

type SmsConfigState = {
  configured: boolean;
  dryRun: boolean;
  region: string;
  secretIdSet: boolean;
  secretKeySet: boolean;
  secretIdSource: string;
  secretKeySource: string;
  appIdSet: boolean;
  signNameSet: boolean;
  templateIdSet: boolean;
  ttlMinutes: number;
  templateParamMode: string;
};

type SmsProviderSettings = {
  region: string;
  appId: string;
  signName: string;
  templateId: string;
  ttlMinutes: number;
  templateParamMode: string;
  dryRun: boolean;
};

type SmsProviderForm = SmsProviderSettings & {
  secretId: string;
  secretKey: string;
};

type SmsProviderSettingsResult = CommandResult<{
  settingsPath: string;
  settings: SmsProviderSettings;
  smsConfig: SmsConfigState;
  secretIdRef: string;
  secretKeyRef: string;
}>;

type LocalEntitlementState = {
  userId: string | null;
  planId: string;
  planName: string;
  dailyTokenLimit: number;
  source: string;
  updatedAtMs: number | null;
};

type LocalAuthResult = CommandResult<{
  authenticated: boolean;
  userId: string | null;
  phone: string | null;
  phoneMasked: string | null;
  loginAtMs: number | null;
  expiresAtMs: number | null;
  deviceId: string | null;
  sessionTtlHours: number;
  sessionExpired: boolean;
  dbPath: string;
  smsConfig: SmsConfigState;
  entitlement: LocalEntitlementState;
}>;

type LocalUsageResult = CommandResult<{
  enabled: boolean;
  dailyTokenLimit: number;
  subjectId: string | null;
  planId: string | null;
  limitSource: string;
  day: string;
  usedTokens: number;
  requestCount: number;
  remainingTokens: number | null;
  dbPath: string;
}>;

type SmsCodeResult = CommandResult<{
  phone: string;
  phoneMasked: string;
  expiresAtMs: number;
  dryRun: boolean;
  devCode: string | null;
  requestId: string | null;
}>;

type LocalLoginResult = CommandResult<{
  userId: string;
  phone: string;
  phoneMasked: string;
  loginAtMs: number;
  expiresAtMs: number;
  deviceId: string;
  sessionTtlHours: number;
  entitlement: LocalEntitlementState;
}>;

type RelayFilesResult = CommandResult<{
  configPath: string;
  authPath: string;
  configContents: string;
  authContents: string;
}>;

type CoordinationStatus = {
  ccswitchDetected: boolean;
  configuredOwnership: ConfigOwnership;
  effectiveOwnership: ConfigOwnership;
  lastWriter: string | null;
  conflictDetected: boolean;
  conflictMessage: string;
  ccswitchCurrentProviderId: string | null;
  ccswitchCurrentProviderName: string | null;
  liveModelProvider: string;
  canWriteLiveConfig: boolean;
  guidance: string;
};

type CoordinationStatusResult = CommandResult<CoordinationStatus>;

type LocalSession = {
  id: string;
  title: string;
  cwd: string;
  modelProvider: string;
  archived: boolean;
  updatedAtMs: number | null;
  rolloutPath: string;
};

type LocalSessionsResult = CommandResult<{
  dbPath: string;
  sessions: LocalSession[];
}>;

type ZedRemoteProject = {
  id: string;
  label: string;
  hostId: string;
  ssh: {
    user: string;
    host: string;
    port: number | null;
  };
  path: string;
  url: string;
  source: "currentThread" | "codexRemoteProject" | "threadWorkspaceHint" | "sqliteThreadCwd" | "recent" | string;
  lastOpenedAtMs: number | null;
  isCurrent: boolean;
};

type ZedRemoteProjectsResult = CommandResult<{
  projects: ZedRemoteProject[];
}>;

type ZedRemoteOpenResult = CommandResult<{
  url: string;
  strategy: ZedOpenStrategy;
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

type LiveContextEntriesResult = CommandResult<{
  entries: CodexContextEntries;
}>;

type ExtractRelayCommonConfigResult = CommandResult<{
  commonConfigContents: string;
  profileConfigContents: string;
}>;

type SettingsBackfillResult = CommandResult<{
  settings: BackendSettings;
}>;

type RelayProfileTestResult = CommandResult<{
  httpStatus: number;
  endpoint: string;
  responsePreview: string;
}>;

type RelayProfileModelsResult = CommandResult<{
  models: string[];
  endpoint: string;
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

type ProviderSyncPayload = {
  syncStatus?: string;
  targetProvider?: string;
  changedSessionFiles?: number;
  skippedLockedRolloutFiles?: string[];
  sqliteRowsUpdated?: number;
  sqliteProviderRowsUpdated?: number;
  sqliteUserEventRowsUpdated?: number;
  sqliteCwdRowsUpdated?: number;
  updatedWorkspaceRoots?: number;
  encryptedContentWarning?: string | null;
};

type ProviderSyncTargetSource = "config" | "rollout" | "sqlite" | "manual";

type ProviderSyncTargetOption = {
  id: string;
  sources: ProviderSyncTargetSource[];
  isCurrentProvider: boolean;
  isManual: boolean;
  isSaved: boolean;
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

type LogsResult = CommandResult<{
  path: string;
  text: string;
  lines: number;
}>;

type DiagnosticsResult = CommandResult<{
  report: string;
}>;

type LocalIdentityExportResult = CommandResult<{
  reportPath: string;
  userCount: number;
  deviceCount: number;
  entitlementCount: number;
  usageSummaryCount: number;
}>;

type IdentitySyncRequestResult = CommandResult<{
  syncRequestPath: string;
  reportPath: string;
  endpoint: string;
  authorization: string;
  userCount: number;
  deviceCount: number;
  entitlementCount: number;
  usageSummaryCount: number;
}>;

type IdentitySyncPostResult = CommandResult<{
  syncRequestPath: string;
  reportPath: string;
  responseAuditPath: string;
  endpoint: string;
  httpStatus: number;
  responsePreview: string;
  userCount: number;
  deviceCount: number;
  entitlementCount: number;
  usageSummaryCount: number;
  backendSessionTokenRef: string | null;
  backendSessionConfigured: boolean;
}>;

type LocalBackendState = {
  dbPath: string;
  initialized: boolean;
  batchCount: number;
  userCount: number;
  blockedUserCount: number;
  deviceCount: number;
  teamCount: number;
  teamMemberCount: number;
  entitlementCount: number;
  billingRenewalCount: number;
  billingPaymentEventCount: number;
  usageSummaryCount: number;
  auditEventCount: number;
  sessionCount: number;
  activeSessionCount: number;
  revokedSessionCount: number;
  lastSyncedAtMs: number | null;
  lastAuditEventAtMs: number | null;
  lastBillingRenewalAtMs: number | null;
  lastBillingPaymentEventAtMs: number | null;
  lastUserAccessUpdatedAtMs: number | null;
  lastSessionIssuedAtMs: number | null;
  lastSessionRevokedAtMs: number | null;
};

type LocalBackendStateResult = CommandResult<LocalBackendState>;

type AdminUserOverview = {
  userId: string;
  phoneMasked: string;
  accessStatus: string;
  accessReason: string | null;
  planId: string | null;
  planName: string | null;
  dailyTokenLimit: number | null;
  deviceCount: number;
  sessionCount: number;
  activeSessionCount: number;
  revokedSessionCount: number;
  todayRequestCount: number;
  todayUsedTokens: number;
  todayRemainingTokens: number | null;
  lastLoginAtMs: number;
  lastSyncedAtMs: number;
  lastUsageAtMs: number | null;
  lastSessionIssuedAtMs: number | null;
  lastSessionSeenAtMs: number | null;
};

type AdminTeamOverview = {
  teamId: string;
  teamName: string;
  planId: string;
  planName: string;
  dailyTokenLimit: number;
  memberCount: number;
  activeMemberCount: number;
  blockedMemberCount: number;
  todayRequestCount: number;
  todayUsedTokens: number;
  todayRemainingTokens: number | null;
  createdAtMs: number;
  updatedAtMs: number;
  lastMemberUpdatedAtMs: number | null;
};

type AdminBillingRenewal = {
  renewalId: string;
  subjectType: string;
  subjectId: string;
  planId: string;
  planName: string;
  dailyTokenLimit: number;
  previousPlanId: string | null;
  previousDailyTokenLimit: number | null;
  amountCents: number;
  currency: string;
  paymentChannel: string;
  externalOrderId: string | null;
  status: string;
  reason: string | null;
  actorType: string;
  actorId: string | null;
  createdAtMs: number;
};

type AdminAuditEvent = {
  eventId: string;
  eventType: string;
  actorType: string;
  actorId: string | null;
  subjectUserId: string | null;
  subjectDeviceId: string | null;
  reason: string | null;
  metadata: unknown;
  createdAtMs: number;
};

type AdminConsoleResult = CommandResult<{
  state: LocalBackendState;
  users: {
    day: string;
    users: AdminUserOverview[];
  };
  teams: {
    day: string;
    teams: AdminTeamOverview[];
  };
  renewals: {
    renewals: AdminBillingRenewal[];
  };
  auditEvents: AdminAuditEvent[];
}>;

type LocalBackendApplyResult = CommandResult<{
  receipt: {
    backendDbPath: string;
    batchId: string;
    receivedAtMs: number;
    usersUpserted: number;
    devicesUpserted: number;
    teamsUpserted: number;
    teamMembersUpserted: number;
    entitlementsUpserted: number;
    usageSummariesUpserted: number;
    sessionsIssued: number;
    activeSession: {
      userId: string;
      deviceId: string;
      issuedAtMs: number;
      expiresAtMs: number;
    } | null;
    totalUserCount: number;
    totalDeviceCount: number;
    totalTeamCount: number;
    totalTeamMemberCount: number;
    totalEntitlementCount: number;
    totalUsageSummaryCount: number;
    totalSessionCount: number;
  };
  state: LocalBackendState;
  backendSessionTokenRef: string | null;
  backendSessionConfigured: boolean;
}>;

type ManagedProxyRuntimeResult = CommandResult<{
  running: boolean;
  pid: number | null;
  endpoint: string;
  listenAddr: string;
  binaryPath: string;
  pidPath: string;
  logPath: string;
  healthChecked: boolean;
  healthHttpStatus: number | null;
  healthStatus: string;
  upstreamBaseUrl: string;
  backendDbPath: string;
  upstreamKeyConfigured: boolean;
  identitySyncKeyConfigured: boolean;
  adminKeyConfigured: boolean;
  userReadKeyConfigured: boolean;
  billingKeyConfigured: boolean;
  paymentWebhookKeyConfigured: boolean;
  paymentWebhookSignatureConfigured: boolean;
  paymentWebhookAlipaySignatureConfigured: boolean;
  paymentWebhookWechatpaySignatureConfigured: boolean;
  accessKeyConfigured: boolean;
  auditKeyConfigured: boolean;
}>;

type WatcherResult = CommandResult<{
  enabled: boolean;
  disabled_flag: string;
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

type ReleaseReadinessItem = {
  id: string;
  label: string;
  status: Status;
  message: string;
  path: string | null;
};

type ReleaseReadinessResult = CommandResult<{
  ready: boolean;
  failures: number;
  warnings: number;
  checkedAtMs: number;
  items: ReleaseReadinessItem[];
}>;

type OfficialIsolationRepairResult = CommandResult<{
  officialHome: string;
  appSupportPaths: string[];
  backupDir: string | null;
  scannedFiles: string[];
  repairedFiles: string[];
  remainingContaminatedFiles: string[];
}>;

type AdItem = {
  id?: string;
  type: "sponsor" | "normal" | string;
  title: string;
  description: string;
  url: string;
  highlights?: string[];
  expires_at?: string;
};

type AdsResult = CommandResult<{
  version: number;
  ads: AdItem[];
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
  sha256: string;
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
  const target = result.targetProvider || "当前 provider";
  const skipped = result.skippedLockedRolloutFiles?.length ?? 0;
  const skippedText = skipped ? `，跳过 ${skipped} 个占用文件` : "";
  return `已同步到 ${target}：修复 ${changed} 个会话文件，更新 ${rows} 行索引${skippedText}。`;
}

const providerSyncSourceLabels: Record<ProviderSyncTargetSource, string> = {
  config: "配置",
  rollout: "会话",
  sqlite: "索引",
  manual: "手动",
};

function providerSyncTargetLabel(target: ProviderSyncTargetOption): string {
  const labels = target.sources.map((source) => providerSyncSourceLabels[source]).filter(Boolean);
  const current = target.isCurrentProvider ? ["当前"] : [];
  return [...labels, ...current].join(" / ") || "发现";
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
  appMode: AppMode;
}>;

type AppMode = "main" | "manager";
type Route = "overview" | "admin" | "relay" | "sessions" | "context" | "enhance" | "zedRemote" | "userScripts" | "recommendations" | "maintenance" | "about" | "settings";
type Theme = "dark" | "light";

const PRODUCT_NAME = "极义codex";
const BAILIAN_BASE_URL = "https://dashscope.aliyuncs.com/compatible-mode/v1";
const APIMART_FALLBACK_BASE_URL = "https://apimart.ai/";
const QWEN_DEFAULT_MODEL = "qwen3.7-plus";
const DEFAULT_RELAY_PROVIDER_NAME = "阿里百炼默认";

const routes: Array<{ id: Route; label: string; icon: LucideIcon }> = [
  { id: "overview", label: "工作台", icon: LayoutDashboard },
  { id: "admin", label: "总后台", icon: ShieldCheck },
  { id: "relay", label: "供应商配置", icon: KeyRound },
  { id: "sessions", label: "会话管理", icon: MessageCircle },
  { id: "context", label: "工具与插件", icon: Network },
  { id: "enhance", label: "页面增强", icon: Hammer },
  { id: "zedRemote", label: "Zed 远程项目", icon: ExternalLink },
  { id: "userScripts", label: "脚本市场", icon: FileCode2 },
  { id: "recommendations", label: "推荐内容", icon: ExternalLink },
  { id: "maintenance", label: "安装维护", icon: Wrench },
  { id: "about", label: "关于", icon: Info },
  { id: "settings", label: "设置", icon: Settings },
];

const scenarioWorkflows: Array<{
  title: string;
  summary: string;
  deliverable: string;
  steps: string[];
  icon: LucideIcon;
  route: Route;
}> = [
  {
    title: "AI Native 项目办公室",
    summary: "建立工作区、上下文和边界，先让 Agent 看懂项目再开工。",
    deliverable: "项目结构说明、风险边界和下一步任务清单",
    steps: ["选工作区", "读取资料", "确认边界", "沉淀 README"],
    icon: ClipboardList,
    route: "settings",
  },
  {
    title: "聊天记录整理",
    summary: "把微信、飞书或会议文本整理成摘要、待办和责任人。",
    deliverable: "结构化总结、待办列表和跟进建议",
    steps: ["导入文本", "识别主题", "提取待办", "生成复盘"],
    icon: MessageCircle,
    route: "userScripts",
  },
  {
    title: "PPT / 汇报稿",
    summary: "从主题或资料夹生成汇报大纲、页面结构和逐页文案。",
    deliverable: "PPT 大纲、页面脚本和素材清单",
    steps: ["确认受众", "梳理材料", "生成大纲", "输出逐页稿"],
    icon: FileText,
    route: "context",
  },
  {
    title: "表格清洗与分析",
    summary: "处理 CSV / Excel，输出清洗建议、统计摘要和结果文件。",
    deliverable: "清洗结果、指标摘要和可导出表格",
    steps: ["读取表格", "识别字段", "清洗数据", "输出摘要"],
    icon: Table2,
    route: "userScripts",
  },
  {
    title: "开发任务最小闭环",
    summary: "按读项目、提方案、小改动、跑测试、写总结推进开发。",
    deliverable: "可验证代码改动、测试结果和变更说明",
    steps: ["读代码", "定方案", "改文件", "跑验证"],
    icon: Code2,
    route: "sessions",
  },
];

const presetCapabilities: Array<{
  name: string;
  type: "插件" | "Skill" | "用户脚本";
  summary: string;
  route: Route;
  icon: LucideIcon;
}> = [
  {
    name: "Browser / Playwright",
    type: "插件",
    summary: "本地网页打开、截图、端到端验收和前端回归检查。",
    route: "context",
    icon: LayoutDashboard,
  },
  {
    name: "GitHub",
    type: "插件",
    summary: "仓库、Issue、PR、CI 和发布协作。",
    route: "context",
    icon: Code2,
  },
  {
    name: "飞书工作流",
    type: "Skill",
    summary: "文档、表格、会议纪要、任务和即时消息整理。",
    route: "context",
    icon: ClipboardList,
  },
  {
    name: "Documents / Presentations / Spreadsheets",
    type: "Skill",
    summary: "Word、PPT、表格类办公交付物生成与编辑。",
    route: "context",
    icon: FileText,
  },
  {
    name: "用户脚本市场",
    type: "用户脚本",
    summary: "按需安装页面增强、快捷操作和团队私有脚本。",
    route: "userScripts",
    icon: FileCode2,
  },
  {
    name: "会话管理与导出",
    type: "用户脚本",
    summary: "删除、导出、迁移、整理本地 Codex 会话。",
    route: "sessions",
    icon: MessageCircle,
  },
];

const defaultSettings: BackendSettings = {
  codexAppPath: "",
  codexExtraArgs: [],
  providerSyncEnabled: false,
  providerSyncSavedProviders: [],
  providerSyncManualProviders: [],
  providerSyncLastSelectedProvider: "",
  relayProfilesEnabled: true,
  ccsLinkEnabled: false,
  configOwnership: "auto",
  enhancementsEnabled: true,
  codexAppPluginEntryUnlock: true,
  codexAppPluginMarketplaceUnlock: true,
  codexAppForcePluginInstall: true,
  codexAppModelWhitelistUnlock: true,
  codexAppSessionDelete: true,
  codexAppMarkdownExport: true,
  codexAppProjectMove: true,
  codexAppConversationTimeline: true,
  codexAppConversationView: false,
  codexAppThreadScrollRestore: true,
  codexAppZedRemoteOpen: true,
  zedRemoteOpenStrategy: "addToFocusedWorkspace",
  zedRemoteProjectRegistryEnabled: true,
  zedRemoteSyncToZedSettings: false,
  codexAppUpstreamWorktreeCreate: true,
  codexAppNativeMenuPlacement: true,
  codexAppServiceTierControls: false,
  codexGoalsEnabled: false,
  launchMode: "patch",
  relayBaseUrl: BAILIAN_BASE_URL,
  relayApiKey: "",
  jiyiLocalProxyEnabled: true,
  jiyiLocalUsageMeterEnabled: true,
  jiyiDailyTokenLimit: 0,
  jiyiIdentitySyncEndpoint: "",
  jiyiIdentitySyncApiKey: "",
  jiyiManagedProxyEnabled: false,
  jiyiManagedProxyEndpoint: "",
  relayProfiles: [
    {
      id: "default",
      linkedCcsProviderId: "",
      name: DEFAULT_RELAY_PROVIDER_NAME,
      model: QWEN_DEFAULT_MODEL,
      baseUrl: BAILIAN_BASE_URL,
      upstreamBaseUrl: BAILIAN_BASE_URL,
      apiKey: "",
      protocol: "chatCompletions",
      relayMode: "pureApi",
      officialMixApiKey: false,
      testModel: "",
      configContents: `model = "${QWEN_DEFAULT_MODEL}"
model_provider = "bailian"

[model_providers.bailian]
name = "阿里百炼"
wire_api = "chat"
requires_openai_auth = true
base_url = "${BAILIAN_BASE_URL}"
`,
      authContents: "",
      useCommonConfig: true,
      contextSelection: emptyContextSelection(),
      contextSelectionInitialized: true,
      contextWindow: "",
      autoCompactLimit: "",
      modelList: "",
      userAgent: "",
    },
  ],
  relayCommonConfigContents: "",
  relayContextConfigContents: "",
  activeRelayId: "default",
  relayTestModel: QWEN_DEFAULT_MODEL,
  cliWrapperEnabled: false,
  cliWrapperBaseUrl: "",
  cliWrapperApiKey: "",
  cliWrapperApiKeyEnv: "CUSTOM_OPENAI_API_KEY",
};

function hasTauriRuntime() {
  return typeof window !== "undefined" && Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
}

function previewCommandResult<T>(command: string, args?: Record<string, unknown>): T {
  const base = { status: "ok", message: "本地预览数据。" };
  const overview = {
    ...base,
    codex_app: { status: "not_checked", path: null },
    codex_version: null,
    silent_shortcut: { status: "not_checked", path: null },
    management_shortcut: { status: "not_checked", path: null },
    latest_launch: null,
    current_version: "1.2.4",
    update_status: "not_checked",
    settings_path: "~/.codex-session-delete/settings.json",
    logs_path: "~/.codex-session-delete/codex-plus.log",
  };
  const localAuth = {
    ...base,
    authenticated: false,
    userId: null,
    phone: null,
    phoneMasked: null,
    loginAtMs: null,
    expiresAtMs: null,
    deviceId: null,
    sessionTtlHours: 24 * 30,
    sessionExpired: false,
    dbPath: "~/.codex-session-delete/jiyi-codex-local.sqlite",
    entitlement: {
      userId: null,
      planId: "local_trial",
      planName: "本地试用",
      dailyTokenLimit: 0,
      source: "preview",
      updatedAtMs: null,
    },
    smsConfig: {
      configured: false,
      dryRun: true,
      region: "ap-guangzhou",
      secretIdSet: false,
      secretKeySet: false,
      secretIdSource: "missing",
      secretKeySource: "missing",
      appIdSet: false,
      signNameSet: false,
      templateIdSet: false,
      ttlMinutes: 10,
      templateParamMode: "code_ttl",
    },
  };
  const smsProviderSettings = {
    ...base,
    settingsPath: "~/.codex-session-delete/sms-provider.json",
    settings: {
      region: "ap-guangzhou",
      appId: "",
      signName: "",
      templateId: "",
      ttlMinutes: 10,
      templateParamMode: "code_ttl",
      dryRun: true,
    },
    smsConfig: localAuth.smsConfig,
    secretIdRef: "jiyi-keychain:tencent-sms:secret-id",
    secretKeyRef: "jiyi-keychain:tencent-sms:secret-key",
  };
  const localUsage = {
    ...base,
    enabled: true,
    dailyTokenLimit: 0,
    subjectId: null,
    planId: null,
    limitSource: "unlimited",
    day: "preview",
    usedTokens: 0,
    requestCount: 0,
    remainingTokens: null,
    dbPath: "~/.codex-session-delete/jiyi-codex-local.sqlite",
  };
  const managedProxyRuntime = {
    ...base,
    message: "本地预览托管代理正在运行。",
    running: true,
    pid: 57421,
    endpoint: "http://127.0.0.1:57421",
    listenAddr: "127.0.0.1:57421",
    binaryPath: "/Applications/极义codex.app/Contents/MacOS/jiyi-managed-proxy",
    pidPath: "~/.codex-session-delete/jiyi-managed-proxy.pid",
    logPath: "~/.codex-session-delete/jiyi-managed-proxy.log",
    healthChecked: true,
    healthHttpStatus: 200,
    healthStatus: "ok",
    upstreamBaseUrl: BAILIAN_BASE_URL,
    backendDbPath: "~/.codex-session-delete/jiyi-codex-local-backend.sqlite",
    upstreamKeyConfigured: true,
    identitySyncKeyConfigured: false,
    adminKeyConfigured: false,
    userReadKeyConfigured: false,
    billingKeyConfigured: false,
    paymentWebhookKeyConfigured: false,
    paymentWebhookSignatureConfigured: false,
    paymentWebhookAlipaySignatureConfigured: false,
    paymentWebhookWechatpaySignatureConfigured: false,
    accessKeyConfigured: false,
    auditKeyConfigured: false,
  };
  const relayStatus = {
    ...base,
    authenticated: false,
    authSource: "preview",
    accountLabel: null,
    configPath: "~/.codex/config.toml",
    configured: false,
    requiresOpenaiAuth: true,
    hasBearerToken: false,
    backupPath: null,
  };
  const emptyContextEntries = { mcpServers: [], skills: [], plugins: [] };
  const adminConsole = {
    ...base,
    message: "本地预览总后台数据已读取。",
    state: {
      dbPath: "~/.codex-session-delete/jiyi-codex-local-backend.sqlite",
      initialized: true,
      batchCount: 1,
      userCount: 1,
      blockedUserCount: 0,
      deviceCount: 1,
      teamCount: 1,
      teamMemberCount: 1,
      entitlementCount: 1,
      billingRenewalCount: 1,
      billingPaymentEventCount: 1,
      usageSummaryCount: 1,
      auditEventCount: 2,
      sessionCount: 1,
      activeSessionCount: 1,
      revokedSessionCount: 0,
      lastSyncedAtMs: Date.now(),
      lastAuditEventAtMs: Date.now(),
      lastBillingRenewalAtMs: Date.now(),
      lastBillingPaymentEventAtMs: Date.now(),
      lastUserAccessUpdatedAtMs: null,
      lastSessionIssuedAtMs: Date.now(),
      lastSessionRevokedAtMs: null,
    },
    users: {
      day: "preview",
      users: [
        {
          userId: "preview-user",
          phoneMasked: "+86 138****5678",
          accessStatus: "active",
          accessReason: null,
          planId: "local_trial",
          planName: "本地试用",
          dailyTokenLimit: 100000,
          deviceCount: 1,
          sessionCount: 1,
          activeSessionCount: 1,
          revokedSessionCount: 0,
          todayRequestCount: 12,
          todayUsedTokens: 26800,
          todayRemainingTokens: 73200,
          lastLoginAtMs: Date.now(),
          lastSyncedAtMs: Date.now(),
          lastUsageAtMs: Date.now(),
          lastSessionIssuedAtMs: Date.now(),
          lastSessionSeenAtMs: Date.now(),
        },
      ],
    },
    teams: {
      day: "preview",
      teams: [
        {
          teamId: "jiyi-default-team",
          teamName: "极义默认团队",
          planId: "team_local_trial",
          planName: "团队本地试用",
          dailyTokenLimit: 500000,
          memberCount: 1,
          activeMemberCount: 1,
          blockedMemberCount: 0,
          todayRequestCount: 12,
          todayUsedTokens: 26800,
          todayRemainingTokens: 473200,
          createdAtMs: Date.now(),
          updatedAtMs: Date.now(),
          lastMemberUpdatedAtMs: Date.now(),
        },
      ],
    },
    renewals: {
      renewals: [
        {
          renewalId: "preview-renewal",
          subjectType: "user",
          subjectId: "preview-user",
          planId: "local_trial",
          planName: "本地试用",
          dailyTokenLimit: 100000,
          previousPlanId: null,
          previousDailyTokenLimit: null,
          amountCents: 9900,
          currency: "CNY",
          paymentChannel: "manual",
          externalOrderId: "preview-order",
          status: "paid",
          reason: "预览续费",
          actorType: "manager_admin_console",
          actorId: null,
          createdAtMs: Date.now(),
        },
      ],
    },
    auditEvents: [
      {
        eventId: "preview-audit",
        eventType: "user_entitlement_updated",
        actorType: "manager_admin_console",
        actorId: null,
        subjectUserId: "preview-user",
        subjectDeviceId: null,
        reason: "预览",
        metadata: { planId: "local_trial" },
        createdAtMs: Date.now(),
      },
    ],
  };
  switch (command) {
    case "startup_options":
      return { ...base, showUpdate: false, appMode: initialAppMode() } as T;
    case "load_overview":
      return overview as T;
    case "load_local_auth_state":
      return localAuth as T;
    case "load_sms_provider_settings":
      return smsProviderSettings as T;
    case "save_sms_provider_settings": {
      const request = args?.request as Partial<SmsProviderForm> | undefined;
      const configured = Boolean(
        (request?.secretId || smsProviderSettings.smsConfig.secretIdSet) &&
          (request?.secretKey || smsProviderSettings.smsConfig.secretKeySet) &&
          request?.appId &&
          request?.signName &&
          request?.templateId,
      );
      return {
        ...smsProviderSettings,
        message: "本地预览短信配置已保存。",
        settings: {
          region: request?.region || "ap-guangzhou",
          appId: request?.appId || "",
          signName: request?.signName || "",
          templateId: request?.templateId || "",
          ttlMinutes: Number(request?.ttlMinutes || 10),
          templateParamMode: request?.templateParamMode || "code_ttl",
          dryRun: request?.dryRun ?? true,
        },
        smsConfig: {
          ...smsProviderSettings.smsConfig,
          configured,
          dryRun: request?.dryRun ?? true,
          secretIdSet: Boolean(request?.secretId) || smsProviderSettings.smsConfig.secretIdSet,
          secretKeySet: Boolean(request?.secretKey) || smsProviderSettings.smsConfig.secretKeySet,
          appIdSet: Boolean(request?.appId),
          signNameSet: Boolean(request?.signName),
          templateIdSet: Boolean(request?.templateId),
          ttlMinutes: Number(request?.ttlMinutes || 10),
          templateParamMode: request?.templateParamMode || "code_ttl",
        },
      } as T;
    }
    case "load_local_usage_state":
      return localUsage as T;
    case "launch_embedded_codex":
      return {
        ...base,
        status: "accepted",
        message: "预览模式已模拟进入 Codex 使用界面。",
        appPath: "/Applications/极义codex.app/Contents/Resources/JiyiCodexClient.app",
        debugPort: 9229,
        helperPort: 57321,
      } as T;
    case "request_local_sms_code":
      return {
        ...base,
        message: "验证码已在本地预览模式生成。",
        phone: String((args?.request as { phone?: string } | undefined)?.phone ?? ""),
        phoneMasked: "+86 138****5678",
        expiresAtMs: Date.now() + 10 * 60 * 1000,
        dryRun: true,
        devCode: "123456",
        requestId: "preview",
      } as T;
    case "login_with_local_sms_code":
      return {
        ...base,
        message: "本地预览登录已模拟。",
        userId: "preview-user",
        phone: String((args?.request as { phone?: string } | undefined)?.phone ?? ""),
        phoneMasked: "+86 138****5678",
        loginAtMs: Date.now(),
        expiresAtMs: Date.now() + 30 * 24 * 60 * 60 * 1000,
        deviceId: "jiyi-device-preview",
        sessionTtlHours: 24 * 30,
        entitlement: {
          userId: "preview-user",
          planId: "local_trial",
          planName: "本地试用",
          dailyTokenLimit: 0,
          source: "preview",
          updatedAtMs: Date.now(),
        },
      } as T;
    case "update_local_entitlement": {
      const request = args?.request as { planId?: string; planName?: string; dailyTokenLimit?: number } | undefined;
      return {
        ...localAuth,
        message: "本地预览套餐已更新。",
        authenticated: true,
        userId: "preview-user",
        phone: "+8613812345678",
        phoneMasked: "+86 138****5678",
        loginAtMs: Date.now(),
        expiresAtMs: Date.now() + 30 * 24 * 60 * 60 * 1000,
        deviceId: "jiyi-device-preview",
        entitlement: {
          userId: "preview-user",
          planId: request?.planId || "local_trial",
          planName: request?.planName || "本地试用",
          dailyTokenLimit: request?.dailyTokenLimit ?? 0,
          source: "preview",
          updatedAtMs: Date.now(),
        },
      } as T;
    }
    case "export_local_identity_report":
      return {
        ...base,
        message: "本地预览账号迁移报告已导出。",
        reportPath: "~/.codex-session-delete/reports/jiyi-local-identity-report-preview.json",
        userCount: 1,
        deviceCount: 1,
        entitlementCount: 1,
        usageSummaryCount: 0,
        backendSessionTokenRef: "jiyi-keychain:local-backend-session:active",
        backendSessionConfigured: true,
      } as T;
    case "prepare_identity_sync_request":
      return {
        ...base,
        message: "本地预览服务端同步请求包已生成。",
        syncRequestPath: "~/.codex-session-delete/reports/jiyi-identity-sync-request-preview.json",
        reportPath: "~/.codex-session-delete/reports/jiyi-local-identity-report-preview.json",
        endpoint: defaultSettings.jiyiIdentitySyncEndpoint || "https://api.example.com/jiyi/codex/identity/sync",
        authorization: "not_configured",
        userCount: 1,
        deviceCount: 1,
        entitlementCount: 1,
        usageSummaryCount: 0,
      } as T;
    case "sync_identity_to_service":
      return {
        ...base,
        message: "本地预览账号数据已同步到服务端。",
        syncRequestPath: "~/.codex-session-delete/reports/jiyi-identity-sync-request-preview.json",
        reportPath: "~/.codex-session-delete/reports/jiyi-local-identity-report-preview.json",
        responseAuditPath: "~/.codex-session-delete/reports/jiyi-identity-sync-response-preview.json",
        endpoint: defaultSettings.jiyiIdentitySyncEndpoint || "https://api.example.com/jiyi/codex/identity/sync",
        httpStatus: 200,
        responsePreview: "{\"ok\":true}",
        userCount: 1,
        deviceCount: 1,
        entitlementCount: 1,
        usageSummaryCount: 0,
      } as T;
    case "load_local_backend_state":
      return {
        ...base,
        message: "本地预览账号服务端状态已读取。",
        dbPath: "~/.codex-session-delete/jiyi-codex-local-backend.sqlite",
        initialized: true,
        batchCount: 1,
        userCount: 1,
        blockedUserCount: 0,
        deviceCount: 1,
        teamCount: 1,
        teamMemberCount: 1,
        entitlementCount: 1,
        billingRenewalCount: 1,
        billingPaymentEventCount: 1,
        usageSummaryCount: 0,
        auditEventCount: 1,
        sessionCount: 1,
        activeSessionCount: 1,
        revokedSessionCount: 0,
        lastSyncedAtMs: Date.now(),
        lastAuditEventAtMs: Date.now(),
        lastBillingRenewalAtMs: Date.now(),
        lastBillingPaymentEventAtMs: Date.now(),
        lastUserAccessUpdatedAtMs: null,
        lastSessionIssuedAtMs: Date.now(),
        lastSessionRevokedAtMs: null,
      } as T;
    case "load_admin_console":
    case "admin_console_set_user_access":
    case "admin_console_update_user_entitlement":
    case "admin_console_update_team_entitlement":
    case "admin_console_record_billing_renewal":
    case "admin_console_reconcile_billing":
      return adminConsole as T;
    case "apply_identity_sync_locally":
      return {
        ...base,
        message: "本地预览账号数据已同步到本地后端。",
        receipt: {
          backendDbPath: "~/.codex-session-delete/jiyi-codex-local-backend.sqlite",
          batchId: "preview-batch",
          receivedAtMs: Date.now(),
          usersUpserted: 1,
          devicesUpserted: 1,
          teamsUpserted: 1,
          teamMembersUpserted: 1,
          entitlementsUpserted: 1,
          usageSummariesUpserted: 0,
          sessionsIssued: 1,
          activeSession: {
            userId: "preview-user",
            deviceId: "jiyi-device-preview",
            issuedAtMs: Date.now(),
            expiresAtMs: Date.now() + 30 * 24 * 60 * 60 * 1000,
          },
          totalUserCount: 1,
          totalDeviceCount: 1,
          totalTeamCount: 1,
          totalTeamMemberCount: 1,
          totalEntitlementCount: 1,
          totalUsageSummaryCount: 0,
          totalSessionCount: 1,
        },
        state: {
          dbPath: "~/.codex-session-delete/jiyi-codex-local-backend.sqlite",
          initialized: true,
          batchCount: 1,
          userCount: 1,
          blockedUserCount: 0,
          deviceCount: 1,
          teamCount: 1,
          teamMemberCount: 1,
          entitlementCount: 1,
          billingRenewalCount: 1,
          billingPaymentEventCount: 1,
          usageSummaryCount: 0,
          auditEventCount: 1,
          sessionCount: 1,
          activeSessionCount: 1,
          revokedSessionCount: 0,
          lastSyncedAtMs: Date.now(),
          lastAuditEventAtMs: Date.now(),
          lastBillingRenewalAtMs: Date.now(),
          lastBillingPaymentEventAtMs: Date.now(),
          lastUserAccessUpdatedAtMs: null,
          lastSessionIssuedAtMs: Date.now(),
          lastSessionRevokedAtMs: null,
        },
        backendSessionTokenRef: "jiyi-keychain:local-backend-session:active",
        backendSessionConfigured: true,
      } as T;
    case "logout_local_auth":
      return localAuth as T;
    case "reset_local_auth_state":
      return localAuth as T;
    case "load_settings":
    case "save_settings":
    case "reset_settings":
    case "repair_backend":
      return { ...base, settings: defaultSettings, settings_path: "~/.codex-session-delete/settings.json", user_scripts: { enabled: true, scripts: [] } } as T;
    case "repair_official_codex_isolation":
      return {
        ...base,
        message: "原版 Codex 未检测到极义写入痕迹。",
        officialHome: "~/.codex",
        appSupportPaths: ["~/Library/Application Support/Codex"],
        backupDir: null,
        scannedFiles: ["~/.codex/config.toml", "~/.codex/auth.json"],
        repairedFiles: [],
        remainingContaminatedFiles: [],
      } as T;
    case "managed_proxy_status":
    case "start_managed_proxy":
      return managedProxyRuntime as T;
    case "stop_managed_proxy":
      return {
        ...managedProxyRuntime,
        message: "本地预览托管代理已停止。",
        running: false,
        pid: null,
        healthHttpStatus: null,
        healthStatus: "unreachable",
      } as T;
    case "relay_status":
    case "apply_relay_injection":
    case "apply_pure_api_injection":
    case "clear_relay_injection":
      return relayStatus as T;
    case "read_relay_files":
      return {
        ...base,
        configPath: "~/.codex/config.toml",
        authPath: "~/.codex/auth.json",
        configContents: defaultSettings.relayProfiles[0]?.configContents ?? "",
        authContents: "",
      } as T;
    case "load_provider_sync_targets":
      return { ...base, currentProvider: "bailian", targets: [] } as T;
    case "refresh_script_market":
      return { ...base, market: { status: "ok", message: "预览模式", indexUrl: "", updatedAt: "", scripts: [] }, user_scripts: { enabled: true, scripts: [] } } as T;
    case "read_live_context_entries":
      return { ...base, entries: emptyContextEntries } as T;
    case "list_context_entries":
      return { ...base, settings: defaultSettings, entries: emptyContextEntries } as T;
    case "list_local_sessions":
      return { ...base, dbPath: "~/.codex/state_5.sqlite", sessions: [] } as T;
    case "list_zed_remote_projects":
      return { ...base, projects: [] } as T;
    case "load_ads":
      return { ...base, version: 0, ads: [] } as T;
    case "load_watcher_state":
      return { ...base, enabled: false, disabled_flag: "~/.codex-session-delete/watcher.disabled" } as T;
    case "check_update":
      return { ...base, currentVersion: "1.2.4", latestVersion: null, updateAvailable: false } as T;
    case "release_readiness":
      return {
        ...base,
        status: "warning",
        message: "发布前检查存在 2 个风险项。",
        ready: false,
        failures: 0,
        warnings: 2,
        checkedAtMs: Date.now(),
        items: [
          {
            id: "official_codex_isolation",
            label: "原版 Codex 配置隔离",
            status: "ok",
            message: "原版 ~/.codex 未检测到极义写入痕迹。",
            path: "~/.codex",
          },
          {
            id: "developer_id_signature",
            label: "Developer ID 签名",
            status: "warning",
            message: "预览环境未检测到 Developer ID 签名。",
            path: "/Applications/极义codex.app",
          },
        ],
      } as T;
    case "read_latest_logs":
      return { ...base, path: "~/.codex-session-delete/codex-plus.log", text: "本地预览暂无日志。", lines: 0 } as T;
    case "copy_diagnostics":
      return { ...base, report: "本地预览诊断报告。" } as T;
    default:
      return { ...base } as T;
  }
}

export function App() {
  const [theme, setTheme] = useState<Theme>(() => loadInitialTheme());
  const [appMode, setAppMode] = useState<AppMode>(() => initialAppMode());
  const [route, setRoute] = useState<Route>(() => loadInitialRoute());
  const [notice, setNotice] = useState<{ title: string; message: string; status?: Status } | null>(null);
  const [overview, setOverview] = useState<OverviewResult | null>(null);
  const [localAuth, setLocalAuth] = useState<LocalAuthResult | null>(null);
  const [smsProvider, setSmsProvider] = useState<SmsProviderSettingsResult | null>(null);
  const [localUsage, setLocalUsage] = useState<LocalUsageResult | null>(null);
  const [localBackend, setLocalBackend] = useState<LocalBackendStateResult | null>(null);
  const [managedProxy, setManagedProxy] = useState<ManagedProxyRuntimeResult | null>(null);
  const [adminConsole, setAdminConsole] = useState<AdminConsoleResult | null>(null);
  const [settings, setSettings] = useState<SettingsResult | null>(null);
  const [relay, setRelay] = useState<RelayResult | null>(null);
  const [relayFiles, setRelayFiles] = useState<RelayFilesResult | null>(null);
  const [localSessions, setLocalSessions] = useState<LocalSessionsResult | null>(null);
  const [zedRemoteProjects, setZedRemoteProjects] = useState<ZedRemoteProjectsResult | null>(null);
  const [liveContextEntries, setLiveContextEntries] = useState<CodexContextEntries | null>(null);
  const [logs, setLogs] = useState<LogsResult | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsResult | null>(null);
  const [watcher, setWatcher] = useState<WatcherResult | null>(null);
  const [update, setUpdate] = useState<UpdateResult | null>(null);
  const [releaseReadiness, setReleaseReadiness] = useState<ReleaseReadinessResult | null>(null);
  const [ads, setAds] = useState<AdsResult | null>(null);
  const [scriptMarket, setScriptMarket] = useState<ScriptMarketResult | null>(null);
  const [launchForm, setLaunchForm] = useState({
    appPath: "",
    debugPort: "9229",
    helperPort: "57321",
  });
  const prevLaunchStatusRef = useRef<string | null>(null);
  const [loginForm, setLoginForm] = useState({
    phone: "",
    code: "",
  });
  const [entitlementForm, setEntitlementForm] = useState({
    planId: "local_trial",
    planName: "本地试用",
    dailyTokenLimit: "0",
  });
  const [smsProviderForm, setSmsProviderForm] = useState<SmsProviderForm>({
    region: "ap-guangzhou",
    appId: "",
    signName: "",
    templateId: "",
    ttlMinutes: 10,
    templateParamMode: "code_ttl",
    dryRun: true,
    secretId: "",
    secretKey: "",
  });
  const [adminUserForm, setAdminUserForm] = useState({
    userId: "",
    planId: "jiyi_pro",
    planName: "极义 Pro",
    dailyTokenLimit: "500000",
    reason: "",
  });
  const [adminTeamForm, setAdminTeamForm] = useState({
    teamId: "jiyi-default-team",
    planId: "team_pro",
    planName: "团队 Pro",
    dailyTokenLimit: "2000000",
    reason: "",
  });
  const [adminRenewalForm, setAdminRenewalForm] = useState({
    subjectType: "user",
    subjectId: "",
    planId: "jiyi_pro",
    planName: "极义 Pro",
    dailyTokenLimit: "500000",
    amountCents: "9900",
    currency: "CNY",
    paymentChannel: "manual",
    externalOrderId: "",
    reason: "",
  });
  const [settingsForm, setSettingsForm] = useState<BackendSettings>({ ...defaultSettings });
  const [providerSyncProgress, setProviderSyncProgress] = useState<ProviderSyncProgress>({
    active: false,
    percent: 0,
    message: "尚未运行历史会话修复。",
    result: null,
  });
  const [providerSyncTargets, setProviderSyncTargets] = useState<ProviderSyncTargetsResult | null>(null);
  const [selectedProviderSyncTarget, setSelectedProviderSyncTarget] = useState("");
  const [removeOwnedData, setRemoveOwnedData] = useState(false);
  const [mainEntryState, setMainEntryState] = useState<{ status: Status; message: string; appPath?: string | null }>({
    status: "not_checked",
    message: "完成手机号验证码登录后，手动进入 Codex 使用界面。",
    appPath: null,
  });

  const call = <T,>(command: string, args?: Record<string, unknown>) =>
    hasTauriRuntime() ? invoke<T>(command, args) : Promise.resolve(previewCommandResult<T>(command, args));

  const logDiagnostic = (event: string, detail: Record<string, unknown> = {}) => {
    void invoke("write_diagnostic_event", { event, detail }).catch(() => {});
  };

  const run = async <T,>(task: () => Promise<T>): Promise<T | null> => {
    try {
      return await task();
    } catch (error) {
      showNotice("调用失败", stringifyError(error), "failed");
      return null;
    }
  };

  const refreshOverview = async (silent = false) => {
    const result = await run(() => call<OverviewResult>("load_overview"));
    if (result) {
      // 崩溃检测：进程从运行状态变为停止/失败 → 弹出通知
      const prev = prevLaunchStatusRef.current;
      const current = result.latest_launch?.status;
      if (prev && prev === "running" && current && (current === "stopped" || current === "failed" || current === "crashed")) {
        showNotice("Codex 意外停止", `进程状态：${current}。是否要重新启动？`, "failed");
      }
      prevLaunchStatusRef.current = current ?? null;
      setOverview(result);
      if (!silent) showResultNotice("概览已检查", result, { silentSuccess: true });
    }
  };

  const refreshLocalAuth = async (silent = false) => {
    const result = await run(() => call<LocalAuthResult>("load_local_auth_state"));
    if (result) {
      setLocalAuth(result);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("本地账号", result, { silentSuccess: true });
    }
    return result;
  };

  const refreshSmsProviderSettings = async (silent = false) => {
    const result = await run(() => call<SmsProviderSettingsResult>("load_sms_provider_settings"));
    if (result) {
      setSmsProvider(result);
      setSmsProviderForm({
        ...result.settings,
        secretId: "",
        secretKey: "",
      });
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("腾讯云短信", result, { silentSuccess: true });
    }
    return result;
  };

  const refreshLocalUsage = async (silent = false) => {
    const result = await run(() => call<LocalUsageResult>("load_local_usage_state"));
    if (result) {
      setLocalUsage(result);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("本地用量", result, { silentSuccess: true });
    }
    return result;
  };

  const saveSmsProviderSettings = async () => {
    const result = await run(() =>
      call<SmsProviderSettingsResult>("save_sms_provider_settings", {
        request: {
          ...smsProviderForm,
          ttlMinutes: numberOrDefault(String(smsProviderForm.ttlMinutes), 10),
        },
      }),
    );
    if (result) {
      setSmsProvider(result);
      setSmsProviderForm({
        ...result.settings,
        secretId: "",
        secretKey: "",
      });
      showNotice("腾讯云短信", result.message, result.status);
      await refreshLocalAuth(true);
    }
  };

  const refreshLocalBackendState = async (silent = false) => {
    const result = await run(() => call<LocalBackendStateResult>("load_local_backend_state"));
    if (result) {
      setLocalBackend(result);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("本地账号服务端", result, { silentSuccess: true });
    }
    return result;
  };

  const refreshManagedProxy = async (silent = false) => {
    const result = await run(() => call<ManagedProxyRuntimeResult>("managed_proxy_status"));
    if (result) {
      setManagedProxy(result);
      if (!silent || result.status === "failed") showResultNotice("本地托管代理", result, { silentSuccess: true });
    }
    return result;
  };

  const refreshAdminConsole = async (silent = false) => {
    const result = await run(() =>
      call<AdminConsoleResult>("load_admin_console", {
        request: {
          limit: 50,
          eventType: "",
          actorType: "",
          subjectUserId: "",
        },
      }),
    );
    if (result) {
      setAdminConsole(result);
      setLocalBackend({ status: result.status, message: result.message, ...result.state });
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("总后台", result, { silentSuccess: true });
    }
    return result;
  };

  const applyAdminConsoleResult = async (title: string, result: AdminConsoleResult | null) => {
    if (!result) return;
    setAdminConsole(result);
    setLocalBackend({ status: result.status, message: result.message, ...result.state });
    showNotice(title, result.message, result.status);
  };

  const updateAdminUserEntitlement = async () => {
    const userId = adminUserForm.userId.trim();
    if (!userId) {
      showNotice("总后台", "请先选择或填写用户 ID。", "failed");
      return;
    }
    const result = await run(() =>
      call<AdminConsoleResult>("admin_console_update_user_entitlement", {
        request: {
          userId,
          planId: adminUserForm.planId.trim(),
          planName: adminUserForm.planName.trim(),
          dailyTokenLimit: numberOrDefault(adminUserForm.dailyTokenLimit, 0),
          reason: adminUserForm.reason.trim(),
        },
      }),
    );
    await applyAdminConsoleResult("用户套餐", result);
  };

  const updateAdminTeamEntitlement = async () => {
    const teamId = adminTeamForm.teamId.trim();
    if (!teamId) {
      showNotice("总后台", "请先选择或填写团队 ID。", "failed");
      return;
    }
    const result = await run(() =>
      call<AdminConsoleResult>("admin_console_update_team_entitlement", {
        request: {
          teamId,
          planId: adminTeamForm.planId.trim(),
          planName: adminTeamForm.planName.trim(),
          dailyTokenLimit: numberOrDefault(adminTeamForm.dailyTokenLimit, 0),
          reason: adminTeamForm.reason.trim(),
        },
      }),
    );
    await applyAdminConsoleResult("团队套餐", result);
  };

  const setAdminUserAccess = async (userId: string, status: "active" | "blocked") => {
    const normalizedUserId = userId.trim();
    if (!normalizedUserId) {
      showNotice("总后台", "请先选择用户。", "failed");
      return;
    }
    const result = await run(() =>
      call<AdminConsoleResult>("admin_console_set_user_access", {
        request: {
          userId: normalizedUserId,
          status,
          reason: adminUserForm.reason.trim() || (status === "blocked" ? "总后台封禁" : ""),
        },
      }),
    );
    await applyAdminConsoleResult(status === "blocked" ? "封禁用户" : "解封用户", result);
  };

  const recordAdminBillingRenewal = async () => {
    const subjectId = adminRenewalForm.subjectId.trim();
    if (!subjectId) {
      showNotice("总后台", "请先填写续费主体 ID。", "failed");
      return;
    }
    const result = await run(() =>
      call<AdminConsoleResult>("admin_console_record_billing_renewal", {
        request: {
          subjectType: adminRenewalForm.subjectType,
          subjectId,
          planId: adminRenewalForm.planId.trim(),
          planName: adminRenewalForm.planName.trim(),
          dailyTokenLimit: numberOrDefault(adminRenewalForm.dailyTokenLimit, 0),
          amountCents: numberOrDefault(adminRenewalForm.amountCents, 0),
          currency: adminRenewalForm.currency.trim() || "CNY",
          paymentChannel: adminRenewalForm.paymentChannel.trim() || "manual",
          externalOrderId: adminRenewalForm.externalOrderId.trim(),
          reason: adminRenewalForm.reason.trim(),
        },
      }),
    );
    await applyAdminConsoleResult("续费落账", result);
  };

  const reconcileAdminBilling = async () => {
    const result = await run(() => call<AdminConsoleResult>("admin_console_reconcile_billing"));
    await applyAdminConsoleResult("支付对账", result);
  };

  const startManagedProxy = async () => {
    const result = await run(() => call<ManagedProxyRuntimeResult>("start_managed_proxy"));
    if (result) {
      setManagedProxy(result);
      showNotice("本地托管代理", result.message, result.status);
      await refreshSettings(true);
    }
  };

  const stopManagedProxy = async () => {
    const result = await run(() => call<ManagedProxyRuntimeResult>("stop_managed_proxy"));
    if (result) {
      setManagedProxy(result);
      showNotice("本地托管代理", result.message, result.status);
    }
  };

  const requestLocalSmsCode = async () => {
    const phone = loginForm.phone.trim();
    if (!phone) {
      showNotice("手机号登录", "请先填写手机号。", "failed");
      return;
    }
    const result = await run(() => call<SmsCodeResult>("request_local_sms_code", { request: { phone } }));
    if (result) {
      const devCodeText = result.devCode ? ` 本地验证码：${result.devCode}` : "";
      if (result.devCode) {
        setLoginForm((current) => ({ ...current, phone: result.phone || current.phone, code: result.devCode || current.code }));
      }
      showNotice("验证码", `${result.message}${devCodeText}`, result.status);
      await refreshLocalAuth(true);
    }
  };

  const enterCodex = async () => {
    setMainEntryState({
      status: "not_checked",
      message: "正在进入 Codex 使用界面…",
      appPath: null,
    });
    const result = await run(() =>
      call<CommandResult<{ appPath: string | null; debugPort: number; helperPort: number }>>("launch_embedded_codex", {
        request: {
          appPath: "",
          debugPort: numberOrDefault(launchForm.debugPort, 9229),
          helperPort: numberOrDefault(launchForm.helperPort, 57321),
        },
      }),
    );
    if (result) {
      setMainEntryState({
        status: result.status,
        message: result.message,
        appPath: result.appPath,
      });
      showNotice("极义codex", result.message, result.status);
    }
    return result;
  };

  const loginWithLocalSmsCode = async () => {
    const phone = loginForm.phone.trim();
    const code = loginForm.code.trim();
    if (!phone || !code) {
      showNotice("手机号登录", "请填写手机号和验证码。", "failed");
      return;
    }
    const result = await run(() => call<LocalLoginResult>("login_with_local_sms_code", { request: { phone, code } }));
    if (result) {
      showNotice("手机号登录", result.message, result.status);
      setLoginForm((current) => ({ ...current, code: "" }));
      const auth = await refreshLocalAuth(true);
      if (appMode === "main" && isSuccessStatus(result.status) && auth?.authenticated) {
        setMainEntryState({
          status: "accepted",
          message: "手机号已验证，请点击进入 Codex。",
          appPath: null,
        });
      }
    }
  };

  const logoutLocalAuth = async () => {
    const result = await run(() => call<LocalAuthResult>("logout_local_auth"));
    if (result) {
      setLocalAuth(result);
      showNotice("本地账号", result.message, result.status);
    }
  };

  const resetLocalAuthState = async () => {
    const result = await run(() => call<LocalAuthResult>("reset_local_auth_state"));
    if (result) {
      setLocalAuth(result);
      setLoginForm({ phone: "", code: "" });
      showNotice("本地账号", result.message, result.status);
      await refreshLocalUsage(true);
      await refreshLocalBackendState(true);
    }
  };

  const updateLocalEntitlement = async () => {
    if (!localAuth?.authenticated) {
      showNotice("本地套餐", "请先完成手机号验证码登录。", "failed");
      return;
    }
    const planId = entitlementForm.planId.trim();
    const planName = entitlementForm.planName.trim();
    const dailyTokenLimit = numberOrDefault(entitlementForm.dailyTokenLimit, 0);
    const result = await run(() =>
      call<LocalAuthResult>("update_local_entitlement", {
        request: {
          planId,
          planName,
          dailyTokenLimit,
        },
      }),
    );
    if (result) {
      setLocalAuth(result);
      showNotice("本地套餐", result.message, result.status);
      await refreshLocalUsage(true);
    }
  };

  const exportLocalIdentityReport = async () => {
    const result = await run(() => call<LocalIdentityExportResult>("export_local_identity_report"));
    if (result) {
      showNotice(
        "账号迁移报告",
        `${result.message} 用户 ${result.userCount} 个，设备 ${result.deviceCount} 个，套餐 ${result.entitlementCount} 个，用量分组 ${result.usageSummaryCount} 个。路径：${result.reportPath}`,
        result.status,
      );
    }
  };

  const saveIdentitySyncSettings = async () => {
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (settingsResult) {
      setSettings(settingsResult);
      setSettingsForm(normalizeSettings(settingsResult.settings));
      if (!isSuccessStatus(settingsResult.status)) {
        showResultNotice("保存服务端同步配置", settingsResult);
        return false;
      }
    }
    return true;
  };

  const prepareIdentitySyncRequest = async () => {
    if (!(await saveIdentitySyncSettings())) return;
    const result = await run(() => call<IdentitySyncRequestResult>("prepare_identity_sync_request"));
    if (result) {
      showNotice(
        "服务端同步请求包",
        `${result.message} Endpoint：${result.endpoint || "未配置"}，授权：${result.authorization}。路径：${result.syncRequestPath}`,
        result.status,
      );
    }
  };

  const syncIdentityToService = async () => {
    if (!(await saveIdentitySyncSettings())) return;
    const result = await run(() => call<IdentitySyncPostResult>("sync_identity_to_service"));
    if (result) {
      showNotice(
        "服务端同步",
        `${result.message} HTTP ${result.httpStatus || "-"}。${result.backendSessionConfigured ? "服务端 token 已写入极义 Keychain。" : "服务端未返回可用 token。"}响应审计：${result.responseAuditPath || "未生成"}`,
        result.status,
      );
    }
  };

  const applyIdentitySyncLocally = async () => {
    const result = await run(() => call<LocalBackendApplyResult>("apply_identity_sync_locally"));
    if (result) {
      setLocalBackend({
        status: result.status,
        message: result.message,
        ...result.state,
      });
      showNotice(
        "本地账号服务端",
        `${result.message} 用户 ${result.receipt.usersUpserted} 个，设备 ${result.receipt.devicesUpserted} 个，团队 ${result.receipt.teamsUpserted} 个，团队成员 ${result.receipt.teamMembersUpserted} 个，套餐 ${result.receipt.entitlementsUpserted} 个，用量分组 ${result.receipt.usageSummariesUpserted} 个，服务端 session ${result.receipt.sessionsIssued} 个。${result.backendSessionConfigured ? "Token 已写入极义 Keychain。" : "当前无有效登录态，未签发服务端 token。"}`,
        result.status,
      );
    }
  };

  const refreshSettings = async (silent = false) => {
    const result = await run(() => call<SettingsResult>("load_settings"));
    if (result) {
      setSettings(result);
      const normalized = normalizeSettings(result.settings);
      setSettingsForm(normalized);
      setLaunchForm((current) => ({
        ...current,
        appPath: current.appPath || result.settings.codexAppPath || "",
      }));
      if (!silent) showResultNotice("设置已加载", result, { silentSuccess: true });
      return normalized;
    }
    return null;
  };

  const refreshScriptMarket = async (silent = false) => {
    const result = await run(() => call<ScriptMarketResult>("refresh_script_market"));
    if (result) {
      setScriptMarket(result);
      setSettings((current) => (current ? { ...current, user_scripts: result.user_scripts } : current));
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("脚本市场", result, { silentSuccess: true });
    }
  };

  const installMarketScript = async (id: string) => {
    const result = await run(() => call<ScriptMarketResult>("install_market_script", { id }));
    if (result) {
      setScriptMarket(result);
      setSettings((current) => (current ? { ...current, user_scripts: result.user_scripts } : current));
      showResultNotice("脚本市场", result);
    }
  };

  const setUserScriptEnabled = async (key: string, enabled: boolean) => {
    const result = await run(() => call<SettingsResult>("set_user_script_enabled", { key, enabled }));
    if (result) {
      setSettings(result);
      setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
      showResultNotice("本地脚本", result);
    }
  };

  const deleteUserScript = async (key: string) => {
    const script = settings?.user_scripts?.scripts?.find((item) => item.key === key);
    const name = script?.name || key;
    if (!window.confirm(`删除脚本“${name}”？此操作会移除本地脚本文件。`)) return;
    const result = await run(() => call<SettingsResult>("delete_user_script", { key }));
    if (result) {
      setSettings(result);
      setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
      showResultNotice("本地脚本", result);
    }
  };

  const refreshRelay = async (silent = false) => {
    const result = await run(() => call<RelayResult>("relay_status"));
    if (result) {
      setRelay(result);
      if (!silent) showResultNotice("登录状态", result, { silentSuccess: true });
    }
  };

  const refreshRelayFiles = async (silent = false) => {
    const result = await run(() => call<RelayFilesResult>("read_relay_files"));
    if (result) {
      setRelayFiles(result);
      if (!silent) showResultNotice("配置文件", result, { silentSuccess: true });
    }
    return result;
  };

  const refreshLocalSessions = async (silent = false) => {
    const result = await run(() => call<LocalSessionsResult>("list_local_sessions"));
    if (result) {
      setLocalSessions(result);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("会话管理", result, { silentSuccess: true });
    }
    return result;
  };

  const refreshZedRemoteProjects = async (silent = false) => {
    const result = await run(() => call<ZedRemoteProjectsResult>("list_zed_remote_projects"));
    if (result) {
      setZedRemoteProjects(result);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("Zed 远程项目", result, { silentSuccess: true });
    }
    return result;
  };

  const openZedRemoteProject = async (
    project: ZedRemoteProject,
    strategy: ZedOpenStrategy = settingsForm.zedRemoteOpenStrategy || "addToFocusedWorkspace",
  ) => {
    const result = await run(() =>
      call<ZedRemoteOpenResult>("open_zed_remote", {
        payload: {
          ssh: project.ssh,
          hostId: project.hostId,
          path: project.path,
          strategy,
          remember: settingsForm.zedRemoteProjectRegistryEnabled !== false,
        },
      }),
    );
    if (result) {
      showResultNotice("Zed 远程打开", result);
      await refreshZedRemoteProjects(true);
    }
  };

  const forgetZedRemoteProject = async (project: ZedRemoteProject) => {
    const result = await run(() => call<ZedRemoteProjectsResult>("forget_zed_remote_project", { id: project.id }));
    if (result) {
      setZedRemoteProjects(result);
      showResultNotice("Zed 远程项目", result);
    }
  };

  const deleteLocalSession = async (session: LocalSession) => {
    const title = session.title || session.id;
    if (!window.confirm(`删除会话“${title}”？此操作会删除本地数据库记录和 rollout 文件，并创建备份。`)) return;
    const result = await run(() =>
      call<DeleteLocalSessionResult>("delete_local_session", {
        request: { sessionId: session.id, title: session.title },
      }),
    );
    if (result) {
      showResultNotice("会话删除", result);
      await refreshLocalSessions(true);
    }
  };

  const refreshLiveContextEntries = async (silent = false) => {
    const result = await run(() => call<LiveContextEntriesResult>("read_live_context_entries"));
    if (result) {
      setLiveContextEntries(result.entries);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("工具与插件", result, { silentSuccess: true });
    }
    return result;
  };

  const syncLiveContextEntries = async (next: BackendSettings, silent = false) => {
    const result = await run(() => call<LiveContextEntriesResult>("sync_live_context_entries", { request: { settings: next } }));
    if (result) {
      setLiveContextEntries(result.entries);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("工具与插件", result, { silentSuccess: true });
    }
    return result;
  };

  const refreshLogs = async (silent = false) => {
    const result = await run(() => call<LogsResult>("read_latest_logs", { request: { lines: 240 } }));
    if (result) {
      setLogs(result);
      if (!silent) showResultNotice("日志已刷新", result, { silentSuccess: true });
    }
  };

  const refreshDiagnostics = async (silent = false) => {
    const result = await run(() => call<DiagnosticsResult>("copy_diagnostics"));
    if (result) {
      setDiagnostics(result);
      if (!silent) showResultNotice("诊断已生成", result, { silentSuccess: true });
    }
  };

  const refreshWatcher = async (silent = false) => {
    const result = await run(() => call<WatcherResult>("load_watcher_state"));
    if (result) {
      setWatcher(result);
      if (!silent) showResultNotice("Watcher 状态", result, { silentSuccess: true });
    }
  };

  const navigate = async (next: Route) => {
    setRoute(next);
    if (next === "overview") {
      await refreshOverview(true);
      await refreshLocalAuth(true);
      await refreshSmsProviderSettings(true);
      await refreshRelay(true);
    }
    if (next === "relay") {
      await refreshSettings(true);
      await refreshRelay(true);
      await refreshRelayFiles(true);
    }
    if (next === "sessions") {
      await refreshSettings(true);
      await refreshLocalSessions(true);
      await refreshProviderSyncTargets(true);
    }
    if (next === "zedRemote") {
      await refreshSettings(true);
      await refreshZedRemoteProjects(true);
    }
    if (next === "context") {
      await refreshSettings(true);
      await refreshRelayFiles(true);
      await refreshLiveContextEntries(true);
    }
    if (next === "settings") {
      await refreshSettings(true);
      await refreshSmsProviderSettings(true);
    }
    if (next === "userScripts") {
      await refreshSettings(true);
      await refreshScriptMarket(true);
    }
    if (next === "recommendations") await refreshAds(true);
    if (next === "about") {
      await refreshOverview(true);
      await refreshLogs(true);
      await refreshDiagnostics(true);
    }
    if (next === "maintenance") {
      await refreshOverview(true);
      await refreshWatcher(true);
    }
  };

  const launch = async () => {
    const result = await launchCommand("launch_codex_plus");
    if (result) {
      showNotice("启动任务", result.message, result.status);
      await refreshOverview(true);
    }
  };

  const restart = async () => {
    const result = await launchCommand("restart_codex_plus");
    if (result) {
      showNotice(`重启 ${PRODUCT_NAME}`, result.message, result.status);
      await refreshOverview(true);
    }
  };

  const launchCommand = async (command: "launch_codex_plus" | "restart_codex_plus") => {
    const result = await run(() =>
      call<CommandResult<Record<string, unknown>>>(command, {
        request: {
          appPath: launchForm.appPath,
          debugPort: numberOrDefault(launchForm.debugPort, 9229),
          helperPort: numberOrDefault(launchForm.helperPort, 57321),
        },
      }),
    );
    return result;
  };

  const repairBackend = async () => {
    const result = await run(() => call<SettingsResult>("repair_backend"));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showNotice("后端修复", result.message, result.status);
    }
  };

  const repairOfficialIsolation = async () => {
    const result = await run(() => call<OfficialIsolationRepairResult>("repair_official_codex_isolation"));
    if (result) {
      const repaired = result.repairedFiles.length;
      const remaining = result.remainingContaminatedFiles.length;
      const suffix =
        remaining > 0
          ? "请退出原版 Codex 后再执行一次。"
          : repaired > 0 && result.backupDir
            ? `备份已写入 ${result.backupDir}。`
            : "";
      showNotice("原版隔离修复", [result.message, suffix].filter(Boolean).join(" "), result.status);
      await checkReleaseReadiness();
    }
  };

  const installEntrypoints = async () => {
    const result = await run(() => call<InstallResult>("install_entrypoints"));
    if (result) {
      showNotice("入口安装", result.message, result.status);
      await refreshOverview(true);
    }
  };

  const uninstallEntrypoints = async () => {
    const result = await run(() =>
      call<InstallResult>("uninstall_entrypoints", {
        options: { removeOwnedData },
      }),
    );
    if (result) {
      showNotice("入口卸载", result.message, result.status);
      await refreshOverview(true);
    }
  };

  const repairShortcuts = async () => {
    const result = await run(() => call<InstallResult>("repair_shortcuts"));
    if (result) {
      showNotice("快捷方式修复", result.message, result.status);
      await refreshOverview(true);
    }
  };

  const watcherAction = async (command: string) => {
    const result = await run(() => call<WatcherResult>(command));
    if (result) {
      setWatcher(result);
      showNotice("Watcher 操作", result.message, result.status);
    }
  };

  const checkUpdate = async (silent = false) => {
    const result = await run(() => call<UpdateResult>("check_update"));
    if (result) {
      setUpdate(result);
      if (!silent || result.updateAvailable) {
        showNotice("GitHub Release 检查", result.message, result.status);
      }
    }
  };

  const checkReleaseReadiness = async () => {
    const result = await run(() => call<ReleaseReadinessResult>("release_readiness"));
    if (result) {
      setReleaseReadiness(result);
      showNotice("发布前检查", result.message, result.status);
    }
  };

  const performUpdate = async () => {
    const release =
      update?.latestVersion && update.assetName && update.assetUrl
        ? {
            version: update.latestVersion,
            url: "",
            body: update.releaseSummary ?? "",
            asset_name: update.assetName,
            asset_url: update.assetUrl,
          }
        : null;
    const result = await run(() => call<UpdateResult>("perform_update", { release }));
    if (result) {
      setUpdate(result);
      showNotice("更新安装", result.message, result.status);
    }
  };

  const saveSettings = async () => {
    const next = await settingsForSave(settingsForm, false);
    const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showNotice("设置保存", result.message, result.status);
    }
  };

  const saveSettingsValue = async (next: BackendSettings, silent = true, preserveLinkedProfiles = false) => {
    const normalized = normalizeSettings(next);
    setSettingsForm(normalized);
    const settingsToSave = await settingsForSave(normalized, preserveLinkedProfiles);
    const result = await run(() => call<SettingsResult>("save_settings", { settings: settingsToSave }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      if (!silent || !isSuccessStatus(result.status)) showNotice("设置保存", result.message, result.status);
    }
  };

  const settingsForSave = async (next: BackendSettings, preserveLinkedProfiles: boolean) => {
    const normalized = normalizeSettings(next);
    if (!normalized.ccsLinkEnabled || preserveLinkedProfiles) return normalized;
    const refreshed = await refreshSettings(true);
    if (!refreshed) return normalized;
    return mergeLiveLinkedRelayProfiles(normalized, normalizeSettings(refreshed));
  };

  const importCcsProviders = async () => {
    const result = await run(() => call<SettingsResult>("import_ccs_providers"));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showResultNotice("联动 cc-switch", result);
    }
  };

  const resetSettings = async () => {
    const result = await run(() => call<SettingsResult>("reset_settings"));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showNotice("设置重置", result.message, result.status);
    }
  };

  const refreshAds = async (silent = false) => {
    const result = await run(() => call<AdsResult>("load_ads"));
    if (result) {
      setAds(result);
      if (!silent) showResultNotice("推荐内容", result, { silentSuccess: true });
    }
  };

  const refreshProviderSyncTargets = async (silent = false) => {
    const result = await run(() => call<ProviderSyncTargetsResult>("load_provider_sync_targets"));
    if (result) {
      setProviderSyncTargets(result);
      const targets = result.targets ?? [];
      const saved = settingsForm.providerSyncLastSelectedProvider;
      const preferred =
        targets.find((target) => target.id === saved)?.id ||
        targets.find((target) => target.isCurrentProvider)?.id ||
        targets[0]?.id ||
        "openai";
      setSelectedProviderSyncTarget((current) => (targets.some((target) => target.id === current) ? current : preferred));
      if (!silent && !isSuccessStatus(result.status)) showNotice("Provider 同步目标", result.message, result.status);
    }
    return result;
  };

  const syncProvidersNow = async () => {
    if (providerSyncProgress.active) return;
    setProviderSyncProgress({
      active: true,
      percent: 12,
      message: selectedProviderSyncTarget ? `正在同步到 ${selectedProviderSyncTarget}…` : "正在扫描历史会话与索引…",
      result: null,
    });
    const progressTimer = window.setInterval(() => {
      setProviderSyncProgress((current) => {
        if (!current.active) return current;
        return {
          ...current,
          percent: Math.min(88, current.percent + 8),
          message: current.percent < 40 ? "正在检查会话 provider 标记…" : "正在写入修复与备份…",
        };
      });
    }, 350);
    try {
      const targetProvider = selectedProviderSyncTarget || undefined;
      const result = await run(() =>
        call<CommandResult<ProviderSyncPayload>>("sync_providers_now", { targetProvider }),
      );
      if (result) {
        setProviderSyncProgress({
          active: false,
          percent: 100,
          message: providerSyncProgressMessage(result),
          result,
        });
        if (targetProvider) {
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
        showNotice("历史会话修复", result.message, result.status);
      } else {
        setProviderSyncProgress({
          active: false,
          percent: 100,
          message: "历史会话修复失败，请查看错误提示后重试。",
          result: null,
        });
      }
    } finally {
      window.clearInterval(progressTimer);
    }
  };

  const applyRelayInjection = async (silent = false) => {
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (settingsResult) {
      setSettings(settingsResult);
      setSettingsForm(normalizeSettings(settingsResult.settings));
      if (!isSuccessStatus(settingsResult.status)) {
        showNotice("设置保存", settingsResult.message, settingsResult.status);
        return false;
      }
    } else {
      return false;
    }
    const result = await run(() => call<RelayResult>("apply_relay_injection"));
    if (result) {
      setRelay(result);
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice("极义纯 API", result.message, result.status);
    }
    return !!result && isSuccessStatus(result.status) && result.configured;
  };

  const saveLaunchMode = async (launchMode: LaunchMode, silent = false, baseSettings: BackendSettings = settingsForm) => {
    const next = { ...baseSettings, launchMode };
    setSettingsForm(next);
    const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      if (!silent) showNotice("页面增强模式", result.message, result.status);
    }
    return result;
  };

  const applyPureApiInjection = async (silent = false) => {
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (settingsResult) {
      setSettings(settingsResult);
      setSettingsForm(normalizeSettings(settingsResult.settings));
      if (!isSuccessStatus(settingsResult.status)) {
        showNotice("设置保存", settingsResult.message, settingsResult.status);
        return false;
      }
    } else {
      return false;
    }
    const result = await run(() => call<RelayResult>("apply_pure_api_injection"));
    if (result) {
      setRelay(result);
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice("纯 API 模式", result.message, result.status);
    }
    return !!result && isSuccessStatus(result.status) && result.configured;
  };

  const clearRelayInjection = async (silent = false) => {
    const result = await run(() => call<RelayResult>("clear_relay_injection"));
    if (result) {
      setRelay(result);
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice("极义原生账号", result.message, result.status);
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
    if (!isSuccessStatus(result.status)) showResultNotice("工具与插件", result);
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
    if (!isSuccessStatus(result.status)) showResultNotice("工具与插件", result);
    return normalized;
  };

  const extractRelayCommonConfig = async (configContents: string) => {
    const result = await run(() =>
      call<ExtractRelayCommonConfigResult>("extract_relay_common_config", {
        request: { configContents },
      }),
    );
    if (result) showResultNotice("通用配置文件", result);
    return result && isSuccessStatus(result.status) ? result : null;
  };

  const testRelayProfile = async (profile: RelayProfile) => {
    const result = await run(() => call<RelayProfileTestResult>("test_relay_profile", { profile }));
    if (result) showNotice("供应商测试", result.message, result.status);
  };

  const fetchRelayProfileModels = async (profile: RelayProfile) => {
    const result = await run(() => call<RelayProfileModelsResult>("fetch_relay_profile_models", { profile }));
    if (result) showNotice("模型列表", result.message, result.status);
    return result && isSuccessStatus(result.status) ? result.models : null;
  };

  const switchOfficialMode = async () => {
    showNotice("极义原生账号", "极义codex 已禁用官方登录模式，请使用阿里百炼 / 极义中转纯 API。", "failed");
  };

  const switchPureApiMode = async () => {
    const switched = await applyPureApiInjection(true);
    if (!switched) return;
    const result = await saveLaunchMode("patch", true);
    if (result) showNotice("纯 API 模式", "已切换到纯 API；页面增强已设为完整增强。", result.status);
  };

  const switchRelayProfile = async (next: BackendSettings, previousActiveRelayId = settingsForm.activeRelayId) => {
    let switchSettings = normalizeSettings(next);
    if (switchSettings.ccsLinkEnabled) {
      const targetRelayId = switchSettings.activeRelayId;
      const refreshed = await refreshSettings(true);
      if (!refreshed) return;
      const latest = normalizeSettings(refreshed);
      if (!latest.relayProfiles.some((profile) => profile.id === targetRelayId)) {
        showNotice("供应商切换", "目标供应商已不在 cc-switch 或本地配置中，请刷新供应商列表后重试。", "failed");
        return;
      }
      switchSettings = syncLegacyRelayFields({ ...latest, activeRelayId: targetRelayId });
    }
    if (!switchSettings.relayProfilesEnabled) {
      showNotice("供应商配置已关闭", "当前不会写入 Codex config.toml / auth.json。打开供应商配置总开关后再切换。", "failed");
      return;
    }
    const targetBeforeSnapshot = activeRelayProfile(switchSettings);
    logDiagnostic("switchRelayProfile.start", {
      currentRelayId: settingsForm.activeRelayId,
      targetRelayId: switchSettings.activeRelayId,
      targetRelayName: targetBeforeSnapshot.name,
      targetRelayMode: targetBeforeSnapshot.relayMode,
      ccsLinkEnabled: switchSettings.ccsLinkEnabled,
    });
    const nextWithSnapshot = await snapshotActiveRelayFilesBeforeSwitch(switchSettings, previousActiveRelayId);
    if (!nextWithSnapshot) {
      logDiagnostic("switchRelayProfile.snapshot_failed", {
        currentRelayId: settingsForm.activeRelayId,
        targetRelayId: switchSettings.activeRelayId,
      });
      return;
    }

    const selectedBeforeSave = activeRelayProfile(nextWithSnapshot);
    const validationError = relayProfileSwitchValidation(selectedBeforeSave);
    if (validationError) {
      logDiagnostic("switchRelayProfile.validation_failed", {
        targetRelayId: selectedBeforeSave.id,
        targetRelayName: selectedBeforeSave.name,
        error: validationError,
      });
      showNotice("供应商配置可能不正确", validationError, "failed");
      return;
    }

    let selectedSettings = nextWithSnapshot;
    logDiagnostic("switchRelayProfile.save_settings_start", {
      targetRelayId: selectedBeforeSave.id,
      targetRelayName: selectedBeforeSave.name,
    });
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: nextWithSnapshot }));
    if (settingsResult) {
      selectedSettings = normalizeSettings(settingsResult.settings);
      setSettings(settingsResult);
      setSettingsForm(selectedSettings);
      if (!isSuccessStatus(settingsResult.status)) {
        logDiagnostic("switchRelayProfile.save_settings_failed", {
          targetRelayId: selectedBeforeSave.id,
          status: settingsResult.status,
          message: settingsResult.message,
        });
        showNotice("供应商切换", settingsResult.message, settingsResult.status);
        return;
      }
    } else {
      logDiagnostic("switchRelayProfile.save_settings_no_result", {
        targetRelayId: selectedBeforeSave.id,
      });
      return;
    }

    const selectedAfterSave = activeRelayProfile(selectedSettings);
    const command = relayProfileSwitchCommand(selectedAfterSave);
    logDiagnostic("switchRelayProfile.apply_start", {
      targetRelayId: selectedAfterSave.id,
      targetRelayName: selectedAfterSave.name,
      command,
    });
    const result = await run(() => call<RelayResult>(command));
    if (!result) {
      logDiagnostic("switchRelayProfile.apply_no_result", {
        targetRelayId: selectedAfterSave.id,
        command,
      });
      return;
    }

    setRelay(result);
    await refreshRelayFiles(true);
    if (!isSuccessStatus(result.status) || (selectedAfterSave.relayMode === "pureApi" && !result.configured)) {
      logDiagnostic("switchRelayProfile.apply_failed", {
        targetRelayId: selectedAfterSave.id,
        command,
        status: result.status,
        message: result.message,
        configured: result.configured,
      });
      showNotice("供应商切换", relayProfileReadinessText(selectedAfterSave, result), result.status);
      return;
    }

    const currentSelected = activeRelayProfile(selectedSettings);
    const launchMode = currentSelected.relayMode === "pureApi" ? "patch" : "relay";
    logDiagnostic("switchRelayProfile.launch_mode_start", {
      targetRelayId: currentSelected.id,
      launchMode,
    });
    const modeResult = await saveLaunchMode(launchMode, true, selectedSettings);
    if (modeResult) {
      logDiagnostic("switchRelayProfile.ok", {
        targetRelayId: currentSelected.id,
        launchMode,
        status: modeResult.status,
      });
      showNotice("供应商切换", relayProfileModeSwitchedText(currentSelected), modeResult.status);
    } else {
      logDiagnostic("switchRelayProfile.launch_mode_no_result", {
        targetRelayId: currentSelected.id,
        launchMode,
      });
    }
  };

  const snapshotActiveRelayFilesBeforeSwitch = async (next: BackendSettings, previousActiveRelayId: string): Promise<BackendSettings | null> => {
    const current = settingsForm.relayProfiles.find((profile) => profile.id === previousActiveRelayId) || activeRelayProfile(settingsForm);
    const selected = activeRelayProfile(next);
    if (current.id === selected.id) return next;

    logDiagnostic("snapshotActiveRelayFilesBeforeSwitch.start", {
      currentRelayId: current.id,
      currentRelayName: current.name,
      selectedRelayId: selected.id,
      selectedRelayName: selected.name,
    });
    const result = await run(() =>
      call<SettingsBackfillResult>("backfill_relay_profile_from_live", {
        request: { settings: next, profileId: current.id },
      }),
    );
    if (!result || !isSuccessStatus(result.status)) {
      logDiagnostic("snapshotActiveRelayFilesBeforeSwitch.failed", {
        currentRelayId: current.id,
        selectedRelayId: selected.id,
        status: result?.status,
        message: result?.message,
      });
      showNotice("供应商切换", result?.message ?? "读取当前配置文件失败，已停止切换以避免覆盖用户改动。", result?.status ?? "failed");
      return null;
    }

    logDiagnostic("snapshotActiveRelayFilesBeforeSwitch.ok", {
      currentRelayId: current.id,
      selectedRelayId: selected.id,
    });
    return syncLegacyRelayFields(normalizeSettings(result.settings));
  };


  const copyText = async (text: string, message: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch (error) {
      showNotice("复制失败", stringifyError(error), "failed");
    }
  };

  const openExternalUrl = async (url: string) => {
    const result = await run(() => call<CommandResult<Record<string, unknown>>>("open_external_url", { url }));
    if (result) {
      showResultNotice("打开链接", result, { silentSuccess: true });
    }
  };

  const showNotice = (title: string, message: string, status?: Status) => {
    setNotice({ title, message, status });
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
    void (async () => {
      const startup = await run(() => call<StartupResult>("startup_options"));
      if (startup?.appMode) {
        setAppMode(startup.appMode === "main" ? "main" : "manager");
      }
      if (startup?.showUpdate) {
        setRoute("about");
        void checkUpdate(false);
      } else {
        void checkUpdate(true);
      }
      await refreshOverview(true);
      await refreshLocalAuth(true);
      await refreshSmsProviderSettings(true);
      await refreshLocalUsage(true);
      await refreshLocalBackendState(true);
      await refreshManagedProxy(true);
      await refreshAdminConsole(true);
      await refreshSettings(true);
      await refreshRelay(true);
      await refreshProviderSyncTargets(true);
    })();
  }, []);

  useEffect(() => {
    const entitlement = localAuth?.entitlement;
    if (!entitlement) return;
    setEntitlementForm({
      planId: entitlement.planId || "local_trial",
      planName: entitlement.planName || "本地试用",
      dailyTokenLimit: String(Math.max(0, entitlement.dailyTokenLimit || 0)),
    });
  }, [
    localAuth?.entitlement?.planId,
    localAuth?.entitlement?.planName,
    localAuth?.entitlement?.dailyTokenLimit,
  ]);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    window.localStorage.setItem("codex-plus-theme", theme);
  }, [theme]);

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

  const actions = useMemo(
    () => ({
      navigateTo: (next: Route) => navigate(next),
      refreshCurrent: () => navigate(route),
      launch,
      restart,
      enterCodex,
      repairBackend,
      repairOfficialIsolation,
      installEntrypoints,
      uninstallEntrypoints,
      repairShortcuts,
      checkUpdate,
      checkReleaseReadiness,
      performUpdate,
      saveSettings,
      saveSettingsValue,
      refreshSettings,
      resetSettings,
      chooseCodexAppPath: async (mode: "folder" | "file") => {
        let selected: unknown;
        try {
          selected = await open(
            mode === "folder"
              ? { directory: true, multiple: false, title: "选择 Codex 应用目录" }
              : {
                  directory: false,
                  multiple: false,
                  title: "选择 JiyiCodexClient.app 或 Codex.exe",
                  filters: [{ name: "Codex 应用", extensions: ["exe", "app"] }],
                },
          );
        } catch (error) {
          // Surface plugin failures (e.g. missing capability permission) so the
          // buttons no longer appear unresponsive — see #345.
          const message = error instanceof Error ? error.message : String(error);
          showNotice("Codex 应用路径", `打开选择器失败：${message}`, "failed");
          return;
        }
        if (typeof selected === "string" && selected.trim()) {
          const result = await saveCodexAppPath(selected.trim());
          if (result) {
            showNotice("Codex 应用路径", "应用路径已保存，之后启动会自动复用。", result.status);
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
          showNotice("Codex 应用路径", "已清除保存路径，后续启动会回到自动探测。", result.status);
          await refreshOverview(true);
        }
      },
      saveManualCodexAppPath: async () => {
        const appPath = launchForm.appPath.trim();
        if (!appPath) {
          showNotice("Codex 应用路径", "请先填写或选择应用路径。", "failed");
          return;
        }
        const result = await saveCodexAppPath(appPath);
        if (result) {
          showNotice("Codex 应用路径", "应用路径已保存，之后启动会自动复用。", result.status);
        }
      },
      refreshLocalAuth,
      refreshSmsProviderSettings,
      saveSmsProviderSettings,
      refreshLocalUsage,
      requestLocalSmsCode,
      loginWithLocalSmsCode,
      updateLocalEntitlement,
      exportLocalIdentityReport,
      prepareIdentitySyncRequest,
      syncIdentityToService,
      refreshLocalBackendState,
      refreshManagedProxy,
      refreshAdminConsole,
      updateAdminUserEntitlement,
      updateAdminTeamEntitlement,
      setAdminUserAccess,
      recordAdminBillingRenewal,
      reconcileAdminBilling,
      startManagedProxy,
      stopManagedProxy,
      applyIdentitySyncLocally,
      logoutLocalAuth,
      resetLocalAuthState,
      syncProvidersNow,
      refreshProviderSyncTargets,
      setProviderSyncTarget: (provider: string) => {
        setSelectedProviderSyncTarget(provider);
        setSettingsForm((current) => ({ ...current, providerSyncLastSelectedProvider: provider }));
      },
      setLaunchMode: async (launchMode: LaunchMode) => {
        await saveLaunchMode(launchMode);
      },
      refreshRelay,
      refreshRelayFiles,
      refreshCoordinationStatus: async () => {
        const result = await run(() => call<CoordinationStatusResult>("get_config_coordination_status"));
        return result?.status === "ok" ? result : null;
      },
      refreshLiveContextEntries,
      syncLiveContextEntries,
      importCcsProviders,
      refreshAds,
      refreshScriptMarket,
      installMarketScript,
      setUserScriptEnabled,
      deleteUserScript,
      refreshLocalSessions,
      deleteLocalSession,
      refreshZedRemoteProjects,
      openZedRemoteProject,
      forgetZedRemoteProject,
      openExternalUrl,
      applyRelayInjection,
      applyPureApiInjection,
      clearRelayInjection,
      saveRelayFile,
      upsertContextEntry,
      deleteContextEntry,
      extractRelayCommonConfig,
      testRelayProfile,
      fetchRelayProfileModels,
      switchRelayProfile,
      switchOfficialMode,
      switchPureApiMode,
      refreshLogs,
      refreshDiagnostics,
      showMessage: async (title: string, message: string, status?: Status) => showNotice(title, message, status),
      copyLogs: () => copyText(logs?.text ?? "", "日志已复制。"),
      copyDiagnostics: () => copyText(diagnostics?.report ?? "", "诊断报告已复制。"),
      goLogs: () => navigate("about"),
      checkHealth: async () => {
        await refreshOverview(true);
        await refreshRelay(true);
        await refreshWatcher(true);
        showNotice("检查完成", "已刷新 Codex 应用、入口和 Watcher 状态。", "ok");
      },
      installWatcher: () => watcherAction("install_watcher"),
      uninstallWatcher: () => watcherAction("uninstall_watcher"),
      enableWatcher: () => watcherAction("enable_watcher"),
      disableWatcher: () => watcherAction("disable_watcher"),
      toggleTheme: () => setTheme((current) => (current === "dark" ? "light" : "dark")),
    }),
    [appMode, route, launchForm, loginForm, entitlementForm, smsProviderForm, adminUserForm, adminTeamForm, adminRenewalForm, settingsForm, settings, removeOwnedData, update, logs, diagnostics, theme, relayFiles, localSessions, zedRemoteProjects, selectedProviderSyncTarget],
  );
  const hasUpdate = update?.updateAvailable === true;

  if (appMode === "main") {
    return (
      <MainEntryScreen
        actions={actions}
        launchState={mainEntryState}
        localAuth={localAuth}
        loginForm={loginForm}
        notice={notice}
        onCloseNotice={() => setNotice(null)}
        onLoginFormChange={setLoginForm}
        theme={theme}
      />
    );
  }

  return (
    <div className={`shell ${theme}`}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">极义</div>
          <div className="brand-copy">
            <div className="brand-title-row">
              <div className="brand-title">{PRODUCT_NAME}</div>
              {hasUpdate ? (
                <button
                  className="update-dot"
                  onClick={() => {
                    setRoute("about");
                    void checkUpdate(false);
                  }}
                  title={`发现新版本 ${update?.latestVersion ?? ""}`}
                  type="button"
                >
                  <CircleArrowUp className="h-4 w-4" aria-hidden="true" />
                </button>
              ) : null}
            </div>
            <div className="brand-subtitle">AI Native 工作台</div>
          </div>
        </div>
        <nav className="nav">
          {routes.map((item) => {
            const Icon = item.icon;
            return (
            <button
              className={`nav-item ${route === item.id ? "active" : ""}`}
              key={item.id}
              onClick={() => void navigate(item.id)}
              title={item.label}
              type="button"
            >
              <span className="nav-icon">
                <Icon className="h-4 w-4" aria-hidden="true" />
              </span>
              <span className="nav-label">{item.label}</span>
            </button>
          );
          })}
        </nav>
      </aside>
      <main className="workspace">
        <header className="topbar" key={`topbar-${route}`}>
          <div>
            <h1>{routeTitle(route)}</h1>
            <p>{routeSubtitle(route)}</p>
          </div>
          <div className="topbar-actions">
            <Button
              onClick={actions.toggleTheme}
              size="icon"
              title={theme === "dark" ? "切换到浅色" : "切换到深色"}
              variant="outline"
            >
              {theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
            </Button>
            <Button onClick={() => void actions.restart()} title={`重启 ${PRODUCT_NAME}`} variant="outline">
              <Rocket className="h-4 w-4" />
              重启 {PRODUCT_NAME}
            </Button>
            <Button onClick={() => void actions.refreshCurrent()} size="icon" title="刷新当前页面" variant="outline">
              <RefreshCw className="h-4 w-4" />
            </Button>
          </div>
        </header>
        <section className="screen" key={route}>
          {route === "overview" ? (
            <OverviewScreen
              overview={overview}
              localAuth={localAuth}
              localUsage={localUsage}
              relay={relay}
              settings={settingsForm}
              loginForm={loginForm}
              entitlementForm={entitlementForm}
              onLoginFormChange={setLoginForm}
              onEntitlementFormChange={setEntitlementForm}
              actions={actions}
            />
          ) : null}
          {route === "admin" ? (
            <AdminConsoleScreen
              console={adminConsole}
              localBackend={localBackend}
              managedProxy={managedProxy}
              userForm={adminUserForm}
              teamForm={adminTeamForm}
              renewalForm={adminRenewalForm}
              onUserFormChange={setAdminUserForm}
              onTeamFormChange={setAdminTeamForm}
              onRenewalFormChange={setAdminRenewalForm}
              actions={actions}
            />
          ) : null}
          {route === "relay" ? (
            <RelayScreen
              settings={settings}
              relayFiles={relayFiles}
              form={settingsForm}
              onFormChange={setSettingsForm}
              actions={actions}
            />
          ) : null}
          {route === "sessions" ? (
            <SessionsScreen
              settings={settings}
              form={settingsForm}
              sessions={localSessions}
              providerSyncProgress={providerSyncProgress}
              providerSyncTargets={providerSyncTargets}
              selectedProviderSyncTarget={selectedProviderSyncTarget}
              onFormChange={setSettingsForm}
              actions={actions}
            />
          ) : null}
          {route === "context" ? (
            <ContextScreen
              form={settingsForm}
              liveEntries={liveContextEntries}
              relayFiles={relayFiles}
              onFormChange={setSettingsForm}
              actions={actions}
            />
          ) : null}
          {route === "enhance" ? (
            <EnhanceScreen form={settingsForm} onFormChange={setSettingsForm} actions={actions} />
          ) : null}
          {route === "zedRemote" ? (
            <ZedRemoteScreen projects={zedRemoteProjects} form={settingsForm} onFormChange={setSettingsForm} actions={actions} />
          ) : null}
          {route === "userScripts" ? <UserScriptsScreen settings={settings} market={scriptMarket} actions={actions} /> : null}
          {route === "recommendations" ? <RecommendationsScreen ads={ads} actions={actions} /> : null}
          {route === "maintenance" ? (
            <MaintenanceScreen
              overview={overview}
              watcher={watcher}
              settings={settings}
              releaseReadiness={releaseReadiness}
              launchForm={launchForm}
              onLaunchFormChange={setLaunchForm}
              removeOwnedData={removeOwnedData}
              onRemoveOwnedDataChange={setRemoveOwnedData}
              actions={actions}
            />
          ) : null}
          {route === "about" ? <AboutScreen overview={overview} update={update} logs={logs} diagnostics={diagnostics} actions={actions} /> : null}
          {route === "settings" ? (
            <SettingsScreen
              settings={settings}
              theme={theme}
              form={settingsForm}
              smsProvider={smsProvider}
              smsProviderForm={smsProviderForm}
              localBackend={localBackend}
              managedProxy={managedProxy}
              onFormChange={setSettingsForm}
              onSmsProviderFormChange={setSmsProviderForm}
              actions={actions}
            />
          ) : null}
        </section>
      </main>
      {notice ? (
        <NoticeDialog
          key={`${notice.title}-${notice.message}-${notice.status ?? ""}`}
          notice={notice}
          onClose={() => setNotice(null)}
        />
      ) : null}
    </div>
  );
}

type Actions = {
  navigateTo: (next: Route) => Promise<void>;
  refreshCurrent: () => Promise<void>;
  launch: () => Promise<void>;
  restart: () => Promise<void>;
  enterCodex: () => Promise<CommandResult<{ appPath: string | null; debugPort: number; helperPort: number }> | null>;
  repairBackend: () => Promise<void>;
  repairOfficialIsolation: () => Promise<void>;
  installEntrypoints: () => Promise<void>;
  uninstallEntrypoints: () => Promise<void>;
  repairShortcuts: () => Promise<void>;
  checkUpdate: () => Promise<void>;
  checkReleaseReadiness: () => Promise<void>;
  performUpdate: () => Promise<void>;
  saveSettings: () => Promise<void>;
  saveSettingsValue: (settings: BackendSettings, silent?: boolean, preserveLinkedProfiles?: boolean) => Promise<void>;
  refreshSettings: (silent?: boolean) => Promise<BackendSettings | null>;
  resetSettings: () => Promise<void>;
  chooseCodexAppPath: (mode: "folder" | "file") => Promise<void>;
  clearCodexAppPath: () => Promise<void>;
  saveManualCodexAppPath: () => Promise<void>;
  refreshLocalAuth: (silent?: boolean) => Promise<LocalAuthResult | null>;
  refreshSmsProviderSettings: (silent?: boolean) => Promise<SmsProviderSettingsResult | null>;
  saveSmsProviderSettings: () => Promise<void>;
  refreshLocalUsage: (silent?: boolean) => Promise<LocalUsageResult | null>;
  requestLocalSmsCode: () => Promise<void>;
  loginWithLocalSmsCode: () => Promise<void>;
  updateLocalEntitlement: () => Promise<void>;
  exportLocalIdentityReport: () => Promise<void>;
  prepareIdentitySyncRequest: () => Promise<void>;
  syncIdentityToService: () => Promise<void>;
  refreshLocalBackendState: (silent?: boolean) => Promise<LocalBackendStateResult | null>;
  refreshManagedProxy: (silent?: boolean) => Promise<ManagedProxyRuntimeResult | null>;
  refreshAdminConsole: (silent?: boolean) => Promise<AdminConsoleResult | null>;
  updateAdminUserEntitlement: () => Promise<void>;
  updateAdminTeamEntitlement: () => Promise<void>;
  setAdminUserAccess: (userId: string, status: "active" | "blocked") => Promise<void>;
  recordAdminBillingRenewal: () => Promise<void>;
  reconcileAdminBilling: () => Promise<void>;
  startManagedProxy: () => Promise<void>;
  stopManagedProxy: () => Promise<void>;
  applyIdentitySyncLocally: () => Promise<void>;
  logoutLocalAuth: () => Promise<void>;
  resetLocalAuthState: () => Promise<void>;
  syncProvidersNow: () => Promise<void>;
  refreshProviderSyncTargets: (silent?: boolean) => Promise<ProviderSyncTargetsResult | null>;
  setProviderSyncTarget: (provider: string) => void;
  setLaunchMode: (launchMode: LaunchMode) => Promise<void>;
  refreshRelay: () => Promise<void>;
  refreshRelayFiles: () => Promise<RelayFilesResult | null>;
  refreshCoordinationStatus: () => Promise<CoordinationStatus | null>;
  refreshLiveContextEntries: () => Promise<LiveContextEntriesResult | null>;
  syncLiveContextEntries: (settings: BackendSettings, silent?: boolean) => Promise<LiveContextEntriesResult | null>;
  importCcsProviders: () => Promise<void>;
  refreshAds: () => Promise<void>;
  refreshScriptMarket: () => Promise<void>;
  installMarketScript: (id: string) => Promise<void>;
  setUserScriptEnabled: (key: string, enabled: boolean) => Promise<void>;
  deleteUserScript: (key: string) => Promise<void>;
  refreshLocalSessions: () => Promise<LocalSessionsResult | null>;
  deleteLocalSession: (session: LocalSession) => Promise<void>;
  refreshZedRemoteProjects: () => Promise<ZedRemoteProjectsResult | null>;
  openZedRemoteProject: (project: ZedRemoteProject, strategy?: ZedOpenStrategy) => Promise<void>;
  forgetZedRemoteProject: (project: ZedRemoteProject) => Promise<void>;
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
  extractRelayCommonConfig: (configContents: string) => Promise<ExtractRelayCommonConfigResult | null>;
  testRelayProfile: (profile: RelayProfile) => Promise<void>;
  fetchRelayProfileModels: (profile: RelayProfile) => Promise<string[] | null>;
  switchRelayProfile: (settings: BackendSettings, previousActiveRelayId?: string) => Promise<void>;
  switchOfficialMode: () => Promise<void>;
  switchPureApiMode: () => Promise<void>;
  refreshLogs: () => Promise<void>;
  refreshDiagnostics: () => Promise<void>;
  showMessage: (title: string, message: string, status?: Status) => Promise<void>;
  copyLogs: () => Promise<void>;
  copyDiagnostics: () => Promise<void>;
  goLogs: () => Promise<void>;
  installWatcher: () => Promise<void>;
  uninstallWatcher: () => Promise<void>;
  enableWatcher: () => Promise<void>;
  disableWatcher: () => Promise<void>;
  toggleTheme: () => void;
  checkHealth: () => Promise<void>;
};

function MainEntryScreen({
  actions,
  launchState,
  localAuth,
  loginForm,
  notice,
  onCloseNotice,
  onLoginFormChange,
  theme,
}: {
  actions: Actions;
  launchState: { status: Status; message: string; appPath?: string | null };
  localAuth: LocalAuthResult | null;
  loginForm: { phone: string; code: string };
  notice: { title: string; message: string; status?: Status } | null;
  onCloseNotice: () => void;
  onLoginFormChange: (value: { phone: string; code: string }) => void;
  theme: Theme;
}) {
  const smsConfig = localAuth?.smsConfig;
  const entitlement = localAuth?.entitlement;
  const authenticated = localAuth?.authenticated === true;
  return (
    <div className={`main-entry ${theme}`}>
      <main className="main-entry-card">
        <div className="main-entry-brand">
          <div className="brand-mark">极义</div>
          <div>
            <h1>极义codex</h1>
            <p>{authenticated ? "本地账号已登录" : "手机号验证码登录"}</p>
          </div>
        </div>

        <div className="main-entry-status">
          <Metric label="短信模式" value={smsConfig ? (smsConfig.dryRun ? "本地干跑" : "腾讯云") : "等待读取"} />
          <Metric label="短信区域" value={smsConfig?.region ?? "ap-guangzhou"} />
          <Metric label="短信密钥" value={formatSmsSecretSource(smsConfig)} />
          <Metric label="有效期" value={`${smsConfig?.ttlMinutes ?? 10} 分钟`} />
          <Metric label="会话" value={`${localAuth?.sessionTtlHours ?? 24 * 30} 小时`} />
          <Metric label="套餐" value={entitlement?.planName ?? "本地试用"} />
          <Metric label="套餐额度" value={formatDailyLimit(entitlement?.dailyTokenLimit ?? 0)} />
        </div>

        {authenticated ? (
          <div className="main-entry-authenticated">
            <div>
              <strong>{localAuth.phoneMasked}</strong>
              <span>
                {localAuth.expiresAtMs ? `会话有效至 ${formatTime(localAuth.expiresAtMs)}` : launchState.message}
              </span>
            </div>
            <Toolbar>
                <Button onClick={() => void actions.enterCodex()}>
                  <Rocket className="h-4 w-4" />
                  进入 Codex
                </Button>
                <Button onClick={() => void actions.resetLocalAuthState()} variant="outline">
                  重置登录态
                </Button>
                <Button onClick={() => void actions.logoutLocalAuth()} variant="outline">
                  退出登录
                </Button>
              </Toolbar>
            {launchState.appPath ? <code>{launchState.appPath}</code> : null}
          </div>
        ) : (
          <div className="main-entry-login">
            <Field label="手机号">
              <Input
                inputMode="tel"
                onChange={(event) => onLoginFormChange({ ...loginForm, phone: event.currentTarget.value })}
                placeholder="13812345678"
                value={loginForm.phone}
              />
            </Field>
            <Field label="验证码">
              <Input
                inputMode="numeric"
                maxLength={6}
                onChange={(event) => onLoginFormChange({ ...loginForm, code: event.currentTarget.value })}
                placeholder="6 位数字"
                value={loginForm.code}
              />
            </Field>
            <Toolbar>
              <Button onClick={() => void actions.requestLocalSmsCode()} variant="secondary">
                获取验证码
              </Button>
              <Button onClick={() => void actions.loginWithLocalSmsCode()}>
                <KeyRound className="h-4 w-4" />
                登录
              </Button>
            </Toolbar>
          </div>
        )}
      </main>
      {notice ? <NoticeDialog notice={notice} onClose={onCloseNotice} /> : null}
    </div>
  );
}

function OverviewScreen({
  overview,
  localAuth,
  localUsage,
  relay,
  settings,
  loginForm,
  entitlementForm,
  onLoginFormChange,
  onEntitlementFormChange,
  actions,
}: {
  overview: OverviewResult | null;
  localAuth: LocalAuthResult | null;
  localUsage: LocalUsageResult | null;
  relay: RelayResult | null;
  settings: BackendSettings;
  loginForm: { phone: string; code: string };
  entitlementForm: { planId: string; planName: string; dailyTokenLimit: string };
  onLoginFormChange: (value: { phone: string; code: string }) => void;
  onEntitlementFormChange: (value: { planId: string; planName: string; dailyTokenLimit: string }) => void;
  actions: Actions;
}) {
  const health = healthItems(overview);
  const activeProfile =
    settings.relayProfiles.find((profile) => profile.id === settings.activeRelayId) ||
    settings.relayProfiles[0] ||
    null;
  const defaultProviderEndpoint = [
    activeProfile?.upstreamBaseUrl,
    activeProfile?.baseUrl,
    settings.relayBaseUrl,
    BAILIAN_BASE_URL,
    APIMART_FALLBACK_BASE_URL,
  ]
    .map((value) => (value || "").trim())
    .find((value) => Boolean(value)) || BAILIAN_BASE_URL;
  const defaultProviderModel = activeProfile?.model || activeProfile?.testModel || settings.relayTestModel || QWEN_DEFAULT_MODEL;
  const apiKeyReady = Boolean(
    relay?.apiKeyConfigured ||
      activeProfile?.apiKey ||
      activeProfile?.authContents ||
      settings.relayApiKey ||
      relay?.hasBearerToken,
  );
  const smsConfig = localAuth?.smsConfig;
  const entitlement = localAuth?.entitlement;
  return (
    <>
      <Panel className="jojocode-overview">
        <CardContent>
          <div className="jojocode-overview-layout">
            <div className="jojocode-overview-main">
              <div className="jojocode-overview-mark">
                <Network className="h-5 w-5" />
              </div>
              <div>
                <span className="eyebrow">官方中转站</span>
                <h2>JOJO Code</h2>
                <p>
                  Codex++ 官方中转站，主打稳定接入和划算价格，支持 GPT-5.5、GPT-5.4、Claude Opus 4.8、Claude Opus 4.7、gpt-image-2 等模型与图像能力。
                </p>
              </div>
            </div>
            <div className="jojocode-overview-side">
              <div className="jojocode-model-tags">
                <span>GPT-5.5</span>
                <span>GPT-5.4</span>
                <span>Opus 4.8</span>
                <span>Opus 4.7</span>
                <span>gpt-image-2</span>
              </div>
              <Button onClick={() => void actions.openExternalUrl("https://jojocode.com/")}>
                <ExternalLink className="h-4 w-4" />
                打开 JOJO Code
              </Button>
            </div>
          </div>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead
          title="本地账号"
          detail={localAuth?.authenticated ? `已登录：${localAuth.phoneMasked ?? ""}` : "手机号验证码登录；本地部署默认支持干跑验收"}
        />
        <CardContent>
          <div className="account-grid">
            <div className="account-status-block">
              <div className="account-status-head">
                <span className="scenario-icon">
                  <ShieldCheck className="h-4 w-4" aria-hidden="true" />
                </span>
                <div>
                  <strong>{localAuth?.authenticated ? "本地账号已登录" : "本地账号未登录"}</strong>
                  <span>
                    {localAuth?.authenticated
                      ? `登录时间：${formatTime(localAuth.loginAtMs ?? 0)}；有效至：${formatTime(localAuth.expiresAtMs ?? 0)}`
                      : localAuth?.sessionExpired
                        ? "本地会话已过期，请重新手机号验证。"
                        : "用于后续本地授权、设备和团队配置承接。"}
                  </span>
                </div>
              </div>
              <div className="account-meta-grid">
                <Metric label="短信模式" value={smsConfig ? (smsConfig.dryRun ? "本地干跑" : "腾讯云") : "等待读取"} />
                <Metric label="短信区域" value={smsConfig?.region ?? "ap-guangzhou"} />
                <Metric label="短信密钥" value={formatSmsSecretSource(smsConfig)} />
                <Metric label="有效期" value={`${smsConfig?.ttlMinutes ?? 10} 分钟`} />
                <Metric label="会话有效期" value={`${localAuth?.sessionTtlHours ?? 24 * 30} 小时`} />
                <Metric label="本地套餐" value={entitlement?.planName ?? "本地试用"} />
                <Metric label="套餐额度" value={formatDailyLimit(entitlement?.dailyTokenLimit ?? 0)} />
              </div>
              <code>{localAuth?.dbPath ?? "等待读取本地账号数据库"}</code>
            </div>
            {localAuth?.authenticated ? (
              <div className="account-login-block">
                <div>
                  <strong>{localAuth.phoneMasked}</strong>
                  <span>
                    当前设备：{localAuth.deviceId ? localAuth.deviceId.slice(0, 24) : "等待读取"}
                  </span>
                </div>
                <Toolbar>
                  <Button onClick={() => void actions.refreshLocalAuth()} variant="secondary">
                    <RefreshCw className="h-4 w-4" />
                    刷新
                  </Button>
                  <Button onClick={() => void actions.exportLocalIdentityReport()} variant="outline">
                    <Download className="h-4 w-4" />
                    导出账号报告
                  </Button>
                  <Button onClick={() => void actions.resetLocalAuthState()} variant="outline">
                    重置登录态
                  </Button>
                  <Button onClick={() => void actions.logoutLocalAuth()} variant="outline">
                    退出登录
                  </Button>
                </Toolbar>
                <div className="entitlement-editor">
                  <div className="inline-fields entitlement-fields">
                    <Field label="套餐 ID">
                      <Input
                        onChange={(event) => onEntitlementFormChange({ ...entitlementForm, planId: event.currentTarget.value })}
                        placeholder="local_trial"
                        value={entitlementForm.planId}
                      />
                    </Field>
                    <Field label="套餐名称">
                      <Input
                        onChange={(event) => onEntitlementFormChange({ ...entitlementForm, planName: event.currentTarget.value })}
                        placeholder="本地试用"
                        value={entitlementForm.planName}
                      />
                    </Field>
                    <Field label="每日额度">
                      <Input
                        inputMode="numeric"
                        min={0}
                        onChange={(event) =>
                          onEntitlementFormChange({ ...entitlementForm, dailyTokenLimit: event.currentTarget.value })
                        }
                        placeholder="0"
                        step={1000}
                        type="number"
                        value={entitlementForm.dailyTokenLimit}
                      />
                    </Field>
                  </div>
                  <Toolbar>
                    <Button onClick={() => void actions.updateLocalEntitlement()} variant="secondary">
                      <Save className="h-4 w-4" />
                      保存套餐
                    </Button>
                    <Button onClick={() => void actions.refreshLocalUsage()} variant="outline">
                      <RefreshCw className="h-4 w-4" />
                      刷新用量
                    </Button>
                  </Toolbar>
                </div>
              </div>
            ) : (
              <div className="account-login-block">
                <div className="inline-fields">
                  <Field label="手机号">
                    <Input
                      inputMode="tel"
                      onChange={(event) => onLoginFormChange({ ...loginForm, phone: event.currentTarget.value })}
                      placeholder="13812345678"
                      value={loginForm.phone}
                    />
                  </Field>
                  <Field label="验证码">
                    <Input
                      inputMode="numeric"
                      maxLength={6}
                      onChange={(event) => onLoginFormChange({ ...loginForm, code: event.currentTarget.value })}
                      placeholder="6 位数字"
                      value={loginForm.code}
                    />
                  </Field>
                </div>
                <Toolbar>
                  <Button onClick={() => void actions.requestLocalSmsCode()} variant="secondary">
                    获取验证码
                  </Button>
                  <Button onClick={() => void actions.loginWithLocalSmsCode()}>
                    <KeyRound className="h-4 w-4" />
                    登录
                  </Button>
                </Toolbar>
              </div>
            )}
          </div>
        </CardContent>
      </Panel>
      <div className="overview-two-col">
        <Panel>
          <CardHead title="阿里百炼默认供应商" detail="默认走千问兼容接口，APIMart 保留为备选" />
          <CardContent>
            <div className="provider-ready">
              <div>
                <strong>{activeProfile?.name ?? DEFAULT_RELAY_PROVIDER_NAME}</strong>
                <span>{defaultProviderModel}</span>
              </div>
              <Badge status={apiKeyReady ? "ok" : "not_checked"} />
            </div>
            <div className="provider-kv">
              <span>Endpoint</span>
              <code>{defaultProviderEndpoint}</code>
              <span>Key</span>
              <strong>{apiKeyReady ? formatRelayApiKeySource(relay?.apiKeySource) : "待填写"}</strong>
              <span>模式</span>
              <strong>{activeProfile?.relayMode === "pureApi" ? "纯 API" : relayModeLabel(activeProfile?.relayMode ?? "pureApi")}</strong>
              <span>今日请求</span>
              <strong>{localUsage ? `${formatCompactNumber(localUsage.requestCount)} 次` : "等待读取"}</strong>
              <span>今日用量</span>
              <strong>{localUsage ? `约 ${formatCompactNumber(localUsage.usedTokens)} tokens` : "等待读取"}</strong>
              <span>每日额度</span>
              <strong>
                {localUsage?.dailyTokenLimit
                  ? `${formatCompactNumber(localUsage.usedTokens)} / ${formatCompactNumber(localUsage.dailyTokenLimit)}`
                  : "只记账"}
              </strong>
              <span>额度来源</span>
              <strong>{usageLimitSourceLabel(localUsage?.limitSource ?? entitlement?.source ?? "unlimited")}</strong>
            </div>
            <Toolbar>
              <Button onClick={() => void actions.navigateTo("relay")} variant="secondary">
                <KeyRound className="h-4 w-4" />
                打开供应商配置
              </Button>
              <Button onClick={() => void actions.refreshLocalUsage()} variant="outline">
                <RefreshCw className="h-4 w-4" />
                刷新用量
              </Button>
              <Button onClick={() => void actions.navigateTo("about")} variant="outline">
                查看日志
              </Button>
            </Toolbar>
          </CardContent>
        </Panel>
        <Panel>
          <CardHead title="预置能力清单" detail="默认推荐的插件、Skill 和用户脚本入口" />
          <CardContent>
            <div className="capability-list">
              {presetCapabilities.map((item) => {
                const Icon = item.icon;
                return (
                  <button className="capability-row" key={item.name} onClick={() => void actions.navigateTo(item.route)} type="button">
                    <span className="scenario-icon">
                      <Icon className="h-4 w-4" aria-hidden="true" />
                    </span>
                    <span>
                      <strong>{item.name}</strong>
                      <small>{item.summary}</small>
                    </span>
                    <Badge status={item.type} />
                  </button>
                );
              })}
            </div>
          </CardContent>
        </Panel>
      </div>
      <Panel>
        <CardHead title="AI Native 场景工作台" detail="选择一个真实任务，按最小闭环推进到可验收结果" />
        <CardContent>
          <div className="scenario-grid">
            {scenarioWorkflows.map((scenario) => {
              const Icon = scenario.icon;
              return (
                <div className="scenario-card" key={scenario.title}>
                  <div className="scenario-card-head">
                    <span className="scenario-icon">
                      <Icon className="h-4 w-4" aria-hidden="true" />
                    </span>
                    <div>
                      <strong>{scenario.title}</strong>
                      <span>{scenario.summary}</span>
                    </div>
                  </div>
                  <div className="scenario-deliverable">
                    <small>交付物</small>
                    <span>{scenario.deliverable}</span>
                  </div>
                  <div className="scenario-steps">
                    {scenario.steps.map((step) => (
                      <span key={step}>{step}</span>
                    ))}
                  </div>
                  <Button onClick={() => void actions.navigateTo(scenario.route)} size="sm" variant="secondary">
                    进入配置
                  </Button>
                </div>
              );
            })}
          </div>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="健康检查" detail="概览只展示关键问题，具体配置在对应页面处理" />
        <CardContent>
          <div className="health-grid">
            <div className={`health-item ${overview?.codex_version ? "ok" : "needs-fix"}`}>
              {overview?.codex_version ? <CheckCircle2 className="h-4 w-4" /> : <Bell className="h-4 w-4" />}
              <div>
                <strong>Codex 版本</strong>
                <span>{overview?.codex_version ?? "未检测到 Codex 应用版本。"}</span>
              </div>
              <Badge status={overview?.codex_version ? "ok" : "not_checked"} />
            </div>
            {health.map((item) => (
              <div className={`health-item ${item.ok ? "ok" : "needs-fix"}`} key={item.title}>
                {item.ok ? <CheckCircle2 className="h-4 w-4" /> : <Bell className="h-4 w-4" />}
                <div>
                  <strong>{item.title}</strong>
                  <span>{item.detail}</span>
                </div>
                <Badge status={item.status} />
              </div>
            ))}
          </div>
          <Toolbar>
            <Button onClick={() => void actions.checkHealth()}>
              <RefreshCw className="h-4 w-4" />
              检查
            </Button>
            <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>
              <Wrench className="h-4 w-4" />
              修复入口
            </Button>
            <Button variant="secondary" onClick={() => void actions.repairBackend()}>
              修复后端
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="最近启动" detail={overview?.logs_path ?? "暂无状态文件"} />
        <CardContent>
          <LatestLaunch status={overview?.latest_launch ?? null} />
          <Toolbar>
            <Button onClick={() => void actions.launch()}>
              <Rocket className="h-4 w-4" />
              启动 {PRODUCT_NAME}
            </Button>
            <Button variant="secondary" onClick={() => void actions.goLogs()}>
              打开关于
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function AdminConsoleScreen({
  console: adminConsole,
  localBackend,
  managedProxy,
  userForm,
  teamForm,
  renewalForm,
  onUserFormChange,
  onTeamFormChange,
  onRenewalFormChange,
  actions,
}: {
  console: AdminConsoleResult | null;
  localBackend: LocalBackendStateResult | null;
  managedProxy: ManagedProxyRuntimeResult | null;
  userForm: { userId: string; planId: string; planName: string; dailyTokenLimit: string; reason: string };
  teamForm: { teamId: string; planId: string; planName: string; dailyTokenLimit: string; reason: string };
  renewalForm: {
    subjectType: string;
    subjectId: string;
    planId: string;
    planName: string;
    dailyTokenLimit: string;
    amountCents: string;
    currency: string;
    paymentChannel: string;
    externalOrderId: string;
    reason: string;
  };
  onUserFormChange: (value: { userId: string; planId: string; planName: string; dailyTokenLimit: string; reason: string }) => void;
  onTeamFormChange: (value: { teamId: string; planId: string; planName: string; dailyTokenLimit: string; reason: string }) => void;
  onRenewalFormChange: (value: {
    subjectType: string;
    subjectId: string;
    planId: string;
    planName: string;
    dailyTokenLimit: string;
    amountCents: string;
    currency: string;
    paymentChannel: string;
    externalOrderId: string;
    reason: string;
  }) => void;
  actions: Actions;
}) {
  const state = adminConsole?.state ?? localBackend;
  const users = adminConsole?.users.users ?? [];
  const teams = adminConsole?.teams.teams ?? [];
  const renewals = adminConsole?.renewals.renewals ?? [];
  const auditEvents = adminConsole?.auditEvents ?? [];
  const selectedUser = users.find((user) => user.userId === userForm.userId) ?? null;
  const selectedTeam = teams.find((team) => team.teamId === teamForm.teamId) ?? null;
  const selectUser = (user: AdminUserOverview) => {
    onUserFormChange({
      userId: user.userId,
      planId: user.planId ?? "jiyi_pro",
      planName: user.planName ?? "极义 Pro",
      dailyTokenLimit: String(user.dailyTokenLimit ?? 500000),
      reason: userForm.reason,
    });
    onRenewalFormChange({
      ...renewalForm,
      subjectType: "user",
      subjectId: user.userId,
      planId: user.planId ?? renewalForm.planId,
      planName: user.planName ?? renewalForm.planName,
      dailyTokenLimit: String(user.dailyTokenLimit ?? numberOrDefault(renewalForm.dailyTokenLimit, 500000)),
    });
  };
  const selectTeam = (team: AdminTeamOverview) => {
    onTeamFormChange({
      teamId: team.teamId,
      planId: team.planId || "team_pro",
      planName: team.planName || "团队 Pro",
      dailyTokenLimit: String(team.dailyTokenLimit || 2000000),
      reason: teamForm.reason,
    });
    onRenewalFormChange({
      ...renewalForm,
      subjectType: "team",
      subjectId: team.teamId,
      planId: team.planId || renewalForm.planId,
      planName: team.planName || renewalForm.planName,
      dailyTokenLimit: String(team.dailyTokenLimit || numberOrDefault(renewalForm.dailyTokenLimit, 2000000)),
    });
  };

  return (
    <>
      <Panel className="admin-hero">
        <CardHead title="运营总览" detail={state?.dbPath ?? "本地后端库未读取"} />
        <CardContent>
          <div className="admin-summary">
            <Metric label="用户" value={String(state?.userCount ?? 0)} />
            <Metric label="封禁" value={String(state?.blockedUserCount ?? 0)} />
            <Metric label="团队" value={String(state?.teamCount ?? 0)} />
            <Metric label="今日用户样本" value={String(users.length)} />
            <Metric label="续费记录" value={String(state?.billingRenewalCount ?? 0)} />
            <Metric label="支付事件" value={String(state?.billingPaymentEventCount ?? 0)} />
            <Metric label="审计事件" value={String(state?.auditEventCount ?? 0)} />
            <Metric label="有效 session" value={String(state?.activeSessionCount ?? 0)} />
            <Metric label="托管代理" value={managedProxy?.running ? "运行中" : "未运行"} />
            <Metric label="管理 Key" value={managedProxy?.adminKeyConfigured ? "已配置" : "未配置"} />
            <Metric label="计费 Key" value={managedProxy?.billingKeyConfigured ? "已配置" : "未配置"} />
            <Metric label="风控 Key" value={managedProxy?.accessKeyConfigured ? "已配置" : "未配置"} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.refreshAdminConsole()}>
              <RefreshCw className="h-4 w-4" />
              刷新总后台
            </Button>
            <Button variant="secondary" onClick={() => void actions.applyIdentitySyncLocally()}>
              <Database className="h-4 w-4" />
              同步本地账号
            </Button>
            <Button variant="secondary" onClick={() => void actions.startManagedProxy()}>
              <Power className="h-4 w-4" />
              启动托管代理
            </Button>
            <Button variant="outline" onClick={() => void actions.reconcileAdminBilling()}>
              支付重对账
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>

      <div className="admin-layout">
        <Panel>
          <CardHead title="用户运营" detail={`当前日：${adminConsole?.users.day || "未读取"}`} />
          <CardContent>
            <div className="admin-table admin-user-table">
              <div className="admin-table-head">
                <span>用户</span>
                <span>状态</span>
                <span>套餐</span>
                <span>今日用量</span>
                <span>Session</span>
                <span>操作</span>
              </div>
              {users.length ? (
                users.map((user) => (
                  <div className="admin-table-row" key={user.userId}>
                    <span>
                      <strong>{user.phoneMasked || user.userId}</strong>
                      <small>{shortId(user.userId)}</small>
                    </span>
                    <span>
                      <Badge status={user.accessStatus === "blocked" ? "failed" : "ok"} />
                      {user.accessReason ? <small>{user.accessReason}</small> : null}
                    </span>
                    <span>
                      <strong>{user.planName ?? "未配置"}</strong>
                      <small>{formatDailyLimit(user.dailyTokenLimit ?? 0)}</small>
                    </span>
                    <span>
                      <strong>{formatCompactNumber(user.todayUsedTokens)}</strong>
                      <small>{formatRemaining(user.todayRemainingTokens)}</small>
                    </span>
                    <span>
                      <strong>{user.activeSessionCount}/{user.sessionCount}</strong>
                      <small>{formatTime(user.lastSyncedAtMs)}</small>
                    </span>
                    <span className="admin-row-actions">
                      <Button size="sm" variant="secondary" onClick={() => selectUser(user)}>选择</Button>
                      {user.accessStatus === "blocked" ? (
                        <Button size="sm" variant="outline" onClick={() => void actions.setAdminUserAccess(user.userId, "active")}>解封</Button>
                      ) : (
                        <Button size="sm" variant="outline" onClick={() => void actions.setAdminUserAccess(user.userId, "blocked")}>封禁</Button>
                      )}
                    </span>
                  </div>
                ))
              ) : (
                <div className="empty">暂无用户。先在工作台完成手机号登录，再同步到本地后端。</div>
              )}
            </div>
          </CardContent>
        </Panel>

        <Panel>
          <CardHead title="用户套餐调整" detail={selectedUser ? selectedUser.phoneMasked : "选择用户后可直接编辑"} />
          <CardContent>
            <div className="form-row">
              <Field label="用户 ID">
                <Input value={userForm.userId} onChange={(event) => onUserFormChange({ ...userForm, userId: event.currentTarget.value })} />
              </Field>
              <Field label="套餐 ID">
                <Input value={userForm.planId} onChange={(event) => onUserFormChange({ ...userForm, planId: event.currentTarget.value })} />
              </Field>
            </div>
            <div className="form-row">
              <Field label="套餐名称">
                <Input value={userForm.planName} onChange={(event) => onUserFormChange({ ...userForm, planName: event.currentTarget.value })} />
              </Field>
              <Field label="每日额度">
                <Input type="number" min={0} value={userForm.dailyTokenLimit} onChange={(event) => onUserFormChange({ ...userForm, dailyTokenLimit: event.currentTarget.value })} />
              </Field>
            </div>
            <Field label="操作原因">
              <Input value={userForm.reason} onChange={(event) => onUserFormChange({ ...userForm, reason: event.currentTarget.value })} placeholder="例如 客服补偿 / 升级套餐" />
            </Field>
            <Toolbar>
              <Button onClick={() => void actions.updateAdminUserEntitlement()}>
                <Save className="h-4 w-4" />
                保存用户套餐
              </Button>
              <Button variant="secondary" onClick={() => void actions.setAdminUserAccess(userForm.userId, "blocked")}>封禁</Button>
              <Button variant="outline" onClick={() => void actions.setAdminUserAccess(userForm.userId, "active")}>解封</Button>
            </Toolbar>
          </CardContent>
        </Panel>
      </div>

      <div className="admin-layout">
        <Panel>
          <CardHead title="团队运营" detail={`当前日：${adminConsole?.teams.day || "未读取"}`} />
          <CardContent>
            <div className="admin-table admin-team-table">
              <div className="admin-table-head">
                <span>团队</span>
                <span>成员</span>
                <span>套餐</span>
                <span>今日用量</span>
                <span>更新时间</span>
                <span>操作</span>
              </div>
              {teams.length ? (
                teams.map((team) => (
                  <div className="admin-table-row" key={team.teamId}>
                    <span>
                      <strong>{team.teamName}</strong>
                      <small>{team.teamId}</small>
                    </span>
                    <span>
                      <strong>{team.activeMemberCount}/{team.memberCount}</strong>
                      <small>封禁 {team.blockedMemberCount}</small>
                    </span>
                    <span>
                      <strong>{team.planName}</strong>
                      <small>{formatDailyLimit(team.dailyTokenLimit)}</small>
                    </span>
                    <span>
                      <strong>{formatCompactNumber(team.todayUsedTokens)}</strong>
                      <small>{formatRemaining(team.todayRemainingTokens)}</small>
                    </span>
                    <span>{formatTime(team.updatedAtMs)}</span>
                    <span className="admin-row-actions">
                      <Button size="sm" variant="secondary" onClick={() => selectTeam(team)}>选择</Button>
                    </span>
                  </div>
                ))
              ) : (
                <div className="empty">暂无团队。同步本地账号后会自动生成默认团队。</div>
              )}
            </div>
          </CardContent>
        </Panel>

        <Panel>
          <CardHead title="团队套餐调整" detail={selectedTeam ? selectedTeam.teamName : "选择团队后可直接编辑"} />
          <CardContent>
            <div className="form-row">
              <Field label="团队 ID">
                <Input value={teamForm.teamId} onChange={(event) => onTeamFormChange({ ...teamForm, teamId: event.currentTarget.value })} />
              </Field>
              <Field label="套餐 ID">
                <Input value={teamForm.planId} onChange={(event) => onTeamFormChange({ ...teamForm, planId: event.currentTarget.value })} />
              </Field>
            </div>
            <div className="form-row">
              <Field label="套餐名称">
                <Input value={teamForm.planName} onChange={(event) => onTeamFormChange({ ...teamForm, planName: event.currentTarget.value })} />
              </Field>
              <Field label="每日额度">
                <Input type="number" min={0} value={teamForm.dailyTokenLimit} onChange={(event) => onTeamFormChange({ ...teamForm, dailyTokenLimit: event.currentTarget.value })} />
              </Field>
            </div>
            <Field label="操作原因">
              <Input value={teamForm.reason} onChange={(event) => onTeamFormChange({ ...teamForm, reason: event.currentTarget.value })} placeholder="例如 团队升级 / 续费补录" />
            </Field>
            <Toolbar>
              <Button onClick={() => void actions.updateAdminTeamEntitlement()}>
                <Save className="h-4 w-4" />
                保存团队套餐
              </Button>
            </Toolbar>
          </CardContent>
        </Panel>
      </div>

      <div className="admin-layout">
        <Panel>
          <CardHead title="续费与落账" detail="支持手工续费、企业转账和支付凭证补录" />
          <CardContent>
            <div className="form-row">
              <Field label="主体类型">
                <select className="select-input" value={renewalForm.subjectType} onChange={(event) => onRenewalFormChange({ ...renewalForm, subjectType: event.currentTarget.value })}>
                  <option value="user">用户</option>
                  <option value="team">团队</option>
                </select>
              </Field>
              <Field label="主体 ID">
                <Input value={renewalForm.subjectId} onChange={(event) => onRenewalFormChange({ ...renewalForm, subjectId: event.currentTarget.value })} />
              </Field>
            </div>
            <div className="form-row">
              <Field label="套餐 ID">
                <Input value={renewalForm.planId} onChange={(event) => onRenewalFormChange({ ...renewalForm, planId: event.currentTarget.value })} />
              </Field>
              <Field label="套餐名称">
                <Input value={renewalForm.planName} onChange={(event) => onRenewalFormChange({ ...renewalForm, planName: event.currentTarget.value })} />
              </Field>
            </div>
            <div className="form-row">
              <Field label="每日额度">
                <Input type="number" min={0} value={renewalForm.dailyTokenLimit} onChange={(event) => onRenewalFormChange({ ...renewalForm, dailyTokenLimit: event.currentTarget.value })} />
              </Field>
              <Field label="金额（分）">
                <Input type="number" min={0} value={renewalForm.amountCents} onChange={(event) => onRenewalFormChange({ ...renewalForm, amountCents: event.currentTarget.value })} />
              </Field>
            </div>
            <div className="form-row">
              <Field label="币种">
                <Input value={renewalForm.currency} onChange={(event) => onRenewalFormChange({ ...renewalForm, currency: event.currentTarget.value })} />
              </Field>
              <Field label="支付渠道">
                <Input value={renewalForm.paymentChannel} onChange={(event) => onRenewalFormChange({ ...renewalForm, paymentChannel: event.currentTarget.value })} placeholder="manual / alipay / wechatpay" />
              </Field>
            </div>
            <div className="form-row">
              <Field label="外部订单号">
                <Input value={renewalForm.externalOrderId} onChange={(event) => onRenewalFormChange({ ...renewalForm, externalOrderId: event.currentTarget.value })} />
              </Field>
              <Field label="备注">
                <Input value={renewalForm.reason} onChange={(event) => onRenewalFormChange({ ...renewalForm, reason: event.currentTarget.value })} />
              </Field>
            </div>
            <Toolbar>
              <Button onClick={() => void actions.recordAdminBillingRenewal()}>记录续费</Button>
              <Button variant="secondary" onClick={() => void actions.reconcileAdminBilling()}>支付重对账</Button>
            </Toolbar>
          </CardContent>
        </Panel>

        <Panel>
          <CardHead title="最近续费" detail={`${renewals.length} 条`} />
          <CardContent>
            <div className="admin-table admin-renewal-table">
              <div className="admin-table-head">
                <span>主体</span>
                <span>套餐</span>
                <span>金额</span>
                <span>渠道</span>
                <span>时间</span>
              </div>
              {renewals.length ? (
                renewals.slice(0, 8).map((renewal) => (
                  <div className="admin-table-row" key={renewal.renewalId}>
                    <span>
                      <strong>{renewal.subjectType}</strong>
                      <small>{shortId(renewal.subjectId)}</small>
                    </span>
                    <span>
                      <strong>{renewal.planName}</strong>
                      <small>{formatDailyLimit(renewal.dailyTokenLimit)}</small>
                    </span>
                    <span>{formatMoneyCents(renewal.amountCents, renewal.currency)}</span>
                    <span>{renewal.paymentChannel}</span>
                    <span>{formatTime(renewal.createdAtMs)}</span>
                  </div>
                ))
              ) : (
                <div className="empty">暂无续费记录。</div>
              )}
            </div>
          </CardContent>
        </Panel>
      </div>

      <Panel>
        <CardHead title="审计事件" detail={`${auditEvents.length} 条最近记录`} />
        <CardContent>
          <div className="admin-table admin-audit-table">
            <div className="admin-table-head">
              <span>事件</span>
              <span>操作者</span>
              <span>用户</span>
              <span>原因</span>
              <span>时间</span>
            </div>
            {auditEvents.length ? (
              auditEvents.slice(0, 12).map((event) => (
                <div className="admin-table-row" key={event.eventId}>
                  <span>
                    <strong>{event.eventType}</strong>
                    <small>{shortId(event.eventId)}</small>
                  </span>
                  <span>{event.actorType}</span>
                  <span>{event.subjectUserId ? shortId(event.subjectUserId) : "-"}</span>
                  <span>{event.reason || auditMetadataSummary(event.metadata)}</span>
                  <span>{formatTime(event.createdAtMs)}</span>
                </div>
              ))
            ) : (
              <div className="empty">暂无审计事件。</div>
            )}
          </div>
        </CardContent>
      </Panel>
    </>
  );
}

function RelayScreen({
  settings: _settings,
  relayFiles,
  form,
  onFormChange,
  actions,
}: {
  settings: SettingsResult | null;
  relayFiles: RelayFilesResult | null;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const normalized = normalizeSettings(form);
  const [detailProfileId, setDetailProfileId] = useState<string | null>(null);
  const [newProfileDraft, setNewProfileDraft] = useState<RelayProfile | null>(null);
  const detailProfile = newProfileDraft || (detailProfileId
    ? normalized.relayProfiles.find((profile) => profile.id === detailProfileId) || null
    : null);
  const isNewProfile = !!newProfileDraft;
  const saveRelaySettings = async (next: BackendSettings, preserveLinkedProfiles = false) => {
    onFormChange(next);
    await actions.saveSettingsValue(next, true, preserveLinkedProfiles);
  };
  const editRelayProfile = async (profileId: string) => {
    let nextSettings = normalized;
    const profile = normalized.relayProfiles.find((item) => item.id === profileId);
    if (profile?.linkedCcsProviderId && normalized.ccsLinkEnabled) {
      const refreshed = await actions.refreshSettings(true);
      if (refreshed) nextSettings = normalizeSettings(refreshed);
    }
    setNewProfileDraft(null);
    setDetailProfileId(
      nextSettings.relayProfiles.some((item) => item.id === profileId) ? profileId : null,
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

  if (detailProfile) {
    return (
      <RelayProfileDetail
        profile={detailProfile}
        relayFiles={!isNewProfile && detailProfile.id === normalized.activeRelayId ? relayFiles : null}
        form={normalized}
        isNew={isNewProfile}
        onBack={() => {
          setNewProfileDraft(null);
          setDetailProfileId(null);
        }}
        onFormChange={saveRelaySettings}
        onSaved={() => {
          setNewProfileDraft(null);
          setDetailProfileId(null);
        }}
        actions={actions}
      />
    );
  }

  return (
    <>
      <Panel>
        <CardHead title="供应商列表" detail={`${normalized.relayProfiles.length} 个供应商配置；可拖动排序，点编辑进入详情`} />
        <CardContent>
          <label className="switch-row relay-master-switch">
            <input
              checked={normalized.relayProfilesEnabled}
              onChange={(event) => {
                const next = { ...normalized, relayProfilesEnabled: event.currentTarget.checked };
                void saveRelaySettings(next);
              }}
              type="checkbox"
            />
            <span>
              <strong>启用供应商配置切换</strong>
              <small>关闭后本工具不会在手动切换时写入 Codex 的 config.toml / auth.json；启动 Codex 时始终不会自动改这些文件。</small>
            </span>
          </label>
          <label className="switch-row relay-local-proxy-switch">
            <input
              checked={normalized.jiyiLocalProxyEnabled}
              onChange={(event) => {
                const next = { ...normalized, jiyiLocalProxyEnabled: event.currentTarget.checked };
                void saveRelaySettings(next);
              }}
              type="checkbox"
            />
            <span>
              <strong>极义本地请求代理</strong>
              <small>开启后内置 Codex 只写 127.0.0.1 和占位 token，真实 Key 留在极义设置或环境变量中。</small>
            </span>
          </label>
          <label className="switch-row relay-usage-meter-switch">
            <input
              checked={normalized.jiyiLocalUsageMeterEnabled}
              onChange={(event) => {
                const next = { ...normalized, jiyiLocalUsageMeterEnabled: event.currentTarget.checked };
                void saveRelaySettings(next);
              }}
              type="checkbox"
            />
            <span>
              <strong>本地用量记账</strong>
              <small>记录 helper 转发的请求数和 token 估算值；每日额度为 0 时只记账不拦截。</small>
            </span>
          </label>
          <Field className="relay-field-quota" label="每日 token 上限">
            <Input
              min={0}
              type="number"
              value={String(normalized.jiyiDailyTokenLimit || 0)}
              onChange={(event) => {
                const value = Number.parseInt(event.currentTarget.value, 10);
                const next = { ...normalized, jiyiDailyTokenLimit: Number.isFinite(value) ? Math.max(0, value) : 0 };
                onFormChange(next);
              }}
              onBlur={(event) => {
                const value = Number.parseInt(event.currentTarget.value, 10);
                const next = { ...normalized, jiyiDailyTokenLimit: Number.isFinite(value) ? Math.max(0, value) : 0 };
                void saveRelaySettings(next);
              }}
              placeholder="0 表示不限制"
            />
          </Field>
          <label className="switch-row relay-link-switch">
            <input
              checked={normalized.ccsLinkEnabled}
              onChange={(event) => {
                if (event.currentTarget.checked) {
                  void actions.importCcsProviders();
                  return;
                }
                const next = { ...normalized, ccsLinkEnabled: false };
                void saveRelaySettings(next);
              }}
              type="checkbox"
            />
            <span>
              <strong>联动 cc-switch</strong>
              <small>开启后读取 cc-switch Codex 供应商并保存时回写；建议配合“配置所有权”避免与 CC Switch 互相覆盖。</small>
            </span>
          </label>
          <label className="switch-row relay-ownership-row">
            <span>
              <strong>配置所有权</strong>
              <small>决定由谁写入 ~/.codex/config.toml 与 auth.json。auto 在开启联动且检测到 CC Switch 时交由 CC Switch 管理。</small>
            </span>
            <select
              className="select-input relay-ownership-select"
              value={normalized.configOwnership}
              disabled={!normalized.relayProfilesEnabled}
              onChange={(event) => {
                const next = {
                  ...normalized,
                  configOwnership: event.currentTarget.value as ConfigOwnership,
                };
                void saveRelaySettings(next);
              }}
            >
              <option value="auto">自动（推荐）</option>
              <option value="ccSwitch">CC Switch 管理</option>
              <option value="codexPlusPlus">Codex++ 管理</option>
            </select>
          </label>
          <CoordinationStatusBanner form={normalized} actions={actions} />
          <div className="relay-add-row">
            <Button
              variant="secondary"
              onClick={() => {
                setNewProfileDraft(createRelayProfile(normalized));
                setDetailProfileId(null);
              }}
            >
              <Plus className="h-4 w-4" />
              添加供应商
            </Button>
          </div>
          <RelayProfileList
            form={normalized}
            onEdit={(profileId) => void editRelayProfile(profileId)}
            onFormChange={saveRelaySettings}
            disabled={!normalized.relayProfilesEnabled}
            actions={actions}
          />
        </CardContent>
      </Panel>
    </>
  );
}

function EnhanceScreen({
  form,
  onFormChange,
  actions,
}: {
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const setEnhanceFlag = (key: keyof BackendSettings, value: boolean) => onFormChange({ ...form, [key]: value });
  const masterEnabled = form.enhancementsEnabled;
  const patchMode = form.launchMode === "patch";
  return (
    <>
      <Panel>
        <CardHead title="页面功能增强" detail="会话删除、导出、项目移动、Timeline 和用户脚本等界面能力" />
        <CardContent>
          <label className="switch-row">
            <input
              checked={form.enhancementsEnabled}
              onChange={(event) => onFormChange({ ...form, enhancementsEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>
              <strong>启用 {PRODUCT_NAME} 页面增强</strong>
              <small>关闭后会停用删除、导出、项目移动、Timeline、插件相关和菜单位置增强。</small>
            </span>
          </label>
          <ModeSelector launchMode={form.launchMode} actions={actions} />
          {form.launchMode === "relay" ? (
            <div className="hint-line">
              <ShieldCheck className="h-4 w-4" />
              <span>当前为兼容增强模式，插件市场解锁、强制解锁入口和特殊插件强制安装不会启用；其他页面功能仍可用。</span>
            </div>
          ) : null}
          <div className="feature-switch-grid">
            <FeatureToggle title="插件市场解锁" detail="API Key 模式下扩展插件市场请求，尽量显示完整插件列表；官方/混合模式通常不需要。" checked={form.codexAppPluginMarketplaceUnlock} disabled={!masterEnabled || !patchMode} onChange={(value) => setEnhanceFlag("codexAppPluginMarketplaceUnlock", value)} />
            <FeatureToggle title="强制解锁入口" detail="恢复 1.1.9 的入口解锁方式，强制显示并启用插件入口。" checked={form.codexAppPluginEntryUnlock} disabled={!masterEnabled || !patchMode} onChange={(value) => setEnhanceFlag("codexAppPluginEntryUnlock", value)} />
            <FeatureToggle title="特殊插件强制安装" detail="解除 App unavailable / 应用不可用导致的前端安装禁用。" checked={form.codexAppForcePluginInstall} disabled={!masterEnabled || !patchMode} onChange={(value) => setEnhanceFlag("codexAppForcePluginInstall", value)} />
            <FeatureToggle title="模型白名单解锁" detail="从环境变量和 config.toml 的 /v1/models 拉取模型并补进模型列表。" checked={form.codexAppModelWhitelistUnlock} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppModelWhitelistUnlock", value)} />
            <FeatureToggle title="Fast 按钮" detail="显示服务模式切换按钮；部分模型支持 service_tier 时显示 Fast，其余按 Standard 发送。" checked={form.codexAppServiceTierControls} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppServiceTierControls", value)} />
            <FeatureToggle title="会话删除" detail="在会话列表悬停显示删除按钮，并支持撤销。" checked={form.codexAppSessionDelete} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppSessionDelete", value)} />
            <FeatureToggle title="Markdown 导出" detail="在会话列表显示导出按钮，导出带时间戳的 Markdown。" checked={form.codexAppMarkdownExport} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppMarkdownExport", value)} />
            <FeatureToggle title="会话项目移动" detail="把会话移动到普通对话或其他本地项目。" checked={form.codexAppProjectMove} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppProjectMove", value)} />
            <FeatureToggle title="对话 Timeline" detail="在对话右侧显示用户提问时间线，支持摘要和跳转。" checked={form.codexAppConversationTimeline} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppConversationTimeline", value)} />
            <FeatureToggle title="对话居中宽度" detail="把主对话和输入框限制到固定最大宽度，适合大屏阅读。" checked={form.codexAppConversationView} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppConversationView", value)} />
            <FeatureToggle title="切换对话保留位置" detail="切换 thread 时恢复上一次浏览位置。" checked={form.codexAppThreadScrollRestore} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppThreadScrollRestore", value)} />
            <FeatureToggle title="Zed Remote open" detail="远程 SSH 文件引用可直接用 Zed Remote Development 打开。" checked={form.codexAppZedRemoteOpen} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppZedRemoteOpen", value)} />
            <FeatureToggle title="Zed 项目记录" detail={`${PRODUCT_NAME} 会维护自己的远程项目最近列表。`} checked={form.zedRemoteProjectRegistryEnabled} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("zedRemoteProjectRegistryEnabled", value)} />
            <FeatureToggle title="同步 Zed settings" detail="高级选项，默认关闭；当前实现不主动改写 Zed settings。" checked={form.zedRemoteSyncToZedSettings} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("zedRemoteSyncToZedSettings", value)} />
            <FeatureToggle title="Upstream worktree" detail="从最新 upstream 分支创建 Git worktree。" checked={form.codexAppUpstreamWorktreeCreate} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppUpstreamWorktreeCreate", value)} />
            <FeatureToggle title="原生菜单栏位置" detail={`把 ${PRODUCT_NAME} 菜单插入 Codex 顶部原生菜单栏。`} checked={form.codexAppNativeMenuPlacement} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppNativeMenuPlacement", value)} />
          </div>
          <div className="zed-remote-settings">
            <Field label="Zed 默认打开策略">
              <select
                className="select-input"
                disabled={!masterEnabled}
                onChange={(event) => onFormChange({ ...form, zedRemoteOpenStrategy: event.currentTarget.value as ZedOpenStrategy })}
                value={form.zedRemoteOpenStrategy}
              >
                <option value="addToFocusedWorkspace">加入当前工作区</option>
                <option value="reuseWindow">复用窗口</option>
                <option value="newWindow">新窗口</option>
                <option value="default">Zed 默认行为</option>
              </select>
            </Field>
          </div>
          <div className="hint-line">
            <Info className="h-4 w-4" />
            <span>保守增强会减少对 Codex 页面结构的改动；完整增强会开启插件入口、强制安装和会话工具。</span>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存增强设置</Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function ZedRemoteScreen({
  projects,
  form,
  onFormChange,
  actions,
}: {
  projects: ZedRemoteProjectsResult | null;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const allProjects = projects?.projects ?? [];
  const currentProjects = allProjects.filter((project) => project.isCurrent);
  const currentIds = new Set(currentProjects.map((project) => project.id));
  const recentProjects = allProjects.filter((project) => !currentIds.has(project.id) && (project.source === "recent" || project.lastOpenedAtMs));
  const recentIds = new Set(recentProjects.map((project) => project.id));
  const discoveredProjects = allProjects.filter((project) => !currentIds.has(project.id) && !recentIds.has(project.id));
  const copyUrl = async (project: ZedRemoteProject) => {
    try {
      await navigator.clipboard.writeText(project.url);
      await actions.showMessage("Zed Remote URL", "ssh:// URL 已复制。", "ok");
    } catch (error) {
      await actions.showMessage("复制失败", stringifyError(error), "failed");
    }
  };
  return (
    <>
      <Panel>
        <CardHead title="Zed 远程项目" detail={`${allProjects.length} 个 ${PRODUCT_NAME} 可识别项目，默认策略：${zedStrategyLabel(form.zedRemoteOpenStrategy)}`} />
        <CardContent>
          <div className="metric-list">
            <Metric label="Current" value={String(currentProjects.length)} />
            <Metric label="Recent" value={String(recentProjects.length)} />
            <Metric label="Discovered" value={String(discoveredProjects.length)} />
          </div>
          <div className="zed-remote-settings">
            <Field label="默认打开策略">
              <select
                className="select-input"
                onChange={(event) => onFormChange({ ...form, zedRemoteOpenStrategy: event.currentTarget.value as ZedOpenStrategy })}
                value={form.zedRemoteOpenStrategy}
              >
                <option value="addToFocusedWorkspace">加入当前工作区</option>
                <option value="reuseWindow">复用窗口</option>
                <option value="newWindow">新窗口</option>
                <option value="default">Zed 默认行为</option>
              </select>
            </Field>
            <label className="switch-row compact">
              <input
                checked={form.zedRemoteProjectRegistryEnabled}
                onChange={(event) => onFormChange({ ...form, zedRemoteProjectRegistryEnabled: event.currentTarget.checked })}
                type="checkbox"
              />
              <span>
                <strong>记录最近打开</strong>
                <small>保存到 {PRODUCT_NAME} state，不改写 Zed settings。</small>
              </span>
            </label>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.refreshZedRemoteProjects()}>
              <RefreshCw className="h-4 w-4" />
              刷新项目
            </Button>
            <Button variant="secondary" onClick={() => void actions.saveSettingsValue(form, false)}>
              <Save className="h-4 w-4" />
              保存策略
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <ZedRemoteProjectSection title="Current" projects={currentProjects} actions={actions} onCopyUrl={copyUrl} />
      <ZedRemoteProjectSection title="Recent" projects={recentProjects} actions={actions} onCopyUrl={copyUrl} />
      <ZedRemoteProjectSection title="Discovered from Codex" projects={discoveredProjects} actions={actions} onCopyUrl={copyUrl} />
    </>
  );
}

function ZedRemoteProjectSection({
  title,
  projects,
  actions,
  onCopyUrl,
}: {
  title: string;
  projects: ZedRemoteProject[];
  actions: Actions;
  onCopyUrl: (project: ZedRemoteProject) => Promise<void>;
}) {
  return (
    <Panel>
      <CardHead title={title} detail={`${projects.length} 个项目`} />
      <CardContent>
        {projects.length ? (
          <div className="zed-remote-project-list">
            {projects.map((project) => (
              <div className="zed-remote-project-row" key={project.id}>
                <div className="zed-remote-project-main">
                  <div>
                    <strong>{project.label}</strong>
                    <span>{zedRemoteHostLabel(project)}</span>
                  </div>
                  <code>{project.path}</code>
                  <small>
                    {zedRemoteSourceLabel(project.source)}
                    {project.lastOpenedAtMs ? ` · ${formatTime(project.lastOpenedAtMs)}` : ""}
                  </small>
                </div>
                <div className="zed-remote-project-actions">
                  <Button onClick={() => void actions.openZedRemoteProject(project, "addToFocusedWorkspace")} size="sm">
                    <ExternalLink className="h-4 w-4" />
                    加入当前工作区
                  </Button>
                  <Button onClick={() => void actions.openZedRemoteProject(project, "reuseWindow")} size="sm" variant="outline">
                    复用窗口
                  </Button>
                  <Button onClick={() => void actions.openZedRemoteProject(project, "newWindow")} size="sm" variant="outline">
                    新窗口
                  </Button>
                  <Button onClick={() => void onCopyUrl(project)} size="icon" title="复制 ssh:// URL" variant="ghost">
                    <Copy className="h-4 w-4" />
                  </Button>
                  {project.source === "recent" ? (
                    <Button onClick={() => void actions.forgetZedRemoteProject(project)} size="icon" title="移除最近记录" variant="ghost">
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  ) : null}
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="empty">暂无项目。</div>
        )}
      </CardContent>
    </Panel>
  );
}

function UserScriptsScreen({ settings, market, actions }: { settings: SettingsResult | null; market: ScriptMarketResult | null; actions: Actions }) {
  const inventory = settings?.user_scripts;
  const scripts = inventory?.scripts ?? [];
  const marketScripts = market?.market.scripts ?? [];
  const installedCount = marketScripts.filter((script) => script.installed).length;
  return (
    <>
      <Panel>
        <CardHead title="脚本市场" detail={`${marketScripts.length} 个市场脚本，已安装 ${installedCount} 个，本地整体 ${inventory?.enabled === false ? "关闭" : "开启"}`} />
        <CardContent>
          <div className="metric-list">
            <Metric label="市场状态" value={market?.market.message ?? "尚未刷新"} />
            <Metric label="远程脚本" value={`${marketScripts.length} 个`} />
            <Metric label="已安装" value={`${installedCount} 个`} />
            <Metric label="本地整体" value={inventory?.enabled === false ? "关闭" : "开启"} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.refreshScriptMarket()}>
              <RefreshCw className="h-4 w-4" />
              刷新市场
            </Button>
            <Button onClick={() => void actions.openExternalUrl(SCRIPT_MARKET_REPOSITORY_URL)} variant="secondary">
              <ExternalLink className="h-4 w-4" />
              投稿
            </Button>
            <Button onClick={() => void actions.refreshCurrent()} variant="secondary">
              <RefreshCw className="h-4 w-4" />
              刷新本地
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="市场脚本" detail={market?.market.updatedAt ? `清单更新时间：${market.market.updatedAt}` : "从 GitHub 静态清单加载"} />
        <CardContent>
          {marketScripts.length ? (
            <div className="script-market-grid">
              {marketScripts.map((script) => (
                <MarketScriptCard key={script.id} script={script} actions={actions} />
              ))}
            </div>
          ) : (
            <div className="empty">{market?.status === "failed" ? market.message : "点击刷新市场加载远程脚本。"}</div>
          )}
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="本地脚本" detail="内置、手动和市场安装脚本；可在这里启停或删除用户脚本" />
        <CardContent>
          <div className="table">
            {scripts.length ? scripts.map((script) => <ScriptRow key={script.key} script={script} actions={actions} />) : <div className="empty">未发现用户脚本。</div>}
          </div>
        </CardContent>
      </Panel>
    </>
  );
}

function SessionsScreen({
  settings,
  form,
  sessions,
  providerSyncProgress,
  providerSyncTargets,
  selectedProviderSyncTarget,
  onFormChange,
  actions,
}: {
  settings: SettingsResult | null;
  form: BackendSettings;
  sessions: LocalSessionsResult | null;
  providerSyncProgress: ProviderSyncProgress;
  providerSyncTargets: ProviderSyncTargetsResult | null;
  selectedProviderSyncTarget: string;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const items = sessions?.sessions ?? [];
  const activeCount = items.filter((item) => !item.archived).length;
  const archivedCount = items.length - activeCount;
  return (
    <>
      <Panel>
        <CardHead title="会话管理" detail="读取 Codex 本地 state_5.sqlite，会删除数据库记录和对应 rollout 文件" />
        <CardContent>
          <div className="metric-list">
            <Metric label="会话总数" value={`${items.length} 个`} />
            <Metric label="未归档" value={`${activeCount} 个`} />
            <Metric label="已归档" value={`${archivedCount} 个`} />
            <Metric label="数据库" value={sessions?.dbPath ?? "~/.codex/state_5.sqlite"} />
          </div>
          <div className="form-row">
            <Field label="同步目标">
              <select
                className="select-input"
                disabled={providerSyncProgress.active || !(providerSyncTargets?.targets ?? []).length}
                value={selectedProviderSyncTarget}
                onChange={(event) => actions.setProviderSyncTarget(event.currentTarget.value)}
              >
                {(providerSyncTargets?.targets ?? []).map((target) => (
                  <option key={target.id} value={target.id}>
                    {target.id}（{providerSyncTargetLabel(target)}）
                  </option>
                ))}
                {!(providerSyncTargets?.targets ?? []).length ? <option value="">当前配置 provider</option> : null}
              </select>
            </Field>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.refreshLocalSessions()}>
              <RefreshCw className="h-4 w-4" />
              刷新会话
            </Button>
            <Button disabled={providerSyncProgress.active} onClick={() => void actions.syncProvidersNow()} variant="outline">
              <RefreshCw className="h-4 w-4" />
              {providerSyncProgress.active ? "正在修复…" : "立刻修复历史会话"}
            </Button>
          </Toolbar>
          <div className="provider-sync-progress" data-active={providerSyncProgress.active}>
            <div className="provider-sync-progress-head">
              <strong>{providerSyncProgress.active ? "正在修复历史会话" : "历史会话修复进度"}</strong>
              <span>{providerSyncProgress.percent}%</span>
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
          <div className="hint-line">
            <Info className="h-4 w-4" />
            <span>删除会创建本地备份；如果 Codex App 正在使用该会话，建议先关闭对应会话窗口再操作。</span>
          </div>
          <label className="switch-row">
            <input
              checked={form.providerSyncEnabled}
              onChange={(event) => onFormChange({ ...form, providerSyncEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>
              <strong>启动前自动修复历史会话</strong>
              <small>开启后，通过 {PRODUCT_NAME} 启动 Codex 前自动整理一次旧对话的归属标记。</small>
            </span>
          </label>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存自动修复设置</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="本地会话" detail={items.length ? "按更新时间倒序显示" : "点击刷新会话读取本地数据库"} />
        <CardContent>
          {items.length ? (
            <div className="session-list">
              {items.map((session) => (
                <div className="session-row" key={session.id}>
                  <div className="session-main">
                    <strong>{session.title || "未命名会话"}</strong>
                    <span>{session.id}</span>
                    <small>{session.cwd || "未记录项目路径"}</small>
                  </div>
                  <div className="session-meta">
                    <Badge status={session.archived ? "archived" : "ok"} />
                    <span>{session.modelProvider || "provider 未记录"}</span>
                    <span>{formatTime(session.updatedAtMs ?? 0)}</span>
                  </div>
                  <Button variant="outline" onClick={() => void actions.deleteLocalSession(session)}>
                    <Trash2 className="h-4 w-4" />
                    删除
                  </Button>
                </div>
              ))}
            </div>
          ) : (
            <div className="empty">未读取到本地会话，或当前 state_5.sqlite 不存在。</div>
          )}
        </CardContent>
      </Panel>
    </>
  );
}

function RecommendationsScreen({ ads, actions }: { ads: AdsResult | null; actions: Actions }) {
  const items = (ads?.ads ?? []).filter((ad) => !isExpiredAd(ad));
  const sponsors = items.filter((ad) => ad.type === "sponsor");
  const normal = items.filter((ad) => ad.type === "normal");
  return (
    <>
      <Panel>
        <CardHead title="推荐内容" detail="与 Codex 内插件菜单使用同一个远端广告源" />
        <CardContent>
          <div className="recommend-hero">
            <div>
              <strong>{ads ? `已加载 ${items.length} 条推荐` : "尚未加载推荐内容"}</strong>
              <span>内容来自 BigPizzaV3/Ad-List，分为赞助商推荐和普通推荐。</span>
            </div>
            <Button onClick={() => void actions.refreshAds()}>
              <RefreshCw className="h-4 w-4" />
              刷新推荐
            </Button>
          </div>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="赞助商推荐" detail={`${sponsors.length} 条`} />
        <CardContent>
          <AdGrid actions={actions} ads={sponsors} empty="暂无赞助商推荐。" />
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="普通推荐" detail={`${normal.length} 条`} />
        <CardContent>
          <AdGrid actions={actions} ads={normal} empty="暂无普通推荐。" />
        </CardContent>
      </Panel>
    </>
  );
}

function MaintenanceScreen({
  overview,
  watcher,
  settings,
  releaseReadiness,
  launchForm,
  onLaunchFormChange,
  removeOwnedData,
  onRemoveOwnedDataChange,
  actions,
}: {
  overview: OverviewResult | null;
  watcher: WatcherResult | null;
  settings: SettingsResult | null;
  releaseReadiness: ReleaseReadinessResult | null;
  launchForm: { appPath: string; debugPort: string; helperPort: string };
  onLaunchFormChange: (next: { appPath: string; debugPort: string; helperPort: string }) => void;
  removeOwnedData: boolean;
  onRemoveOwnedDataChange: (value: boolean) => void;
  actions: Actions;
}) {
  const savedCodexAppPath = settings?.settings.codexAppPath ?? "";
  const readinessStatus = !releaseReadiness
    ? "not_checked"
    : releaseReadiness.failures > 0
      ? "failed"
      : releaseReadiness.warnings > 0
        ? "warning"
        : "ok";
  return (
    <>
      <Panel>
        <CardHead title="检查与修复" detail="检查入口、Codex 应用和 Watcher 状态" />
        <CardContent>
          <div className="status-table">
            <StatusRow title="Codex 应用" status={overview?.codex_app.status} path={overview?.codex_app.path} />
            <StatusRow title="静默启动入口" status={overview?.silent_shortcut.status} path={overview?.silent_shortcut.path} />
            <StatusRow title="管理控制台入口" status={overview?.management_shortcut.status} path={overview?.management_shortcut.path} />
            <StatusRow title="Watcher 自动接管" status={watcher?.enabled ? "ok" : "disabled"} path={watcher?.disabled_flag} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.checkHealth()}>检查</Button>
            <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>修复快捷方式</Button>
            <Button variant="secondary" onClick={() => void actions.repairBackend()}>修复后端</Button>
            <Button variant="secondary" onClick={() => void actions.repairOfficialIsolation()}>修复原版隔离</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="发布前检查" detail="检查 DMG、签名、bundle id、原版 Codex 隔离和 Key 分发风险" />
        <CardContent>
          <div className="metric-list release-readiness-summary">
            <Metric label="发布状态" value={statusLabel(readinessStatus)} />
            <Metric label="失败项" value={String(releaseReadiness?.failures ?? 0)} />
            <Metric label="风险项" value={String(releaseReadiness?.warnings ?? 0)} />
            <Metric label="检查时间" value={releaseReadiness?.checkedAtMs ? formatTime(releaseReadiness.checkedAtMs) : "尚未检查"} />
          </div>
          <div className="status-table release-readiness-table">
            {releaseReadiness?.items.length ? (
              releaseReadiness.items.map((item) => <ReleaseReadinessRow item={item} key={item.id} />)
            ) : (
              <ReleaseReadinessRow
                item={{
                  id: "not_checked",
                  label: "发布前检查",
                  status: "not_checked",
                  message: "点击按钮后检查极义codex是否仍会影响原版 Codex，以及安装包是否为完整客户端。",
                  path: null,
                }}
              />
            )}
          </div>
          <Toolbar>
            <Button onClick={() => void actions.checkReleaseReadiness()}>
              <ShieldCheck className="h-4 w-4" />
              运行发布前检查
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="入口管理" detail="快捷方式写入系统实际桌面位置，不使用写死桌面路径" />
        <CardContent>
          <label className="check-row">
            <input checked={removeOwnedData} onChange={(event) => onRemoveOwnedDataChange(event.currentTarget.checked)} type="checkbox" />
            <span>卸载时移除 {PRODUCT_NAME} 托管数据</span>
          </label>
          <Toolbar>
            <Button onClick={() => void actions.installEntrypoints()}>安装入口</Button>
            <Button variant="secondary" onClick={() => void actions.uninstallEntrypoints()}>卸载入口</Button>
            <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>修复入口</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="自动接管" detail={`Watcher 用于保持 ${PRODUCT_NAME} 接管状态`} />
        <CardContent>
          <Toolbar>
            <Button variant="secondary" onClick={() => void actions.installWatcher()}>安装 watcher</Button>
            <Button variant="secondary" onClick={() => void actions.uninstallWatcher()}>移除 watcher</Button>
            <Button variant="secondary" onClick={() => void actions.enableWatcher()}>启用</Button>
            <Button variant="secondary" onClick={() => void actions.disableWatcher()}>禁用</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="Codex 应用路径" detail="免安装版或解包版只需要选择一次，之后静默启动会自动复用" />
        <CardContent>
          <div className="status-table">
            <StatusRow title="保存路径" status={savedCodexAppPath ? "ok" : "not_checked"} path={savedCodexAppPath || null} />
            <StatusRow title="当前识别" status={overview?.codex_app.status} path={overview?.codex_app.path} />
          </div>
          <Field label="保存的应用路径">
            <Input
              value={settings?.settings.codexAppPath ?? ""}
              placeholder="选择 JiyiCodexClient.app、Codex.exe、app 目录或解包目录"
              readOnly
            />
          </Field>
          <Toolbar>
            <Button onClick={() => void actions.chooseCodexAppPath("folder")}>选择应用目录</Button>
            <Button variant="secondary" onClick={() => void actions.chooseCodexAppPath("file")}>选择 Codex.exe</Button>
            <Button variant="secondary" onClick={() => void actions.clearCodexAppPath()}>清除保存路径</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="手动启动" detail="应用路径留空时使用已保存路径；没有保存路径时使用自动探测" />
        <CardContent>
          <Field label="应用路径覆盖">
            <Input
              value={launchForm.appPath}
              onChange={(event) => onLaunchFormChange({ ...launchForm, appPath: event.currentTarget.value })}
              placeholder={savedCodexAppPath || "例如 C:\\Program Files\\WindowsApps\\OpenAI.Codex...\\app"}
            />
          </Field>
          <div className="form-row">
            <Field label="Debug 端口">
              <Input
                value={launchForm.debugPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, debugPort: event.currentTarget.value })}
              />
            </Field>
            <Field label="Helper 端口">
              <Input
                value={launchForm.helperPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, helperPort: event.currentTarget.value })}
              />
            </Field>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.launch()}>启动 {PRODUCT_NAME}</Button>
            <Button variant="secondary" onClick={() => void actions.saveManualCodexAppPath()}>
              保存为默认路径
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
  logs,
  diagnostics,
  actions,
}: {
  overview: OverviewResult | null;
  update: UpdateResult | null;
  logs: LogsResult | null;
  diagnostics: DiagnosticsResult | null;
  actions: Actions;
}) {
  return (
    <>
      <Panel>
        <CardHead title={`关于 ${PRODUCT_NAME}`} detail="本地 Codex 增强、管理工具和安装包维护" />
        <CardContent>
          <div className="metric-list">
            <Metric label={`${PRODUCT_NAME} 版本`} value={overview?.current_version ?? update?.currentVersion ?? "-"} />
            <Metric label="Codex 版本" value={overview?.codex_version ?? "未检测到"} />
            <Metric label="项目地址" value="github.com/BigPizzaV3/CodexPlusPlus" />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.openExternalUrl("https://github.com/BigPizzaV3/CodexPlusPlus")} variant="secondary">
              <ExternalLink className="h-4 w-4" />
              打开项目主页
            </Button>
            <Button onClick={() => void actions.openExternalUrl("https://github.com/BigPizzaV3/CodexPlusPlus/issues")} variant="secondary">
              <ExternalLink className="h-4 w-4" />
              反馈问题
            </Button>
            <Button onClick={() => void actions.openExternalUrl("https://discord.gg/y96kX7A76v")} variant="secondary">
              <MessageCircle className="h-4 w-4" />
              Discord
            </Button>
            <Button onClick={() => void actions.openExternalUrl("https://t.me/CodexPlusPlus")} variant="secondary">
              <MessageCircle className="h-4 w-4" />
              Telegram
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="GitHub Release 更新" detail={`当前版本 ${overview?.current_version ?? update?.currentVersion ?? "-"}`} />
        <CardContent>
          <div className="metric-list">
            <Metric label="状态" value={update?.status ?? "not_checked"} />
            <Metric label="最新版本" value={update?.latestVersion ?? "未检查"} />
            <Metric label="资源" value={update?.assetName ?? "-"} />
            <Metric label="进度" value={`${update?.progress ?? 0}%`} />
          </div>
          <Textarea className="log-view" readOnly value={update?.releaseSummary || update?.message || "尚未检查 GitHub Release；更新会下载并启动安装包。"} />
          <Toolbar>
            <Button onClick={() => void actions.checkUpdate()}>检查更新</Button>
            <Button variant="secondary" onClick={() => void actions.performUpdate()}>下载并运行安装包</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <LogsPanel logs={logs} actions={actions} />
      <DiagnosticsPanel diagnostics={diagnostics} actions={actions} />
    </>
  );
}

function SettingsScreen({
  settings,
  theme,
  form,
  smsProvider,
  smsProviderForm,
  localBackend,
  managedProxy,
  onFormChange,
  onSmsProviderFormChange,
  actions,
}: {
  settings: SettingsResult | null;
  theme: Theme;
  form: BackendSettings;
  smsProvider: SmsProviderSettingsResult | null;
  smsProviderForm: SmsProviderForm;
  localBackend: LocalBackendStateResult | null;
  managedProxy: ManagedProxyRuntimeResult | null;
  onFormChange: (value: BackendSettings) => void;
  onSmsProviderFormChange: (value: SmsProviderForm) => void;
  actions: Actions;
}) {
  return (
    <>
      <Panel>
        <CardHead title="基础设置" detail={settings?.settings_path ?? ""} />
        <CardContent>
          <div className="theme-row">
            <div>
              <strong>界面主题</strong>
              <span>当前为{theme === "dark" ? "深色" : "浅色"}模式。</span>
            </div>
            <Button variant="secondary" onClick={actions.toggleTheme}>切换主题</Button>
          </div>
            <Field label="供应商测试模型">
              <Input
                value={form.relayTestModel}
                onChange={(event) => onFormChange({ ...form, relayTestModel: event.currentTarget.value })}
                placeholder="例如 qwen3.7-plus"
              />
            </Field>
          <label className="check-row">
            <input
              checked={form.cliWrapperEnabled}
              onChange={(event) => onFormChange({ ...form, cliWrapperEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>启用 Codex 命令包装器</span>
          </label>
          <div className="form-row">
            <Field label="包装器 Base URL">
              <Input
                value={form.cliWrapperBaseUrl}
                onChange={(event) => onFormChange({ ...form, cliWrapperBaseUrl: event.currentTarget.value })}
              />
            </Field>
            <Field label="API Key 环境变量">
              <Input
                value={form.cliWrapperApiKeyEnv}
                onChange={(event) => onFormChange({ ...form, cliWrapperApiKeyEnv: event.currentTarget.value })}
              />
            </Field>
          </div>
          <Field label="API Key">
            <Input
              type="password"
              value={form.cliWrapperApiKey}
              onChange={(event) => onFormChange({ ...form, cliWrapperApiKey: event.currentTarget.value })}
            />
          </Field>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存设置</Button>
            <Button variant="secondary" onClick={() => void actions.resetSettings()}>
              重置设置
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="腾讯云短信" detail={smsProvider?.settingsPath ?? "极义本地短信配置"} />
        <CardContent>
          <div className="metric-list">
            <Metric label="状态" value={smsProvider?.smsConfig.configured ? "参数完整" : "参数未完整"} />
            <Metric label="发送模式" value={smsProvider?.smsConfig.dryRun ? "本地干跑" : "腾讯云"} />
            <Metric label="密钥来源" value={formatSmsSecretSource(smsProvider?.smsConfig)} />
            <Metric label="SecretId" value={smsProvider?.smsConfig.secretIdSet ? "已配置" : "未配置"} />
            <Metric label="SecretKey" value={smsProvider?.smsConfig.secretKeySet ? "已配置" : "未配置"} />
            <Metric label="参数顺序" value={smsProvider?.settings.templateParamMode ?? "code_ttl"} />
          </div>
          <div className="form-row">
            <Field label="短信区域">
              <Input
                value={smsProviderForm.region}
                onChange={(event) => onSmsProviderFormChange({ ...smsProviderForm, region: event.currentTarget.value })}
                placeholder="ap-guangzhou"
              />
            </Field>
            <Field label="SmsSdkAppId">
              <Input
                value={smsProviderForm.appId}
                onChange={(event) => onSmsProviderFormChange({ ...smsProviderForm, appId: event.currentTarget.value })}
                placeholder="腾讯云短信应用 ID"
              />
            </Field>
          </div>
          <div className="form-row">
            <Field label="短信签名">
              <Input
                value={smsProviderForm.signName}
                onChange={(event) => onSmsProviderFormChange({ ...smsProviderForm, signName: event.currentTarget.value })}
                placeholder="签名内容"
              />
            </Field>
            <Field label="模板 ID">
              <Input
                value={smsProviderForm.templateId}
                onChange={(event) => onSmsProviderFormChange({ ...smsProviderForm, templateId: event.currentTarget.value })}
                placeholder="TemplateId"
              />
            </Field>
          </div>
          <div className="form-row">
            <Field label="验证码有效期">
              <Input
                min={1}
                max={60}
                type="number"
                value={String(smsProviderForm.ttlMinutes)}
                onChange={(event) =>
                  onSmsProviderFormChange({ ...smsProviderForm, ttlMinutes: numberOrDefault(event.currentTarget.value, 10) })
                }
              />
            </Field>
            <Field label="模板参数">
              <select
                className="select-input"
                value={smsProviderForm.templateParamMode}
                onChange={(event) => onSmsProviderFormChange({ ...smsProviderForm, templateParamMode: event.currentTarget.value })}
              >
                <option value="code_ttl">验证码 + 有效期</option>
                <option value="code">仅验证码</option>
                <option value="ttl_code">有效期 + 验证码</option>
              </select>
            </Field>
          </div>
          <div className="form-row">
            <Field label="SecretId">
              <Input
                type="password"
                value={smsProviderForm.secretId}
                onChange={(event) => onSmsProviderFormChange({ ...smsProviderForm, secretId: event.currentTarget.value })}
                placeholder={smsProvider?.smsConfig.secretIdSet ? smsProvider.secretIdRef : "保存后写入极义钥匙串"}
              />
            </Field>
            <Field label="SecretKey">
              <Input
                type="password"
                value={smsProviderForm.secretKey}
                onChange={(event) => onSmsProviderFormChange({ ...smsProviderForm, secretKey: event.currentTarget.value })}
                placeholder={smsProvider?.smsConfig.secretKeySet ? smsProvider.secretKeyRef : "保存后写入极义钥匙串"}
              />
            </Field>
          </div>
          <label className="check-row">
            <input
              checked={smsProviderForm.dryRun}
              onChange={(event) => onSmsProviderFormChange({ ...smsProviderForm, dryRun: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>保持本地干跑模式</span>
          </label>
          <p className="field-hint">
            SecretId 和 SecretKey 只写入极义 macOS 钥匙串；留空保存会保留已有密钥。关闭干跑且参数完整后才会发送真实短信。
          </p>
          <Toolbar>
            <Button onClick={() => void actions.saveSmsProviderSettings()}>
              <Save className="h-4 w-4" />
              保存短信配置
            </Button>
            <Button variant="secondary" onClick={() => void actions.refreshSmsProviderSettings()}>
              <RefreshCw className="h-4 w-4" />
              刷新短信配置
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="极义账号服务端" detail="国产账号体系预留接口；不使用 ChatGPT 登录态。" />
        <CardContent>
          <div className="metric-list">
            <Metric label="本地后端库" value={localBackend?.initialized ? "已初始化" : "未读取"} />
            <Metric label="同步批次" value={String(localBackend?.batchCount ?? 0)} />
            <Metric label="承接用户" value={String(localBackend?.userCount ?? 0)} />
            <Metric label="封禁用户" value={String(localBackend?.blockedUserCount ?? 0)} />
            <Metric label="承接团队" value={String(localBackend?.teamCount ?? 0)} />
            <Metric label="团队成员" value={String(localBackend?.teamMemberCount ?? 0)} />
            <Metric label="续费记录" value={String(localBackend?.billingRenewalCount ?? 0)} />
            <Metric label="支付事件" value={String(localBackend?.billingPaymentEventCount ?? 0)} />
            <Metric label="审计事件" value={String(localBackend?.auditEventCount ?? 0)} />
            <Metric label="服务端 session" value={String(localBackend?.sessionCount ?? 0)} />
            <Metric label="有效 session" value={String(localBackend?.activeSessionCount ?? 0)} />
            <Metric label="已吊销 session" value={String(localBackend?.revokedSessionCount ?? 0)} />
            <Metric label="最后同步" value={localBackend?.lastSyncedAtMs ? formatTime(localBackend.lastSyncedAtMs) : "尚未同步"} />
            <Metric label="最近审计" value={localBackend?.lastAuditEventAtMs ? formatTime(localBackend.lastAuditEventAtMs) : "尚未记录"} />
            <Metric label="最近续费" value={localBackend?.lastBillingRenewalAtMs ? formatTime(localBackend.lastBillingRenewalAtMs) : "尚未记录"} />
            <Metric
              label="最近支付回调"
              value={localBackend?.lastBillingPaymentEventAtMs ? formatTime(localBackend.lastBillingPaymentEventAtMs) : "尚未记录"}
            />
            <Metric
              label="最近访问控制"
              value={localBackend?.lastUserAccessUpdatedAtMs ? formatTime(localBackend.lastUserAccessUpdatedAtMs) : "尚未变更"}
            />
            <Metric label="最近签发" value={localBackend?.lastSessionIssuedAtMs ? formatTime(localBackend.lastSessionIssuedAtMs) : "尚未签发"} />
            <Metric label="最近吊销" value={localBackend?.lastSessionRevokedAtMs ? formatTime(localBackend.lastSessionRevokedAtMs) : "尚未吊销"} />
          </div>
          <Field label="本地后端数据库">
            <code>{localBackend?.dbPath ?? "等待读取本地账号服务端库"}</code>
          </Field>
          <Field label="同步 Endpoint">
            <Input
              value={form.jiyiIdentitySyncEndpoint}
              onChange={(event) => onFormChange({ ...form, jiyiIdentitySyncEndpoint: event.currentTarget.value })}
              placeholder="https://api.example.com/jiyi/codex/identity/sync"
            />
          </Field>
          <Field label="同步 API Key">
            <Input
              type="password"
              value={form.jiyiIdentitySyncApiKey}
              onChange={(event) => onFormChange({ ...form, jiyiIdentitySyncApiKey: event.currentTarget.value })}
              placeholder={form.jiyiIdentitySyncApiKey.startsWith("jiyi-keychain:") ? "已保存到极义钥匙串" : ""}
            />
          </Field>
          <p className="field-hint">
            请求体只包含脱敏手机号、设备、本地套餐和用量摘要；同步 API Key 会写入极义自己的 macOS 钥匙串。
          </p>
          <label className="check-row">
            <input
              checked={form.jiyiManagedProxyEnabled}
              onChange={(event) => onFormChange({ ...form, jiyiManagedProxyEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>启用极义托管代理</span>
          </label>
          <Field label="托管代理 Endpoint">
            <Input
              value={form.jiyiManagedProxyEndpoint}
              onChange={(event) => onFormChange({ ...form, jiyiManagedProxyEndpoint: event.currentTarget.value })}
              placeholder="https://api.example.com/v1"
            />
          </Field>
          <p className="field-hint">
            开启后，内置 Codex 仍只连接本机代理；本机 helper 使用极义后端 session token 转发到托管代理，不把百炼或中转站主 key 写进客户端配置。
          </p>
          <div className="metric-list">
            <Metric label="本地托管代理" value={managedProxy?.running ? "运行中" : "未运行"} />
            <Metric label="PID" value={managedProxy?.pid ? String(managedProxy.pid) : "-"} />
            <Metric
              label="健康检查"
              value={
                managedProxy?.healthHttpStatus
                  ? `${managedProxy.healthStatus} / HTTP ${managedProxy.healthHttpStatus}`
                  : managedProxy?.healthStatus ?? "未检查"
              }
            />
            <Metric label="监听地址" value={managedProxy?.listenAddr ?? "127.0.0.1:57421"} />
            <Metric label="上游 Key" value={managedProxy?.upstreamKeyConfigured ? "已配置" : "未配置"} />
            <Metric label="同步 Key" value={managedProxy?.identitySyncKeyConfigured ? "已配置" : "未配置"} />
            <Metric label="管理 Key" value={managedProxy?.adminKeyConfigured ? "已配置" : "未配置"} />
            <Metric label="用户只读 Key" value={managedProxy?.userReadKeyConfigured ? "已配置" : "未配置"} />
            <Metric label="计费 Key" value={managedProxy?.billingKeyConfigured ? "已配置" : "未配置"} />
            <Metric label="支付回调 Key" value={managedProxy?.paymentWebhookKeyConfigured ? "已配置" : "未配置"} />
            <Metric label="通用支付验签" value={managedProxy?.paymentWebhookSignatureConfigured ? "已配置" : "未配置"} />
            <Metric label="支付宝验签" value={managedProxy?.paymentWebhookAlipaySignatureConfigured ? "已配置" : "未配置"} />
            <Metric label="微信验签" value={managedProxy?.paymentWebhookWechatpaySignatureConfigured ? "已配置" : "未配置"} />
            <Metric label="风控 Key" value={managedProxy?.accessKeyConfigured ? "已配置" : "未配置"} />
            <Metric label="审计 Key" value={managedProxy?.auditKeyConfigured ? "已配置" : "未配置"} />
          </div>
          <Field label="本地托管代理 Endpoint">
            <code>{managedProxy?.endpoint ?? "http://127.0.0.1:57421"}</code>
          </Field>
          <Field label="本地托管代理上游">
            <code>{managedProxy?.upstreamBaseUrl ?? APIMART_FALLBACK_BASE_URL}</code>
          </Field>
          <Field label="托管代理后端库">
            <code>{managedProxy?.backendDbPath ?? "~/.codex-session-delete/jiyi-codex-local-backend.sqlite"}</code>
          </Field>
          <Field label="本地托管代理日志">
            <code>{managedProxy?.logPath ?? "~/.codex-session-delete/jiyi-managed-proxy.log"}</code>
          </Field>
          <Toolbar>
            <Button onClick={() => void actions.applyIdentitySyncLocally()}>
              <Database className="h-4 w-4" />
              同步到本地后端
            </Button>
            <Button onClick={() => void actions.syncIdentityToService()}>
              <Network className="h-4 w-4" />
              同步到服务端
            </Button>
            <Button variant="secondary" onClick={() => void actions.refreshLocalBackendState()}>
              <RefreshCw className="h-4 w-4" />
              刷新本地后端
            </Button>
            <Button variant="secondary" onClick={() => void actions.refreshManagedProxy()}>
              <RefreshCw className="h-4 w-4" />
              检查托管代理
            </Button>
            <Button onClick={() => void actions.startManagedProxy()}>
              <Power className="h-4 w-4" />
              启动本地托管代理
            </Button>
            <Button variant="secondary" onClick={() => void actions.stopManagedProxy()}>
              <PowerOff className="h-4 w-4" />
              停止本地托管代理
            </Button>
            <Button onClick={() => void actions.prepareIdentitySyncRequest()}>
              <FileText className="h-4 w-4" />
              生成同步请求包
            </Button>
            <Button variant="secondary" onClick={() => void actions.saveSettings()}>
              保存设置
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="Codex 启动参数" detail="启动 Codex App 时追加到默认 CDP 参数后。留空则保持默认启动行为。" />
        <CardContent>
          <Field label="额外参数">
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
          </Field>
          <p className="field-hint">每行一个参数，例如 --force_high_performance_gpu。不需要填写 open 或 --args。</p>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存设置</Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function LogsPanel({ logs, actions }: { logs: LogsResult | null; actions: Actions }) {
  const lines = splitLogLines(logs?.text ?? "");
  return (
    <Panel>
      <CardHead title="最近日志" detail={logs?.path ?? ""} />
      <CardContent>
        <div className="log-lines">
          {lines.length ? (
            lines.map((line, index) => (
              <div className="log-line" key={`${index}-${line.slice(0, 12)}`}>
                <span>{index + 1}</span>
                <code>{line || " "}</code>
              </div>
            ))
          ) : (
            <div className="empty">暂无日志。</div>
          )}
        </div>
        <Toolbar>
          <Button onClick={() => void actions.refreshLogs()}>刷新</Button>
          <Button variant="secondary" onClick={() => void actions.copyLogs()}>
            复制
          </Button>
        </Toolbar>
      </CardContent>
    </Panel>
  );
}

function DiagnosticsPanel({ diagnostics, actions }: { diagnostics: DiagnosticsResult | null; actions: Actions }) {
  return (
    <Panel>
      <CardHead title="诊断报告" detail="包含版本、路径、设置和平台信息" />
      <CardContent>
        <Textarea className="log-view tall" readOnly value={diagnostics?.report ?? "尚未生成诊断报告。"} />
        <Toolbar>
          <Button onClick={() => void actions.refreshDiagnostics()}>重新生成</Button>
          <Button variant="secondary" onClick={() => void actions.copyDiagnostics()}>
            复制报告
          </Button>
        </Toolbar>
      </CardContent>
    </Panel>
  );
}

function RelayProfileList({
  form,
  onFormChange,
  onEdit,
  disabled = false,
  actions,
}: {
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  onEdit: (id: string) => void;
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
      <SortableContext items={form.relayProfiles.map((profile) => profile.id)} strategy={verticalListSortingStrategy}>
        <div className="relay-profile-list">
          {form.relayProfiles.map((profile, index) => (
            <SortableRelayProfileCard
              actions={actions}
              form={form}
              index={index}
              key={profile.id}
              onEdit={onEdit}
              onFormChange={onFormChange}
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
  disabled = false,
  actions,
}: {
  form: BackendSettings;
  profile: RelayProfile;
  index: number;
  onFormChange: (value: BackendSettings) => void;
  onEdit: (id: string) => void;
  disabled?: boolean;
  actions: Actions;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: profile.id });
  const active = profile.id === form.activeRelayId;
  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <div
      className={`relay-profile-card ${active ? "active" : ""} ${isDragging ? "dragging" : ""}`}
      data-relay-profile-id={profile.id}
      key={profile.id}
      onKeyDown={(event) => {
        if (event.key === "Enter") onEdit(profile.id);
      }}
      ref={setNodeRef}
      style={style}
      tabIndex={0}
    >
      <button
        aria-label="拖动排序"
        className="relay-drag"
        title="拖动排序"
        type="button"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="h-4 w-4" />
      </button>
      <span className="relay-index" title={profile.name || "未命名供应商"}>
        {providerInitial(profile.name)}
      </span>
      <span className="relay-summary">
        <strong>{profile.name || "未命名供应商"}</strong>
        <small>{relayProfileSourceLabel(profile)} · {relayModeLabel(profile.relayMode)} · {relayProtocolLabel(profile.protocol)} · {relayProfileConfigBrief(profile)}</small>
      </span>
      <span className="relay-card-actions">
        <Button
          className={`relay-use-button ${active ? "active" : ""}`}
          disabled={disabled}
          onClick={(event) => {
            event.stopPropagation();
            if (disabled) return;
            const previousActiveRelayId = form.activeRelayId;
            const next = syncLegacyRelayFields({ ...form, activeRelayId: profile.id });
            void actions.switchRelayProfile(next, previousActiveRelayId);
          }}
          size="sm"
          title={disabled ? "供应商配置总开关已关闭" : active ? "当前正在使用" : "设为当前"}
          variant={active ? "secondary" : "outline"}
        >
          <CheckCircle2 className="h-4 w-4" />
          {active ? "使用中" : "使用"}
        </Button>
        <span className="relay-card-extra">
          <Button
            onClick={(event) => {
              event.stopPropagation();
              void actions.testRelayProfile(profile);
            }}
            size="icon"
            title="发送 hi 测试"
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
            title="编辑"
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
            title="复制"
            variant="ghost"
          >
            <Copy className="h-4 w-4" />
          </Button>
          <Button
            disabled={form.relayProfiles.length <= 1}
            onClick={(event) => {
              event.stopPropagation();
              onFormChange(removeRelayProfile(form, profile.id));
            }}
            size="icon"
            title="删除供应商"
            variant="ghost"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </span>
      </span>
    </div>
  );
}

function MarketScriptCard({ script, actions }: { script: ScriptMarketItem; actions: Actions }) {
  const status = script.updateAvailable ? "可更新" : script.installed ? `已安装 ${script.installedVersion}` : "未安装";
  return (
    <div className="script-market-card">
      <div className="script-market-title">
        <div>
          <strong>{script.name}</strong>
          <span>{script.author || "未知作者"}</span>
        </div>
        <UiBadge variant={script.updateAvailable ? "default" : script.installed ? "secondary" : "outline"}>{status}</UiBadge>
      </div>
      <p className="script-market-description">{script.description || "暂无描述。"}</p>
      <div className="script-market-tags">
        <span className="script-market-tag">v{script.version}</span>
        {script.tags.map((tag) => (
          <span className="script-market-tag" key={tag}>{tag}</span>
        ))}
      </div>
      <div className="script-market-actions">
        <Button onClick={() => void actions.installMarketScript(script.id)} size="sm">
          <Download className="h-4 w-4" />
          {script.updateAvailable ? "更新" : script.installed ? "重新安装" : "安装"}
        </Button>
        {script.homepage ? (
          <Button onClick={() => void actions.openExternalUrl(script.homepage)} size="sm" variant="secondary">
            <ExternalLink className="h-4 w-4" />
            主页
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
  onBack,
  onFormChange,
  onSaved,
  actions,
}: {
  profile: RelayProfile;
  relayFiles: RelayFilesResult | null;
  form: BackendSettings;
  isNew?: boolean;
  onBack: () => void;
  onFormChange: (value: BackendSettings, preserveLinkedProfiles?: boolean) => void | Promise<void>;
  onSaved?: () => void;
  actions: Actions;
}) {
  const [draft, setDraft] = useState<RelayProfile>(profile);
  const isActive = !isNew && profile.id === form.activeRelayId;
  useEffect(() => {
    setDraft(
      deriveRelayProfileFromFiles(
        isActive && relayFiles
          ? {
            ...profile,
            configContents: relayFiles.configContents,
            authContents: relayFiles.authContents,
          }
          : profile,
      ),
    );
  }, [profile.id, isActive, isNew, relayFiles?.configContents, relayFiles?.authContents]);
  const saveDraft = async () => {
    const normalizedDraft = deriveRelayProfileFromFiles(draft);
    const next = isNew
      ? addRelayProfile(form, normalizedDraft)
      : updateRelayProfile(form, profile.id, normalizedDraft);
    await onFormChange(next, !!normalizedDraft.linkedCcsProviderId);
    if (isActive) {
      await actions.saveRelayFile(
        "config",
        effectiveRelayConfigPreview(normalizedDraft, form, normalizedDraft),
        true,
      );
      await actions.saveRelayFile("auth", normalizedDraft.authContents, true);
    }
    onSaved?.();
  };
  const switchDraft = () => {
    if (isNew || !form.relayProfilesEnabled) return;
    const normalizedDraft = deriveRelayProfileFromFiles(draft);
    const previousActiveRelayId = form.activeRelayId;
    const next = syncLegacyRelayFields({
      ...form,
      relayProfiles: form.relayProfiles.map((item) => (item.id === profile.id ? normalizedDraft : item)),
      activeRelayId: profile.id,
    });
    void actions.switchRelayProfile(next, previousActiveRelayId);
  };
  return (
    <div className="relay-detail-page" key={profile.id}>
      <div className="relay-detail-sticky">
        <Toolbar>
          <Button onClick={onBack} variant="secondary">
            <ArrowLeft className="h-4 w-4" />
            返回列表
          </Button>
          <Button onClick={() => void saveDraft()}>
            <Save className="h-4 w-4" />
            保存
          </Button>
        </Toolbar>
      </div>
        <RelayProfileEditor profile={draft} form={form} isNew={isNew} onProfileChange={setDraft} onSwitch={switchDraft} actions={actions} />
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
      <CardHead title="Codex 工具与插件" detail="独立管理 Codex 的 MCP、Skills、Plugins；切换任意供应商都会带上。" />
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

function RelayProfileEditor({
  profile,
  form,
  isNew = false,
  onProfileChange,
  onSwitch,
  actions,
}: {
  profile: RelayProfile;
  form: BackendSettings;
  isNew?: boolean;
  onProfileChange: (value: RelayProfile) => void;
  onSwitch: () => void;
  actions: Actions;
}) {
  const showApiFields = true;
  const [showAdvanced, setShowAdvanced] = useState(false);
  const updateDraft = (patch: Partial<RelayProfile>) => {
    onProfileChange(applyRelayProfilePatchToFiles(profile, patch, { allowGenerateFiles: isNew }));
  };
  return (
    <div className="relay-profile-editor">
      <div className="relay-editor-head">
        <div>
          <strong>{profile.name || "未命名供应商"}</strong>
          <span>{relayProfileEditorStatus(profile, form, isNew)}</span>
        </div>
        {isNew ? null : (
          <Button
            disabled={!form.relayProfilesEnabled}
            onClick={onSwitch}
            title={!form.relayProfilesEnabled ? "供应商配置总开关已关闭" : undefined}
            variant={profile.id === form.activeRelayId ? "secondary" : "default"}
          >
            {profile.id === form.activeRelayId ? "使用中" : "设为当前"}
          </Button>
        )}
      </div>
      {isNew ? (
        <ProviderPresetSelector
          onSelect={(patch: PresetPatch) => {
            updateDraft(patch as unknown as Partial<RelayProfile>);
          }}
        />
      ) : null}
      <div className="relay-fields">
        <Field className="relay-field-name" label="名称">
          <Input
            value={profile.name}
            onChange={(event) => updateDraft({ name: event.currentTarget.value })}
          />
        </Field>
        <Field className="relay-field-mode" label="接入模式">
          <select
            className="field-select"
            value={profile.relayMode === "pureApi" ? "pureApi" : profile.relayMode}
            onChange={(event) => {
              const relayMode = event.currentTarget.value as RelayMode;
              updateDraft({ relayMode, officialMixApiKey: false });
            }}
          >
            <option value="pureApi">极义 / 百炼纯 API</option>
            {profile.relayMode !== "pureApi" ? (
              <option disabled value={profile.relayMode}>
                历史官方模式（已禁用）
              </option>
            ) : null}
          </select>
        </Field>
            <Field className="relay-field-config-model" label="配置模型">
              <Input
                value={profile.model}
                onChange={(event) => updateDraft({ model: event.currentTarget.value })}
                placeholder="写入 config.toml 的 model 字段，例如 qwen3.7-plus"
              />
            </Field>
        <Field className="relay-field-goals" label="Codex 目标">
          <label className="inline-check">
            <input
              checked={configHasCodexGoalsFeature(profile.configContents)}
              onChange={(event) =>
                updateDraft({
                  configContents: setCodexGoalsFeatureInConfig(profile.configContents, event.currentTarget.checked),
                })
              }
              type="checkbox"
            />
            <span>启用目标功能</span>
          </label>
        </Field>
        <div className="relay-advanced-toggle">
          <Button
            aria-expanded={showAdvanced}
            onClick={() => setShowAdvanced((current) => !current)}
            size="sm"
            type="button"
            variant="secondary"
          >
            <Settings className="h-4 w-4" />
            更多选项
          </Button>
        </div>
        {showAdvanced ? (
          <div className="relay-advanced-fields">
            <Field className="relay-field-test-model" label="测试模型">
              <Input
                value={profile.testModel}
                onChange={(event) => updateDraft({ testModel: event.currentTarget.value })}
                placeholder={`留空使用默认：${form.relayTestModel || defaultSettings.relayTestModel}`}
              />
            </Field>
            <Field className="relay-field-context-window" label="上下文大小">
              <Input
                inputMode="numeric"
                value={profile.contextWindow}
                onChange={(event) => updateDraft({ contextWindow: event.currentTarget.value.replace(/[^\d]/g, "") })}
                placeholder="留空不改写，例如 200000"
              />
            </Field>
            <Field className="relay-field-auto-compact" label="压缩上下文大小">
              <Input
                inputMode="numeric"
                value={profile.autoCompactLimit}
                onChange={(event) => updateDraft({ autoCompactLimit: event.currentTarget.value.replace(/[^\d]/g, "") })}
                placeholder="留空不改写，例如 160000"
              />
            </Field>
          </div>
        ) : null}
        {profile.relayMode !== "pureApi" ? (
          <div className="hint-line relay-protocol-hint">
            <ShieldCheck className="h-4 w-4" />
            <span>极义codex 不使用 ChatGPT 官方账号体系；请把此供应商改为纯 API 后再保存或切换。</span>
          </div>
        ) : null}
        {showApiFields ? (
          <div className="relay-api-fields">
            <Field className="relay-field-base-url" label="Base URL">
              <Input
                value={profile.baseUrl}
                onChange={(event) => updateDraft({ baseUrl: event.currentTarget.value })}
                placeholder="填写中转服务 Base URL"
              />
            </Field>
            <Field className="relay-field-key" label="Key">
              <Input
                type="password"
                value={profile.apiKey}
                onChange={(event) => updateDraft({ apiKey: event.currentTarget.value })}
                placeholder="输入中转服务的 API Key"
              />
            </Field>
            <Field className="relay-field-protocol" label="上游协议">
              <div className="protocol-options">
                <button
                  className={`protocol-option ${profile.protocol === "responses" ? "active" : ""}`}
                  onClick={() => updateDraft({ protocol: "responses" })}
                  type="button"
                >
                  Responses API
                </button>
                <button
                  className={`protocol-option ${profile.protocol === "chatCompletions" ? "active" : ""}`}
                  onClick={() => updateDraft({ protocol: "chatCompletions" })}
                  type="button"
                >
                  Chat Completions
                </button>
              </div>
            </Field>
          </div>
        ) : null}
        {showApiFields ? (
          <Field className="relay-field-model-list" label="模型列表">
            <div className="relay-model-list-tools">
              <Textarea
                value={profile.modelList}
                onChange={(event) => updateDraft({ modelList: event.currentTarget.value })}
                placeholder="每行一个模型，例如 qwen3-coder"
              />
              <Button
                onClick={async () => {
                  const models = await actions.fetchRelayProfileModels(profile);
                  if (models?.length) updateDraft({ modelList: models.join("\n") });
                }}
                size="sm"
                type="button"
                variant="secondary"
              >
                <Download className="h-4 w-4" />
                从上游获取
              </Button>
            </div>
          </Field>
        ) : null}
        {showApiFields ? (
          <Field className="relay-field-user-agent" label="User-Agent">
            <Input
              value={profile.userAgent}
              onChange={(event) => updateDraft({ userAgent: event.currentTarget.value })}
              placeholder="留空使用默认值"
            />
          </Field>
        ) : null}
      </div>
      {showApiFields && profile.protocol === "chatCompletions" ? (
        <div className="hint-line relay-protocol-hint">
          <MessageCircle className="h-4 w-4" />
          <span>此上游会通过本地 127.0.0.1:57321 转成 Responses API，需要从 {PRODUCT_NAME} 启动 Codex。</span>
        </div>
      ) : null}
      <div className="hint-line relay-protocol-hint">
        <ShieldCheck className="h-4 w-4" />
        <span>{relayProfileModeHelp(profile)}</span>
      </div>
      {profile.linkedCcsProviderId ? (
        <div className="hint-line relay-protocol-hint">
          <Link2 className="h-4 w-4" />
          <span>
            此供应商联动自 cc-switch：{profile.linkedCcsProviderId}。开启“保存时回写 cc-switch”后，本页保存会同步修改 cc-switch 数据库中的同一供应商。
          </span>
        </div>
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
  const visibleEntries = contextEntriesByKind(entries, activeKind);
  const label = contextKindLabel(activeKind);

  const saveEntry = async (kind: ContextKind, id: string, tomlBody: string) => {
    const next = await actions.upsertContextEntry(form, kind, id, tomlBody);
    if (!next) return;
    onFormChange(next);
    setEditor(null);
  };

  const toggleContextEntryEnabled = async (entry: CodexContextEntry) => {
    const nextBody = setContextEntryEnabled(entry.tomlBody, !entry.enabled);
    const next = await actions.upsertContextEntry(form, entry.kind, entry.id, nextBody);
    if (!next) return;
    onFormChange(next);
    const syncResult = await actions.syncLiveContextEntries(next, true);
    if (syncResult && isSuccessStatus(syncResult.status)) {
      void actions.refreshRelayFiles();
    }
  };

  const deleteEntry = async (entry: CodexContextEntry) => {
    const next = await actions.deleteContextEntry(form, entry.kind, entry.id);
    if (!next) return;
    onFormChange(next);
  };

  return (
    <div className="relay-context-panel">
      <div className="relay-context-head">
        <div>
          <strong>Codex 工具与插件</strong>
          <span>MCP、Skills、Plugins 作为全局配置独立管理，切换任意供应商都会合并。</span>
        </div>
        <div className="relay-context-head-actions">
          <Button onClick={() => setEditor({ kind: activeKind })} size="sm" variant="secondary">
            <Plus className="h-4 w-4" />
            新增{label}
          </Button>
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
            <small>{contextEntriesByKind(entries, option.kind).length}</small>
          </button>
        ))}
      </div>
      <div className="relay-context-summary">
        当前共有 {visibleEntries.length} 个{label}；这些条目独立于供应商保存，会写入所有供应商切换后的 config.toml。
      </div>
      <div className="relay-context-list">
        {visibleEntries.length ? (
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
                  title={entry.enabled ? "禁用此扩展项" : "启用此扩展项"}
                  type="button"
                >
                  <span className="context-switch-track" aria-hidden="true">
                    <span className="context-switch-thumb" />
                  </span>
                </button>
                <Button onClick={() => setEditor({ kind: entry.kind, entry })} size="icon" title="编辑扩展项" variant="ghost">
                  <Edit3 className="h-4 w-4" />
                </Button>
                <Button
                  className="relay-context-delete"
                  onClick={() => void deleteEntry(entry)}
                  size="icon"
                  title="删除扩展项"
                  variant="ghost"
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ))
        ) : (
          <div className="empty">暂无{label}，可以从通用配置文件或这里新增。</div>
        )}
      </div>
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
        <Field label="类型">
          <select
            className="field-select"
            disabled={!!entry}
            value={draftKind}
            onChange={(event) => setDraftKind(event.currentTarget.value as ContextKind)}
          >
            {contextKindOptions.map((option) => (
              <option key={option.kind} value={option.kind}>{option.label}</option>
            ))}
          </select>
        </Field>
        <Field label="ID">
          <Input
            disabled={!!entry}
            value={id}
            onChange={(event) => setId(event.currentTarget.value.trim())}
            placeholder="例如 context7"
          />
        </Field>
      </div>
      <Field label="TOML 配置体">
        <Textarea
          className="context-editor-textarea"
          value={tomlBody}
          onChange={(event) => setTomlBody(event.currentTarget.value)}
          placeholder={'只填写表头下面的内容，例如：\ncommand = "npx"\nargs = ["-y", "@upstash/context7-mcp"]'}
          spellCheck={false}
        />
      </Field>
      <Toolbar>
        <Button disabled={!canSave} onClick={() => onSave(draftKind, id.trim(), tomlBody)} size="sm">
          <Save className="h-4 w-4" />
          保存扩展项
        </Button>
        <Button onClick={onCancel} size="sm" variant="secondary">取消</Button>
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
      <div className="relay-file-panel">
        <div className="relay-file-head">
          <div>
            <strong>config.toml 预览</strong>
            <span>{isActive ? "当前供应商切换后会写入的预览；上下文开关变化会立即反映" : "切换到此供应商时会写入的预览；上下文开关变化会立即反映"}</span>
          </div>
        </div>
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
            }));
          }}
        />
      </div>
      <div className="relay-file-panel">
        <div className="relay-file-head">
          <div>
            <strong>通用配置文件</strong>
            <span>只保留非 MCP、Skills、Plugins 的跨供应商配置；工具与插件在独立页面管理。</span>
          </div>
          <Button
            onClick={async () => {
              const extracted = await actions.extractRelayCommonConfig(profile.configContents || "");
              if (!extracted) return;
              const split = splitContextConfigText(extracted.commonConfigContents || "");
              if (!split.common.trim() && !split.context.trim()) {
                await actions.showMessage("通用配置文件", "当前供应商 config.toml 里没有可提取的通用配置。", "failed");
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
            提取当前供应商配置
          </Button>
        </div>
        <SyncedTextarea
          className="relay-file-textarea"
          value={form.relayCommonConfigContents}
          onValueChange={(value) => onFormChange({ ...form, relayCommonConfigContents: value })}
        />
      </div>
      <div className="relay-file-panel">
        <div className="relay-file-head">
          <div>
            <strong>auth.json</strong>
            <span>{isActive ? "当前使用中：打开时从 ~/.codex/auth.json 回填，保存后会作为此供应商 auth 存档" : "切换到此供应商时会写入 ~/.codex/auth.json"}</span>
          </div>
        </div>
        <SyncedTextarea
          className="relay-file-textarea"
          value={profile.authContents}
          onValueChange={(value) => onProfileChange(deriveRelayProfileFromFiles({ ...profile, authContents: value }))}
        />
      </div>
    </div>
  );
}

function ModeSelector({ launchMode, actions }: { launchMode: LaunchMode; actions: Actions }) {
  return (
    <div className="mode-grid">
      <button
        className={`mode-option ${launchMode === "relay" ? "active" : ""}`}
        onClick={() => void actions.setLaunchMode("relay")}
        type="button"
      >
        <strong>兼容增强</strong>
        <span>适合先稳定验证极义纯 API；保留会话删除、导出、项目移动、Timeline 和用户脚本，关闭插件入口相关增强。</span>
      </button>
      <button
        className={`mode-option ${launchMode === "patch" ? "active" : ""}`}
        onClick={() => void actions.setLaunchMode("patch")}
        type="button"
      >
        <strong>完整增强</strong>
        <span>适合纯 API；启用插件入口、强制安装、会话删除导出、项目移动等全部页面能力。</span>
      </button>
    </div>
  );
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
  return (
    <label className={`feature-toggle ${disabled ? "disabled" : ""}`}>
      <input
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
        type="checkbox"
      />
      <span>
        <strong>{title}</strong>
        <small>{detail}</small>
      </span>
      <Badge status={!disabled && checked ? "ok" : "disabled"} />
    </label>
  );
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

function NoticeDialog({
  notice,
  onClose,
}: {
  notice: { title: string; message: string; status?: Status };
  onClose: () => void;
}) {
  useEffect(() => {
    const timer = window.setTimeout(onClose, 4200);
    return () => window.clearTimeout(timer);
  }, []);

  return (
    <div className="toast-wrap" role="status" aria-live="polite">
      <div className={`toast-card ${notice.status === "failed" ? "failed" : ""}`}>
        <div className="toast-progress" />
        <div className="toast-icon">
          {notice.status === "failed" ? <Bell className="h-5 w-5" /> : <CheckCircle2 className="h-5 w-5" />}
        </div>
        <div className="toast-body">
          <h2>{notice.title}</h2>
          <p>{notice.message}</p>
        </div>
        <button className="toast-close" onClick={onClose} type="button">×</button>
      </div>
    </div>
  );
}

function Panel({ children, fill = false, className = "" }: { children: React.ReactNode; fill?: boolean; className?: string }) {
  return (
    <Card className={`panel ${fill ? "fill" : ""} ${className}`}>
      {children}
    </Card>
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

function Toolbar({ children }: { children: React.ReactNode }) {
  return <div className="toolbar">{children}</div>;
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
      <code>{path || "未记录路径"}</code>
    </div>
  );
}

function ReleaseReadinessRow({ item }: { item: ReleaseReadinessItem }) {
  return (
    <div className="status-row release-row">
      <span>{item.label}</span>
      <Badge status={item.status} />
      <small className="release-message">{item.message}</small>
      <code>{item.path || "未记录路径"}</code>
    </div>
  );
}

function Badge({ status }: { status: string }) {
  return <UiBadge className={statusClass(status)} variant="secondary">{statusLabel(status)}</UiBadge>;
}

function LatestLaunch({ status }: { status: LaunchStatus | null }) {
  if (!status) return <div className="empty">暂无启动状态。</div>;
  return (
    <div className="metric-list">
      <Metric label="状态" value={status.status} />
      <Metric label="消息" value={status.message} />
      <Metric label="Debug" value={String(status.debug_port ?? "-")} />
      <Metric label="Helper" value={String(status.helper_port ?? "-")} />
      <Metric label="时间" value={formatTime(status.started_at_ms)} />
    </div>
  );
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
  const source = script.market_id ? `市场 · ${script.version || "未知版本"}` : script.source === "builtin" ? "内置" : "用户";
  const canDelete = script.source === "user";
  return (
    <div className="table-row">
      <span>{script.name}</span>
      <span>{source}</span>
      <span>{script.enabled ? "启用" : "关闭"}</span>
      <span>{script.status}</span>
      <div className="script-row-actions">
        <Button onClick={() => void actions.setUserScriptEnabled(script.key, !script.enabled)} size="sm" variant="secondary">
          {script.enabled ? <PowerOff className="h-4 w-4" /> : <Power className="h-4 w-4" />}
          {script.enabled ? "禁用" : "启用"}
        </Button>
        {canDelete ? (
          <Button onClick={() => void actions.deleteUserScript(script.key)} size="sm" variant="outline">
            <Trash2 className="h-4 w-4" />
            删除
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function AdGrid({ ads, empty, actions }: { ads: AdItem[]; empty: string; actions: Actions }) {
  if (!ads.length) return <div className="empty">{empty}</div>;
  return (
    <div className="ad-grid">
      {ads.map((ad) => (
        <button className="ad-card" key={ad.id || `${ad.type}-${ad.title}`} onClick={() => void actions.openExternalUrl(ad.url)} type="button">
          <div>
            <strong>{ad.title}</strong>
            <p>{ad.description}</p>
          </div>
          {ad.highlights?.length ? (
            <div className="ad-tags">
              {ad.highlights.map((item) => (
                <span key={item}>{item}</span>
              ))}
            </div>
          ) : null}
          <span className="ad-link">
            打开
            <ExternalLink className="h-4 w-4" />
          </span>
        </button>
      ))}
    </div>
  );
}

function isExpiredAd(ad: AdItem) {
  if (!ad.expires_at) return false;
  const expiresAt = Date.parse(ad.expires_at);
  return Number.isFinite(expiresAt) && expiresAt < Date.now();
}

function routeTitle(route: Route) {
  return routes.find((item) => item.id === route)?.label ?? "概览";
}

function routeSubtitle(route: Route) {
  const subtitles: Record<Route, string> = {
    overview: "检查问题、启动与快速修复",
    admin: "用户、团队、续费、风控和审计集中管理",
    relay: "管理 API 供应商、协议、Key 与配置文件",
    sessions: "查看、删除和修复 Codex 本地会话",
    context: "独立管理 MCP、Skills、Plugins",
    enhance: "会话删除、导出、项目移动和脚本能力",
    zedRemote: "管理 Codex SSH 项目并加入 Zed workspace",
    userScripts: "内置和用户自定义脚本清单",
    recommendations: "赞助商推荐与普通推荐",
    maintenance: "入口安装、修复、Watcher 与手动启动",
    about: "版本信息、项目链接、GitHub Release 更新、日志与诊断",
    settings: "主题、命令包装器和启动参数",
  };
  return subtitles[route];
}

const contextKindOptions: Array<{ kind: ContextKind; label: string; tableName: string }> = [
  { kind: "mcp", label: "MCP", tableName: "mcp_servers" },
  { kind: "skill", label: "Skills", tableName: "skills" },
  { kind: "plugin", label: "插件", tableName: "plugins" },
];

function contextKindLabel(kind: ContextKind) {
  return contextKindOptions.find((option) => option.kind === kind)?.label ?? "扩展项";
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
  return live ? { ...entry, enabled: live.enabled } : { ...entry, enabled: false };
}

function contextEntriesForProfile(settings: BackendSettings, _profile: RelayProfile): CodexContextEntries {
  return contextEntriesFromSettings(settings);
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

function configHasCodexGoalsFeature(configContents: string): boolean {
  let inFeatures = false;
  for (const line of configContents.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^\[features\]$/.test(trimmed)) {
      inFeatures = true;
      continue;
    }
    if (inFeatures && /^\[[^\]]+\]$/.test(trimmed)) {
      inFeatures = false;
    }
    if (inFeatures && /^goals\s*=\s*true\b/.test(trimmed)) {
      return true;
    }
  }
  return false;
}

function setCodexGoalsFeatureInConfig(configContents: string, enabled: boolean): string {
  const lines = configContents.split(/\r?\n/);
  const next: string[] = [];
  let inFeatures = false;
  let sawFeatures = false;
  let featuresHasGoals = false;

  const maybeInsertGoals = () => {
    if (enabled && sawFeatures && !featuresHasGoals) {
      next.push("goals = true");
      featuresHasGoals = true;
    }
  };

  for (const line of lines) {
    const trimmed = line.trim();
    if (/^\[features\]$/.test(trimmed)) {
      if (inFeatures) maybeInsertGoals();
      inFeatures = true;
      sawFeatures = true;
      featuresHasGoals = false;
      next.push(line);
      continue;
    }
    if (inFeatures && /^\[[^\]]+\]$/.test(trimmed)) {
      maybeInsertGoals();
      inFeatures = false;
    }
    if (inFeatures && /^goals\s*=/.test(trimmed)) {
      if (enabled && !featuresHasGoals) {
        next.push("goals = true");
        featuresHasGoals = true;
      }
      continue;
    }
    next.push(line);
  }

  if (inFeatures) maybeInsertGoals();
  if (enabled && !sawFeatures) {
    const trimmed = ensureTrailingNewline(next.join("\n").trimEnd());
    return joinTomlSections([trimmed, "[features]\ngoals = true"]);
  }

  return ensureTrailingNewline(next.join("\n").trimEnd());
}

function effectiveRelayConfigPreview(profile: RelayProfile, settings: BackendSettings, contextProfile = profile): string {
  const entries = contextEntriesForProfile(settings, contextProfile);
  const isolatedConfig = stripContextEntriesFromConfig(profile.configContents, entries);
  const configWithLimits = applyContextLimitPreview(isolatedConfig, profile);
  return joinTomlSectionsRootFirst([configWithLimits, settings.relayCommonConfigContents || "", selectedContextConfigToml(entries)]);
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
  const withoutContext = stripAllContextEntriesFromConfig(liveConfigContents);
  return joinTomlSectionsRootFirst([withoutContext, selectedContextConfigToml(entries)]);
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

function stripAllContextEntriesFromConfig(configContents: string): string {
  const lines = configContents.split(/\r?\n/);
  const kept: string[] = [];
  let skipping = false;

  for (const line of lines) {
    const contextHeader = contextHeaderFromLine(line);
    if (contextHeader) {
      skipping = true;
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
  for (const line of contents.split(/\r?\n/)) {
    if (/^\s*\[[^\]]+\]\s*$/.test(line)) inRoot = false;
    if (inRoot && new RegExp(`^\\s*${key}\\s*=`).test(line)) continue;
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
  const firstTable = lines.findIndex((line) => /^\s*\[[^\]]+\]\s*$/.test(line));
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

function normalizeConfigOwnership(value: ConfigOwnership | undefined): ConfigOwnership {
  if (value === "codexPlusPlus" || value === "ccSwitch" || value === "auto") return value;
  return "auto";
}

function configOwnershipLabel(value: ConfigOwnership): string {
  if (value === "codexPlusPlus") return "Codex++";
  if (value === "ccSwitch") return "CC Switch";
  return "自动";
}

function CoordinationStatusBanner({
  form,
  actions,
}: {
  form: BackendSettings;
  actions: Actions;
}) {
  const [status, setStatus] = useState<CoordinationStatus | null>(null);
  useEffect(() => {
    void actions.refreshCoordinationStatus().then(setStatus);
  }, [actions, form.ccsLinkEnabled, form.configOwnership, form.relayProfilesEnabled, form.activeRelayId]);
  if (!status) return null;
  const tone = status.conflictDetected ? "failed" : status.effectiveOwnership === "ccSwitch" ? "success" : "info";
  return (
    <div className={`relay-coordination-banner relay-coordination-${tone}`}>
      <strong>配置协调状态</strong>
      <p>{status.guidance}</p>
      {status.ccswitchDetected ? (
        <small>
          有效所有权：{configOwnershipLabel(status.effectiveOwnership)}；live model_provider：{status.liveModelProvider || "（空）"}
          {status.ccswitchCurrentProviderName ? `；CC Switch 当前：${status.ccswitchCurrentProviderName}` : ""}
          {status.lastWriter ? `；上次写入方：${status.lastWriter}` : ""}
        </small>
      ) : null}
      {status.conflictDetected ? <small>{status.conflictMessage}</small> : null}
    </div>
  );
}

function relayProfileSourceLabel(profile: RelayProfile) {
  return profile.linkedCcsProviderId ? "cc-switch 联动" : "本地";
}

function relayProfileEditorStatus(profile: RelayProfile, form: BackendSettings, isNew: boolean) {
  if (isNew) return "新建供应商需要先保存到列表";
  if (!form.relayProfilesEnabled) return "供应商配置总开关已关闭；当前只保存配置，不写入 Codex live 文件";
  if (profile.linkedCcsProviderId && form.ccsLinkEnabled && form.configOwnership !== "codexPlusPlus") {
    return "联动 cc-switch；切换时从 cc-switch 数据库应用配置，避免覆盖冲突";
  }
  if (profile.linkedCcsProviderId && form.ccsLinkEnabled) return "联动 cc-switch；保存后会回写外部供应商数据库";
  if (profile.linkedCcsProviderId) return "联动 cc-switch；当前未开启保存回写";
  return profile.id === form.activeRelayId ? "当前正在使用" : "编辑后保存列表，再切换模式时会使用新配置";
}

function providerInitial(name: string) {
  const trimmed = (name || "供应商").trim();
  return Array.from(trimmed)[0]?.toUpperCase() || "供";
}

function statusLabel(status: string) {
  const labels: Record<string, string> = {
    found: "已找到",
    missing: "缺失",
    installed: "已安装",
    ok: "正常",
    running: "运行中",
    failed: "失败",
    warning: "风险",
    archived: "已归档",
    accepted: "已受理",
    not_checked: "未检查",
    not_implemented: "未实现",
    disabled: "已禁用",
    unknown: "未知",
  };
  return labels[status] ?? status;
}

function statusClass(status: string) {
  if (["found", "installed", "ok", "running"].includes(status)) return "good";
  if (["failed", "missing"].includes(status)) return "bad";
  return "warn";
}

function isSuccessStatus(status?: Status) {
  return status === "ok" || status === "accepted";
}

function healthItems(overview: OverviewResult | null) {
  return [
    {
      title: "Codex 应用",
      status: overview?.codex_app.status ?? "not_checked",
      ok: overview?.codex_app.status === "found",
      detail: overview?.codex_app.path || "尚未检查 Codex 应用路径。",
    },
    {
      title: "静默启动入口",
      status: overview?.silent_shortcut.status ?? "not_checked",
      ok: overview?.silent_shortcut.status === "installed",
      detail: overview?.silent_shortcut.path || `缺少 ${PRODUCT_NAME} 静默启动快捷方式时可在安装维护页修复。`,
    },
    {
      title: "管理工具入口",
      status: overview?.management_shortcut.status ?? "not_checked",
      ok: overview?.management_shortcut.status === "installed",
      detail: overview?.management_shortcut.path || "缺少管理工具快捷方式时可在安装维护页修复。",
    },
  ];
}

function normalizeSettings(settings: BackendSettings): BackendSettings {
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
      ? settings.relayProfiles.map((profile) => normalizeRelayProfile(profile, defaultContextSelection))
      : [
          {
            id: settings.activeRelayId || "default",
            linkedCcsProviderId: "",
            name: DEFAULT_RELAY_PROVIDER_NAME,
            model: QWEN_DEFAULT_MODEL,
            baseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
            upstreamBaseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
            apiKey: settings.relayApiKey || "",
            protocol: "chatCompletions" as RelayProtocol,
            relayMode: "pureApi" as RelayMode,
            officialMixApiKey: false,
            testModel: QWEN_DEFAULT_MODEL,
            configContents: "",
            authContents: "",
            useCommonConfig: true,
            contextSelection: defaultContextSelection,
            contextSelectionInitialized: true,
            contextWindow: "",
            autoCompactLimit: "",
            modelList: "",
            userAgent: "",
          },
        ];
  const activeRelayId = profiles.some((profile) => profile.id === settings.activeRelayId)
    ? settings.activeRelayId
    : profiles[0]?.id || "default";
  return syncLegacyRelayFields({
    ...defaultSettings,
    ...settings,
    relayProfilesEnabled: settings.relayProfilesEnabled !== false,
    ccsLinkEnabled: settings.ccsLinkEnabled === true,
    configOwnership: normalizeConfigOwnership(settings.configOwnership),
    relayCommonConfigContents,
    relayContextConfigContents,
    relayProfiles: profiles,
    activeRelayId,
  });
}

function codexExtraArgsToInput(args: string[] | undefined) {
  return (args ?? []).join("\n");
}

function inputToCodexExtraArgs(value: string) {
  return value === "" ? [] : value.split(/\r?\n/);
}

function normalizeRelayProfile(profile: RelayProfile, defaultContextSelection = emptyContextSelection()): RelayProfile {
  const legacyMixedApi = profile.relayMode === "mixedApi";
  let normalized: RelayProfile = {
    ...profile,
    linkedCcsProviderId: profile.linkedCcsProviderId || "",
    model: profile.model || "",
    baseUrl: profile.baseUrl || defaultSettings.relayBaseUrl,
    upstreamBaseUrl: profile.upstreamBaseUrl || profile.baseUrl || "",
    apiKey: profile.apiKey || "",
    protocol: profile.protocol === "chatCompletions" ? "chatCompletions" : "responses",
    relayMode: normalizeRelayMode(profile.relayMode),
    officialMixApiKey: profile.officialMixApiKey === true || legacyMixedApi,
    testModel: profile.testModel || "",
    configContents: profile.configContents || "",
    authContents: profile.authContents || "",
    useCommonConfig: profile.useCommonConfig !== false,
    contextSelection: profile.contextSelectionInitialized
      ? normalizeContextSelection(profile.contextSelection)
      : normalizeContextSelection(undefined, defaultContextSelection),
    contextSelectionInitialized: true,
    contextWindow: profile.contextWindow || "",
    autoCompactLimit: profile.autoCompactLimit || "",
    modelList: profile.modelList || "",
    userAgent: profile.userAgent || "",
  };
  return deriveRelayProfileFromFiles(normalized);
}

function activeRelayProfile(settings: BackendSettings): RelayProfile {
  return (
    settings.relayProfiles.find((profile) => profile.id === settings.activeRelayId) ||
    settings.relayProfiles[0] ||
    defaultSettings.relayProfiles[0]
  );
}

function relayProtocolLabel(protocol: RelayProtocol): string {
  return protocol === "chatCompletions" ? "Chat Completions 转 Responses" : "Responses API";
}

function normalizeRelayMode(mode: RelayMode | undefined): RelayMode {
  if (mode === "pureApi") return mode;
  return "pureApi";
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
  if (mode === "pureApi") return "极义纯 API";
  if (mode === "mixedApi") return "历史混合 API（已禁用）";
  return "历史官方模式（已禁用）";
}

function relayProfileConfigBrief(profile: RelayProfile): string {
  if (profile.relayMode !== "pureApi") return "已禁用";
  return profile.baseUrl || "未填写 URL";
}

function relayProfileModeHelp(profile: RelayProfile): string {
  if (profile.relayMode === "pureApi") {
    return "此供应商会写入极义/百炼纯 API 配置；启动 Codex 前不会要求官方账号登录。";
  }
  return "极义codex 已禁用官方登录和混合 API 模式；请切换为纯 API。";
}

function relayProfileReadinessText(profile: RelayProfile, relay: RelayResult | null): string {
  if (profile.relayMode !== "pureApi") {
    return "当前供应商仍是历史官方/混合模式，极义版不会切换到官方账号体系。";
  }
  const hasFiles = profile.configContents.trim() && profile.authContents.trim();
  if (!hasFiles) return "当前供应商还没有完整 config.toml / API Key 存档。";
  if (relay && !relay.configured) return "纯 API 配置未完整写入：请检查此供应商是否有 OPENAI_API_KEY，且 config.toml 是否包含 model_provider / provider / base_url。";
  return "极义纯 API 就绪：会同时写入 config.toml 和 auth.json。";
}

function relayProfileSwitchCommand(profile: RelayProfile): "clear_relay_injection" | "apply_relay_injection" | "apply_pure_api_injection" {
  return "apply_pure_api_injection";
}

function relayProfileModeSwitchedText(profile: RelayProfile): string {
  if (profile.relayMode === "pureApi") return "已按此供应商切换到极义纯 API；页面增强已设为完整增强。";
  return "极义codex 已拒绝切换到历史官方/混合模式。";
}

function withGeneratedRelayFiles(profile: RelayProfile): RelayProfile {
  const pureApiProfile = {
    ...profile,
    relayMode: "pureApi" as RelayMode,
    officialMixApiKey: false,
  };
  return {
    ...pureApiProfile,
    configContents: buildRelayConfigToml(pureApiProfile, { includeBearerToken: false }),
    authContents: buildRelayAuthJson(pureApiProfile),
  };
}

function buildRelayConfigToml(
  profile: Pick<RelayProfile, "model" | "baseUrl" | "upstreamBaseUrl" | "apiKey" | "protocol">,
  options: { includeBearerToken: boolean },
): string {
  const baseUrl = profile.protocol === "chatCompletions" ? PROTOCOL_PROXY_BASE_URL : profile.baseUrl.trim();
  const apiKey = profile.apiKey.trim();
  const rootLines = [
    profile.model.trim() ? `model = "${tomlString(profile.model.trim())}"` : null,
    'model_provider = "custom"',
    "",
  ].filter((line): line is string => line !== null);
  return [
    ...rootLines,
    "[model_providers.custom]",
    'name = "custom"',
    'wire_api = "responses"',
    "requires_openai_auth = true",
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
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return "";
    delete parsed.OPENAI_API_KEY;
    return `${JSON.stringify(parsed, null, 2)}\n`;
  } catch {
    return "";
  }
}

function deriveRelayProfileFromFiles(profile: RelayProfile): RelayProfile {
  const configContents = profile.configContents || "";
  const authContents = profile.relayMode === "official" ? buildOfficialRelayAuthJson(profile.authContents || "") : profile.authContents || "";
  const configBaseUrl = codexBaseUrlFromConfig(configContents);
  const chatUpstreamBaseUrl = rootTomlStringValue(configContents, CHAT_UPSTREAM_BASE_URL_KEY);
  const isProxyConfig = configBaseUrl === PROTOCOL_PROXY_BASE_URL;
  const upstreamBaseUrl = profile.upstreamBaseUrl || chatUpstreamBaseUrl || (configBaseUrl && !isProxyConfig ? configBaseUrl : profile.baseUrl || "");
  const configApiKey = codexExperimentalBearerTokenFromConfig(configContents);
  return {
    ...profile,
    model: codexModelFromConfig(configContents),
    baseUrl: upstreamBaseUrl,
    upstreamBaseUrl,
    apiKey: profile.relayMode === "official"
      ? configApiKey || profile.apiKey || ""
      : codexApiKeyFromAuth(authContents) || configApiKey || "",
    contextWindow: codexTopLevelIntFromConfig(configContents, "model_context_window"),
    autoCompactLimit: codexTopLevelIntFromConfig(configContents, "model_auto_compact_token_limit"),
    configContents,
    authContents,
  };
}

function applyRelayProfilePatchToFiles(
  profile: RelayProfile,
  patch: Partial<RelayProfile>,
  options: { allowGenerateFiles?: boolean } = {},
): RelayProfile {
  let next: RelayProfile = { ...profile, ...patch };
  const shouldHaveFiles =
    next.relayMode !== "official" || next.officialMixApiKey || next.configContents.trim() || next.authContents.trim();
  const needsAuthFile = next.relayMode === "pureApi";
  if (options.allowGenerateFiles && shouldHaveFiles && (!next.configContents.trim() || (needsAuthFile && !next.authContents.trim()))) {
    next = withGeneratedRelayFiles(next);
  }

  if ("model" in patch) {
    next.configContents = setRootTomlStringKey(next.configContents, "model", patch.model || "");
  }
  if ("apiKey" in patch) {
    if (next.relayMode === "pureApi") {
      next.authContents = setAuthOpenAiApiKey(next.authContents, patch.apiKey || "");
      next.configContents = removeCodexExperimentalBearerToken(next.configContents);
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
  if ("baseUrl" in patch || "upstreamBaseUrl" in patch || "protocol" in patch) {
    const baseUrlForConfig = next.protocol === "chatCompletions" ? PROTOCOL_PROXY_BASE_URL : next.upstreamBaseUrl || next.baseUrl;
    next.configContents = setCodexProviderStringKey(next.configContents, "base_url", baseUrlForConfig);
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
  for (const line of contents.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    if (trimmed.startsWith("[")) break;
    const match = /^model\s*=\s*(["'])(.*)\1\s*$/.exec(trimmed);
    if (match) return match[2].replace(/\\(["'\\])/g, "$1");
  }
  return "";
}

function codexBaseUrlFromConfig(contents: string): string {
  return codexProviderStringFromConfig(contents, "base_url");
}

function codexExperimentalBearerTokenFromConfig(contents: string): string {
  return codexProviderStringFromConfig(contents, "experimental_bearer_token");
}

function codexProviderStringFromConfig(contents: string, key: string): string {
  const provider = rootTomlStringValue(contents, "model_provider");
  const targetSection = provider ? `model_providers.${provider}` : "";
  const lines = contents.split(/\r?\n/);
  let currentSection = "";
  const matches: string[] = [];

  for (const line of lines) {
    const section = tomlSectionName(line);
    if (section !== null) {
      currentSection = section;
      continue;
    }
    const value = tomlStringAssignmentValue(line, key);
    if (value === null) continue;
    if (targetSection && currentSection === targetSection) return value;
    if (!currentSection || !currentSection.startsWith("model_providers.")) matches.push(value);
  }

  return matches.length === 1 ? matches[0] : "";
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
  for (const line of topLevel.split(/\r?\n/)) {
    const match = pattern.exec(line);
    if (match) return match[1];
  }
  return "";
}

function rootTomlStringValue(contents: string, key: string): string {
  const topLevel = splitTomlRootAndTables(contents).root;
  for (const line of topLevel.split(/\r?\n/)) {
    const value = tomlStringAssignmentValue(line, key);
    if (value !== null) return value;
  }
  return "";
}

function tomlSectionName(line: string): string | null {
  const match = /^\s*\[([^\]]+)\]\s*$/.exec(line);
  return match ? match[1].trim() : null;
}

function tomlStringAssignmentValue(line: string, key: string): string | null {
  const match = new RegExp(`^\\s*${key}\\s*=\\s*([\"'])(.*)\\1\\s*(?:#.*)?$`).exec(line.trim());
  if (!match) return null;
  return match[2].replace(/\\(["'\\])/g, "$1");
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

function setRootTomlIntKey(contents: string, key: string, value: string): string {
  const trimmed = value.replace(/[^\d]/g, "");
  if (!trimmed) return removeRootTomlKey(contents, key);
  return setRootTomlLine(contents, key, `${key} = ${trimmed}`);
}

function setRootTomlLine(contents: string, key: string, lineText: string): string {
  const lines = contents.split(/\r?\n/);
  const firstTable = lines.findIndex((line) => /^\s*\[[^\]]+\]\s*$/.test(line));
  const rootEnd = firstTable >= 0 ? firstTable : lines.length;
  for (let index = 0; index < rootEnd; index += 1) {
    if (new RegExp(`^\\s*${key}\\s*=`).test(lines[index])) {
      lines[index] = lineText;
      return ensureTrailingNewline(lines.join("\n").trimEnd());
    }
  }
  const insertAt = key === "model" ? 0 : rootEnd;
  lines.splice(insertAt, 0, lineText);
  return ensureTrailingNewline(lines.join("\n").trimEnd());
}

function setCodexProviderStringKey(contents: string, key: string, value: string): string {
  const provider = rootTomlStringValue(contents, "model_provider") || "custom";
  let next = contents;
  if (!rootTomlStringValue(next, "model_provider")) {
    next = setRootTomlStringKey(next, "model_provider", provider);
  }
  next = ensureCodexProviderDefaults(next, provider);
  return setTomlSectionStringKey(next, `model_providers.${provider}`, key, value);
}

function setCodexExperimentalBearerToken(contents: string, apiKey: string): string {
  const trimmed = apiKey.trim();
  return trimmed
    ? setCodexProviderStringKey(contents, "experimental_bearer_token", trimmed)
    : removeCodexExperimentalBearerToken(contents);
}

function removeCodexExperimentalBearerToken(contents: string): string {
  const provider = rootTomlStringValue(contents, "model_provider") || "custom";
  return removeTomlSectionKey(contents, `model_providers.${provider}`, "experimental_bearer_token");
}

function ensureCodexProviderDefaults(contents: string, provider: string): string {
  let next = contents;
  const section = `model_providers.${provider}`;
  next = setTomlSectionStringKey(next, section, "name", provider);
  next = setTomlSectionStringKey(next, section, "wire_api", "responses");
  return setTomlSectionBoolKey(next, section, "requires_openai_auth", true);
}

function setTomlSectionBoolKey(contents: string, sectionName: string, key: string, value: boolean): string {
  return setTomlSectionRawKey(contents, sectionName, key, value ? "true" : "false");
}

function setTomlSectionStringKey(contents: string, sectionName: string, key: string, value: string): string {
  return setTomlSectionRawKey(contents, sectionName, key, `"${tomlString(value.trim())}"`);
}

function setTomlSectionRawKey(contents: string, sectionName: string, key: string, value: string): string {
  const lines = contents.split(/\r?\n/);
  let sectionStart = -1;
  let sectionEnd = lines.length;
  for (let index = 0; index < lines.length; index += 1) {
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
    if (new RegExp(`^\\s*${key}\\s*=`).test(lines[index])) {
      lines[index] = replacement;
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
  let sectionStart = -1;
  let sectionEnd = lines.length;
  for (let index = 0; index < lines.length; index += 1) {
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
    if (index <= sectionStart || index >= sectionEnd) return true;
    return !new RegExp(`^\\s*${key}\\s*=`).test(line);
  });
  return ensureTrailingNewline(next.join("\n").trimEnd());
}

function relayProfileSwitchValidation(profile: RelayProfile): string | null {
  if (profile.relayMode !== "pureApi") {
    return `供应商「${profile.name || profile.id}」仍是历史官方/混合模式。极义codex 不使用官方账号体系，请先改为“极义 / 百炼纯 API”。`;
  }
  if (!profile.configContents.trim()) {
    return `供应商「${profile.name || profile.id}」缺少独立 config.toml，已停止切换，避免继续显示上一套配置文件。请先在该供应商详情里保存 config.toml。`;
  }
  return null;
}

function tomlString(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

function syncLegacyRelayFields(settings: BackendSettings): BackendSettings {
  const relayProfiles = settings.relayProfiles.map(deriveRelayProfileFromFiles);
  const active = activeRelayProfile({ ...settings, relayProfiles });
  return {
    ...settings,
    relayProfiles,
    activeRelayId: active.id,
    relayBaseUrl: active.baseUrl,
    relayApiKey: active.apiKey,
  };
}

function mergeLiveLinkedRelayProfiles(settings: BackendSettings, liveSettings: BackendSettings): BackendSettings {
  const liveLinkedById = new Map(
    liveSettings.relayProfiles
      .filter((profile) => profile.linkedCcsProviderId.trim())
      .map((profile) => [profile.id, profile]),
  );
  if (!liveLinkedById.size) return settings;
  const existingIds = new Set(settings.relayProfiles.map((profile) => profile.id));
  const relayProfiles = [
    ...settings.relayProfiles.map((profile) => liveLinkedById.get(profile.id) ?? profile),
    ...liveSettings.relayProfiles.filter((profile) => profile.linkedCcsProviderId.trim() && !existingIds.has(profile.id)),
  ];
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles,
    activeRelayId: relayProfiles.some((profile) => profile.id === settings.activeRelayId)
      ? settings.activeRelayId
      : liveSettings.activeRelayId,
  });
}

function updateRelayProfile(settings: BackendSettings, id: string, patch: Partial<RelayProfile>): BackendSettings {
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
    linkedCcsProviderId: "",
    name: `供应商 ${settings.relayProfiles.length + 1}`,
    model: "",
    baseUrl: defaultSettings.relayBaseUrl,
    upstreamBaseUrl: defaultSettings.relayBaseUrl,
    apiKey: "",
    protocol: "chatCompletions" as RelayProtocol,
    relayMode: "pureApi" as RelayMode,
    officialMixApiKey: false,
    testModel: "",
    configContents: "",
    authContents: "",
    useCommonConfig: true,
    contextSelection,
    contextSelectionInitialized: true,
    contextWindow: "",
    autoCompactLimit: "",
    modelList: "",
    userAgent: "",
  };
  return withGeneratedRelayFiles(next);
}

function addRelayProfile(settings: BackendSettings, profile: RelayProfile): BackendSettings {
  const nextWithFiles = deriveRelayProfileFromFiles(
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
    linkedCcsProviderId: "",
    name: `${source.name || "未命名供应商"} 副本`,
  };
  const relayProfiles = [...settings.relayProfiles];
  relayProfiles.splice(sourceIndex >= 0 ? sourceIndex + 1 : relayProfiles.length, 0, next);
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
  const profiles = settings.relayProfiles.filter((profile) => profile.id !== id);
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: profiles.length ? profiles : defaultSettings.relayProfiles,
    activeRelayId: settings.activeRelayId === id ? profiles[0]?.id || "default" : settings.activeRelayId,
  });
}

function numberOrDefault(value: string, fallback: number) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function splitLogLines(text: string) {
  return text.trimEnd().split(/\r?\n/).filter((line, index, lines) => line.length > 0 || index < lines.length - 1);
}

function zedStrategyLabel(strategy: ZedOpenStrategy) {
  if (strategy === "reuseWindow") return "复用窗口";
  if (strategy === "newWindow") return "新窗口";
  if (strategy === "default") return "Zed 默认行为";
  return "加入当前工作区";
}

function zedRemoteHostLabel(project: ZedRemoteProject) {
  const user = project.ssh.user ? `${project.ssh.user}@` : "";
  const port = project.ssh.port ? `:${project.ssh.port}` : "";
  return `${user}${project.ssh.host}${port}`;
}

function zedRemoteSourceLabel(source: string) {
  if (source === "currentThread") return "当前会话";
  if (source === "codexRemoteProject") return "Codex remote project";
  if (source === "threadWorkspaceHint") return "Thread workspace hint";
  if (source === "sqliteThreadCwd") return "SQLite cwd";
  if (source === "recent") return "最近打开";
  return source || "未知来源";
}

function formatTime(value: number) {
  if (!value) return "-";
  return new Date(value).toLocaleString("zh-CN");
}

function formatCompactNumber(value: number) {
  return new Intl.NumberFormat("zh-CN", { notation: "compact", maximumFractionDigits: 1 }).format(value || 0);
}

function formatDailyLimit(value: number) {
  return value > 0 ? `${formatCompactNumber(value)} tokens / 天` : "未限额";
}

function formatRemaining(value: number | null) {
  return value == null ? "未限额" : `剩余 ${formatCompactNumber(value)}`;
}

function formatMoneyCents(value: number, currency: string) {
  const normalizedCurrency = (currency || "CNY").trim().toUpperCase();
  return `${normalizedCurrency} ${(value / 100).toFixed(2)}`;
}

function shortId(value: string) {
  if (!value) return "-";
  return value.length > 18 ? `${value.slice(0, 10)}…${value.slice(-6)}` : value;
}

function auditMetadataSummary(value: unknown) {
  if (!value || typeof value !== "object") return "-";
  const text = JSON.stringify(value);
  return text.length > 80 ? `${text.slice(0, 77)}...` : text;
}

function formatSmsSecretSource(config?: SmsConfigState) {
  if (!config) return "等待读取";
  if (!config.secretIdSet || !config.secretKeySet) return "未配置";
  const sources = new Set([config.secretIdSource, config.secretKeySource]);
  if (sources.has("default_keychain")) return "极义钥匙串";
  if (sources.has("env_keychain_ref")) return "环境变量引用钥匙串";
  if (sources.has("env_plaintext")) return "环境变量";
  return "已配置";
}

function formatRelayApiKeySource(source?: string) {
  switch (source) {
    case "relay_profile_keychain":
    case "settings_keychain":
      return "极义钥匙串";
    case "env_jiyi_codex_api_key":
    case "env_dashscope_api_key":
    case "env_bailian_api_key":
    case "env_aliyun_bailian_api_key":
    case "env_qwen_api_key":
      return "百炼环境变量";
    case "downloads_default_api_key_file":
      return "下载目录百炼 Key";
    case "api_key_file_env":
      return "指定 Key 文件";
    case "env_apimart_api_key":
      return "APIMart 备选";
    case "env_custom_openai_api_key":
      return "自定义环境变量";
    case "relay_profile":
    case "settings":
      return "本机设置";
    default:
      return "已配置";
  }
}

function usageLimitSourceLabel(source: string) {
  if (source === "local_entitlement") return "本地套餐";
  if (source === "settings") return "全局设置";
  if (source === "unlimited") return "未限额";
  if (source === "preview") return "预览";
  return source || "未限额";
}

function formatDuration(startedAtMs: number): string {
  if (!startedAtMs) return "-";
  const elapsed = Date.now() - startedAtMs;
  if (elapsed < 0) return formatTime(startedAtMs);
  const mins = Math.floor(elapsed / 60000);
  if (mins < 1) return "刚刚启动";
  if (mins < 60) return `已运行 ${mins} 分钟`;
  const hours = Math.floor(mins / 60);
  const remainMins = mins % 60;
  return `已运行 ${hours} 小时 ${remainMins} 分钟`;
}

function stringifyError(error: unknown) {
  if (error instanceof Error) return error.message;
  return String(error);
}

function loadInitialTheme(): Theme {
  if (typeof window === "undefined") return "dark";
  return window.localStorage.getItem("codex-plus-theme") === "light" ? "light" : "dark";
}

function initialAppMode(): AppMode {
  if (typeof window === "undefined") return "manager";
  const params = new URLSearchParams(window.location.search);
  const mode = params.get("mode")?.trim().toLowerCase();
  return mode === "main" || mode === "app" ? "main" : "manager";
}

function loadInitialRoute(): Route {
  if (typeof window === "undefined") return "overview";
  const params = new URLSearchParams(window.location.search);
  if (params.get("showUpdate") === "1" || window.location.hash === "#about") {
    return "about";
  }
  return "overview";
}
