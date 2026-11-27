// 管理器主界面（App 与各页面）共用的后端载荷与设置类型。
import type { ManagerPageSummary } from "./extension-manager-pages";
import type { CodexContextEntries, ContextKind } from "./context-config-text";
import type { BlockedInstall, CuratedPackage } from "./CuratedPackages";
import type { ExtensionConfigField } from "./extension-config-form";
import type { UsageCost } from "./gateway-model";
import type { ExtensionHost } from "./extension-host-model";
import type { LocalSessionFilter, SessionProjectCount } from "./management-model";
import type { WhaleBalanceSettings } from "./whale-settings";
import type { DictationSettings } from "./dictation-settings";
import type { RelayModelRoute } from "./model-routes";
import type { RelayApiKey } from "./provider-api-keys";
import type { FrontendRecoveryReport } from "./history-recovery-report";
import type { DreamSkinThemeConfig } from "./dream-skin";
import type { ThemePreference } from "./theme-mode";
import type { MultiLaunchInstance } from "./multi-launch-model";

export type Status = "ok" | "failed" | "not_implemented" | "not_checked" | string;

export type CommandResult<T> = T & {
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
  /** 阶段；侧栏「停止 Codex」写的是 user_stopped，用来和意外退出区分。 */
  phase?: string | null;
  started_at_ms: number;
  debug_port: number | null;
  helper_port: number | null;
  codex_app: string | null;
  aumid: string | null;
};

export type RuntimeServiceState = { status: string; address: string | null; message: string };

export type OverviewResult = CommandResult<{
  codex_app: PathState;
  codex_version: string | null;
  silent_shortcut: PathState;
  management_shortcut: PathState;
  latest_launch: LaunchStatus | null;
  runtime_health: {
    codex_app: RuntimeServiceState;
    local_server: RuntimeServiceState;
    debugger: RuntimeServiceState;
    protocol_conversion_enabled: boolean | null;
    /** Codex 在运行但不是经 Codex++ 启动的（外部启动、增强脚本未挂载）；旧后端没有这个字段。 */
    codex_unattached?: boolean;
  };
  current_version: string;
  update_status: string;
  settings_path: string;
  logs_path: string;
}>;

export type MultiLaunchInstancesResult = CommandResult<{
  instances: MultiLaunchInstance[];
}>;

/** 顶层的 `codexAppWhaleBalance*` 是大肥鱼（Agent 桌宠类型之一）的余额配置。 */
export type BackendSettings = WhaleBalanceSettings & {
  revision?: string;
  /** 旧版内置桌宠的设置（与 Rust `agent_pet` 对应）。桌宠已改成拓展、设置在拓展配置里，这里只原样带回给后端。 */
  agentPet?: unknown;
  codexAppPath: string;
  /** 自动接管官方 App（与 Rust `codex_app_auto_takeover` 对应）。 */
  codexAppAutoTakeover: boolean;
  codexAppAutoTakeoverWhenActive: string;
  /** Claude Desktop 自动接管（与 Rust `claude_app_auto_takeover` 对应，只在完整功能模式下动作）。 */
  claudeAppAutoTakeover: boolean;
  claudeAppExtensions?: boolean;
  claudeDesktopLocal?: ClaudeLocalConfig;
  codexExtraArgs: string[];
  dictation: DictationSettings;
  providerSyncEnabled: boolean;
  /** 「启动前修复会话索引」与 providerSyncEnabled 分开（#2846）：只补投影、不改会话归属。 */
  sessionIndexRepairEnabled?: boolean;
  /** 「启动引导出现过」的记号（#2902）：非空即出现过，本地存储丢了时靠它不再重复弹。原来记的是已取消的使用模式。 */
  managerAgentFocus?: string;
  threadModelFollowSwitchEnabled?: boolean;
  /** 非官方登录时保留本地项目（#2947 / #3018）：启动前挪走 Codex 的账号隔离容器，切回官方登录时放回。 */
  keepProjectsWithoutAccount?: boolean;
  /** 纯 API Key 解锁 ChatGPT 登录专属入口：本地 ChatGPT 兼容服务 + 改写 ~/.codex，关闭时还原。 */
  chatgptCompatEnabled?: boolean;
  /** 本地账号显示的名字、用户名、邮箱；留空用默认值。 */
  chatgptCompatName?: string;
  chatgptCompatUsername?: string;
  chatgptCompatEmail?: string;
  nativeAuthSelection?: { mode: "none" } | { mode: "account"; accountRef: string } | null;
  /** 启动时及运行期间自动检查 Codex++ 更新（#1272），默认开。 */
  updateAutoCheck?: boolean;
  /** 检查到新版本时在后台下好安装包，但不安装；点「安装」才安装。默认开。 */
  updateAutoDownload?: boolean;
  /** 更新提示里「跳过此版本」记下的版本（#1272）。 */
  updateSkippedVersion?: string;
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
  codexAppSessionDeleteSkipConfirm: boolean;
  codexAppSideChatSave: boolean;
  codexAppMarkdownExport: boolean;
  codexAppUsageStatusLine: boolean;
  usageHistoryEnabled: boolean;
  codexAppProxyRequestStatus: boolean;
  codexAppPasteFix: boolean;
  codexAppPromptArgs: boolean;
  codexAppForceChineseLocale: boolean;
  codexAppThreadIdBadge: boolean;
  codexAppConversationView: boolean;
  codexAppThreadScrollRestore: boolean;
  codexAppHistoryTurnRecovery: boolean;
  codexAppNativeMenuPlacement: boolean;
  codexAppNativeBrowserRequireIdentification: boolean;
  codexAppNativeFastUnlock: boolean;
  /** 原生输入预测（实验）：自定义 API 的本机对话使用 Codex 自带的灰字预测与 Tab 接受，默认关。 */
  codexAppNativePredictions: boolean;
  /** 第三方模型回答完自动折叠过程（本地代理补阶段标记），默认关。 */
  codexAppThirdPartyReasoningCollapse: boolean;
  /** 拿不到远程功能开关（国内网络）时补开「替我审批」入口（#2817），默认关。 */
  codexAppOfflineAutoReview: boolean;
  /** Statsig 界面增强：隐藏升级推销、放宽网络超时、补开记忆，默认关。 */
  codexAppStatsigOverrides: boolean;
  /** Statsig 开关逐项覆盖（Codex 页面 Feature Lab 里改）：开关 ID → "on" / "off" / "native"。管理器只原样保留。 */
  codexAppStatsigGateOverrides?: Record<string, "on" | "off" | "native">;
  /** Codex App 和 CLI 已经经过本地代理的请求按上报用量判断压缩，默认关。 */
  codexAppReportedUsageCompaction: boolean;
  /** 纯 API Key 模式启动 Codex 前把 Work 视图写回 Codex，避开 Work browser access 报错。 */
  codexAppDisableWorkView: boolean;
  codexAppPetRealMouseLook: boolean;
  codexAppActivityOverlayRight: boolean;
  /** 把不在当前供应商模型列表里的请求模型（Codex 后台任务的 gpt-* / codex-auto-review）改写为该供应商默认模型。与 Rust `protocol_proxy_map_unlisted_models` 对应。 */
  protocolProxyMapUnlistedModels: boolean;
  /** 本地协议代理单次请求体上限（MiB，32~512，默认 128）。与 Rust `helper_max_request_body_mib` 对应。 */
  helperMaxRequestBodyMib: number;
  /** 流式请求等上游响应头的上限（秒，120~3600；null 表示用默认 120）。与 Rust `upstream_stream_header_timeout_secs` 对应。 */
  upstreamStreamHeaderTimeoutSecs: number | null;
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
  weixinConnectAllowOutboundAttachments: boolean;
  weixinConnectRouteTag: string;
  weixinConnectWorkDir: string;
  weixinConnectModel: string;
  weixinConnectReasoningEffort: string;
  weixinConnectProjectId: string;
  weixinConnectSandbox: "read-only" | "workspace-write" | "danger-full-access";
  weixinConnectCodexPath: string;
  weixinConnectLinkedUserId: string;
  weixinConnectDefaultMode: "jiajia" | "direct";
  weixinConnectPushToWeixin: boolean;
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

export type ToolEntry = {
  id: string;
  name: string;
  homeDir: string;
  switchable: boolean;
  active: boolean;
  activeRelayName: string | null;
  relayCount: number;
};


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
  inactiveMixedApi?: Pick<RelayProfile, "configContents" | "upstreamBaseUrl" | "modelRoutes">;
  officialAccountRef?: string;
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
  /** Multi-Agent 子代理协议（#2683）：空串=自动，v1/v2=供应商级显式选择（目录每模型兜底）。 */
  multiAgent?: string;
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
  /** #228：转发前移除托管生图工具（image_generation），默认关闭。 */
  stripImageGenerationTool?: boolean;
  /** #2870：转发前移除 Responses 图片 `detail`，默认关闭。 */
  stripResponsesImageDetail?: boolean;
  /** 转发前移除 Responses `include` 里的 `output.encrypted_content`，默认关闭。 */
  stripResponsesOutputEncryptedContent?: boolean;
  /** #2868：不向上游发送 tool_search，默认关闭。 */
  disableToolSearch?: boolean;
  /** #1433：纯 API 下让 Codex 注册原生生图工具，生图请求经本地代理发给本供应商，默认关闭。 */
  pureApiImageGeneration?: boolean;
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

export type RelayAggregateStrategy = "failover" | "conversationRoundRobin" | "requestRoundRobin" | "weightedRoundRobin";
export type AggregateMode = "provider" | "model";
type RelayAggregateMember = {
  profileId: string;
  weight: number;
};
export type RelayAggregateRoute = {
  pattern: string;
  profileId: string;
  priority: number;
};
export type RelayAggregateConfig = {
  strategy: RelayAggregateStrategy;
  members: RelayAggregateMember[];
  routes?: RelayAggregateRoute[];
  modelAggregationEnabled?: boolean;
  /** 默认供应商：解析不到成员的普通模型请求交给该成员转发；空串表示未设置。 */
  defaultRelayId?: string;
  /** 把默认成员的官方登录同步进 Codex（写 auth.json 的 tokens 段）。默认关。 */
  syncDefaultMemberLogin?: boolean;
  /** 审批成员（#2703）：codex-auto-review 只发给它，模型取它的「自动审批模型」；空串表示未设置。 */
  autoReviewRelayId?: string;
};
type AggregateRelayMember = {
  relayId: string;
  weight: number;
};
export type AggregateRelayProfile = {
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
  /** 审批成员（#2703），空串表示未设置。 */
  autoReviewRelayId?: string;
};

export type RelayContextSelection = {
  mcpServers: string[];
  skills: string[];
  plugins: string[];
};

export type RelayProtocol = "responses" | "chatCompletions" | "anthropicMessages" | "gemini";
export type StepwiseProtocol = "auto" | "chat_completions" | "responses" | "anthropic_messages";
export type StepwiseGenerationMode = "auto" | "manual";
export type RelayMode = "official" | "mixedApi" | "pureApi" | "aggregate";
export type RelaySessionProvider = "custom" | "openai";
export type RelayEditorMode = "official" | "mixedApi" | "pureApi";


export type BuiltinExtension = {
  id: string;
  key: string;
  name: string;
  description: string;
  group?: string;
  enabled: boolean;
  replacedBy?: string | null;
  /** 启停读写的设置字段；没有时启停记在用户脚本配置里。 */
  setting?: string | null;
  params?: string[];
  restartRequired?: boolean;
  replaceable?: boolean;
};

export type UserScriptInventory = {
  enabled?: boolean;
  claude_enabled?: boolean;
  skip_next_start?: boolean;
  builtins?: BuiltinExtension[];
  scripts?: Array<{
    key: string;
    name: string;
    source: string;
    kind?: "script" | "package";
    package?: {
      id: string; version: string; description: string; engine: number; routes: string[]; hookEvents?: string[]; tools?: string[];
      configSchema?: ExtensionConfigField[]; config?: Record<string, unknown>; backend?: boolean;
      gatewayRequest?: number; gatewayResponse?: number; gatewayError?: number; services?: string[];
      providers?: Array<{ id: string; name: string; models: string[] }>;
      toolPipeline?: { preExecute?: number; execute?: number; postExecute?: number }; skippedAssets?: string[];
      locales?: Record<string, { title?: string | null; description?: string | null }>;
      http?: boolean; schedules?: Array<{ method: string; everyMinutes?: number | null; at?: string | null }>;
      codex?: { skills?: string[]; agents?: string[]; mcpServers?: string[]; instructions?: boolean; prompts?: string[]; resources?: string[] };
      claudeDesktop?: Record<string, unknown> | null;
      /** 管理器面板（codexpp.json 的 manager）：可以进侧栏（nav）、顶替内置页面（replaces）、读只读数据（data）。 */
      managerPage?: ManagerPageSummary | null;
      /** 拓展自己的设置页（codexpp.json 的 settings.page），「设置」打开它而不是通用表单。 */
      settingsPage?: boolean;
      /** 顶替的内置拓展、关掉的内置网关环节、页面最早期脚本、启动前后脚本与启动参数、dsh 插件。 */
      replaces?: string[]; gatewayReplaces?: string[]; preload?: { scripts?: number };
      launch?: { args?: string[]; before?: boolean; after?: boolean }; dsh?: boolean;
    } | null;
    enabled: boolean;
    claude_enabled?: boolean | null;
    status: string;
    /** 清单本身的错误，或拓展在 Codex 页面里运行时报的错（带运行状态的清单才有后者）。 */
    error: string;
    /** 只含清单本身的问题，空串表示清单没问题；管理器页面能不能用只看这一项。旧版后端没有。 */
    manifestError?: string;
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

export type RequestLogEntry = {
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
  /** 成功那次上游请求真正发出的 unix 毫秒时间：转发前排队/重试可据此单独统计（#3267）。旧日志没有该字段。 */
  upstreamStartedAtMs?: number | null;
  /** 运行中按流式增量粗估的输出 token。 */
  estimatedOutputTokens?: number | null;
  error?: string | null;
  streamOutcome?: string | null;
  usage?: RequestLogUsage | null;
  cacheHitRatio?: number | null;
  /** 按供应商里填的价格估算的费用（后端读日志时补上；没填价格时没有）。 */
  cost?: UsageCost | null;
};

export type SettingsResult = CommandResult<SettingsPayload>;
export type SettingsPayload = {
  revision: string;
  settings: BackendSettings;
  settings_path: string;
  user_scripts: UserScriptInventory;
};

export type WeixinConnectStatusResult = CommandResult<{
  state: string;
  message: string;
  accountId: string;
  hasToken: boolean;
  lastPeerId: string;
  lastMessageAtMs: number;
  processedMessages: number;
}>;

export type WeixinQrResult = CommandResult<{
  qrStatus: string;
  qrContent: string;
  qrSvg: string;
  accountId: string;
  linkedUserId: string;
  hasToken: boolean;
}>;

export type RelayFilesResult = CommandResult<{
  configPath: string;
  authPath: string;
  configContents: string;
  authContents: string;
}>;

export type LocalSession = {
  id: string;
  title: string;
  cwd: string;
  modelProvider: string;
  archived: boolean;
  updatedAtMs: number | null;
  rolloutPath: string;
  dbPath: string;
};

export type LocalSessionsResult = CommandResult<{
  dbPath: string;
  dbPaths: string[];
  sessions: LocalSession[];
  offset: number;
  limit: number;
  hasMore: boolean;
  /** 筛选后的会话总数。 */
  totalCount: number;
  /** 筛选结果里每个项目的会话数（不受分页影响）。 */
  projects: SessionProjectCount[];
  /** 这一页对应的搜索词和归档筛选。 */
  query: string;
  archived: boolean | null;
}>;

export type McpImportPreviewResult = CommandResult<{
  entries: Array<{ id: string; tomlBody: string }>;
  warnings: string[];
}>;

export type LiveContextEntriesResult = CommandResult<{
  entries: CodexContextEntries;
}>;

export type ExtractRelayCommonConfigResult = CommandResult<{
  commonConfigContents: string;
  profileConfigContents: string;
}>;

/** 冲突弹窗里的两种继续方式，对应后端 LiveConflictResolution。 */
export type LiveConflictResolution = "importAsNewProfile" | "discardLiveChanges";

type ProviderDoctorCheck = {
  id: string;
  title: string;
  status: Status;
  detail: string;
};

export type ProviderDoctorResult = CommandResult<{
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

export type CcsProvidersResult = CommandResult<{
  dbPath: string;
  configuredDbPath: string;
  fallbackReason?: string | null;
  providers: CcsProviderImport[];
  /** 已经导入过的条目（`sourceId`）；旧版后端没有这一项。 */
  imported?: string[];
}>;

/** 「从第三方导入」里 Magpie 的供应商（不含 Key）。 */
export type MagpieProvidersResult = CommandResult<{
  /** 实际读的 `providers.json`。 */
  path: string;
  /** 这个文件在不在（不在就是没装 Magpie）。 */
  found: boolean;
  providers: Array<{
    sourceId: string;
    name: string;
    baseUrl: string;
    protocol: RelayProtocol;
    /** 在 Magpie 里关掉了。 */
    disabled: boolean;
    hasKey: boolean;
    imported: boolean;
  }>;
  /** 把 Magpie 本身当作一个上游（指向它的本地网关）；没装 Magpie 时没有。 */
  gateway?: { sourceId: string; baseUrl: string; imported: boolean } | null;
}>;

export type ProviderImportRequest = {
  name: string;
  baseUrl: string;
  apiKey: string;
  wireApi: string;
  relayMode: string;
  configContents: string;
  authContents: string;
  /** 空为直连模式供应商；"pool" 为网关共享供应商。 */
  target?: string;
};

export type EnvConflict = {
  name: string;
  source: "process" | "user" | string;
  valuePresent: boolean;
};

// 本机回环中转地址遇上 Codex 的 SSH 远端连接（issue #1195），后端只读检测。
export type SshRemoteLoopbackNoticeData = {
  loopbackEndpoints: Array<{ key: string; url: string; port: number }>;
  ports: number[];
  remotes: Array<{ label: string; forwardCommand: string; sshConfigLines: string[] }>;
};

export type EnvConflictsResult = CommandResult<{
  conflicts: EnvConflict[];
  sshRemoteLoopback?: SshRemoteLoopbackNoticeData | null;
}>;

export type RelayEnvironmentResult = CommandResult<{
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

export type ProviderSyncPayload = {
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

export type SessionIndexCleanupCandidate = {
  id: string;
  threadName: string;
  updatedAt: string;
};

export type ProviderSyncTargetSource = "config" | "rollout" | "sqlite" | "manual";

export type ProviderSyncTargetOption = {
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

export type ProviderSyncTargetsResult = CommandResult<ProviderSyncTargetsPayload>;

export type ProviderSyncProgress = {
  active: boolean;
  percent: number;
  message: string;
  result: CommandResult<ProviderSyncPayload> | null;
};

export type TaskProgress = {
  active: boolean;
  percent: number;
  message: string;
  indeterminate?: boolean;
  failed?: boolean;
};

export type SessionIndexRepairReport = {
  frontendRecovery?: FrontendRecoveryReport | null;
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

export type LogsResult = CommandResult<{
  path: string;
  text: string;
  lines: number;
  truncated: boolean;
  fileSize: number;
}>;

export type DiagnosticsResult = CommandResult<{
  report: string;
}>;

export type WatcherResult = CommandResult<{
  enabled: boolean;
  disabled_flag: string;
  start_minimized: boolean;
  start_codex: boolean;
  start_codex_on_open: boolean;
}>;

// 与 Rust 侧 `auto_takeover::AutoTakeoverPayload`（rename_all = "camelCase"）逐字段对齐。
// 少一个字段的表现是「状态行永远空白」，所以两边要一起改。
export type AutoTakeoverResult = CommandResult<{
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

export type UpdateResult = CommandResult<{
  currentVersion: string;
  latestVersion?: string | null;
  releaseSummary?: string;
  assetName?: string | null;
  assetUrl?: string | null;
  updateAvailable?: boolean;
  /** 安装包已经在后台下好（「有可用更新时自动下载」），点「安装」直接安装。 */
  downloaded?: boolean;
  installedPath?: string;
  progress?: number;
}>;

/** 后台下载更新的状态：只下载、不安装。 */
export type UpdateDownloadState = { version: string; state: "downloading" | "ready" | "failed"; percent: number | null; message?: string };

export type AdItem = {
  id?: string;
  type: "sponsor" | "normal" | string;
  title: string;
  description: string;
  url: string;
  image?: string;
  highlights?: string[];
  expires_at?: string;
};

export type AdsResult = CommandResult<{
  version: number;
  ads: AdItem[];
  /// 兼容旧数据和旧管理端消费者的首条置顶赞助位。
  topAd?: AdItem;
  /// 独立置顶赞助位列表，供概览横幅轮播。
  topAds?: AdItem[];
}>;

export type ScriptMarketItem = {
  id: string;
  name: string;
  description: string;
  version: string;
  author: string;
  tags: string[];
  homepage: string;
  script_url: string;
  /// 可选图标地址，清单多数条目没有。
  icon?: string;
  /// 该脚本最近更新时间（ISO），清单逐条提供；老清单没有时为空（#2781）。
  updatedAt?: string;
  installed: boolean;
  installedVersion: string;
  updateAvailable: boolean;
};

export type ScriptMarketResult = CommandResult<{
  market: {
    status: string;
    message: string;
    indexUrl: string;
    updatedAt: string;
    scripts: ScriptMarketItem[];
    /** 精选拓展包（含官方），锁定到审核过的提交。老版本后端没有这个字段。 */
    packages?: CuratedPackage[];
    blockedInstalled?: BlockedInstall[];
  };
  user_scripts: UserScriptInventory;
}>;
export type Theme = "dark" | "light";

export type AutoSaveOutcome = { status: "saved" } | { status: "invalid" | "failed"; message: string };

export type Actions = {
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
  /// 乐观写入表单（不落盘）：拖拽排序松手时先让列表立即重排，落位动画才连贯。
  previewSettingsForm: (settings: BackendSettings) => void;
  refreshSettings: (silent?: boolean) => Promise<BackendSettings | null>;
  resetSettings: () => Promise<void>;
  exportSettings: () => Promise<void>;
  importSettings: () => Promise<void>;
  chooseSettingsPath: () => Promise<void>;
  resetSettingsPath: () => Promise<void>;
  chooseCodexAppPath: (mode: "folder" | "file") => Promise<void>;
  clearCodexAppPath: () => Promise<void>;
  restoreDreamSkin: () => Promise<void>;
  saveManualCodexAppPath: () => Promise<void>;
  syncProvidersNow: () => Promise<void>;
  recoverProviderSync: () => Promise<void>;
  repairSessionIndex: () => Promise<void>;
  refreshProviderSyncTargets: (silent?: boolean) => Promise<ProviderSyncTargetsResult | null>;
  setProviderSyncTarget: (provider: string) => void;
  refreshRelayFiles: () => Promise<RelayFilesResult | null>;
  refreshEnvConflicts: (silent?: boolean) => Promise<EnvConflictsResult | null>;
  refreshRelayEnvironment: (silent?: boolean) => Promise<RelayEnvironmentResult | null>;
  removeEnvConflicts: (names: string[]) => Promise<void>;
  refreshCcsProviders: (silent?: boolean) => Promise<CcsProvidersResult | null>;
  /** `only` 是导入对话框里勾选的条目（`sourceId`）；不传就是全部。 */
  importCcsProviders: (bindings?: Record<string, string>, only?: string[]) => Promise<void>;
  importMagpieProviders: (ids: string[]) => Promise<void>;
  refreshLiveContextEntries: () => Promise<LiveContextEntriesResult | null>;
  syncLiveContextEntries: (settings: BackendSettings, silent?: boolean) => Promise<LiveContextEntriesResult | null>;
  refreshAds: () => Promise<void>;
  refreshScriptMarket: (silent?: boolean, preserveRuntime?: boolean, force?: boolean) => Promise<void>;
  refreshUserScriptInventory: () => Promise<SettingsResult | null>;
  reloadUserScripts: () => Promise<void>;
  installMarketScript: (id: string) => Promise<void>;
  setUserScriptsEnabled: (enabled: boolean, host?: ExtensionHost) => Promise<void>;
  setUserScriptsSkipNextStart: (skip: boolean) => Promise<void>;
  setUserScriptEnabled: (key: string, enabled: boolean, host?: ExtensionHost) => Promise<void>;
  setExtensionConfig: (key: string, values: Record<string, unknown>, options?: { silent?: boolean; merge?: boolean }) => Promise<boolean>;
  /** 拓展面板里的 codexPlusManager.navigate(target)：跳得过去返回 true。 */
  navigateFromExtension: (target: string) => boolean;
  showExtensionNotice: (title: string, message: string) => void;
  deleteUserScript: (key: string) => Promise<void>;
  refreshLocalSessions: (silent?: boolean, offset?: number, filter?: LocalSessionFilter) => Promise<LocalSessionsResult | null>;
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
  applyContextEntry: (settings: BackendSettings, kind: ContextKind, id: string, tomlBody: string | null) => Promise<boolean>;
  previewMcpServersJson: (json: string) => Promise<McpImportPreviewResult | null>;
  importMcpServersJson: (settings: BackendSettings, json: string) => Promise<BackendSettings | null>;
  extractRelayCommonConfig: (configContents: string) => Promise<ExtractRelayCommonConfigResult | null>;
  testRelayProfile: (profile: RelayProfile) => Promise<void>;
  diagnoseRelayProfile: (profile: RelayProfile) => Promise<ProviderDoctorResult | null>;
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
  setThemePreference: (preference: ThemePreference) => void;
  checkHealth: () => Promise<void>;
};

/**
 * 右下角的提示卡：左侧状态色条 + 图标，按成功 / 警告 / 失败 / 信息分色。
 * 鼠标停在卡片上时暂停倒计时；失败提示停留更久，并可一键复制详情。
 */
export type NoticeAction = { label: string; run: () => void };
export type NoticeState = { title: string; message: string; status?: Status; action?: NoticeAction };

export type ClaudeBashTimeoutSettings = {
  enabled: boolean;
  defaultEnabled: boolean;
  defaultSeconds: number;
  maxEnabled: boolean;
  maxSeconds: number;
};

export type ClaudeLocalConfig = {
  enabled: boolean;
  model: string;
  localization: boolean;
  voiceInput: boolean;
  computerUse: boolean;
  backgroundMode: boolean;
  /** 无视 Claude 对后台模式的 macOS 版本上限（默认关）。 */
  forceBackgroundMode?: boolean;
  displayName?: string;
  email?: string;
  plan?: string;
  sessionSync?: boolean;
  listAllModels?: boolean;
  hideUnroutedModels?: boolean;
  /** Code 页签是否和命令行共用 ~/.claude；不写按默认（共用）。 */
  codeShareHome?: boolean;
  /** 启动前把命令行建的会话加进 Desktop 的会话列表；不写按默认（开）。 */
  importCliSessions?: boolean;
  /** Cowork 总开关；不写按默认（开）。 */
  cowork?: boolean;
  /** 合并式主页（Claude 的实验布局）；不写按默认（关）。 */
  hubHome?: boolean;
  /** 界面用哪个录制版本（build id）；空串按最新的。 */
  template?: string;
};

/** 内嵌的一个录制版本；builtAt 是该版本前端的构建时间（秒）。 */
export type ClaudeTemplateVersion = { build: string; builtAt?: number | null };

export type ClaudeLocalStatus = {
  supported: boolean;
  sourceApp: string;
  version: string;
  claudeRunning: boolean;
  running: boolean;
  /** Claude 仍挂在本机代理上（代理进程可能已经退了）：这种半开状态也要能按「停止」收尾。 */
  localProcess?: boolean;
  connected: boolean;
  port?: number | null;
  caReady: boolean;
  caTrusted: boolean;
  /** 系统级根证书（Cowork 虚拟机用）：只有 Windows 支持。 */
  caMachineSupported?: boolean;
  caMachineTrusted?: boolean;
  /** 内嵌的录制版本，新的在前。 */
  templates?: ClaudeTemplateVersion[];
  nodeAvailable: boolean;
  dictationReady: boolean;
  config: ClaudeLocalConfig;
  models: string[];
  /** 当前默认模型实际下发的上下文窗口（供应商声明值）；null / 缺省表示未声明，Claude 按 200k 处理。 */
  contextWindow?: number | null;
  /** 因为窗口比默认模型小、没有列给 Claude 的模型。 */
  hiddenModels?: string[];
  warning?: string | null;
  needsRestart: boolean;
};

export type ClaudeLocalizationStatus = {
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
  extensions: boolean;
  computerUse: boolean;
  /** 当前平台能否真正应用 Computer Use；Windows 为 false（#2673）。 */
  computerUseSupported: boolean;
  bashTimeout: ClaudeBashTimeoutSettings;
  bashTimeoutWarning?: string | null;
  bashTimeoutPending?: boolean;
};
