// 有余额接口的厂商（DeepSeek、Kimi / Moonshot、OpenRouter、SiliconFlow）的余额：「用量」页和供应商池卡片
// 共用同一份查询结果（#2793）。后端 `load_gateway_balances` 一次查完池里所有这类供应商，
// 这里只做内存缓存和并发合并：同一时间只有一个请求在路上，几张卡片一起要也只查一次。

import type { ProviderBalance } from "./gateway-model";

/** 卡片自动刷新的间隔：余额会变，但没必要每次展开页面都查。 */
export const PROVIDER_BALANCE_TTL_MS = 5 * 60_000;

export type ProviderBalanceCache = {
  /** 最近一次查询结果；还没查过为 null。 */
  balances: ProviderBalance[] | null;
  /** 查询完成的时间。 */
  at: number;
};

type Loader = () => Promise<{ status?: string; balances?: ProviderBalance[] }>;

const cache: ProviderBalanceCache = { balances: null, at: 0 };
let inflight: Promise<ProviderBalance[]> | null = null;
const listeners = new Set<(balances: ProviderBalance[]) => void>();

export function cachedProviderBalances(): ProviderBalanceCache {
  return cache;
}

/** 不强制刷新时能否直接用缓存。 */
export function providerBalancesFresh(entry: ProviderBalanceCache, now: number, ttl = PROVIDER_BALANCE_TTL_MS): boolean {
  return entry.balances !== null && now - entry.at < ttl;
}

/** 某个供应商的余额；不在结果里（没有余额接口或已关闭）为 null。 */
export function providerBalanceFor(balances: ProviderBalance[] | null, id: string): ProviderBalance | null {
  return balances?.find((balance) => balance.providerId === id) ?? null;
}

/** 有结果时订阅更新（「用量」页刷新后卡片跟着变，反之亦然）；返回取消订阅。 */
export function subscribeProviderBalances(listener: (balances: ProviderBalance[]) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * 取余额。`maxAge` 为 null 时只要查过就用缓存（「用量」页的老行为：进页面查一次，点刷新才重查）；
 * 给了毫秒数就超时重查；`force` 一定重查。进行中的请求会被复用，不会重复发。
 */
export function loadProviderBalances(loader: Loader, options: { force?: boolean; maxAge?: number | null } = {}): Promise<ProviderBalance[]> {
  const { force = false, maxAge = PROVIDER_BALANCE_TTL_MS } = options;
  if (inflight) return inflight;
  if (!force && cache.balances !== null && (maxAge === null || providerBalancesFresh(cache, Date.now(), maxAge))) {
    return Promise.resolve(cache.balances);
  }
  inflight = loader()
    .then((result) => result.balances ?? [])
    // 查不到时保留上次的结果，不清空已显示的余额。
    .catch(() => cache.balances ?? [])
    .then((balances) => {
      cache.balances = balances;
      cache.at = Date.now();
      for (const listener of listeners) listener(balances);
      return balances;
    })
    .finally(() => {
      inflight = null;
    });
  return inflight;
}

/** 测试用：清掉缓存和进行中的请求。 */
export function resetProviderBalancesForTest() {
  cache.balances = null;
  cache.at = 0;
  inflight = null;
  listeners.clear();
}
