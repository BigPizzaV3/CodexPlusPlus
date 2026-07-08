export type BedrockAuthMode = "bearerToken" | "awsProfile";

export type BedrockConfig = {
  authMode: BedrockAuthMode | null; // null = 尚未选择路径（仅草稿态）
  providerId: string;
  region: string;
  awsProfile: string;
  iamUserName: string;
  iamKeyValidityDays: string;
};

// ---------------------------------------------------------------------------
// 保留 provider 标识符（与 Rust 端同步）
// ---------------------------------------------------------------------------

/** 与 Rust 端 RESERVED_MODEL_PROVIDER_IDS 保持同步的保留 provider 标识符列表 */
export const RESERVED_MODEL_PROVIDER_IDS_MIRROR: readonly string[] = [
  "amazon-bedrock",
  "openai",
  "ollama",
  "lmstudio",
  "oss",
  "ollama-chat",
] as const;

/** 检查给定 id（trim 后）是否命中保留 provider 标识符 */
export function isReservedProviderId(id: string): boolean {
  return RESERVED_MODEL_PROVIDER_IDS_MIRROR.includes(id.trim());
}

// ---------------------------------------------------------------------------
// Bedrock Mantle URL 模板常量
//
// 与 Rust 端 `crates/codex-plus-core/src/relay_config.rs` 的
// `BEDROCK_MANTLE_URL_PREFIX` / `BEDROCK_MANTLE_URL_SUFFIX` 保持一致。
// 修改这里必须同步修改 Rust 常量（前后端都在生成与识别端使用）。
// ---------------------------------------------------------------------------
export const BEDROCK_MANTLE_URL_PREFIX = "https://bedrock-mantle.";
export const BEDROCK_MANTLE_URL_SUFFIX = ".api.aws/openai/v1";

/**
 * 校验 Bedrock 配置的必填字段，返回第一个错误信息或 null（全部通过）。
 *
 * 当前实现直接返回展示用的中文文本，`t()` 在缺失映射时会回落到原文，所以在
 * 前端表现为原样中文；未来若要真正做多语言，可以把这里改成稳定英文 key
 * 并在 `i18n-en.ts` / `i18n.ts` 添加映射。
 *
 * 优先级：region → (bearerToken) providerId 空 → providerId 保留字冲突 → apiKey 空。
 * 该函数负责为 saveDraft 门控生成"任意一个 error 都拦保存"的信号；
 * UI 里各条 error 的具体展示（inline / 底部）由 `bedrockValidationErrorForBottom`
 * 与 `isReservedProviderId` 组合决定，避免"保留字冲突"同时被 inline 与底部展示。
 */
export function bedrockValidationError(bedrock: BedrockConfig, profileApiKey: string): string | null {
  if (!bedrock.region.trim()) return "region 为必填项";
  if (bedrock.authMode === "bearerToken") {
    if (!bedrock.providerId.trim()) return "provider 标识符为必填项";
    if (isReservedProviderId(bedrock.providerId)) return "该标识符与保留供应商冲突";
    if (!profileApiKey.trim()) return "Bedrock API Key 为必填项";
  }
  return null;
}

/**
 * `bedrockValidationError` 的子集，仅返回**非"保留字冲突"**类别的错误。
 *
 * 用于 `BedrockRelayProfileEditor` 底部聚合展示：保留字冲突已经由 Provider ID
 * 输入框下方的 inline `<p>` 展示，如果底部再展示同一条消息就会重复；
 * 但如果同时还有 region 或 apiKey 空的错误，底部仍需展示这些错误。
 */
export function bedrockValidationErrorForBottom(bedrock: BedrockConfig, profileApiKey: string): string | null {
  if (!bedrock.region.trim()) return "region 为必填项";
  if (bedrock.authMode === "bearerToken") {
    if (!bedrock.providerId.trim()) return "provider 标识符为必填项";
    // 保留字冲突交给 inline 展示，此处跳过
    if (!profileApiKey.trim()) return "Bedrock API Key 为必填项";
  }
  return null;
}

// ---------------------------------------------------------------------------
// TOML 解析辅助函数（纯字符串解析，不依赖 App.tsx 的私有函数）
// ---------------------------------------------------------------------------

/** 从一行中提取 `[sectionName]` 形式的表头名称，找不到返回 null */
function tomlSectionNameFromLine(line: string): string | null {
  const match = /^\s*\[([^\]]+)\]\s*$/.exec(line);
  return match ? match[1].trim() : null;
}

/**
 * 转义在 RegExp 中具有特殊含义的字符，以便把任意字符串安全地拼进正则字面量。
 * 当前调用点只传硬编码 key，但作为 helper 语义上必须是安全的。
 */
function escapeRegExp(input: string): string {
  return input.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** 从一行中提取 `key = "value"` 或 `key = 'value'` 的字符串赋值，找不到返回 null */
function tomlStringAssignment(line: string, key: string): string | null {
  const match = new RegExp(`^\\s*${escapeRegExp(key)}\\s*=\\s*([\"'])(.*?)\\1\\s*(?:#.*)?$`).exec(line.trim());
  return match ? match[2].replace(/\\(["'\\])/g, "$1") : null;
}

/** 从一行中提取 `key = true` 或 `key = false` 的布尔赋值，找不到返回 null */
function tomlBoolAssignment(line: string, key: string): boolean | null {
  const match = new RegExp(`^\\s*${escapeRegExp(key)}\\s*=\\s*(true|false)\\s*(?:#.*)?$`).exec(line.trim());
  return match ? match[1] === "true" : null;
}

/** 读取根级（第一个 `[...]` 表头之前）的字符串赋值 */
function rootTomlStringValue(contents: string, key: string): string {
  const lines = contents.split(/\r?\n/);
  for (const line of lines) {
    // 遇到表头即停止（根级结束）
    if (tomlSectionNameFromLine(line) !== null) break;
    const value = tomlStringAssignment(line, key);
    if (value !== null) return value;
  }
  return "";
}

/**
 * 读取当前活跃 provider 表下的布尔字段值。
 *
 * 逻辑：先从根级获取 `model_provider` 的值，然后在
 * `[model_providers.<provider>]` section 中查找 `key = true|false`。
 */
export function codexProviderBoolFromConfig(contents: string, key: string): boolean {
  const provider = rootTomlStringValue(contents, "model_provider");
  if (!provider) return false;
  const targetSection = `model_providers.${provider}`;
  const lines = contents.split(/\r?\n/);
  let currentSection = "";

  for (const line of lines) {
    const section = tomlSectionNameFromLine(line);
    if (section !== null) {
      currentSection = section;
      continue;
    }
    if (currentSection === targetSection) {
      const value = tomlBoolAssignment(line, key);
      if (value !== null) return value;
    }
  }

  return false;
}

/**
 * 读取指定 TOML section 下的字符串字段值。
 *
 * `sectionName` 就是方括号中的内容，例如 `model_providers.amazon-bedrock.aws`
 * 对应 TOML 中的 `[model_providers.amazon-bedrock.aws]`。
 *
 * 遍历属于该 section 的行（从匹配的表头开始，到下一个 `[...]` 表头结束），
 * 查找 `key = "value"` 形式的字符串赋值。
 */
export function tomlSectionStringValue(contents: string, sectionName: string, key: string): string {
  const lines = contents.split(/\r?\n/);
  let inSection = false;

  for (const line of lines) {
    const section = tomlSectionNameFromLine(line);
    if (section !== null) {
      if (inSection) {
        // 遇到下一个表头，目标 section 结束
        break;
      }
      if (section === sectionName) {
        inSection = true;
      }
      continue;
    }
    if (inSection) {
      const value = tomlStringAssignment(line, key);
      if (value !== null) return value;
    }
  }

  return "";
}

/**
 * 读取当前活跃 provider 表下的字符串字段值。
 *
 * 逻辑：先从根级获取 `model_provider` 的值，然后在
 * `[model_providers.<provider>]` section 中查找 `key = "value"`。
 */
function codexProviderStringFromConfig(contents: string, key: string): string {
  const provider = rootTomlStringValue(contents, "model_provider");
  if (!provider) return "";
  return tomlSectionStringValue(contents, `model_providers.${provider}`, key);
}

/**
 * 从 config.toml 文本中识别 Bedrock 配置并还原为 BedrockConfig。
 *
 * 逻辑（镜像 Rust 端 `bedrock_config_from_config_text`）：
 * 1. 读取根级 `model_provider`
 * 2. 若为 `"amazon-bedrock"` → AWS Profile 路径
 * 3. 否则检查活跃 provider 的 `requires_openai_auth = true` 且
 *    `base_url` 匹配 bedrock-mantle 正则 → Bearer Token 路径
 * 4. 否则返回 null
 */
export function deriveBedrockConfigFromConfigText(configContents: string): BedrockConfig | null {
  const modelProvider = rootTomlStringValue(configContents, "model_provider");

  // AWS Profile 路径
  if (modelProvider === "amazon-bedrock") {
    const region = tomlSectionStringValue(configContents, "model_providers.amazon-bedrock.aws", "region");
    const awsProfile = tomlSectionStringValue(configContents, "model_providers.amazon-bedrock.aws", "profile");
    return {
      authMode: "awsProfile",
      providerId: "",
      region,
      awsProfile,
      iamUserName: "",
      iamKeyValidityDays: "90",
    };
  }

  // Bearer Token 路径：匹配 `<PREFIX><region><SUFFIX>`，其中 region 不含 `.` 或 `/`。
  if (modelProvider) {
    const requiresOpenaiAuth = codexProviderBoolFromConfig(configContents, "requires_openai_auth");
    if (requiresOpenaiAuth) {
      const baseUrl = codexProviderStringFromConfig(configContents, "base_url");
      const pattern = new RegExp(
        `^${escapeRegExp(BEDROCK_MANTLE_URL_PREFIX)}([^./]+)${escapeRegExp(BEDROCK_MANTLE_URL_SUFFIX)}$`,
      );
      const match = pattern.exec(baseUrl);
      if (match) {
        return {
          authMode: "bearerToken",
          providerId: modelProvider,
          region: match[1],
          awsProfile: "",
          iamUserName: "",
          iamKeyValidityDays: "90",
        };
      }
    }
  }

  return null;
}

// 最小接口，避免与 App.tsx 循环依赖
type HasBedrock = { bedrock?: BedrockConfig | null };

/** 判断 profile 是否为 Bedrock 类型 */
export function isBedrockRelayProfile(profile: HasBedrock): boolean {
  return !!profile.bedrock;
}

/**
 * 在 `deriveRelayProfileFromFiles` 收尾时决定 `profile.bedrock` 的最终值。
 *
 * 语义分层：
 * 1. **磁盘 config 已有 Bedrock 特征**（Bearer Token URL 或 amazon-bedrock provider）
 *    → 以 `deriveBedrockConfigFromConfigText(configContents)` 的推导结果为准（磁盘权威）。
 * 2. **磁盘 config 尚未写入 Bedrock 特征**，但 UI 已通过 patch 主动设置了 `existingBedrock`
 *    （例如刚点了「Amazon Bedrock」按钮，`bedrock.authMode` 还是 `null` 中间态）
 *    → 保留 `existingBedrock`，让编辑器能够切换到 `BedrockRelayProfileEditor`
 *    并等待用户填写。
 * 3. 其余情况 → 返回 `null`。
 *
 * 若直接采用「无条件用推导结果覆盖」的写法，第 2 类中间态会被立即抹掉，
 * 表现为「点了 Amazon Bedrock 按钮但界面没有切换」。见 issue: onSelectBedrock
 * 中间态被 deriveRelayProfileFromFiles 吞掉。
 */
export function resolveBedrockAfterDerive(
  existingBedrock: BedrockConfig | null | undefined,
  configContents: string,
): BedrockConfig | null {
  const derived = deriveBedrockConfigFromConfigText(configContents);
  if (derived) return derived;
  return existingBedrock ?? null;
}

/** 判断 profile 是否允许 provider 连通性测试（awsProfile 模式下不允许） */
export function bedrockAllowsProviderTesting(profile: HasBedrock): boolean {
  return !profile.bedrock || profile.bedrock.authMode !== "awsProfile";
}

/**
 * `ProviderPresetSelector` 的 Bedrock 入口按钮渲染判定：
 *
 * 仅当宿主组件提供了 `onSelectBedrock` 回调（说明当前允许创建 Bedrock 供应商）
 * 才渲染入口按钮。该函数纯粹依据 prop 类型判断，方便在无 DOM 的测试环境中直接
 * 验证渲染分支的开关逻辑。
 */
export function shouldRenderBedrockPresetButton(
  onSelectBedrock: (() => void) | undefined,
): boolean {
  return typeof onSelectBedrock === "function";
}

// ---------------------------------------------------------------------------
// CLI 命令生成函数
// ---------------------------------------------------------------------------

/** Long-Term Key 有效期天数的默认回退值，与 `default_bedrock_iam_key_validity_days` 对齐。 */
const DEFAULT_LONG_TERM_KEY_DAYS = "90";

/**
 * 生成创建 Long-Term Bedrock API Key 的 AWS CLI 命令文本。
 *
 * - 用户名为空/空白时使用占位符 <IAM_USER_NAME>。
 * - 有效期若不是正整数字符串（例如空、字母、负号）则回退到 90，
 *   避免把 `abc` / `-5` 之类的值原样嵌入到示例命令里让用户拷去 shell 报错。
 */
export function bedrockLongTermApiKeyCommand(iamUserName: string, validityDays: string): string {
  const user = iamUserName.trim() || "<IAM_USER_NAME>";
  const daysTrimmed = validityDays.trim();
  const days = /^[1-9][0-9]*$/.test(daysTrimmed) ? daysTrimmed : DEFAULT_LONG_TERM_KEY_DAYS;
  return `aws iam create-service-specific-credential \\\n  --user-name ${user} \\\n  --service-name bedrock.amazonaws.com \\\n  --credential-age-days ${days}`;
}

/**
 * Short-Term Bedrock API Key 的说明性文本。
 * 不包含 `aws iam create-service-specific-credential`，
 * 包含 "12 小时" 有效期与权限继承说明。
 */
export function bedrockShortTermApiKeyGuidanceText(): string {
  return "短期 Bedrock API Key 通过 aws-bedrock-token-generator 生成，有效期不超过 12 小时，并继承生成时使用的 AWS 主体权限。请参考 aws-bedrock-token-generator 文档在本机生成，本工具不会代为调用 AWS API。";
}

/**
 * 生成 AWS SSO configure 命令。profile 名称非空非空白时附加 --profile 参数。
 */
export function bedrockAwsProfileConfigureCommand(awsProfile: string): string {
  const profileFlag = awsProfile.trim() ? ` --profile ${awsProfile.trim()}` : "";
  return `aws configure sso${profileFlag}`;
}

/**
 * 生成 AWS SSO login 命令。profile 名称非空非空白时附加 --profile 参数。
 */
export function bedrockAwsProfileLoginCommand(awsProfile: string): string {
  const profileFlag = awsProfile.trim() ? ` --profile ${awsProfile.trim()}` : "";
  return `aws sso login${profileFlag}`;
}

// ---------------------------------------------------------------------------
// BedrockRelayProfileEditor 渲染逻辑辅助函数（保持 UI 组件与纯函数解耦）
// ---------------------------------------------------------------------------

/**
 * 判断当前是否应该渲染鉴权路径专属字段。
 *
 * `authMode === null` 时（中间态，用户尚未选择路径）不渲染任何专属字段，
 * 只保留路径选择按钮；两条路径任选其一后才渲染各自的字段集合。
 */
export function bedrockShouldRenderPathFields(bedrock: BedrockConfig | null | undefined): boolean {
  return !!bedrock && bedrock.authMode !== null;
}

/**
 * Bearer Token 分支需要渲染的字段名集合，对应 Requirement 1.3。
 *
 * 字段：region、providerId、apiKey（复用 profile.apiKey）、iamUserName、iamKeyValidityDays。
 * 顺序仅表达"分支下必须至少存在的字段"，不代表 UI 具体排布。
 */
export function bedrockBearerTokenFieldKeys(): readonly string[] {
  return ["region", "providerId", "apiKey", "iamUserName", "iamKeyValidityDays"] as const;
}

/**
 * AWS Profile 分支需要渲染的字段名集合，对应 Requirement 1.4。
 *
 * 字段：region、awsProfile（AWS profile 名称可选）。
 */
export function bedrockAwsProfileFieldKeys(): readonly string[] {
  return ["region", "awsProfile"] as const;
}

/**
 * Bedrock 模型 ID 是否需要展示"非 openai.* 前缀"提示。
 *
 * Bedrock via OpenAI 兼容 endpoint 通常需要 `openai.*` 前缀的模型 ID；
 * 用户填写非该前缀的模型时展示警告，但不阻塞保存（Requirement 6.1）。
 * 空/全空白的模型串不触发提示。
 */
export function bedrockModelNeedsWarning(model: string): boolean {
  const trimmed = model.trim();
  return trimmed.length > 0 && !trimmed.startsWith("openai.");
}

/**
 * AWS Profile 分支的 CLI 命令文本区域内容：
 * 把 `aws configure sso [--profile ...]` 与 `aws sso login [--profile ...]` 拼接在一起，
 * 便于在同一个 `<pre>`/`<textarea>` 中一次性展示两条命令（Requirement 8.4）。
 */
export function bedrockAwsProfileCommandBlock(awsProfile: string): string {
  return `${bedrockAwsProfileConfigureCommand(awsProfile)}\n${bedrockAwsProfileLoginCommand(awsProfile)}`;
}
