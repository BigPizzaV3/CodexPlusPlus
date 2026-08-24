/**
 * 把上游返回的模型 ID 列表按家族分组，供「默认模型」下拉展示。
 *
 * 分组只为好找，不参与任何配置写入：认不出家族的一律归到最后的 Other。
 */

export interface ModelGroup {
  label: string;
  models: string[];
}

/**
 * 家族名（正则片段） -> 展示名。顺序即分组顺序。
 *
 * 家族名后面允许直接贴版本号（qwen3、glm4、o3），所以不能用 \b：数字也是
 * 单词字符，\b 在 qwen|3 之间并不成立。统一按「家族名 + 可选版本号 + 分隔符
 * 或结束」匹配。
 */
const MODEL_FAMILIES: ReadonlyArray<readonly [string, string]> = [
  ["gpt|chatgpt", "GPT"],
  ["o[1-9]", "OpenAI o-series"],
  ["codex", "Codex"],
  ["claude", "Claude"],
  ["gemini", "Gemini"],
  ["deepseek", "DeepSeek"],
  ["qwen|qwq", "Qwen"],
  ["glm", "GLM"],
  ["kimi", "Kimi"],
  ["grok", "Grok"],
  ["llama", "Llama"],
  ["mistral", "Mistral"],
];

const FAMILY_PATTERNS: ReadonlyArray<readonly [RegExp, string]> = MODEL_FAMILIES.map(
  ([names, label]) => [new RegExp(`^(?:${names})[0-9]*(?:-|$)`), label] as const,
);

export const OTHER_MODEL_GROUP = "Other";

function familyOf(model: string): string {
  // 各种分隔符先统一成 -，这样 qwen_vl / glm/4.6 / gpt.5 走同一条规则
  const normalized = model.trim().toLowerCase().replace(/[._/:]/g, "-");
  const matched = FAMILY_PATTERNS.find(([pattern]) => pattern.test(normalized));
  return matched ? matched[1] : OTHER_MODEL_GROUP;
}

/**
 * 去重、按家族分组，家族内按名称排序。
 * 空白项被丢掉；Other 永远排在最后。
 */
export function groupModelIds(models: readonly string[]): ModelGroup[] {
  const seen = new Set<string>();
  const buckets = new Map<string, string[]>();

  for (const raw of models) {
    const model = raw.trim();
    if (!model || seen.has(model)) continue;
    seen.add(model);
    const family = familyOf(model);
    const bucket = buckets.get(family);
    if (bucket) bucket.push(model);
    else buckets.set(family, [model]);
  }

  const order = [...MODEL_FAMILIES.map(([, label]) => label), OTHER_MODEL_GROUP];
  return order
    .filter((label) => buckets.has(label))
    .map((label) => ({
      label,
      models: (buckets.get(label) ?? []).sort((a, b) => a.localeCompare(b, "en")),
    }));
}

/** 按关键字过滤后再分组；query 为空则等价于 groupModelIds。 */
export function filterModelGroups(models: readonly string[], query: string): ModelGroup[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return groupModelIds(models);
  return groupModelIds(models.filter((model) => model.toLowerCase().includes(normalized)));
}
