/** 网关模式的前端数据模型：与 `codex_plus_core::gateway` 的序列化结构一一对应，外加表单转换。
 *
 * 网关模式与直连模式完全独立：供应商池、模型组只属于网关；所有接入的 Agent 共用整个池。
 * 模型名三种写法：裸模型名（池里提供它的供应商依次尝试）、`<供应商 id>/<模型>`、`group/<组 id>`。
 */
import { PRESETS } from "./presets.ts";

export type GatewayAgentId = "codex" | "claude" | "grok";
export type UnknownModelPolicy = "mapToDefault" | "passthrough";
export type KeyRotationMode = "sticky" | "roundRobin";
export type GroupMode = "inOrder" | "rotate" | "weighted" | "usage" | "fast" | "manual";
/** 同一会话在组里固定用哪个成员：缓存有效期内（默认）/ 整个会话 / 不固定。 */
export type GroupSticky = "cache" | "session" | "off";

export type ClaudeRoleModels = { opus: string; sonnet: string; haiku: string; subagent: string };

export type AgentBinding = {
  enabled: boolean;
  model: string;
  extraModels: string[];
  claudeRoles: ClaudeRoleModels;
  unknownModel: UnknownModelPolicy;
  /** 审批模型（#2703，Codex 用）：codex-auto-review 改发到这个模型；空或没有是不单独指定（旧版后端没有）。 */
  autoReviewModel?: string;
  /** 起名模型（Codex 用）：Codex 给会话起名的请求改发到这个模型；空或没有是不改（旧版后端没有）。 */
  titleModel?: string;
  contextWindow?: number | null;
  claudeEnableEffort: boolean;
  /** 这个 Agent 不用的池内供应商（「挑选」里取消勾选的）。 */
  poolExclude?: string[];
  /** 往池里添加供应商时默认也给它用（旧版后端没有这个字段，视为开）。 */
  autoJoinProviders?: boolean;
  token: string;
  restore?: { claudePreviousProviderId?: string; appliedPort?: number | null };
};

export type GatewaySettings = {
  version: number;
  bindings: Partial<Record<GatewayAgentId, AgentBinding>>;
  keyPolicy: { mode: KeyRotationMode };
  restoreOnExit: boolean;
  /** 左键点托盘图标弹出快捷面板（旧版后端没有这个字段，视为开）。 */
  trayPanel?: boolean;
  /** 多家都有的模型额外列一份「自动」（按顺序在各家之间切换）。默认关。 */
  autoModels?: boolean;
  /** 多家都有的模型在 Codex 里名字后面带不带「（供应商）」（旧版后端没有，视为带）。 */
  modelSuffix?: boolean;
  /** 模型收藏（管理器模型下拉里的星标），Codex 的选择器里也显示。 */
  favoriteModels?: string[];
  /** 各 Agent 的生图模型（键是 Agent id）：没有或空是自动，`off` 是关闭，其余是 `<供应商 id>/<模型>`。 */
  imageModels?: Record<string, string>;
  /** 生图结果只给链接时原样转发、不下载成图片数据的 Agent（默认都转）。 */
  imageKeepUrls?: string[];
};

export type GatewayPoolEntry = {
  id: string;
  name: string;
  baseUrl: string;
  protocol: "responses" | "chatCompletions" | "anthropicMessages";
  official: boolean;
  models: string[];
  keyCount: number;
  /** 后端旧版没有这两个字段（直连模式里挑出来的供应商列表）。 */
  enabled?: boolean;
  source?: string;
  /** 每百万 token 的价格。 */
  price?: ProviderPrice | null;
  /** 按模型的价格（上游模型名 → 价格）；没填的模型用 `price`。 */
  modelPrices?: Record<string, ProviderPrice>;
  /** 能画图的模型（不进聊天模型列表）；ChatGPT 账号是内置的一项。旧版后端没有。 */
  imageModels?: string[];
  /** 按模型的模型资料（思考等级等）JSON，格式同直连模式的「模型配置」。 */
  modelMetadata?: string;
  /** 按模型的上下文窗口（上游模型名 → token）；没写的模型用 `contextWindow`。 */
  modelWindows?: Record<string, number>;
  /** 没单独写窗口的模型用的窗口；都没有就用模型自带资料。 */
  contextWindow?: number | null;
  /** 网关供应商单独走的 HTTP 代理，空串表示不单独走代理。 */
  proxy?: string;
  /** 主动限速：每个 Key 每分钟最多请求数；没有或 null 表示不限。旧版后端没有。 */
  keyRpm?: number | null;
  /** 主动限速：每个 Key 同时进行的最多请求数；没有或 null 表示不限。旧版后端没有。 */
  keyConcurrency?: number | null;
  /** 转发前移除托管生图工具（image_generation，#228）。旧版后端没有。 */
  stripImageGenerationTool?: boolean;
  /** 登录得到的账号：chatgpt / copilot / grok；普通供应商为空。 */
  account?: string;
  /** 订阅账号的套餐，如「ChatGPT Pro」「Copilot Pro+」。 */
  plan?: string;
  /** 后台检查时上游有、列表里没有的模型。 */
  upstreamNew?: string[];
  /** 后台检查时列表里有、上游已经没有的模型。 */
  upstreamGone?: string[];
  /** 这个 ChatGPT 账号正借给 Codex 登录。 */
  lentToCodex?: boolean;
  /** 订阅账号的剩余额度；查不到或普通供应商为空。 */
  quota?: ProviderQuota | null;
  /** 已保存的 Key（打码，#2782）：订阅账号等不用 Key 的为空。旧版后端没有。 */
  keys?: ProviderKeyView[];
  /** 有余额接口的厂商（DeepSeek、Kimi 等，#2793）：卡片用「用量」页同一份余额。旧版后端没有。 */
  balanceApi?: boolean;
  /** new-api 访问令牌（打码，#2793）：只用来查账户余额；没配置为空。令牌原文不出后端。 */
  newapiAccess?: NewApiAccessView | null;
};

/** 已保存的 new-api 访问令牌：只有打码后的样子和用户 ID。 */
export type NewApiAccessView = { masked: string; userId: string };

/** 供应商已保存的一个 Key：后端只给标识和打码后的样子，原文不出后端。删除 / 替换按 `id`。 */
export type ProviderKeyView = { id: string; masked: string };

/** Key 池健康状态里的 Key 标识换成打码后的 Key 显示；只剩一个 Key 时 Key 池按 `primary` 记。找不到就原样显示标识。 */
export function keyLabel(keys: ProviderKeyView[] | undefined, keyId: string): string {
  const list = keys ?? [];
  const found = list.find((key) => key.id === keyId) ?? (keyId === "primary" && list.length === 1 ? list[0] : undefined);
  return found ? found.masked : keyId;
}

export type ProviderPrice = { currency: string; input: number; cachedInput: number; output: number };

export type GroupMember = { model: string; weight: number };
export type GatewayGroupEntry = {
  id: string;
  name: string;
  mode: GroupMode;
  sticky?: GroupSticky;
  members: GroupMember[];
  /** 隐藏成员：组里的模型不再单独出现在 Agent 的模型列表里，只能通过组来选。 */
  hideMembers?: boolean;
  /** 手动模式下选中的成员（成员的模型名）。 */
  pick?: string;
};

/** 自动组：池里多家都有的模型各一个，`name` 是模型名，`id` 是 `auto:<小写模型名>`。 */
export type AutoGroupEntry = GatewayGroupEntry & { hidden: boolean; customized: boolean };

/** 订阅账号的一个额度窗口：windowMinutes 300 是 5 小时、10080 是每周，null 是按月（Copilot 高级请求）。 */
export type QuotaWindow = { windowMinutes: number | null; usedPercent: number; resetsAtMs: number | null };
export type ProviderQuota = { windows: QuotaWindow[]; updatedAtMs: number };
export type SpeedView = { relayId: string; model: string; ms: number; samples: number };

export type GatewayAgentView = {
  agent: GatewayAgentId;
  name: string;
  /** 本机 CLI 版本，后台探测完成前为空。 */
  version?: string | null;
  /** 网关改写的配置文件。 */
  configPath?: string;
  enabled: boolean;
  baseUrl: string;
  defaultModel: string;
  models: string[];
  /** 开着网关、但配置已被别的工具改掉：认得出的工具名与现在连的地址。 */
  drift?: { by?: string | null; endpoint: string } | null;
};

export type KeyHealthView = {
  relayId: string;
  keyId: string;
  state: "ok" | "cooling" | "disabled";
  remainingMs?: number | null;
  lastFailure?: string | null;
  lastFailureAtMs?: number | null;
};

export type ProviderHealthView = {
  relayId: string;
  breakerOpen: boolean;
  remainingMs?: number | null;
  consecutiveErrors: number;
};

/** 路由动态里的一次尝试。 */
export type RouteStepView = {
  relayId: string;
  relayName: string;
  keyId: string;
  model: string;
  outcome: "answered" | "failed" | "transport";
  failure?: string | null;
  status?: number | null;
  elapsedMs: number;
};

export type RouteRecordView = {
  requestedModel: string;
  model: string;
  groupId?: string | null;
  planned: Array<{ relayId: string; relayName: string; model: string }>;
  steps: RouteStepView[];
  result: "answered" | "failed" | "cooling";
  coolingMs?: number | null;
  finishedAtMs: number;
  elapsedMs: number;
};

export type AgentActivityView = {
  agent: string;
  requests: number;
  rerouted: number;
  errors: number;
  inFlight: string[];
  last?: RouteRecordView | null;
};

export type GatewayPayload = {
  gateway: GatewaySettings;
  port: number;
  helperRunning: boolean;
  /** 网关自己的供应商池（不含 Key）。 */
  pool: GatewayPoolEntry[];
  groups: GatewayGroupEntry[];
  agents: GatewayAgentView[];
  health: { keys: KeyHealthView[]; providers: ProviderHealthView[] };
  /** 旧版后端没有这个字段。 */
  activity?: { agents: AgentActivityView[] };
  /** 各（供应商, 模型）最近的平均首包耗时；模型名是小写。 */
  speeds?: SpeedView[];
  /** 可复制进网关的直连模式供应商。 */
  codexProviders: GatewayPoolEntry[];
  claudeProviders: Array<{ id: string; name: string }>;
  grokProfiles: Array<{ id: string; name: string }>;
  /** OpenCode 里保存的 OpenCode Go / Zen Key。 */
  opencodeKeys?: Array<{ id: string; name: string }>;
  /** 直连模式里能搬成模型组的聚合供应商（按模型聚合、没有成员的不在这里）：成员进池，建同名模型组。 */
  codexAggregates?: Array<{ id: string; name: string; members: number; mode: GroupMode; sticky: GroupSticky; imported: boolean }>;
  /** 直连模式的官方登录账号：不能复制，要在网关里单独登录。名字已打码。 */
  codexOfficial?: Array<{ id: string; name: string }>;
  /** 自动组（旧版后端没有这个字段）。 */
  autoGroups?: AutoGroupEntry[];
  /** 「生图模型」能选的：ChatGPT 订阅账号和各家的生图模型（旧版后端没有）。 */
  imageOptions?: ImageOption[];
  /** 生图选「自动」时先用的那一项。 */
  imageAuto?: ImageOption | null;
};

export type ImageOption = {
  /** 存进 `imageModel` 的值：`<供应商 id>/<模型>`。 */
  id: string;
  providerId: string;
  providerName: string;
  model: string;
  /** ChatGPT 订阅账号：后端自己选生图模型。 */
  vendorPicks: boolean;
};

export type GatewayResult = { status: string; message: string } & Partial<GatewayPayload>;

/** 编辑表单：多行文本、数字都先当字符串处理，提交时再转。 */
export type BindingDraft = {
  model: string;
  extraModels: string;
  opus: string;
  sonnet: string;
  haiku: string;
  subagent: string;
  unknownModel: UnknownModelPolicy;
  autoReviewModel: string;
  titleModel: string;
  contextWindow: string;
  claudeEnableEffort: boolean;
  poolExclude: string[];
  autoJoinProviders: boolean;
};

export function blankBinding(): AgentBinding {
  return {
    enabled: false,
    model: "",
    extraModels: [],
    claudeRoles: { opus: "", sonnet: "", haiku: "", subagent: "" },
    unknownModel: "mapToDefault",
    contextWindow: null,
    claudeEnableEffort: false,
    token: "",
  };
}

export function draftFromBinding(binding: AgentBinding | undefined): BindingDraft {
  const source = binding ?? blankBinding();
  return {
    model: source.model,
    extraModels: source.extraModels.join("\n"),
    opus: source.claudeRoles.opus,
    sonnet: source.claudeRoles.sonnet,
    haiku: source.claudeRoles.haiku,
    subagent: source.claudeRoles.subagent,
    unknownModel: source.unknownModel,
    autoReviewModel: source.autoReviewModel ?? "",
    titleModel: source.titleModel ?? "",
    contextWindow: source.contextWindow ? String(source.contextWindow) : "",
    claudeEnableEffort: source.claudeEnableEffort,
    poolExclude: [...(source.poolExclude ?? [])],
    autoJoinProviders: source.autoJoinProviders !== false,
  };
}

/** 添加供应商时默认勾选的 Agent：没关掉「添加供应商时自动加入」的。 */
export function defaultProviderAgents(payload: GatewayPayload | null): GatewayAgentId[] {
  return (payload?.agents ?? [])
    .map((agent) => agent.agent)
    .filter((agent) => payload?.gateway.bindings[agent]?.autoJoinProviders !== false);
}

/** 添加命令带上的 `skipAgents`：没勾选的 Agent。 */
export function skippedAgents(payload: GatewayPayload | null, picked: GatewayAgentId[]): GatewayAgentId[] {
  return (payload?.agents ?? []).map((agent) => agent.agent).filter((agent) => !picked.includes(agent));
}

/** 窗口支持纯数字和 `200k` / `1M` 写法。无效时返回 NaN。 */
export function parseContextWindow(value: string): number | null {
  const text = value.trim().toLowerCase();
  if (!text) return null;
  const match = /^(\d+(?:\.\d+)?)\s*([km]?)$/.exec(text);
  if (!match) return Number.NaN;
  const base = Number(match[1]);
  const scale = match[2] === "m" ? 1_000_000 : match[2] === "k" ? 1_000 : 1;
  return Math.round(base * scale);
}

/** 主动限速的上限：留空是不限（null），正整数是上限；0、小数、负数或别的写法无效（NaN）。 */
export function parseKeyLimit(value: string): number | null {
  const text = value.trim();
  if (!text) return null;
  if (!/^\d+$/.test(text)) return Number.NaN;
  const limit = Number(text);
  return limit > 0 && limit <= 1_000_000 ? limit : Number.NaN;
}

/** 请求日志「重试原因」里网关记的限速排队：`local_rate_limit_wait_ms:<毫秒>`。 */
export const QUEUE_REASON_PREFIX = "local_rate_limit_wait_ms:";

/**
 * 请求日志「重试原因」里的说明：换上游后被拒收，去掉了别家签发的压缩上下文（compaction），
 * 较早的对话内容上游已经看不到。只是说明，不算一次重试。
 */
export const FOREIGN_COMPACTION_DROPPED_REASON = "foreign_compaction_dropped";

/** 这次请求是否去掉过别家签发的压缩上下文。 */
export function foreignCompactionDropped(reasons: string[] | null | undefined): boolean {
  return (reasons ?? []).includes(FOREIGN_COMPACTION_DROPPED_REASON);
}

/** 这次请求因 Codex++ 本地限速排队了多久（毫秒）；没排过队返回 null。 */
export function queueWaitMs(reasons: string[] | null | undefined): number | null {
  const reason = (reasons ?? []).find((item) => item.startsWith(QUEUE_REASON_PREFIX));
  if (reason === undefined) return null;
  const ms = Number(reason.slice(QUEUE_REASON_PREFIX.length));
  return Number.isFinite(ms) && ms >= 0 ? ms : null;
}

/** 排队时长的写法：10 秒内保留一位小数（0.4s、3.2s），再长取整（45s）。 */
export function formatQueueWait(ms: number): string {
  return ms < 10_000 ? `${(ms / 1000).toFixed(1)}s` : `${Math.round(ms / 1000)}s`;
}

export function splitModels(text: string): string[] {
  const seen = new Set<string>();
  return text
    .split(/[\n,]/)
    .map((line) => line.trim())
    .filter((line) => {
      if (!line || seen.has(line)) return false;
      seen.add(line);
      return true;
    });
}

/** 表单 → 提交给后端的绑定。token 与恢复现场由后端维护，这里沿用旧值。 */
export function bindingFromDraft(draft: BindingDraft, previous: AgentBinding | undefined): AgentBinding {
  const base = previous ?? blankBinding();
  const window = parseContextWindow(draft.contextWindow);
  return {
    ...base,
    model: draft.model.trim(),
    extraModels: splitModels(draft.extraModels),
    claudeRoles: {
      opus: draft.opus.trim(),
      sonnet: draft.sonnet.trim(),
      haiku: draft.haiku.trim(),
      subagent: draft.subagent.trim(),
    },
    unknownModel: draft.unknownModel,
    autoReviewModel: draft.autoReviewModel.trim(),
    titleModel: draft.titleModel.trim(),
    contextWindow: window && Number.isFinite(window) ? window : null,
    claudeEnableEffort: draft.claudeEnableEffort,
    poolExclude: [...draft.poolExclude],
    autoJoinProviders: draft.autoJoinProviders,
  };
}

export const GROUP_MODEL_PREFIX = "group/";

/** 模型组当作模型时的名字：`group/<组 id>`。 */
export function groupModelId(id: string): string {
  return `${GROUP_MODEL_PREFIX}${id}`;
}

/** 名字 → 短名，与后端 `slugify` 一致（只用于预览，真正的 id 由后端生成）。 */
export function slugify(name: string): string {
  const slug = name.trim().toLowerCase().replace(/[\s_/]+/g, "-").replace(/^-+|-+$/g, "");
  return slug || "unnamed";
}

/** 池里这个供应商是否打开。 */
export function providerEnabled(entry: GatewayPoolEntry): boolean {
  return entry.enabled !== false;
}

/** Agent 能用的池内供应商：打开的，再去掉它「挑选」里排除的。 */
export function agentPoolProviders(payload: GatewayPayload | null, exclude: string[] = []): GatewayPoolEntry[] {
  return (payload?.pool ?? []).filter((entry) => providerEnabled(entry) && !exclude.includes(entry.id));
}

/** Agent 能用的全部模型：池内供应商的模型（去重，按池顺序）+ 有成员的模型组。与后端 `pool_models` 一致。 */
export function agentModels(payload: GatewayPayload | null, exclude: string[] = []): string[] {
  const models: string[] = [];
  // 按小写去重；用 Set 查重，池大时轮询刷新也不会 O(n²)。
  const seen = new Set<string>();
  for (const entry of agentPoolProviders(payload, exclude)) {
    for (const model of entry.models) {
      const key = model.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      models.push(model);
    }
  }
  for (const group of payload?.groups ?? []) {
    if (group.members.length) models.push(groupModelId(group.id));
  }
  return models;
}

/** `<供应商 id>/<模型>` 里的供应商（必须是 Agent 能用的）。 */
export function qualifiedProvider(payload: GatewayPayload | null, model: string, exclude: string[] = []): { entry: GatewayPoolEntry; model: string } | null {
  if (model.startsWith(GROUP_MODEL_PREFIX)) return null;
  const index = model.indexOf("/");
  if (index <= 0 || !model.slice(index + 1).trim()) return null;
  const prefix = model.slice(0, index).toLowerCase();
  const entry = agentPoolProviders(payload, exclude).find((item) => item.id.toLowerCase() === prefix);
  return entry ? { entry, model: model.slice(index + 1).trim() } : null;
}

/** 模型名能否在池里路由。 */
export function modelRoutable(payload: GatewayPayload | null, model: string, exclude: string[] = []): boolean {
  const lower = model.trim().toLowerCase();
  return agentModels(payload, exclude).some((known) => known.toLowerCase() === lower) || qualifiedProvider(payload, model.trim(), exclude) !== null;
}

/** 表单校验：返回错误文案（中文原文，界面再翻译），没问题返回 null。 */
export function validateDraft(draft: BindingDraft, payload: GatewayPayload | null): string | null {
  if (payload) {
    if (!agentPoolProviders(payload, draft.poolExclude).length) return "供应商池是空的，先到「供应商」里添加";
    const model = draft.model.trim();
    if (model && !modelRoutable(payload, model, draft.poolExclude)) return "供应商池里没有这个模型";
  }
  const window = parseContextWindow(draft.contextWindow);
  if (window !== null && (!Number.isFinite(window) || window < 1_000)) return "上下文窗口格式不对，例如 200000、200k 或 1M";
  return null;
}

export function protocolLabel(protocol: GatewayPoolEntry["protocol"]): string {
  switch (protocol) {
    case "chatCompletions":
      return "Chat";
    case "anthropicMessages":
      return "Anthropic";
    default:
      return "Responses";
  }
}

/** Key 健康状态里的失败分类 → 中文说明。 */
export function failureLabel(value: string | null | undefined): string {
  switch (value) {
    case "rate_limited":
      return "被限流";
    case "quota_exhausted":
      return "额度耗尽";
    case "auth_invalid":
      return "Key 无效";
    case "server_error":
      return "上游出错";
    case "failover":
      return "已切换";
    case "client_error":
      return "请求被拒";
    default:
      return "";
  }
}

export function formatRemaining(ms: number | null | undefined): string {
  if (!ms || ms <= 0) return "";
  const seconds = Math.ceil(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  // 一天以上（每周额度的重置时间）只写到小时。
  if (hours >= 24) return `${Math.floor(hours / 24)}d${hours % 24 ? `${hours % 24}h` : ""}`;
  return `${hours}h${minutes % 60 ? `${minutes % 60}m` : ""}`;
}


/** Claude 供应商列表里网关托管的那一条。 */
export const GATEWAY_CLAUDE_PROVIDER_ID = "codex-plus-gateway";

export const GATEWAY_AGENT_NAMES: Record<string, string> = { claude: "Claude Code", grok: "Grok", codex: "Codex" };

export function agentName(agent: string): string {
  return GATEWAY_AGENT_NAMES[agent] ?? agent;
}

export type PickerItem = {
  /** 选中后写进绑定的模型名。 */
  model: string;
  label: string;
  sub: string;
  avatar: string;
  custom?: boolean;
};

export type PickerSection = { key: string; title: string; avatar: string; items: PickerItem[] };

function matches(query: string, ...values: string[]): boolean {
  if (!query) return true;
  return values.some((value) => value.toLowerCase().includes(query));
}

/**
 * 模型选择器的分组：每个能用的池内供应商一组列出它的模型，模型组（`group/<id>`）排在后面。
 * 查询词不是任何已知模型时，另给一组「自定义模型」：`<供应商 id>/<输入>`，直接发给那个供应商。
 * `skipGroup` 用来在编辑模型组时排除它自己。`avatar` 由调用方算（避免这里依赖界面工具函数）。
 */
export function pickerSections(
  payload: GatewayPayload | null,
  rawQuery: string,
  avatar: (name: string) => string,
  exclude: string[] = [],
  skipGroup?: string,
): PickerSection[] {
  if (!payload) return [];
  const query = rawQuery.trim().toLowerCase();
  const sections: PickerSection[] = [];
  const providers = agentPoolProviders(payload, exclude);
  // 多家都有的模型：每家名下那份固定发给这一家（`<供应商 id>/<模型>`）；它的自动组生效时另列一份裸名，
  // 选它按自动组的方式走。只有一家的模型用裸名。隐藏了成员的组，成员不单独列出。与写给 Codex 的模型列表一致。
  const unlisted = unlistedModels(payload);
  const shared: string[] = [];
  for (const entry of providers) {
    const providerHit = matches(query, entry.name, entry.id);
    const items = entry.models
      .filter((model) => providerHit || matches(query, model))
      .flatMap((model) => {
        const multi = servingProviders(payload, model, exclude).length > 1;
        if (multi && !shared.some((known) => known.toLowerCase() === model.toLowerCase())) shared.push(model);
        const value = multi ? `${entry.id}/${model}` : model;
        if (unlisted.has(value.toLowerCase())) return [];
        return [{ model: value, label: prettyModelName(model), sub: maskAccountName(entry.name), avatar: avatar(entry.name) }];
      });
    if (items.length) sections.push({ key: `provider:${entry.id}`, title: maskAccountName(entry.name), avatar: avatar(entry.name), items });
  }
  const autoItems = shared
    .filter((model) => activeAutoGroup(payload, model))
    .map((model) => ({
      model,
      label: prettyModelName(model),
      sub: servingProviders(payload, model, exclude).map((entry) => maskAccountName(entry.name)).join(" → "),
      avatar: "",
    }));
  if (autoItems.length) sections.push({ key: "auto", title: "自动组", avatar: "", items: autoItems });
  const groups = payload.groups
    .filter((group) => group.id !== skipGroup && group.members.length && matches(query, group.name, groupModelId(group.id)))
    .map((group) => ({ model: groupModelId(group.id), label: group.name, sub: groupModelId(group.id), avatar: avatar(group.name) }));
  if (groups.length) sections.push({ key: "group", title: "我的组", avatar: "", items: groups });
  const typed = rawQuery.trim();
  if (typed && !/\s/.test(typed) && !modelRoutable(payload, typed, exclude)) {
    sections.push({
      key: "custom",
      title: "自定义模型",
      avatar: "",
      items: providers.slice(0, 6).map((entry) => ({
        model: `${entry.id}/${typed}`,
        label: typed,
        sub: maskAccountName(entry.name),
        avatar: avatar(entry.name),
        custom: true,
      })),
    });
  }
  return sections;
}

/** `auto`：多家都有的模型的裸名，按池顺序在这几家之间自动切换（`sub` 是这几家，按尝试顺序）。 */
export type ModelView = { title: string; sub: string; group: boolean; auto?: boolean };

/** 提供这个模型的供应商（Agent 能用的、按池顺序）。 */
export function servingProviders(payload: GatewayPayload | null, model: string, exclude: string[] = []): GatewayPoolEntry[] {
  const name = model.trim().toLowerCase();
  return agentPoolProviders(payload, exclude).filter((entry) => entry.models.some((known) => known.toLowerCase() === name));
}

/** 一个模型名在按钮上的显示：模型名 + 提供它的供应商；模型组显示组名。空串用池里第一个模型。 */
export function modelDisplay(payload: GatewayPayload | null, model: string, exclude: string[] = []): ModelView {
  const title = model.trim() || agentModels(payload, exclude)[0] || "";
  if (title.startsWith(GROUP_MODEL_PREFIX)) {
    const group = payload?.groups.find((item) => groupModelId(item.id) === title);
    return { title: group?.name ?? title, sub: title, group: true };
  }
  const qualified = qualifiedProvider(payload, title, exclude);
  if (qualified) return { title: prettyModelName(qualified.model), sub: maskAccountName(qualified.entry.name), group: false };
  const serving = servingProviders(payload, title, exclude);
  if (serving.length > 1) {
    return { title: prettyModelName(title), sub: serving.map((entry) => maskAccountName(entry.name)).join(" → "), group: false, auto: true };
  }
  const entry = serving[0];
  return { title: prettyModelName(title), sub: entry ? maskAccountName(entry.name) : "", group: false };
}

/** 品牌词的正式写法。 */
const MODEL_WORDS: Record<string, string> = {
  gpt: "GPT",
  glm: "GLM",
  deepseek: "DeepSeek",
  minimax: "MiniMax",
  qwen: "Qwen",
  qwq: "QwQ",
  kimi: "Kimi",
  grok: "Grok",
  claude: "Claude",
  gemini: "Gemini",
  gemma: "Gemma",
  llama: "Llama",
  mistral: "Mistral",
  codestral: "Codestral",
  doubao: "Doubao",
  ernie: "ERNIE",
  hunyuan: "Hunyuan",
  step: "Step",
  mimo: "MiMo",
  longcat: "LongCat",
};

/** 一段模型名：品牌词按正式写法；`v4`、`k2`、`r1` 这类字母 + 数字大写；`235b` → `235B`；`o3` 这类 OpenAI 推理模型保持小写；其余首字母大写。 */
function prettyModelToken(token: string): string {
  const lower = token.toLowerCase();
  if (MODEL_WORDS[lower]) return MODEL_WORDS[lower];
  if (/^o\d/.test(lower)) return lower;
  if (/^[a-z]\d+(\.\d+)?[a-z]?$/.test(lower)) return lower.toUpperCase();
  if (/^\d+(\.\d+)?[bkm]$/.test(lower)) return lower.toUpperCase();
  if (/^\d/.test(lower)) return lower;
  const word = /^([a-z]+)(.*)$/.exec(lower);
  if (!word) return token;
  const head = MODEL_WORDS[word[1]] ?? word[1].charAt(0).toUpperCase() + word[1].slice(1);
  return head + word[2];
}

/** 模型 ID 的显示写法，如 `gpt-6.1-sol` → `GPT-6.1-Sol`、`deepseek-v4-pro` → `DeepSeek-V4-Pro`。
 * 只用于显示，请求和配置里仍用原始 ID；`vendor/` 前缀和 `[1M]` 后缀原样保留。 */
export function prettyModelName(model: string): string {
  const match = /^(.*\/)?([^/[]+)(\[.*\])?$/.exec(model.trim());
  if (!match) return model;
  const [, prefix = "", name, suffix = ""] = match;
  if (/[A-Z]/.test(name)) return model.trim();
  return prefix + name.split(/([-_])/).map((part) => (part === "-" || part === "_" ? part : prettyModelToken(part))).join("") + suffix;
}

export function groupModeLabel(mode: string): string {
  switch (mode) {
    case "rotate":
      return "轮换";
    case "weighted":
      return "按权重";
    case "usage":
      return "额度优先";
    case "fast":
      return "速度优先";
    case "manual":
      return "手动";
    default:
      return "依次";
  }
}

export function groupModeDescription(mode: string): string {
  switch (mode) {
    case "rotate":
      return "轮换：每个新会话从下一个成员开始；成员答不了时换下一个，每个成员再按自己的 Key 轮换。";
    case "weighted":
      return "按权重：按成员权重决定先试谁；成员答不了时换下一个，每个成员再按自己的 Key 轮换。";
    case "usage":
      return "额度优先：订阅账号里剩余额度最多的先试；没有额度数据的排在后面，额度用完的排最后。";
    case "fast":
      return "速度优先：最近响应最快的先试；还没测过的先试一次，之后按实测速度排。";
    case "manual":
      return "手动：只发给选中的那个成员，它答不了也不换；在成员列表里点一下就换成它。";
    default:
      return "依次：按成员顺序，前一个答不了才换下一个，每个成员再按自己的 Key 轮换。";
  }
}

export function groupStickyLabel(sticky: string | undefined): string {
  switch (sticky) {
    case "session":
      return "整个会话固定";
    case "off":
      return "不固定";
    default:
      return "缓存期内固定";
  }
}

export function groupStickyDescription(sticky: string | undefined): string {
  switch (sticky) {
    case "session":
      return "同一会话一直先试上次回答的成员，它答不了才换，最省缓存。";
    case "off":
      return "每个请求都按模式重新排，会话中途可能换成员，上游的提示词缓存会失效。";
    default:
      return "同一会话 10 分钟内继续先试上次回答的成员，保住上游的提示词缓存；空闲超过 10 分钟再按模式重新排。";
  }
}

/** 权重输入框的文字 → 权重：空串、读不成数时返回 null（保留原值），否则取 ≥ 1 的整数。 */
export function parseWeight(text: string): number | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  if (!Number.isFinite(value)) return null;
  return Math.max(1, Math.floor(value));
}

/** 通配符成员：带 `*`，按 `<供应商 id>/<模型>` 或裸模型名匹配池里现有的模型。 */
export function isPattern(model: string): boolean {
  return model.includes("*");
}

/** 与后端 `route::glob_matches` 一致：`*` 匹配任意一段字符（包括 `/`），不区分大小写。 */
export function globMatches(pattern: string, text: string): boolean {
  const escaped = pattern.toLowerCase().split("*").map((part) => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&"));
  return new RegExp(`^${escaped.join(".*")}$`, "s").test(text.toLowerCase());
}

/** 通配符现在匹配到的 `<供应商 id>/<模型>`：打开的供应商，按池顺序。 */
export function patternMatches(payload: GatewayPayload | null, pattern: string): string[] {
  const matches: string[] = [];
  for (const entry of agentPoolProviders(payload)) {
    for (const model of entry.models) {
      const qualified = `${entry.id}/${model}`;
      if (globMatches(pattern, qualified) || globMatches(pattern, model)) matches.push(qualified);
    }
  }
  return matches;
}

/** 某个上游最近的平均首包耗时（毫秒），没测过为 null。 */
export function speedOf(payload: GatewayPayload | null, relayId: string, model: string): number | null {
  const found = payload?.speeds?.find((item) => item.relayId === relayId && item.model === model.toLowerCase());
  return found ? found.ms : null;
}

/** 额度窗口的名字（中文模板 + 参数，界面再翻译）：5 小时 / 每周 / 本月，其它按时长写。 */
export function quotaWindowLabel(window: QuotaWindow): { text: string; args: number[] } {
  const minutes = window.windowMinutes;
  if (minutes === null) return { text: "本月", args: [] };
  if (minutes === 300) return { text: "5 小时", args: [] };
  if (minutes === 10080) return { text: "每周", args: [] };
  if (minutes % 1440 === 0) return { text: "{0} 天", args: [minutes / 1440] };
  if (minutes % 60 === 0) return { text: "{0} 小时", args: [minutes / 60] };
  return { text: "{0} 分钟", args: [minutes] };
}

/** 剩余比例（0–1），取最紧的窗口；过了重置时间的窗口按满额。与后端 `quota::remaining_at` 一致。 */
export function quotaRemaining(quota: ProviderQuota, nowMs: number): number {
  return quota.windows.reduce((least, window) => {
    const left = window.resetsAtMs !== null && window.resetsAtMs <= nowMs ? 1 : Math.min(1, Math.max(0, 1 - window.usedPercent / 100));
    return Math.min(least, left);
  }, 1);
}

/** 模型组成员在列表里的显示：「供应商 · 模型」「组名」，通配符原样显示。 */
export function memberLabel(payload: GatewayPayload | null, model: string): string {
  if (isPattern(model)) return model;
  const view = modelDisplay(payload, model);
  if (view.group) return view.title;
  // 多家自动切换：模型名在前，后面是按顺序尝试的几家。
  if (view.auto) return `${view.title} · ${view.sub}`;
  return view.sub ? `${view.sub} · ${view.title}` : view.title;
}

export type MemberStatus = { state: "busy" | "answered" | "cooling" | "idle"; text: string; args: Array<string | number> };

/** 供应商在调度表、总览里的状态：正在回答 / 熔断或 Key 全部冷却 / 10 分钟内哪个 Agent 用过 / Key 数。文案是中文原文，界面再翻译。 */
/** 「某 Agent 刚用过」只显示这么久，之后回到套餐 / Key 数。 */
export const RECENT_USE_MS = 10 * 60 * 1000;

export function memberStatus(payload: GatewayPayload | null, relayId: string, nowMs: number = Date.now()): MemberStatus {
  const activity = payload?.activity?.agents ?? [];
  if (activity.some((agent) => agent.inFlight.includes(relayId))) {
    return { state: "busy", text: "正在回答…", args: [] };
  }
  const breaker = payload?.health.providers.find((item) => item.relayId === relayId && item.breakerOpen);
  if (breaker) {
    const left = formatRemaining(breaker.remainingMs);
    return left ? { state: "cooling", text: "连续出错 · {0} 后恢复", args: [left] } : { state: "cooling", text: "连续出错", args: [] };
  }
  const entry = payload?.pool.find((item) => item.id === relayId);
  const keyCount = entry?.keyCount ?? 1;
  const unhealthy = (payload?.health.keys ?? []).filter((key) => key.relayId === relayId && key.state !== "ok");
  if (unhealthy.length >= keyCount && unhealthy.length > 0) {
    const cooling = unhealthy.filter((key) => key.state === "cooling");
    if (!cooling.length) return { state: "cooling", text: "Key 都已停用", args: [] };
    // 只看还有剩余时间的 Key：剩余时间为 null / 0 的不该把「最快恢复」拉成 0。
    const waits = cooling.map((key) => key.remainingMs ?? 0).filter((ms) => ms > 0);
    const left = waits.length ? formatRemaining(Math.min(...waits)) : "";
    const quota = cooling.some((key) => key.lastFailure === "quota_exhausted");
    if (!left) return { state: "cooling", text: quota ? "额度耗尽" : "被限流", args: [] };
    return { state: "cooling", text: quota ? "额度耗尽 · {0} 后恢复" : "被限流 · {0} 后恢复", args: [left] };
  }
  // 最近 10 分钟内由它回答过的 Agent 请求：显示「Codex 刚用过」。
  const answered = activity
    .filter((agent) => agent.last?.result === "answered" && agent.last.steps.at(-1)?.relayId === relayId && nowMs - agent.last.finishedAtMs < RECENT_USE_MS)
    .sort((left, right) => (right.last?.finishedAtMs ?? 0) - (left.last?.finishedAtMs ?? 0))[0];
  if (answered) return { state: "answered", text: "{0} 刚用过", args: [agentName(answered.agent)] };
  if (unhealthy.length) return { state: "idle", text: "{0} 个 Key · {1} 个冷却中", args: [keyCount, unhealthy.length] };
  return { state: "idle", text: "{0} 个 Key", args: [Math.max(keyCount, 1)] };
}

/**
 * 供应商池「按模型」矩阵、路由页组成员里一格（供应商 × 模型）的状态：冷却、熔断按整家供应商算；「刚用过」只标在真正被请求的那个模型上。
 * 其它情况不显示（返回 null）。
 */
export function dispatchStatus(payload: GatewayPayload | null, relayId: string, model: string, nowMs: number = Date.now()): MemberStatus | null {
  const status = memberStatus(payload, relayId, nowMs);
  if (status.state === "cooling") return status;
  const used = (payload?.activity?.agents ?? [])
    .filter((agent) => {
      const step = agent.last?.result === "answered" ? agent.last.steps.at(-1) : undefined;
      return step?.relayId === relayId && step.model.toLowerCase() === model.toLowerCase() && nowMs - (agent.last?.finishedAtMs ?? 0) < RECENT_USE_MS;
    })
    .sort((left, right) => (right.last?.finishedAtMs ?? 0) - (left.last?.finishedAtMs ?? 0))[0];
  return used ? { state: "answered", text: "{0} 刚用过", args: [agentName(used.agent)] } : null;
}

export function formatElapsed(ms: number): string {
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.round(ms / 1000)} s`;
}

/** 路由说明的参数：`{ zh }` 是界面文案（失败原因等），显示前要翻译；其它（供应商名、模型名、Key id）原样显示。 */
export type NarrativeArg = string | number | { zh: string };
export type NarrativeLine = { tone: "muted" | "step" | "ok" | "fail"; text: string; args: NarrativeArg[] };

/** 总览「最近请求」展开后的路由经过：按记录逐条生成。文案是中文模板，界面用 tf 翻译。 */
export function routeNarrative(agent: string, record: RouteRecordView, payload: GatewayPayload | null): NarrativeLine[] {
  const lines: NarrativeLine[] = [];
  const who = agentName(agent);
  const requested = record.requestedModel || record.model;
  const plan = record.planned.map((member) => `${maskAccountName(member.relayName)}/${member.model}`).join(", ");
  if (record.groupId) {
    const group = payload?.groups.find((item) => item.id === record.groupId);
    lines.push({ tone: "muted", text: "{0} 请求 {1}，交给模型组「{2}」：{3}", args: [who, requested, group?.name ?? record.groupId, plan] });
    if (record.planned[0]) lines.push({ tone: "step", text: "{0} 排在第一个。", args: [maskAccountName(record.planned[0].relayName)] });
  } else {
    lines.push({ tone: "muted", text: "{0} 请求 {1}，转发到 {2}", args: [who, requested, plan] });
  }
  record.steps.forEach((step, index) => {
    const last = index === record.steps.length - 1;
    if (step.outcome === "answered") {
      lines.push({ tone: "ok", text: "{0}（{1}）回答，用时 {2}。", args: [maskAccountName(step.relayName), step.model, formatElapsed(record.elapsedMs)] });
      return;
    }
    const reason = { zh: step.outcome === "transport" ? "连不上" : failureLabel(step.failure) || "出错" };
    const status = step.status ? ` · HTTP ${step.status}` : "";
    lines.push({
      tone: last ? "fail" : "step",
      text: last ? "{0} 的 Key {1} {2}{3}，错误交回给 {4}。" : "{0} 的 Key {1} {2}{3}，换下一个。",
      args: last ? [maskAccountName(step.relayName), step.keyId, reason, status, who] : [maskAccountName(step.relayName), step.keyId, reason, status],
    });
  });
  if (record.result === "cooling") {
    lines.push({ tone: "fail", text: "所有 Key 都在冷却，约 {0} 后恢复。", args: [formatRemaining(record.coolingMs ?? 0) || "1s"] });
  }
  return lines;
}


/** 已接入网关的 Agent（它们共用整个池）。 */
export function connectedAgents(payload: GatewayPayload | null): GatewayAgentId[] {
  return (payload?.agents ?? []).filter((view) => view.enabled).map((view) => view.agent);
}

export type DispatchProvider = { id: string; name: string; baseUrl: string; excludedBy: GatewayAgentId[] };
export type DispatchRow = { model: string; providers: DispatchProvider[] };

/**
 * 供应商池「按模型」矩阵：每个模型名列出池里提供它的供应商，顺序就是网关的尝试顺序（与后端 `expand_model` 一致：
 * 打开的供应商按池顺序）。有备用的模型排前面；`excludedBy` 是在「挑选」里排除了这家的已接入 Agent。
 */
export function modelDispatch(payload: GatewayPayload | null): DispatchRow[] {
  const connected = connectedAgents(payload);
  const rows: DispatchRow[] = [];
  // 模型名（小写）→ 行；每行再记已列出的供应商 id。池大时轮询刷新也不会 O(n²)。
  const byModel = new Map<string, { row: DispatchRow; ids: Set<string> }>();
  for (const entry of agentPoolProviders(payload)) {
    const excludedBy = connected.filter((agent) => payload?.gateway.bindings[agent]?.poolExclude?.includes(entry.id));
    for (const model of entry.models) {
      const key = model.toLowerCase();
      let slot = byModel.get(key);
      if (!slot) {
        slot = { row: { model, providers: [] }, ids: new Set() };
        byModel.set(key, slot);
        rows.push(slot.row);
      }
      if (!slot.ids.has(entry.id)) {
        slot.ids.add(entry.id);
        slot.row.providers.push({ id: entry.id, name: maskAccountName(entry.name), baseUrl: entry.baseUrl, excludedBy });
      }
    }
  }
  // sort 是稳定的：同样家数的模型保持池里的先后。
  return rows.sort((left, right) => Number(right.providers.length > 1) - Number(left.providers.length > 1));
}

/* —————————————— 内置厂商预设 —————————————— */

export type GatewayProtocol = GatewayPoolEntry["protocol"];
export type PresetSection = "sponsor" | "vendor" | "relay" | "local";

export type GatewayPreset = {
  key: string;
  name: string;
  baseUrl: string;
  protocol: GatewayProtocol;
  model: string;
  modelList: string[];
  section: PresetSection;
  /** 本机服务不校验 Key，预填一个占位值。 */
  defaultKey?: string;
  apiKeyUrl?: string;
};

/** 应用里通用预设之外、网关额外提供的几个：Anthropic 原生协议、xAI 和本机服务。 */
const GATEWAY_EXTRA_PRESETS: GatewayPreset[] = [
  {
    key: "anthropic",
    name: "Anthropic",
    baseUrl: "https://api.anthropic.com",
    protocol: "anthropicMessages",
    model: "claude-sonnet-5-5",
    modelList: ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
    section: "vendor",
    apiKeyUrl: "https://console.anthropic.com/settings/keys",
  },
  {
    key: "xai",
    name: "xAI",
    baseUrl: "https://api.x.ai/v1",
    protocol: "chatCompletions",
    model: "",
    modelList: [],
    section: "vendor",
    apiKeyUrl: "https://console.x.ai",
  },
  // OpenCode 的模型网关：Go 是按月订阅的开源编码模型，Zen 按量计费。每个模型只在一种协议上提供，
  // 网关按 models.dev 目录逐个模型选协议，这里的协议只是占位。
  {
    key: "opencode-go",
    name: "OpenCode Go",
    baseUrl: "https://opencode.ai/zen/go/v1",
    protocol: "chatCompletions",
    model: "",
    modelList: [],
    section: "relay",
    apiKeyUrl: "https://opencode.ai/auth",
  },
  {
    key: "opencode-zen",
    name: "OpenCode Zen",
    baseUrl: "https://opencode.ai/zen/v1",
    protocol: "chatCompletions",
    model: "",
    modelList: [],
    section: "relay",
    apiKeyUrl: "https://opencode.ai/auth",
  },
  {
    key: "ollama",
    name: "Ollama",
    baseUrl: "http://127.0.0.1:11434/v1",
    protocol: "chatCompletions",
    model: "",
    modelList: [],
    section: "local",
    defaultKey: "ollama",
  },
  {
    key: "lm-studio",
    name: "LM Studio",
    baseUrl: "http://127.0.0.1:1234/v1",
    protocol: "chatCompletions",
    model: "",
    modelList: [],
    section: "local",
    defaultKey: "lm-studio",
  },
];

/** 「添加供应商」里的全部预设：厂商（官方 / 国内官方）、中转、本机。 */
/**
 * 赞助商：「添加供应商」目录里单独一组，放在最前面。到期日与赞助商广告清单一致，
 * 过了到期日（当天仍显示）就不再出现在目录里。续约时改这里的日期。
 */
export const SPONSORS: Array<{ id: string; expiresAt: string }> = [
  { id: "jojocode-max", expiresAt: "2029-06-15" },
  { id: "runapi", expiresAt: "2026-11-07" },
  { id: "apikeyfun", expiresAt: "2026-11-22" },
];

/** 本地日期（YYYY-MM-DD），与到期日按天比较。 */
function localDay(now: Date): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

export function activeSponsorIds(now: Date = new Date()): string[] {
  const today = localDay(now);
  return SPONSORS.filter((sponsor) => sponsor.expiresAt >= today).map((sponsor) => sponsor.id);
}

export function gatewayPresets(now: Date = new Date()): GatewayPreset[] {
  const sponsorIds = activeSponsorIds(now);
  const expired = new Set(SPONSORS.map((sponsor) => sponsor.id).filter((id) => !sponsorIds.includes(id)));
  const shared = PRESETS.filter((preset) => !expired.has(preset.id)).map((preset): GatewayPreset => ({
    key: preset.id,
    name: preset.name,
    baseUrl: preset.baseUrl,
    protocol: preset.protocol,
    model: preset.model,
    modelList: preset.modelList ?? (preset.model ? [preset.model] : []),
    section: sponsorIds.includes(preset.id)
      ? "sponsor"
      : preset.category === "official" || preset.category === "cn_official" ? "vendor" : "relay",
    apiKeyUrl: preset.apiKeyUrl,
  }));
  const vendors = [...GATEWAY_EXTRA_PRESETS.filter((preset) => preset.section === "vendor"), ...shared.filter((preset) => preset.section === "vendor")];
  const relays = [...GATEWAY_EXTRA_PRESETS.filter((preset) => preset.section === "relay"), ...shared.filter((preset) => preset.section === "relay")];
  // 赞助商按名单顺序排。
  const sponsors = sponsorIds.flatMap((id) => shared.filter((preset) => preset.key === id));
  return [...sponsors, ...vendors, ...relays, ...GATEWAY_EXTRA_PRESETS.filter((preset) => preset.section === "local")];
}

function normalizeUrl(url: string): string {
  return url.trim().replace(/\/+$/, "").toLowerCase();
}

/** 预设卡片上的副标题：地址的主机名（本机服务显示端口）。 */
export function presetHost(baseUrl: string): string {
  try {
    const url = new URL(baseUrl);
    return url.hostname === "127.0.0.1" || url.hostname === "localhost" ? `localhost:${url.port}` : url.hostname;
  } catch {
    return baseUrl;
  }
}

/** 已从这个预设添加过（看来源），或池里已有同一地址：卡片显示为已添加（仍可再加一个，比如第二个账号）。 */
export function presetAdded(payload: GatewayPayload | null, preset: GatewayPreset): boolean {
  const target = normalizeUrl(preset.baseUrl);
  return Boolean(payload?.pool.some((entry) => entry.source === `preset:${preset.key}` || normalizeUrl(entry.baseUrl) === target));
}

/** 新建 / 编辑供应商的表单。`keys` 每行一个，多个 Key 自动轮换；编辑时留空表示不改。 */
export type ProviderDraft = {
  name: string;
  baseUrl: string;
  keys: string;
  protocol: GatewayProtocol;
  model: string;
  modelList: string;
  /** 每百万 token 价格，空串表示不填。 */
  priceCurrency: string;
  priceInput: string;
  priceCached: string;
  priceOutput: string;
  /** 单独走的 HTTP 代理，空串表示不单独走代理（沿用系统代理）。 */
  proxy: string;
  /** 按模型的价格：新添加时按公开价格表自动查；不带表示不改。 */
  modelPrices?: Record<string, ProviderPrice> | null;
  /** 转发前移除托管生图工具（image_generation）：不支持生图的 Responses 中转用（#228）。 */
  stripImageGenerationTool?: boolean;
};

const NO_PRICE = { priceCurrency: "CNY", priceInput: "", priceCached: "", priceOutput: "", proxy: "", stripImageGenerationTool: false };

export function providerDraftFromPreset(preset: GatewayPreset | null): ProviderDraft {
  if (!preset) return { name: "", baseUrl: "", keys: "", protocol: "chatCompletions", model: "", modelList: "", ...NO_PRICE };
  return {
    name: preset.name,
    baseUrl: preset.baseUrl,
    keys: preset.defaultKey ?? "",
    protocol: preset.protocol,
    model: preset.model,
    modelList: preset.modelList.join("\n"),
    ...NO_PRICE,
  };
}

/** 编辑已有供应商：Key 不回显（留空表示不改）。 */
export function providerDraftFromEntry(entry: GatewayPoolEntry): ProviderDraft {
  return {
    name: entry.name,
    baseUrl: entry.baseUrl,
    keys: "",
    protocol: entry.protocol,
    model: entry.models[0] ?? "",
    modelList: entry.models.join("\n"),
    priceCurrency: entry.price?.currency || "CNY",
    priceInput: entry.price ? String(entry.price.input) : "",
    priceCached: entry.price ? String(entry.price.cachedInput) : "",
    priceOutput: entry.price ? String(entry.price.output) : "",
    proxy: entry.proxy ?? "",
    stripImageGenerationTool: entry.stripImageGenerationTool === true,
  };
}

/** 提交给后端的 `ProviderInput`。 */
export function providerInput(draft: ProviderDraft) {
  return {
    name: draft.name.trim(),
    baseUrl: draft.baseUrl.trim(),
    protocol: draft.protocol,
    keys: draft.keys.split(/\n/).map((line) => line.trim()).filter(Boolean),
    model: draft.model.trim(),
    modelList: splitModels(draft.modelList).join("\n"),
    price: priceFromDraft(draft),
    modelPrices: draft.modelPrices ?? null,
    proxy: draft.proxy.trim(),
    stripImageGenerationTool: draft.stripImageGenerationTool === true,
  };
}

function priceNumber(text: string): number {
  const value = Number(text.trim());
  return Number.isFinite(value) && value > 0 ? value : 0;
}

/** 三个价格都没填（或都是 0）时不带价格。 */
export function priceFromDraft(draft: ProviderDraft): ProviderPrice | null {
  const price = { currency: draft.priceCurrency || "CNY", input: priceNumber(draft.priceInput), cachedInput: priceNumber(draft.priceCached), output: priceNumber(draft.priceOutput) };
  return price.input || price.cachedInput || price.output ? price : null;
}

/** 供应商代理设成这个值：不走任何代理，直接连上游（与后端 `relay_proxy::DIRECT_PROXY` 一致，#2760）。 */
export const DIRECT_PROXY = "direct";

export function isDirectProxy(proxy: string | undefined): boolean {
  return (proxy ?? "").trim().toLowerCase() === DIRECT_PROXY;
}

/** 代理地址格式错误时返回中文错误原文；留空或直连都合法。 */
export function proxyAddressError(proxy: string): string | null {
  const value = proxy.trim();
  if (!value || isDirectProxy(value)) return null;
  return /^(https?|socks5h?):\/\/\S+$/i.test(value) ? null : "代理地址要写成 http://主机:端口（也支持 https:// 与 socks5://）";
}

/** 表单校验：返回中文错误原文，没问题返回 null。编辑时可以不填 Key。 */
export function validateProviderDraft(draft: ProviderDraft, editing = false): string | null {
  if (!draft.name.trim()) return "请填写名称";
  if (!/^https?:\/\/\S+$/i.test(draft.baseUrl.trim())) return "地址要以 http:// 或 https:// 开头";
  if (!editing && !draft.keys.trim()) return "请填写 API Key";
  if (!draft.model.trim() && splitModels(draft.modelList).length === 0) return "至少填一个模型";
  return proxyAddressError(draft.proxy);
}

/* —————————————— 用量 —————————————— */

export type UsageCounts = {
  calls: number;
  errors: number;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  reasoningTokens: number;
  /** 记了耗时的请求的总耗时；旧版后端和旧数据没有。 */
  durationMs?: number;
  timedCalls?: number;
  /** 其中成功的生图请求；旧版后端没有，没有时也不写（视为 0）。 */
  images?: number;
};
export type UsageCost = { currency: string; amount: number };
export type UsageDay = UsageCounts & { date: string };
/** `costs` 只含填了价格的部分，旧版后端没有。 */
export type UsageGroup = UsageCounts & { key: string; label: string; costs?: UsageCost[] };
export type UsageSummary = {
  totals: UsageCounts;
  days: UsageDay[];
  byAgent: UsageGroup[];
  byProvider: UsageGroup[];
  byModel: UsageGroup[];
  costs: UsageCost[];
};
export type ProviderBalance = { providerId: string; provider: string; baseUrl: string; amount?: number | null; currency: string; error?: string | null; note?: string | null };
export type UsageRange = "today" | "7d" | "30d" | "all";

/** 统计范围从哪一刻开始（本地时间当天零点往前推）；「全部」没有起点。 */
export function usageRangeStart(range: UsageRange, now: Date = new Date()): number | undefined {
  const days = range === "today" ? 1 : range === "7d" ? 7 : range === "30d" ? 30 : 0;
  if (!days) return undefined;
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  start.setDate(start.getDate() - (days - 1));
  return start.getTime();
}

/** 一条请求的费用：不到一分钱时多留几位，免得都显示成 0.00。 */
export function formatRequestCost(cost: UsageCost): string {
  if (cost.amount <= 0) return "—";
  if (cost.amount >= 0.01) return formatMoney(cost.amount, cost.currency);
  return `${currencySymbol(cost.currency)}${cost.amount.toPrecision(2)}`;
}

/** 23M / 2.2M / 158K / 940。 */
export function formatTokens(value: number): string {
  if (value >= 1_000_000_000) return `${trimNumber(value / 1_000_000_000)}B`;
  if (value >= 1_000_000) return `${trimNumber(value / 1_000_000)}M`;
  if (value >= 1_000) return `${trimNumber(value / 1_000)}K`;
  return String(value);
}

function trimNumber(value: number): string {
  return value >= 100 ? value.toFixed(0) : value >= 10 ? value.toFixed(0) : value.toFixed(1).replace(/\.0$/, "");
}

export function currencySymbol(currency: string): string {
  switch (currency.toUpperCase()) {
    case "CNY":
      return "¥";
    case "USD":
      return "$";
    case "EUR":
      return "€";
    default:
      return `${currency.toUpperCase()} `;
  }
}

export function formatMoney(amount: number, currency: string): string {
  return `${currencySymbol(currency)}${amount.toFixed(2)}`;
}

/** 缓存命中率（命中的输入 / 全部输入），没有输入时为 null。 */
export function cacheHitRate(counts: UsageCounts): number | null {
  return counts.inputTokens > 0 ? counts.cachedTokens / counts.inputTokens : null;
}

/** 平均耗时（毫秒）；没有计时数据时为 null。 */
export function averageDuration(counts: UsageCounts): number | null {
  return counts.timedCalls ? (counts.durationMs ?? 0) / counts.timedCalls : null;
}

/** 多币种费用连起来：`$3.22 + ¥1.50`；没有费用为空串。 */
export function costText(costs: UsageCost[] | undefined): string {
  return (costs ?? []).filter((cost) => cost.amount > 0).map((cost) => formatMoney(cost.amount, cost.currency)).join(" + ");
}

/** 柱状图的纵轴上限：取 1 / 2 / 5 × 10ⁿ 里不小于最大值的那个，刻度好读。 */
export function niceCeiling(value: number): number {
  if (value <= 0) return 1;
  const power = 10 ** Math.floor(Math.log10(value));
  for (const step of [1, 2, 5, 10]) {
    if (step * power >= value) return step * power;
  }
  return 10 * power;
}

/** 这段时间里的用量概况：有请求的天数、单日峰值（按 token）、有请求的日子的日均 token。 */
export function usageDayStats(days: UsageDay[]): { activeDays: number; peak: UsageDay | null; dailyAverage: number } {
  const active = days.filter((day) => day.calls > 0);
  const total = (day: UsageDay) => day.inputTokens + day.outputTokens;
  const peak = active.reduce<UsageDay | null>((best, day) => (!best || total(day) > total(best) ? day : best), null);
  const sum = active.reduce((acc, day) => acc + total(day), 0);
  return { activeDays: active.length, peak, dailyAverage: active.length ? sum / active.length : 0 };
}

/** 横轴标签：最多 `count` 个，均匀分布，总是包含第一天和最后一天。 */
export function axisLabelIndexes(length: number, count = 7): number[] {
  if (length <= 0) return [];
  if (length <= count) return Array.from({ length }, (_, index) => index);
  const step = (length - 1) / (count - 1);
  return Array.from(new Set(Array.from({ length: count }, (_, index) => Math.round(index * step))));
}

/** `2026-09-01` → `9月1日` / `Sep 1`（界面按语言选）。 */
export function shortDate(date: string, english = false): string {
  const [, month, day] = date.split("-").map(Number);
  if (!month || !day) return date;
  if (english) return `${["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][month - 1]} ${day}`;
  return `${month}月${day}日`;
}

/* —————————————— 会话 —————————————— */

export type SessionAgent = "codex" | "claude" | "grok";
export type AgentSessionView = { agent: SessionAgent; id: string; title: string; cwd: string; updatedAtMs: number };

/** 合并各 Agent 的会话：按更新时间倒序，按 Agent 和关键词（标题、目录、id）过滤。 */
export function filterSessions(sessions: AgentSessionView[], agent: SessionAgent | "all", query: string): AgentSessionView[] {
  const needle = query.trim().toLowerCase();
  return sessions
    .filter((session) => agent === "all" || session.agent === agent)
    .filter((session) => !needle || [session.title, session.cwd, session.id].some((value) => value.toLowerCase().includes(needle)))
    .sort((left, right) => right.updatedAtMs - left.updatedAtMs);
}

/** `/Users/me/code/app` → `~/code/app`；太长时只留最后两级。 */
export function shortPath(path: string, home = ""): string {
  let text = path.trim();
  if (home && text.startsWith(home)) text = `~${text.slice(home.length)}`;
  else text = text.replace(/^\/Users\/[^/]+/, "~").replace(/^\/home\/[^/]+/, "~");
  const parts = text.split("/").filter(Boolean);
  return parts.length > 4 ? `…/${parts.slice(-2).join("/")}` : text;
}

/** 相对时间：刚刚 / N 分钟前 / N 小时前 / N 天前 / 日期。返回中文模板与参数，界面再翻译。 */
export function relativeTime(ms: number, now: number): { text: string; args: Array<string | number> } {
  const diff = Math.max(0, now - ms);
  const minute = 60_000;
  if (diff < minute) return { text: "刚刚", args: [] };
  if (diff < 60 * minute) return { text: "{0} 分钟前", args: [Math.floor(diff / minute)] };
  if (diff < 24 * 60 * minute) return { text: "{0} 小时前", args: [Math.floor(diff / (60 * minute))] };
  if (diff < 30 * 24 * 60 * minute) return { text: "{0} 天前", args: [Math.floor(diff / (24 * 60 * minute))] };
  const date = new Date(ms);
  return { text: "{0}", args: [`${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`] };
}

/**
 * 名字里的邮箱。前面紧挨着 `*` 等字符的不算（那是已经打过码的邮箱尾巴），保证打码可以重复调用。
 * 不用后行断言 `(?<!…)`：旧版系统 WebKit（16.4 以前的版本）不支持，模块一加载就抛 SyntaxError，
 * 整个管理器白屏 / 黑屏（#2753）。改成把前一个字符（或行首）捕获进第 1 组，替换时原样放回。
 */
const EMAIL_PATTERN = /(^|[^A-Za-z0-9._%+*-])([A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,})/g;

function maskEmailsIn(text: string): string {
  return text.replace(EMAIL_PATTERN, (_match, lead: string, email: string) => lead + maskEmail(email));
}

/**
 * 订阅账号的显示名打码：「ChatGPT Pro · me@example.com」「Copilot · octocat」这类名字里的账号部分只露头尾，
 * 名字里出现的邮箱也一样。只在显示时处理，存储的名字不变；普通供应商名不受影响。
 */
export function maskAccountName(name: string): string {
  const masked = maskEmailsIn(name);
  const match = /^((?:ChatGPT|Copilot|SuperGrok|GitHub Copilot)[^·]*·\s*)(.+)$/.exec(masked);
  // 已经打过码的（带 *）原样返回：名字可能先打码再传给头像等组件，再打一次会变形。
  if (!match || match[2].includes("@") || match[2].includes("*")) return masked;
  return match[1] + maskToken(match[2].trim());
}

/**
 * 一段提示文字里的账号打码：邮箱一律打码，「」里的供应商名按 `maskAccountName` 处理。
 * 后端的提示（如「已添加「ChatGPT Pro · me@example.com」」）原样弹 toast 前用它过一遍。
 */
export function maskAccountText(text: string): string {
  // 按「」切开分别处理，避免已经打过码的邮箱被再打一次。
  return text
    .split(/(「[^」]+」)/)
    .map((part) => (part.startsWith("「") && part.endsWith("」")
      ? `「${maskAccountName(part.slice(1, -1))}」`
      : maskEmailsIn(part)))
    .join("");
}

function maskEmail(email: string): string {
  const at = email.indexOf("@");
  return `${maskToken(email.slice(0, at))}${email.slice(at)}`;
}

/** 4 个字符以内只留第一个；更长的留头两个、尾两个，中间固定 4 个星号（不暴露长度）。 */
function maskToken(value: string): string {
  const chars = [...value];
  if (chars.length <= 4) return `${chars[0] ?? ""}***`;
  return `${chars.slice(0, 2).join("")}****${chars.slice(-2).join("")}`;
}

/* —————————————— 从直连模式导入 —————————————— */

export type ImportSource = "codex" | "claude" | "grok" | "opencode" | "aggregate";
export type ImportCandidate = { source: ImportSource; id: string; name: string; imported: boolean };

/** 能导入网关的供应商（不含官方登录账号）：已导入过的标 `imported`。 */
export function importCandidates(payload: GatewayPayload | null): ImportCandidate[] {
  const pooled = (source: string, id: string) => Boolean(payload?.pool.some((entry) => entry.source === `${source}:${id}`));
  const plain = (source: ImportSource, items: Array<{ id: string; name: string }> | undefined) =>
    (items ?? []).map((item) => ({ source, id: item.id, name: item.name, imported: pooled(source, item.id) }));
  return [
    ...plain("codex", payload?.codexProviders),
    ...(payload?.codexAggregates ?? []).map((item) => ({ source: "aggregate" as const, id: item.id, name: item.name, imported: item.imported })),
    ...plain("claude", payload?.claudeProviders),
    ...plain("grok", payload?.grokProfiles),
    ...plain("opencode", payload?.opencodeKeys),
  ];
}

/** 还没导入网关的供应商数（引导卡片用）。 */
export function pendingImportCount(payload: GatewayPayload | null): number {
  return importCandidates(payload).filter((item) => !item.imported).length;
}

/* —————————————— 模型收藏 —————————————— */

/** `<供应商 id>/<模型>` 是哪个模型的固定那份：返回裸模型名；不是固定写法返回 null。 */
function pinnedBare(payload: GatewayPayload | null, model: string): string | null {
  const slash = model.indexOf("/");
  if (slash <= 0) return null;
  const id = model.slice(0, slash).toLowerCase();
  const rest = model.slice(slash + 1);
  const entry = agentPoolProviders(payload).find((item) => item.id.toLowerCase() === id);
  return entry && entry.models.some((known) => known.toLowerCase() === rest.toLowerCase()) ? rest : null;
}

/** 这个模型在各家名下固定的那几份。 */
function pinnedCopies(payload: GatewayPayload | null, bare: string): string[] {
  const serving = servingProviders(payload, bare);
  return serving.length > 1 ? serving.map((entry) => `${entry.id}/${bare}`) : [];
}

/**
 * 收藏按模型名存。旧版本收藏的是裸名，那时多家都有的模型只有一份；现在每家名下各一份 `<供应商 id>/<模型>`。
 * 兼容：某个模型还没有任何「固定那份」被单独收藏过时，裸名收藏同时点亮它在各家名下的那几份（和「自动」那份）。
 */
export function isFavoriteModel(favorites: string[], model: string, payload: GatewayPayload | null): boolean {
  if (favorites.includes(model)) return true;
  const bare = pinnedBare(payload, model);
  if (!bare || !favorites.some((item) => item.toLowerCase() === bare.toLowerCase())) return false;
  return !pinnedCopies(payload, bare).some((copy) => favorites.includes(copy));
}

/** 切换收藏。取消一份时只取消这一份：裸名兼容点亮的那几份先展开成各自的收藏再去掉这一份。 */
export function toggleFavoriteModel(favorites: string[], model: string, payload: GatewayPayload | null): string[] {
  if (!isFavoriteModel(favorites, model, payload)) return [...favorites, model];
  const bare = pinnedBare(payload, model);
  if (bare && !favorites.includes(model)) {
    // 这一份是被旧的裸名收藏点亮的：其余几家的那份改成单独收藏，「自动」那份（裸名）保留。
    const others = pinnedCopies(payload, bare).filter((copy) => copy !== model && !favorites.includes(copy));
    return [...favorites, ...others];
  }
  const next = favorites.filter((item) => item !== model);
  // 取消的是裸名（「自动」那份或旧收藏）：之前靠它点亮的各家那几份，改成单独收藏留着。
  const copies = pinnedCopies(payload, model);
  if (copies.length && !copies.some((copy) => favorites.includes(copy))) return [...next, ...copies];
  return next;
}

/* —————————————— 路由：自动组、隐藏成员 —————————————— */

/** 这个裸模型名现在走不走自动组：自动建组开着、这个自动组没被隐藏（与后端 `active_auto_group` 一致）。 */
export function activeAutoGroup(payload: GatewayPayload | null, model: string): AutoGroupEntry | null {
  if (!payload || payload.gateway.autoModels === false) return null;
  const found = payload.autoGroups?.find((group) => group.name.toLowerCase() === model.toLowerCase());
  return found && !found.hidden ? found : null;
}

/** 隐藏了成员的组（自建的、生效中的自动组）里，不再单独出现在模型列表里的模型（小写）。 */
export function unlistedModels(payload: GatewayPayload | null): Set<string> {
  const hidden = new Set<string>();
  const add = (group: GatewayGroupEntry) => {
    for (const member of group.members) {
      const model = member.model.trim();
      if (model && !model.includes("*") && !model.startsWith(GROUP_MODEL_PREFIX)) hidden.add(model.toLowerCase());
    }
  };
  for (const group of payload?.groups ?? []) if (group.hideMembers) add(group);
  if (payload?.gateway.autoModels !== false) {
    for (const group of payload?.autoGroups ?? []) if (group.hideMembers && !group.hidden) add(group);
  }
  return hidden;
}

/** 组成员指向的具体上游：`<供应商 id>/<模型>` 或只有一家的裸模型名才有；多家的裸名、组、通配符返回 null。 */
export function memberTarget(payload: GatewayPayload | null, model: string): { entry: GatewayPoolEntry; model: string } | null {
  const qualified = qualifiedProvider(payload, model.trim());
  if (qualified) return qualified;
  const serving = servingProviders(payload, model);
  return serving.length === 1 ? { entry: serving[0], model: model.trim() } : null;
}
