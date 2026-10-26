/**
 * Claude Code 供应商的纯逻辑：预设、表单字段与 settingsConfig（settings.json 同形 JSON）互转。
 *
 * 预设与字段语义参考 CC Switch（MIT，见 THIRD_PARTY_NOTICES.md），只收厂商官方端点，
 * 链接去掉了推广参数。写盘规则在后端 `tools/claude.rs`。
 */

export type ClaudeSettingsConfig = Record<string, unknown> & { env?: Record<string, unknown> };

export type ClaudeProvider = {
  id: string;
  name: string;
  websiteUrl: string;
  notes: string;
  settingsConfig: ClaudeSettingsConfig;
  createdAt: number;
  ccsSourceId?: string;
};

export type ClaudeLiveSummary = {
  settingsPath: string;
  exists: boolean;
  parseError: string | null;
  baseUrl: string;
  model: string;
  authConfigured: boolean;
  matchesCurrent: boolean;
  hasKeyFields: boolean;
};

export type ClaudeProvidersPayload = {
  storePath: string;
  claudeHome: string;
  cliPath: string | null;
  currentId: string;
  providers: ClaudeProvider[];
  live: ClaudeLiveSummary;
  backupPath: string | null;
  imported: number;
};

export type ClaudeAuthField = "ANTHROPIC_AUTH_TOKEN" | "ANTHROPIC_API_KEY";

export type ClaudePresetCategory = "official" | "cn_official" | "aggregator";

export type ClaudePreset = {
  id: string;
  name: string;
  category: ClaudePresetCategory;
  websiteUrl: string;
  env: Record<string, string>;
};

/** 表单直接编辑的字段；其余键只能在 JSON 里改。 */
export type ClaudeFormFields = {
  baseUrl: string;
  apiKey: string;
  authField: ClaudeAuthField;
  model: string;
  haikuModel: string;
  sonnetModel: string;
  opusModel: string;
  fableModel: string;
};

export const MODEL_ROLE_KEYS = {
  model: "ANTHROPIC_MODEL",
  haikuModel: "ANTHROPIC_DEFAULT_HAIKU_MODEL",
  sonnetModel: "ANTHROPIC_DEFAULT_SONNET_MODEL",
  opusModel: "ANTHROPIC_DEFAULT_OPUS_MODEL",
  fableModel: "ANTHROPIC_DEFAULT_FABLE_MODEL",
} as const;

/** 网关常用的兼容开关（都是供应商独有字段，切走时随这一家移除）。 */
export const COMPAT_SWITCHES: Array<{ key: string; on: string; label: string; detail: string }> = [
  {
    key: "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
    on: "1",
    label: "关闭实验性 beta 请求头",
    detail: "上游不认 anthropic-beta 里的实验特性、请求报 400 时打开。",
  },
  {
    key: "CLAUDE_CODE_DISABLE_ARTIFACT",
    on: "1",
    label: "关闭 Artifact 工具",
    detail: "上游严格校验工具 schema（如 DeepSeek）、每个请求都 400 时打开。",
  },
  {
    key: "CLAUDE_CODE_AUTO_MODE_SERVER",
    on: "0",
    label: "关闭 auto mode 服务端分类器",
    detail: "只有官方端点支持；网关下会话被阻断式提示卡住时打开。",
  },
];

const coding = (model: string) => ({
  ANTHROPIC_MODEL: model,
  ANTHROPIC_DEFAULT_HAIKU_MODEL: model,
  ANTHROPIC_DEFAULT_SONNET_MODEL: model,
  ANTHROPIC_DEFAULT_OPUS_MODEL: model,
});

export const CLAUDE_PRESETS: ClaudePreset[] = [
  { id: "official", name: "Claude 官方", category: "official", websiteUrl: "https://www.anthropic.com/claude-code", env: {} },
  {
    id: "deepseek", name: "DeepSeek", category: "cn_official", websiteUrl: "https://platform.deepseek.com",
    env: {
      ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic", ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "deepseek-v4-pro", ANTHROPIC_DEFAULT_HAIKU_MODEL: "deepseek-flash",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "deepseek-v4-pro", ANTHROPIC_DEFAULT_OPUS_MODEL: "deepseek-v4-pro",
    },
  },
  {
    id: "zhipu", name: "智谱 GLM", category: "cn_official", websiteUrl: "https://open.bigmodel.cn",
    env: { ANTHROPIC_BASE_URL: "https://open.bigmodel.cn/api/anthropic", ANTHROPIC_AUTH_TOKEN: "", ...coding("glm-5.3") },
  },
  {
    id: "zai", name: "Z.ai GLM", category: "cn_official", websiteUrl: "https://z.ai",
    env: { ANTHROPIC_BASE_URL: "https://api.z.ai/api/anthropic", ANTHROPIC_AUTH_TOKEN: "", ...coding("glm-5.3") },
  },
  {
    id: "kimi", name: "Kimi", category: "cn_official", websiteUrl: "https://platform.kimi.com",
    env: { ANTHROPIC_BASE_URL: "https://api.moonshot.cn/anthropic", ANTHROPIC_AUTH_TOKEN: "", ...coding("kimi-k2.7-code") },
  },
  {
    id: "kimi-coding", name: "Kimi For Coding", category: "cn_official", websiteUrl: "https://www.kimi.com/code/",
    env: {
      ANTHROPIC_BASE_URL: "https://api.kimi.com/coding/", ANTHROPIC_AUTH_TOKEN: "", ...coding("kimi-for-coding"),
      CLAUDE_CODE_MAX_CONTEXT_TOKENS: "262144", CLAUDE_CODE_AUTO_COMPACT_WINDOW: "262144",
    },
  },
  {
    id: "bailian-coding", name: "阿里云百炼 Coding Plan", category: "cn_official", websiteUrl: "https://bailian.console.aliyun.com",
    env: { ANTHROPIC_BASE_URL: "https://coding.dashscope.aliyuncs.com/apps/anthropic", ANTHROPIC_AUTH_TOKEN: "" },
  },
  {
    id: "qwen", name: "千问 AI 平台", category: "cn_official", websiteUrl: "https://platform.qianwenai.com",
    env: {
      ANTHROPIC_BASE_URL: "https://dashscope.aliyuncs.com/apps/anthropic", ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "qwen3.8-max", ANTHROPIC_DEFAULT_HAIKU_MODEL: "qwen3.8-flash",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "qwen3.7-plus", ANTHROPIC_DEFAULT_OPUS_MODEL: "qwen3.8-max",
      CLAUDE_CODE_MAX_CONTEXT_TOKENS: "983616",
    },
  },
  {
    id: "volc-coding", name: "火山方舟 Coding Plan", category: "cn_official", websiteUrl: "https://www.volcengine.com/product/ark",
    env: { ANTHROPIC_BASE_URL: "https://ark.cn-beijing.volces.com/api/coding", ANTHROPIC_AUTH_TOKEN: "", ...coding("ark-code-latest") },
  },
  {
    id: "minimax", name: "MiniMax", category: "cn_official", websiteUrl: "https://platform.minimax.cn",
    env: {
      ANTHROPIC_BASE_URL: "https://api.minimax.cn/anthropic", ANTHROPIC_AUTH_TOKEN: "",
      CLAUDE_CODE_AUTO_COMPACT_WINDOW: "1000000", ...coding("MiniMax-M3[1M]"),
    },
  },
  {
    id: "minimax-intl", name: "MiniMax 国际", category: "cn_official", websiteUrl: "https://platform.minimax.io",
    env: {
      ANTHROPIC_BASE_URL: "https://api.minimax.io/anthropic", ANTHROPIC_AUTH_TOKEN: "",
      CLAUDE_CODE_AUTO_COMPACT_WINDOW: "1000000", ...coding("MiniMax-M3[1M]"),
    },
  },
  {
    id: "stepfun", name: "阶跃星辰 Step Plan", category: "cn_official", websiteUrl: "https://platform.stepfun.com/step-plan",
    env: { ANTHROPIC_BASE_URL: "https://api.stepfun.com/step_plan", ANTHROPIC_AUTH_TOKEN: "", ...coding("step-3.5-flash-2603") },
  },
  {
    id: "mimo", name: "小米 MiMo", category: "cn_official", websiteUrl: "https://platform.xiaomimimo.com",
    env: { ANTHROPIC_BASE_URL: "https://api.xiaomimimo.com/anthropic", ANTHROPIC_AUTH_TOKEN: "", ...coding("mimo-v2.6-pro") },
  },
  {
    id: "longcat", name: "LongCat", category: "cn_official", websiteUrl: "https://longcat.chat/platform",
    env: {
      ANTHROPIC_BASE_URL: "https://api.longcat.chat/anthropic", ANTHROPIC_AUTH_TOKEN: "", ...coding("LongCat-2.0"),
      CLAUDE_CODE_MAX_OUTPUT_TOKENS: "131072",
    },
  },
  {
    id: "modelscope", name: "魔搭 ModelScope", category: "aggregator", websiteUrl: "https://modelscope.cn",
    env: { ANTHROPIC_BASE_URL: "https://api-inference.modelscope.cn", ANTHROPIC_AUTH_TOKEN: "", ...coding("ZhipuAI/GLM-5.2") },
  },
  {
    id: "siliconflow", name: "硅基流动", category: "aggregator", websiteUrl: "https://siliconflow.cn",
    env: { ANTHROPIC_BASE_URL: "https://api.siliconflow.cn", ANTHROPIC_AUTH_TOKEN: "", ...coding("Pro/MiniMaxAI/MiniMax-M2.5") },
  },
  {
    id: "openrouter", name: "OpenRouter", category: "aggregator", websiteUrl: "https://openrouter.ai",
    env: {
      ANTHROPIC_BASE_URL: "https://openrouter.ai/api", ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "anthropic/claude-sonnet-5", ANTHROPIC_DEFAULT_HAIKU_MODEL: "anthropic/claude-haiku-4.5",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "anthropic/claude-sonnet-5", ANTHROPIC_DEFAULT_OPUS_MODEL: "anthropic/claude-opus-5",
    },
  },
];

export const PRESET_CATEGORY_LABELS: Record<ClaudePresetCategory, string> = {
  official: "官方",
  cn_official: "模型厂商",
  aggregator: "聚合平台",
};

/** 自定义供应商的起点：只有地址与 Key 两个空位。 */
export function blankClaudeProvider(): ClaudeProvider {
  return {
    id: "",
    name: "",
    websiteUrl: "",
    notes: "",
    settingsConfig: { env: { ANTHROPIC_BASE_URL: "", ANTHROPIC_AUTH_TOKEN: "" } },
    createdAt: 0,
  };
}

export function providerFromPreset(preset: ClaudePreset): ClaudeProvider {
  return {
    id: "",
    name: preset.name,
    websiteUrl: preset.websiteUrl,
    notes: "",
    settingsConfig: { env: { ...preset.env } },
    createdAt: 0,
  };
}

/** 复制一份：去掉 id 与 CC Switch 来源，名字加后缀。 */
export function duplicateClaudeProvider(provider: ClaudeProvider, suffix: string): ClaudeProvider {
  return {
    ...provider,
    id: "",
    name: `${provider.name} ${suffix}`.trim(),
    settingsConfig: structuredClone(provider.settingsConfig),
    createdAt: 0,
    ccsSourceId: undefined,
  };
}

function envOf(config: ClaudeSettingsConfig): Record<string, unknown> {
  const env = config.env;
  return env && typeof env === "object" && !Array.isArray(env) ? env : {};
}

function envString(config: ClaudeSettingsConfig, key: string): string {
  const value = envOf(config)[key];
  return typeof value === "string" ? value : value === undefined || value === null ? "" : String(value);
}

export function readFormFields(config: ClaudeSettingsConfig): ClaudeFormFields {
  const env = envOf(config);
  // 只有 ANTHROPIC_API_KEY 时按它编辑；两个都有时以 AUTH_TOKEN 为准（Claude Code 也会警告两者同时存在）。
  const authField: ClaudeAuthField = "ANTHROPIC_API_KEY" in env && !("ANTHROPIC_AUTH_TOKEN" in env)
    ? "ANTHROPIC_API_KEY"
    : "ANTHROPIC_AUTH_TOKEN";
  return {
    baseUrl: envString(config, "ANTHROPIC_BASE_URL"),
    apiKey: envString(config, authField),
    authField,
    model: envString(config, MODEL_ROLE_KEYS.model),
    haikuModel: envString(config, MODEL_ROLE_KEYS.haikuModel),
    sonnetModel: envString(config, MODEL_ROLE_KEYS.sonnetModel),
    opusModel: envString(config, MODEL_ROLE_KEYS.opusModel),
    fableModel: envString(config, MODEL_ROLE_KEYS.fableModel),
  };
}

/** 把表单字段写回配置：空值删键，其余键原样保留。 */
export function applyFormFields(config: ClaudeSettingsConfig, fields: ClaudeFormFields): ClaudeSettingsConfig {
  const env: Record<string, unknown> = { ...envOf(config) };
  const put = (key: string, value: string) => {
    const trimmed = value.trim();
    if (trimmed) env[key] = trimmed;
    else delete env[key];
  };
  put("ANTHROPIC_BASE_URL", fields.baseUrl);
  const otherAuth: ClaudeAuthField = fields.authField === "ANTHROPIC_AUTH_TOKEN" ? "ANTHROPIC_API_KEY" : "ANTHROPIC_AUTH_TOKEN";
  delete env[otherAuth];
  put(fields.authField, fields.apiKey);
  for (const [field, key] of Object.entries(MODEL_ROLE_KEYS)) {
    put(key, fields[field as keyof typeof MODEL_ROLE_KEYS]);
  }
  return { ...config, env };
}

export function compatEnabled(config: ClaudeSettingsConfig, key: string, on: string): boolean {
  return envString(config, key) === on;
}

export function setCompat(config: ClaudeSettingsConfig, key: string, on: string, enabled: boolean): ClaudeSettingsConfig {
  const env: Record<string, unknown> = { ...envOf(config) };
  if (enabled) env[key] = on;
  else delete env[key];
  return { ...config, env };
}

export type ParsedSettings = { ok: true; value: ClaudeSettingsConfig } | { ok: false; error: string };

/** 解析 JSON 编辑框：顶层必须是对象，env（若有）也必须是对象、值只能是标量。 */
export function parseSettingsConfig(text: string): ParsedSettings {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    return { ok: false, error: `JSON 格式错误：${error instanceof Error ? error.message : String(error)}` };
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ok: false, error: "顶层必须是 JSON 对象" };
  }
  const env = (value as ClaudeSettingsConfig).env;
  if (env !== undefined) {
    if (!env || typeof env !== "object" || Array.isArray(env)) return { ok: false, error: "env 必须是 JSON 对象" };
    for (const [key, item] of Object.entries(env)) {
      if (!["string", "number", "boolean"].includes(typeof item)) {
        return { ok: false, error: `env.${key} 必须是字符串、数字或布尔值` };
      }
    }
  }
  return { ok: true, value: value as ClaudeSettingsConfig };
}

export function formatSettingsConfig(config: ClaudeSettingsConfig): string {
  return JSON.stringify(config, null, 2);
}

/** 卡片副标题：地址 · 主模型。没有地址时说明走官方登录。 */
export function providerBrief(provider: ClaudeProvider): { endpoint: string; model: string } {
  const fields = readFormFields(provider.settingsConfig);
  return {
    endpoint: fields.baseUrl ? fields.baseUrl.replace(/^https?:\/\//, "").replace(/\/+$/, "") : "",
    model: fields.model || fields.sonnetModel || fields.opusModel,
  };
}

/** 保存前的检查：名字必填；填了 Base URL 必须是 http(s) 地址。 */
export function validateProviderDraft(provider: ClaudeProvider): string | null {
  if (!provider.name.trim()) return "请填写供应商名称";
  const baseUrl = readFormFields(provider.settingsConfig).baseUrl.trim();
  if (baseUrl && !/^https?:\/\/\S+$/i.test(baseUrl)) return "Base URL 需要以 http:// 或 https:// 开头";
  return null;
}
