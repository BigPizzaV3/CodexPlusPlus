// DeepSeek 官方 Codex 接入向导的纯数据逻辑，便于独立单测。
// 与 App.tsx 的 RelayProfile 类型保持结构兼容，不反向依赖组件代码。

export const DEEPSEEK_PROFILE_ID = "deepseek";
export const DEEPSEEK_OFFICIAL_BASE_URL = "https://api.deepseek.com";
export const DEEPSEEK_MODEL_LIST = "deepseek-v4-flash[1M]\ndeepseek-v4-pro[1M]";

export const DEEPSEEK_MODELS = ["deepseek-v4-flash", "deepseek-v4-pro"] as const;
export type DeepSeekModel = (typeof DEEPSEEK_MODELS)[number];

type RelayProtocol = "responses" | "chatCompletions";
type RelayMode = "official" | "mixedApi" | "pureApi" | "aggregate";

export type DeepSeekProfile = {
  id: string;
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
  contextSelection: { mcpServers: string[]; skills: string[]; plugins: string[] };
  contextSelectionInitialized: boolean;
  contextWindow: string;
  autoCompactLimit: string;
  modelList: string;
  modelWindows: string;
  modelVlm: string;
  vlmApiKey: string;
  vlmModel: string;
  vlmBaseUrl: string;
  userAgent: string;
  sub2apiEnabled: boolean;
  sub2apiMultiplier: string;
  deepseekOfficialMetadata: boolean;
};

export type DeepSeekProfilePatch = Partial<DeepSeekProfile>;

/** 新建 DeepSeek profile 时的完整默认骨架（configContents 等由应用流程生成）。 */
export function createDeepSeekProfileBase(): DeepSeekProfile {
  return {
    id: DEEPSEEK_PROFILE_ID,
    name: "DeepSeek",
    model: "deepseek-v4-flash",
    baseUrl: DEEPSEEK_OFFICIAL_BASE_URL,
    upstreamBaseUrl: DEEPSEEK_OFFICIAL_BASE_URL,
    apiKey: "",
    protocol: "responses",
    relayMode: "pureApi",
    officialMixApiKey: false,
    testModel: "deepseek-v4-flash",
    configContents: "",
    authContents: "",
    useCommonConfig: true,
    contextSelection: { mcpServers: [], skills: [], plugins: [] },
    contextSelectionInitialized: true,
    contextWindow: "",
    autoCompactLimit: "",
    modelList: DEEPSEEK_MODEL_LIST,
    modelWindows: "",
    modelVlm: "",
    vlmApiKey: "",
    vlmModel: "",
    vlmBaseUrl: "",
    userAgent: "",
    sub2apiEnabled: false,
    sub2apiMultiplier: "",
    deepseekOfficialMetadata: true,
  };
}

/**
 * 生成写入 profile 的字段补丁。
 * 显式清空 contextWindow / autoCompactLimit：窗口交给 catalog（官方元数据 1M），
 * 避免顶层 model_context_window / model_auto_compact_token_limit 覆盖 catalog。
 */
export function buildDeepSeekProfilePatch(
  model: DeepSeekModel,
  apiKey: string,
): DeepSeekProfilePatch {
  return {
    name: "DeepSeek",
    model,
    baseUrl: DEEPSEEK_OFFICIAL_BASE_URL,
    upstreamBaseUrl: DEEPSEEK_OFFICIAL_BASE_URL,
    apiKey: apiKey.trim(),
    protocol: "responses",
    relayMode: "pureApi",
    officialMixApiKey: false,
    testModel: model,
    modelList: DEEPSEEK_MODEL_LIST,
    contextWindow: "",
    autoCompactLimit: "",
    deepseekOfficialMetadata: true,
  };
}

/**
 * 把 DeepSeek profile 插入（或覆盖）settings.relayProfiles，
 * 并将 activeRelayId 切到该 profile，返回新 settings。
 */
export function upsertDeepSeekProfile<
  T extends { relayProfiles: Array<{ id: string }>; activeRelayId: string },
>(settings: T, profile: DeepSeekProfile): T {
  const exists = settings.relayProfiles.some((item) => item.id === DEEPSEEK_PROFILE_ID);
  const relayProfiles = exists
    ? settings.relayProfiles.map((item) => (item.id === DEEPSEEK_PROFILE_ID ? profile : item))
    : [...settings.relayProfiles, profile];
  return { ...settings, relayProfiles, activeRelayId: DEEPSEEK_PROFILE_ID };
}
