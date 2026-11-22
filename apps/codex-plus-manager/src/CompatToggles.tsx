/** 四个请求兼容开关（移除托管生图工具、移除图片 detail、不发送 output.encrypted_content、不发送 tool_search）：
 * 网关供应商的编辑弹窗和直连供应商编辑页共用一张选项表，文案只写一份；直连多一句「请求改走本地代理」之类的说明。 */
import { getLanguage, t } from "./i18n";

export type CompatToggleKey = "stripImageGenerationTool" | "stripResponsesImageDetail" | "stripResponsesOutputEncryptedContent" | "disableToolSearch";

export type CompatToggleOption = {
  key: CompatToggleKey;
  title: string;
  /** 两边都成立的说明。 */
  description: string;
  /** 只对直连供应商成立的补充（走本地代理、需要重启等）。 */
  directNote: string;
  /** 直连编辑页里这一行的定位类名（分区跳转、测试用）。 */
  fieldClass: string;
};

export const COMPAT_TOGGLE_OPTIONS: CompatToggleOption[] = [
  {
    key: "stripImageGenerationTool",
    title: "移除请求里的托管生图工具（image_generation）",
    description: "Codex 每次请求都会带上托管生图工具，不支持生图的中转会整条请求返回 403 或报错。开启后转发前从 tools 里去掉它（及指向它的 tool_choice）。",
    directNote: "请求改走本地代理；Codex 里将无法调用内置生图。",
    fieldClass: "relay-field-strip-image-generation",
  },
  {
    key: "stripResponsesImageDetail",
    title: "移除 Responses 图片 detail 字段",
    description: "部分 Responses 上游不认 OpenAI 的 input_image.detail，只会返回泛化的参数错误。开启后转发前从普通消息和工具输出里的图片块删除 detail。",
    directNote: "请求改走本地代理。",
    fieldClass: "relay-field-strip-responses-image-detail",
  },
  {
    key: "stripResponsesOutputEncryptedContent",
    title: "不发送 output.encrypted_content（上游不支持时开启）",
    description: "部分严格 Responses 上游不接受 Codex 请求 include 里的 output.encrypted_content，会整条返回 400。开启后只删除这一项，保留 reasoning.encrypted_content 和其他项；Kimi 地址或模型名会自动处理。",
    directNote: "请求改走本地代理。",
    fieldClass: "relay-field-strip-responses-output-encrypted-content",
  },
  {
    key: "disableToolSearch",
    title: "不发送 tool_search（上游不支持时开启）",
    description: "部分第三方 Responses 上游不认 Codex 的托管工具 tool_search，供应商测试能通过，但实际聊天会返回 400（tool type \"tool_search\" is not supported）。开启后模型目录关闭 supports_search_tool，Codex 改为直接列出 MCP 工具。",
    directNote: "Kimi 官方地址会自动处理。重启 Codex 后生效。",
    fieldClass: "relay-field-disable-tool-search",
  },
];

/** 一个开关的完整说明：通用说明（直连再加补充），最后是「默认关闭」。英文句子之间补空格。 */
export function compatToggleDescription(option: CompatToggleOption, variant: "gateway" | "direct"): string {
  const parts = [t(option.description), variant === "direct" ? t(option.directNote) : "", t("默认关闭。")].filter(Boolean);
  return parts.join(getLanguage() === "en" ? " " : "");
}

export function CompatToggles({
  variant,
  values,
  onChange,
}: {
  /** gateway：编辑弹窗里的 pf-switch 样式；direct：直连编辑页的 switch-row 样式。 */
  variant: "gateway" | "direct";
  values: Partial<Record<CompatToggleKey, boolean | undefined>>;
  onChange: (patch: Partial<Record<CompatToggleKey, boolean>>) => void;
}) {
  return (
    <>
      {COMPAT_TOGGLE_OPTIONS.map((option) => {
        const input = (
          <input
            checked={values[option.key] === true}
            onChange={(event) => onChange({ [option.key]: event.currentTarget.checked })}
            type="checkbox"
          />
        );
        const text = (
          <span>
            <strong>{t(option.title)}</strong>
            <small>{compatToggleDescription(option, variant)}</small>
          </span>
        );
        return variant === "gateway" ? (
          <label className="pf-switch" key={option.key}>
            {input}
            {text}
          </label>
        ) : (
          <label className={`switch-row compact relay-switch-row ${option.fieldClass}`} key={option.key}>
            {input}
            {text}
            <span aria-hidden="true" className="toggle-switch-visual">
              <span className="toggle-switch-thumb" />
            </span>
          </label>
        );
      })}
    </>
  );
}
