export const DEFAULT_AUTO_COMPACT_PERCENT = "90%";

/** 校验用户输入的自动压缩比例；空值由 Codex++ 保存为明确的 90% 默认值。 */
export function isValidAutoCompactPercent(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return true;
  if (!/^\d+(?:\.\d{1,6})?%?$/.test(trimmed)) return false;
  const numeric = Number(trimmed.replace(/%$/, ""));
  return Number.isFinite(numeric) && numeric > 0 && numeric <= 100;
}

/** 将合法比例统一为带百分号的文本；非法值原样返回，便于界面显示错误。 */
export function normalizeAutoCompactPercent(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  if (!isValidAutoCompactPercent(trimmed)) return trimmed;
  return `${trimmed.replace(/%$/, "").trim()}%`;
}

/** 编辑带后缀的百分比时，把误输入到 % 后面的数字移回数字部分。 */
export function normalizeAutoCompactEditing(value: string, previousValue = ""): string {
  if (!value.trim()) return "";
  const numeric = value.replace(/%/g, "");
  // 编辑时不强行补回末尾百分号：从 90% 删除到 9% 时，用户可以直接继续输入 20。
  // 失焦逻辑会统一补回百分号。若用户在百分号后输入字符，则把字符移回数字部分。
  if (value.trim().endsWith("%")) return numeric;
  if (value.includes("%")) return `${numeric}%`;
  return numeric;
}
