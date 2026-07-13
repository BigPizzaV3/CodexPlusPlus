import { useMemo, type ReactNode } from "react";
import type { RelayProfile } from "../App.tsx";
import type { BedrockConfig, BedrockAuthMode } from "../bedrock-config.ts";
import {
  bedrockLongTermApiKeyCommand,
  bedrockShortTermApiKeyGuidanceText,
  bedrockAwsProfileCommandBlock,
  bedrockValidationErrorForBottom,
  bedrockShouldRenderPathFields,
  bedrockModelNeedsWarning,
  isReservedProviderId,
} from "../bedrock-config.ts";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { t } from "@/i18n";

/**
 * 只暴露 `profile` + `onChange`。保存/取消按钮由外层通用编辑器 header 统一处理
 * （详见 App.tsx 中 relay-editor-head 上的「返回列表」/「保存」），
 * 编辑器内部不再自绘一组按钮，避免和顶部重复且实际点了没反应。
 */
export type BedrockRelayProfileEditorProps = {
  profile: RelayProfile;
  onChange: (patch: Partial<RelayProfile>) => void;
};

/**
 * 与 App.tsx 内部的 `Field` 组件同结构（`<Label className="field"><span>label</span>{children}</Label>`），
 * 复用通用主题类 `.field` / `.field span`，不再自绘 `.bedrock-field` 那套并列样式。
 * 独立定义在这里，避免仅为共享 wrapper 就把 App.tsx 里的 Field 导出（改动面更大）。
 */
function BedrockField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Label className="field">
      <span>{label}</span>
      {children}
    </Label>
  );
}

/**
 * CLI 示例区分隔线：一条横线中间嵌一句说明，把下方"命令行生成示例"与上方的实际
 * 配置字段在视觉上隔开。这些命令仅用于在本机生成 / 获取凭据（Long-Term Key、
 * AWS SSO 登录等），本工具不会代为调用 AWS API，对已保存的 profile 配置没有直接
 * 影响；分隔线用来提醒用户不要把它们误当成需要填写的配置项。
 */
function BedrockCliExamplesDivider() {
  return (
    <div className="bedrock-cli-divider" role="separator">
      <span>{t("以下为命令行生成示例，不影响实际配置")}</span>
    </div>
  );
}

export function BedrockRelayProfileEditor({ profile, onChange }: BedrockRelayProfileEditorProps) {
  const bedrock = profile.bedrock ?? {
    authMode: null as BedrockAuthMode | null,
    providerId: "",
    region: "",
    awsProfile: "",
    iamUserName: "",
    iamKeyValidityDays: "90",
  };

  // 底部聚合的校验提示只承担"非保留字冲突"类别的错误；保留字冲突由 Provider ID
  // 字段下方的 inline `<p>` 展示，避免同一条错误被两处红字重复渲染（P0 bug）。
  // 保存按钮的门控仍然使用完整的 `bedrockValidationError`（在 App.tsx 里），
  // 因此保留字冲突时保存按钮会照常禁用。
  const bottomValidationError = useMemo(
    () => (bedrock.authMode ? bedrockValidationErrorForBottom(bedrock as BedrockConfig, profile.apiKey) : null),
    [bedrock, profile.apiKey]
  );

  const setBedrock = (patch: Partial<BedrockConfig>) => {
    onChange({ bedrock: { ...(bedrock as BedrockConfig), ...patch } });
  };

  const isNonOpenAiModel = bedrockModelNeedsWarning(profile.model);
  const providerIdReserved = bedrock.authMode === "bearerToken" && isReservedProviderId(bedrock.providerId);

  return (
    <div className="bedrock-editor relay-fields">
      <h3>{t("Amazon Bedrock 配置")}</h3>

      {/* 供应商显示名——列表和切换菜单里用来标识这个 profile。
       * 与 Bearer Token 分支下的「Provider 标识符」不同：后者会写入 config.toml 的
       * `model_provider` 字段（受 TOML key 语法约束），这个 name 只是 UI 展示。 */}
      <BedrockField label={t("名称")}>
        <Input
          type="text"
          value={profile.name}
          onChange={(e) => onChange({ name: e.target.value })}
          placeholder={t("例如 Bedrock 生产环境")}
        />
      </BedrockField>

      {/* 鉴权路径二选一——沿用 Bedrock 专用的 radio 分段样式（.bedrock-auth-mode） */}
      <div className="bedrock-auth-mode">
        <label>{t("鉴权路径")}</label>
        <div className="bedrock-auth-mode-choices">
          <label>
            <input
              type="radio"
              name="bedrockAuthMode"
              value="bearerToken"
              checked={bedrock.authMode === "bearerToken"}
              onChange={() => setBedrock({ authMode: "bearerToken" })}
            />
            {t("Bedrock API Key (Bearer Token)")}
          </label>
          <label>
            <input
              type="radio"
              name="bedrockAuthMode"
              value="awsProfile"
              checked={bedrock.authMode === "awsProfile"}
              onChange={() => setBedrock({ authMode: "awsProfile" })}
            />
            {t("AWS Profile (SSO)")}
          </label>
        </div>
      </div>

      {/* 通用字段：Region
       * AWS region 命名规范全部小写（如 us-east-1、ap-southeast-2），
       * 但 macOS Safari/WKWebView 会对文本输入首字母自动大写；
       * 关掉 autocapitalize 只是第一道防线，onChange 里再统一 toLowerCase()
       * 强制小写，兼容用户手动输入大写的情况。 */}
      {bedrockShouldRenderPathFields(bedrock) && (
        <BedrockField label={t("AWS Region")}>
          <Input
            type="text"
            value={bedrock.region}
            onChange={(e) => setBedrock({ region: e.target.value.toLowerCase() })}
            placeholder="us-east-1"
            autoCapitalize="off"
            autoCorrect="off"
            autoComplete="off"
            spellCheck={false}
          />
        </BedrockField>
      )}

      {/* Bearer Token 分支 */}
      {bedrock.authMode === "bearerToken" && (
        <>
          <BedrockField label={t("Provider 标识符")}>
            <Input
              type="text"
              value={bedrock.providerId}
              onChange={(e) => setBedrock({ providerId: e.target.value })}
              placeholder="my-bedrock"
              autoCapitalize="off"
              autoCorrect="off"
              autoComplete="off"
              spellCheck={false}
            />
            {providerIdReserved && (
              <p className="bedrock-error">{t("该标识符与保留供应商冲突")}</p>
            )}
          </BedrockField>
          <BedrockField label={t("Bedrock API Key")}>
            <Input
              type="password"
              value={profile.apiKey}
              onChange={(e) => onChange({ apiKey: e.target.value })}
              placeholder={t("从 AWS IAM 生成的 Long-Term 或 Short-Term Key")}
            />
          </BedrockField>
        </>
      )}

      {/* AWS Profile 分支 */}
      {bedrock.authMode === "awsProfile" && (
        <BedrockField label={t("AWS Profile 名称（可选）")}>
          <Input
            type="text"
            value={bedrock.awsProfile}
            onChange={(e) => setBedrock({ awsProfile: e.target.value })}
            placeholder="default"
            autoCapitalize="off"
            autoCorrect="off"
            autoComplete="off"
            spellCheck={false}
          />
        </BedrockField>
      )}

      {/* 模型字段（Bedrock Mantle 的 /v1/responses endpoint 只暴露 GPT-5.x 系列）。
       * `model` 会写入 config.toml，属于影响实际配置的字段，因此放在配置字段区、
       * CLI 示例分隔线的上方，不与"命令行生成示例"混在一起。 */}
      <BedrockField label={t("模型")}>
        <Input
          type="text"
          value={profile.model}
          onChange={(e) => onChange({ model: e.target.value })}
          placeholder="openai.gpt-5.5"
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          spellCheck={false}
        />
        <p className="field-hint">
          {t("Bedrock Mantle 的 /v1/responses 端点仅暴露 GPT-5.x：openai.gpt-5.5（仅 us-east-2）、openai.gpt-5.4。GPT-OSS 等 Bedrock native 模型 ID 走 Mantle 会返回 404，需要通过 LiteLLM 中转。")}
        </p>
        {isNonOpenAiModel && (
          <p className="bedrock-warning">
            {t("Bedrock via OpenAI-兼容 endpoint 通常需要 openai.* 前缀的模型 ID；已按当前值保存")}
          </p>
        )}
      </BedrockField>

      {/* CLI 示例区：统一放在模型等实际配置字段的下方，用分隔线与配置区隔开。
       * 这些命令仅用于在本机生成 / 获取凭据（Long-Term Key、AWS SSO 登录等），
       * 工具不会代为调用 AWS API，对已保存的 profile 配置没有直接影响。 */}
      {bedrock.authMode === "bearerToken" && (
        <>
          <BedrockCliExamplesDivider />
          {/* IAM 用户名 / 有效期只作为下方示例命令的参数，不参与 config.toml 生成，
           * 因此放在分隔线下方的示例区，随输入实时刷新命令文本。 */}
          <BedrockField label={t("IAM 用户名")}>
            <Input
              type="text"
              value={bedrock.iamUserName}
              onChange={(e) => setBedrock({ iamUserName: e.target.value })}
              placeholder="my-iam-user"
              autoCapitalize="off"
              autoCorrect="off"
              autoComplete="off"
              spellCheck={false}
            />
            <p className="field-hint">
              {t("仅用于拼接下方 Long-Term Key 示例命令的 --user-name 参数，不写入配置；留空时渲染成 <IAM_USER_NAME> 占位符。")}
            </p>
          </BedrockField>
          <BedrockField label={t("Long-Term Key 有效期（天）")}>
            <Input
              type="text"
              value={bedrock.iamKeyValidityDays}
              onChange={(e) => setBedrock({ iamKeyValidityDays: e.target.value })}
              placeholder="90"
            />
            <p className="field-hint">
              {t("仅用于拼接下方 Long-Term Key 示例命令的 --credential-age-days 参数，不写入配置。")}
            </p>
          </BedrockField>
          <div className="bedrock-cli-hint">
            <h4>{t("Long-Term API Key 生成命令")}</h4>
            <pre>{bedrockLongTermApiKeyCommand(bedrock.iamUserName, bedrock.iamKeyValidityDays)}</pre>
          </div>
          <div className="bedrock-cli-hint">
            <h4>{t("Short-Term API Key 说明")}</h4>
            <p>{bedrockShortTermApiKeyGuidanceText()}</p>
          </div>
        </>
      )}

      {bedrock.authMode === "awsProfile" && (
        <>
          <BedrockCliExamplesDivider />
          <div className="bedrock-cli-hint">
            <h4>{t("AWS SSO 配置/登录命令")}</h4>
            <pre>{bedrockAwsProfileCommandBlock(bedrock.awsProfile)}</pre>
          </div>
        </>
      )}

      {/* 底部聚合的校验提示（region / providerId / apiKey 空缺）。
       * 保留字冲突已经由 Provider ID 字段下方的 inline `<p>` 展示，此处刻意跳过
       * 以避免重复；保存按钮的门控仍然由 App.tsx 里的 `bedrockValidationError`
       * 覆盖所有情况。 */}
      {bottomValidationError && (
        <p className="bedrock-error">{t(bottomValidationError)}</p>
      )}
    </div>
  );
}
