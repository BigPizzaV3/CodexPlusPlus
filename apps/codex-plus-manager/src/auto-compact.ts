export const DEFAULT_AUTO_COMPACT_PERCENT = "90%";

export function isValidAutoCompactPercent(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return true;
  if (!/^\d+(?:\.\d{1,6})?%?$/.test(trimmed)) return false;
  const percent = Number(trimmed.replace(/%$/, ""));
  return Number.isFinite(percent) && percent > 0 && percent <= 100;
}

export function normalizeAutoCompactPercent(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  if (!isValidAutoCompactPercent(trimmed)) return trimmed;
  const numeric = trimmed.replace(/%$/, "").trim();
  return `${numeric}%`;
}
