/** 网关供应商池的自动压缩阈值（#3100）：窗口的百分比，按模型 + 默认值。
 * 只有真的设了才传给后端；空串表示没设，交给 Codex 默认压缩。 */
import type { GatewayPoolEntry } from "./gateway-model";

/** 百分比输入：1~100 之间（含两位小数）才算合法；空串表示没设，返回 null；写错返回 NaN。 */
export function parseCompactPercent(text: string): number | null {
  const raw = text.trim();
  if (!raw) return null;
  const trimmed = raw.replace(/%$/, "").trim();
  if (!trimmed) return Number.NaN;
  if (!/^\d+(\.\d{1,2})?$/.test(trimmed)) return Number.NaN;
  const value = Number(trimmed);
  return value >= 1 && value <= 100 ? value : Number.NaN;
}

/** 弹窗只列了部分模型（编辑弹窗里按编辑中的列表打开）时，没列出的模型原来单独设的阈值照旧带上：
 * 保存是整表替换，不能把它们丢掉。 */
export function withUnlistedAutoCompact(
  entry: GatewayPoolEntry,
  shown: string[],
  autoCompact: { models: Record<string, number>; fallback: number | null },
): { models: Record<string, number>; fallback: number | null } {
  const kept = Object.entries(entry.autoCompact?.models ?? {}).filter(
    ([model]) => !shown.some((item) => item.toLowerCase() === model.toLowerCase()),
  );
  return { ...autoCompact, models: { ...Object.fromEntries(kept), ...autoCompact.models } };
}
