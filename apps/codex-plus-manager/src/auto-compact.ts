export const DEFAULT_AUTO_COMPACT_PERCENT = "90%";

/** 校验用户输入的自动压缩比例；空值表示沿用 Codex 默认行为。 */
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
