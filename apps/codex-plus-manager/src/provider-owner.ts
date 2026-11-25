// 共享供应商的归属（纯数据，不碰 React 和 i18n，文案由 ProviderOwner.tsx 翻译）：
// Codex 的直连供应商默认以引用的方式在共享供应商里（owner = codex），Claude 页加的、从 Claude 那几个来源导入的
// 是 claude，本地服务页加的没有归属。两边的供应商页都有「显示其他 Agent 的供应商」，默认勾上，按页记在 localStorage。
import type { GatewayPoolEntry } from "./gateway-model.ts";

export type ProviderPage = "codex" | "claude";

/** 每页一个记忆键：Codex 页和 Claude 页各记各的。 */
export function showOtherAgentsKey(page: ProviderPage): string {
  return `codex-plus-plus.show-other-agent-providers.${page}`;
}

/** 读记下的选择：没记过（或存不了）时是勾上。 */
export function readShowOtherAgents(page: ProviderPage, storage: Pick<Storage, "getItem"> | null): boolean {
  try {
    return storage?.getItem(showOtherAgentsKey(page)) !== "0";
  } catch {
    return true;
  }
}

/** 这一项是不是别的 Agent 的：返回它归哪个 Agent，自己的或没有归属的返回 null。 */
export function otherAgentOwner(entry: Pick<GatewayPoolEntry, "owner">, page: ProviderPage): ProviderPage | null {
  const owner = entry.owner;
  return owner && owner !== page ? owner : null;
}

/** 按「显示其他 Agent 的供应商」筛：不勾时去掉别的 Agent 的。 */
export function entriesForPage<T extends Pick<GatewayPoolEntry, "owner">>(entries: T[], page: ProviderPage, showOthers: boolean): T[] {
  return showOthers ? entries : entries.filter((entry) => !otherAgentOwner(entry, page));
}

/**
 * 行上的来源标记是哪一种：别的 Agent 的是 `codex` / `claude`（「来自 Codex」「来自 Claude」）；
 * Codex 页「共享」分页里 Codex 自己的直连供应商是 `direct`（「来自直连」：就是旁边直连分页里的同一个），
 * 还没转成引用的旧版复制品是 `direct-copy`（同样写「来自直连」，悬停说明两边各改各的）。其余不标。
 */
export type OwnerBadgeKind = "codex" | "claude" | "direct" | "direct-copy";

export function ownerBadgeKind(entry: Pick<GatewayPoolEntry, "owner" | "linkedCodexRelay">, page: ProviderPage): OwnerBadgeKind | null {
  const other = otherAgentOwner(entry, page);
  if (other) return other;
  if (page === "codex" && entry.owner === "codex") return entry.linkedCodexRelay ? "direct" : "direct-copy";
  return null;
}
