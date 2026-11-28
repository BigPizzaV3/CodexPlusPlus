// Codex 统一供应商列表的纯函数：直连供应商和共享供应商合成一份，直连 / 聚合两种模式共用。
// 管理器「连接与供应商」页和 Codex 里的供应商页（经 42 分片打包，scripts/assemble-provider-editing.mjs）用同一份规则。
import type { RelayProfile } from "./app-types";
import type { GatewayPoolEntry } from "./gateway-model";

export type CodexUnifiedProvider = {
  key: string;
  name: string;
  baseUrl: string;
  profile: RelayProfile | null;
  poolEntry: GatewayPoolEntry | null;
};

/** 合并 Codex 直连供应商和共享供应商为同一份列表（按 linkedCodexRelay 去重，保持原顺序）。
 *  当列表里已经有 ChatGPT 订阅账号时，纯空壳的「OpenAI Official」（非混合 API、且当前不是以外部登录在使用）
 *  不再重复占一行，由具体的 ChatGPT 账号同时承接聚合与直连。 */
export function buildCodexUnifiedProviders(
  profiles: RelayProfile[],
  entries: GatewayPoolEntry[],
  options?: {
    activeOfficialExternalId?: string | null;
  },
): CodexUnifiedProvider[] {
  const byRelayId = new Map<string, GatewayPoolEntry>();
  const hasChatgptAccount = entries.some((entry) => entry.account === "chatgpt");
  for (const entry of entries) {
    const relayId = entry.linkedCodexRelay ?? (entry.source?.startsWith("codex:") ? entry.source.slice(6) : null);
    if (relayId && !byRelayId.has(relayId)) byRelayId.set(relayId, entry);
  }
  const matchedPoolIds = new Set<string>();
  const result: CodexUnifiedProvider[] = [];
  for (const profile of profiles) {
    const isPureOfficial = profile.relayMode === "official" && !profile.officialMixApiKey;
    if (isPureOfficial && hasChatgptAccount && options?.activeOfficialExternalId !== profile.id) {
      continue;
    }
    const poolEntry = byRelayId.get(profile.id) ?? null;
    if (poolEntry) matchedPoolIds.add(poolEntry.id);
    result.push({
      key: `relay:${profile.id}`,
      name: profile.name || poolEntry?.name || profile.id,
      baseUrl: profile.baseUrl || poolEntry?.baseUrl || "",
      profile,
      poolEntry,
    });
  }
  for (const entry of entries) {
    if (matchedPoolIds.has(entry.id)) continue;
    result.push({
      key: `pool:${entry.id}`,
      name: entry.name,
      baseUrl: entry.baseUrl,
      profile: null,
      poolEntry: entry,
    });
  }
  return result;
}

/** 这一项能否在聚合模式里启用（订阅账号，或有有效地址的共享供应商）。 */
export function codexItemSupportsAggregate(item: CodexUnifiedProvider): boolean {
  const entry = item.poolEntry;
  if (!entry) return false;
  if (entry.account) return true;
  if (entry.linkedCodexRelay && !entry.baseUrl.trim()) return false;
  return true;
}

/** 这一项能否在直连模式里选用（Codex 直连供应商，或可直接写入 ~/.codex/auth.json 的 ChatGPT 订阅账号）。 */
export function codexItemSupportsDirect(item: CodexUnifiedProvider): boolean {
  return item.profile !== null || item.poolEntry?.account === "chatgpt";
}
