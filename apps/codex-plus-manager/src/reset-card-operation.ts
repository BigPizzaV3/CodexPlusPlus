import type { GatewayPoolEntry, ResetCredit } from "./gateway-model";

export function resetCardUsable(card: ResetCredit, now = Date.now()): boolean {
  return card.usable && card.status === "available"
    && (card.expiresAtMs == null || (Number.isFinite(card.expiresAtMs) && card.expiresAtMs > now));
}

/** 能用的重置卡张数；没有读数时返回 null（界面显示「尚未查询」）。 */
export function usableResetCardCount(entry?: GatewayPoolEntry | null, now = Date.now()): number | null {
  const cards = entry?.resetCredits;
  if (!cards) return null;
  if (!cards.credits?.length) return cards.applicableAvailableCount ?? cards.availableCount ?? null;
  return cards.credits.filter((card) => resetCardUsable(card, now)).length;
}

/** 正常可用且有明确到期时间的卡里，最早的到期时间。 */
export function soonestResetCardExpiry(entry?: GatewayPoolEntry | null, now = Date.now()): number | null {
  const cards = entry?.resetCredits?.credits?.filter((card) => resetCardUsable(card, now)) ?? [];
  const expiries = cards
    .map((card) => card.expiresAtMs)
    .filter((at): at is number => typeof at === "number" && at > 0);
  return expiries.length ? Math.min(...expiries) : null;
}

/** 持久保存未确认的操作标识；后端另按官方账号落盘，不能靠重建页面开启另一次消费。 */
type Storage = Pick<globalThis.Storage, "getItem" | "setItem" | "removeItem">;
const key = (providerId: string) => `codexpp:reset-card-operation:${encodeURIComponent(providerId)}`;

export function pendingResetCardOperation(storage: Storage, providerId: string): string | null {
  const id = storage.getItem(key(providerId));
  if (id !== null && (!id.trim() || id !== id.trim() || id.length > 128 || /[\x00-\x1f\x7f]/.test(id))) {
    throw new Error("重置操作记录无效，请先核实上次结果");
  }
  return id;
}

export function beginResetCardOperation(storage: Storage, providerId: string, createId: () => string): string {
  const pending = pendingResetCardOperation(storage, providerId);
  if (pending) return pending;
  const id = createId();
  storage.setItem(key(providerId), id);
  if (pendingResetCardOperation(storage, providerId) !== id) throw new Error("无法保存重置操作，请稍后重试");
  return id;
}

export function finishResetCardOperation(storage: Storage, providerId: string, id: string): void {
  if (pendingResetCardOperation(storage, providerId) === id) storage.removeItem(key(providerId));
}

/** 后端给的确定结果（已重置、此前已生效、无卡、无需重置、被拒绝）：这次操作结束，可以清掉标识。 */
const SETTLED_OUTCOMES = new Set(["reset", "alreadyRedeemed", "noCredit", "nothingToReset", "rejected"]);

export function resetOutcomeSettled(outcome: unknown): boolean {
  return typeof outcome === "string" && SETTLED_OUTCOMES.has(outcome);
}

/** 用户确认放弃上次未确认的操作：不论记录是否损坏都清掉，下次使用另起新标识。 */
export function abandonResetCardOperation(storage: Storage, providerId: string): void {
  storage.removeItem(key(providerId));
}
