/** 网关模式的前端数据模型：与 `codex_plus_core::gateway` 的序列化结构一一对应，外加表单转换。
 *
 * 网关模式与直连模式完全独立：供应商池、路由组只属于网关；所有接入的 Agent 共用整个池。
 * 模型名三种写法：裸模型名（池里提供它的供应商依次尝试）、`<供应商 id>/<模型>`、`group/<组 id>`。
 */
import { PRESETS } from "./presets.ts";

export type GatewayAgentId = "codex" | "claude" | "grok";
export type UnknownModelPolicy = "mapToDefault" | "passthrough";
export type KeyRotationMode = "sticky" | "roundRobin";
export type GroupMode = "inOrder" | "rotate" | "weighted";

export type ClaudeRoleModels = { opus: string; sonnet: string; haiku: string; subagent: string };

export type AgentBinding = {
  enabled: boolean;
  model: string;
  extraModels: string[];
  claudeRoles: ClaudeRoleModels;
  unknownModel: UnknownModelPolicy;
  contextWindow?: number | null;
  claudeEnableEffort: boolean;
  /** 这个 Agent 不用的池内供应商（「挑选」里取消勾选的）。 */
  poolExclude?: string[];
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
  /** 网关供应商单独走的 HTTP 代理，空串表示不单独走代理。 */
  proxy?: string;
  /** 登录得到的账号：chatgpt / copilot / grok；普通供应商为空。 */
  account?: string;
  /** 这个 ChatGPT 账号正借给 Codex 登录。 */
  lentToCodex?: boolean;
};

export type ProviderPrice = { currency: string; input: number; cachedInput: number; output: number };

export type GroupMember = { model: string; weight: number };
export type GatewayGroupEntry = { id: string; name: string; mode: GroupMode; members: GroupMember[] };

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
  /** 可复制进网关的直连模式供应商。 */
  codexProviders: GatewayPoolEntry[];
  claudeProviders: Array<{ id: string; name: string }>;
  grokProfiles: Array<{ id: string; name: string }>;
  /** OpenCode 里保存的 OpenCode Go / Zen Key。 */
  opencodeKeys?: Array<{ id: string; name: string }>;
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
  contextWindow: string;
  claudeEnableEffort: boolean;
  poolExclude: string[];
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
    contextWindow: source.contextWindow ? String(source.contextWindow) : "",
    claudeEnableEffort: source.claudeEnableEffort,
    poolExclude: [...(source.poolExclude ?? [])],
  };
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
    contextWindow: window && Number.isFinite(window) ? window : null,
    claudeEnableEffort: draft.claudeEnableEffort,
    poolExclude: [...draft.poolExclude],
  };
}

export const GROUP_MODEL_PREFIX = "group/";

/** 路由组当作模型时的名字：`group/<组 id>`。 */
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

/** Agent 能用的全部模型：池内供应商的模型（去重，按池顺序）+ 有成员的路由组。与后端 `pool_models` 一致。 */
export function agentModels(payload: GatewayPayload | null, exclude: string[] = []): string[] {
  const models: string[] = [];
  for (const entry of agentPoolProviders(payload, exclude)) {
    for (const model of entry.models) {
      if (!models.some((known) => known.toLowerCase() === model.toLowerCase())) models.push(model);
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
  return `${Math.floor(minutes / 60)}h${minutes % 60 ? `${minutes % 60}m` : ""}`;
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
 * 模型选择器的分组：每个能用的池内供应商一组列出它的模型，路由组（`group/<id>`）排在后面。
 * 查询词不是任何已知模型时，另给一组「自定义模型」：`<供应商 id>/<输入>`，直接发给那个供应商。
 * `skipGroup` 用来在编辑路由组时排除它自己。`avatar` 由调用方算（避免这里依赖界面工具函数）。
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
  for (const entry of providers) {
    const providerHit = matches(query, entry.name, entry.id);
    const items = entry.models
      .filter((model) => providerHit || matches(query, model))
      .map((model) => ({ model, label: model, sub: maskAccountName(entry.name), avatar: avatar(entry.name) }));
    if (items.length) sections.push({ key: `provider:${entry.id}`, title: maskAccountName(entry.name), avatar: avatar(entry.name), items });
  }
  const groups = payload.groups
    .filter((group) => group.id !== skipGroup && group.members.length && matches(query, group.name, groupModelId(group.id)))
    .map((group) => ({ model: groupModelId(group.id), label: group.name, sub: groupModelId(group.id), avatar: avatar(group.name) }));
  if (groups.length) sections.push({ key: "group", title: "路由组", avatar: "", items: groups });
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

export type ModelView = { title: string; sub: string; group: boolean };

/** 一个模型名在按钮上的显示：模型名 + 提供它的供应商；路由组显示组名。空串用池里第一个模型。 */
export function modelDisplay(payload: GatewayPayload | null, model: string, exclude: string[] = []): ModelView {
  const title = model.trim() || agentModels(payload, exclude)[0] || "";
  if (title.startsWith(GROUP_MODEL_PREFIX)) {
    const group = payload?.groups.find((item) => groupModelId(item.id) === title);
    return { title: group?.name ?? title, sub: title, group: true };
  }
  const qualified = qualifiedProvider(payload, title, exclude);
  if (qualified) return { title: qualified.model, sub: maskAccountName(qualified.entry.name), group: false };
  const entry = agentPoolProviders(payload, exclude).find((item) => item.models.some((known) => known.toLowerCase() === title.toLowerCase()));
  return { title, sub: entry?.name ?? "", group: false };
}

export function groupModeLabel(mode: string): string {
  switch (mode) {
    case "rotate":
      return "轮换";
    case "weighted":
      return "按权重";
    default:
      return "依次";
  }
}

export function groupModeDescription(mode: string): string {
  switch (mode) {
    case "rotate":
      return "轮换：每个请求从下一个成员开始；成员答不了时换下一个，每个成员再按自己的 Key 轮换。";
    case "weighted":
      return "按权重：按成员权重决定先试谁；成员答不了时换下一个，每个成员再按自己的 Key 轮换。";
    default:
      return "依次：按成员顺序，前一个答不了才换下一个，每个成员再按自己的 Key 轮换。";
  }
}

/** 路由组成员在列表里的显示：「供应商 · 模型」或「组名」。 */
export function memberLabel(payload: GatewayPayload | null, model: string): string {
  const view = modelDisplay(payload, model);
  if (view.group) return view.title;
  return view.sub ? `${view.sub} · ${view.title}` : view.title;
}

export type MemberStatus = { state: "busy" | "answered" | "cooling" | "idle"; text: string; args: Array<string | number> };

/** 供应商在路由图里的状态：正在回答 / 熔断或 Key 全部冷却 / 最近回答了谁 / Key 数。文案是中文原文，界面再翻译。 */
export function memberStatus(payload: GatewayPayload | null, relayId: string): MemberStatus {
  const activity = payload?.activity?.agents ?? [];
  if (activity.some((agent) => agent.inFlight.includes(relayId))) {
    return { state: "busy", text: "正在回答…", args: [] };
  }
  const breaker = payload?.health.providers.find((item) => item.relayId === relayId && item.breakerOpen);
  if (breaker) return { state: "cooling", text: "连续出错 · {0} 后恢复", args: [formatRemaining(breaker.remainingMs)] };
  const entry = payload?.pool.find((item) => item.id === relayId);
  const keyCount = entry?.keyCount ?? 1;
  const unhealthy = (payload?.health.keys ?? []).filter((key) => key.relayId === relayId && key.state !== "ok");
  if (unhealthy.length >= keyCount && unhealthy.length > 0) {
    const cooling = unhealthy.filter((key) => key.state === "cooling");
    if (!cooling.length) return { state: "cooling", text: "Key 都已停用", args: [] };
    const soonest = Math.min(...cooling.map((key) => key.remainingMs ?? 0));
    const label = cooling.some((key) => key.lastFailure === "quota_exhausted") ? "额度耗尽 · {0} 后恢复" : "被限流 · {0} 后恢复";
    return { state: "cooling", text: label, args: [formatRemaining(soonest)] };
  }
  const answered = activity
    .filter((agent) => agent.last?.result === "answered" && agent.last.steps.at(-1)?.relayId === relayId)
    .sort((left, right) => (right.last?.finishedAtMs ?? 0) - (left.last?.finishedAtMs ?? 0))[0];
  if (answered) return { state: "answered", text: "回答了 {0}", args: [agentName(answered.agent)] };
  if (unhealthy.length) return { state: "idle", text: "{0} 个 Key · {1} 个冷却中", args: [keyCount, unhealthy.length] };
  return keyCount > 1 ? { state: "idle", text: "{0} 个 Key", args: [keyCount] } : { state: "idle", text: "一个 Key", args: [] };
}

export function formatElapsed(ms: number): string {
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.round(ms / 1000)} s`;
}

export type NarrativeLine = { tone: "muted" | "step" | "ok" | "fail"; text: string; args: Array<string | number> };

/** 「上一个请求是怎么路由的」：按记录逐条生成。文案是中文模板，界面用 tf 翻译。 */
export function routeNarrative(agent: string, record: RouteRecordView, payload: GatewayPayload | null): NarrativeLine[] {
  const lines: NarrativeLine[] = [];
  const who = agentName(agent);
  const requested = record.requestedModel || record.model;
  const plan = record.planned.map((member) => `${maskAccountName(member.relayName)}/${member.model}`).join(", ");
  if (record.groupId) {
    const group = payload?.groups.find((item) => item.id === record.groupId);
    lines.push({ tone: "muted", text: "{0} 请求 {1}，交给路由组「{2}」：{3}", args: [who, requested, group?.name ?? record.groupId, plan] });
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
    const reason = step.outcome === "transport" ? "连不上" : failureLabel(step.failure) || "出错";
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


/** 所有接入的 Agent 共用整个池：路由页只有一张卡片。 */
export function connectedAgents(payload: GatewayPayload | null): GatewayAgentId[] {
  return (payload?.agents ?? []).filter((view) => view.enabled).map((view) => view.agent);
}

/** 路由卡片右侧的成员：池里打开的供应商；模型取最近一次计划里的，没有就用供应商默认模型。 */
export function routeMembers(payload: GatewayPayload | null, records: RouteRecordView[]): Array<{ relayId: string; name: string; model: string }> {
  return agentPoolProviders(payload).map((entry) => {
    const planned = records.flatMap((record) => record.planned).find((member) => member.relayId === entry.id);
    return { relayId: entry.id, name: maskAccountName(entry.name), model: planned?.model ?? entry.models[0] ?? "" };
  });
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
};

const NO_PRICE = { priceCurrency: "CNY", priceInput: "", priceCached: "", priceOutput: "", proxy: "" };

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
    proxy: draft.proxy.trim(),
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

/** 表单校验：返回中文错误原文，没问题返回 null。编辑时可以不填 Key。 */
export function validateProviderDraft(draft: ProviderDraft, editing = false): string | null {
  if (!draft.name.trim()) return "请填写名称";
  if (!/^https?:\/\/\S+$/i.test(draft.baseUrl.trim())) return "地址要以 http:// 或 https:// 开头";
  if (!editing && !draft.keys.trim()) return "请填写 API Key";
  if (!draft.model.trim() && splitModels(draft.modelList).length === 0) return "至少填一个模型";
  if (draft.proxy.trim() && !/^(https?|socks5h?):\/\/\S+$/i.test(draft.proxy.trim())) return "代理地址要写成 http://主机:端口（也支持 https:// 与 socks5://）";
  return null;
}

/* —————————————— 用量 —————————————— */

export type UsageCounts = { calls: number; errors: number; inputTokens: number; outputTokens: number; cachedTokens: number; reasoningTokens: number };
export type UsageDay = UsageCounts & { date: string };
export type UsageGroup = UsageCounts & { key: string; label: string };
export type UsageSummary = {
  totals: UsageCounts;
  days: UsageDay[];
  byAgent: UsageGroup[];
  byProvider: UsageGroup[];
  byModel: UsageGroup[];
  costs: Array<{ currency: string; amount: number }>;
};
export type ProviderBalance = { providerId: string; provider: string; baseUrl: string; amount?: number | null; currency: string; error?: string | null };
export type UsageRange = "today" | "7d" | "30d" | "all";

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
 * 订阅账号的显示名打码：「ChatGPT Pro · me@example.com」「Copilot · octocat」这类名字里的账号部分只露头尾，
 * 名字里出现的邮箱也一样。只在显示时处理，存储的名字不变；普通供应商名不受影响。
 */
export function maskAccountName(name: string): string {
  const masked = name.replace(/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g, maskEmail);
  const match = /^((?:ChatGPT|Copilot|SuperGrok|GitHub Copilot)[^·]*·\s*)(.+)$/.exec(masked);
  if (!match || match[2].includes("@")) return masked;
  return match[1] + maskToken(match[2].trim());
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
