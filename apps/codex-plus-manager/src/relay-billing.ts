// 供应商卡片上的倍率 / 余额：识别结果缓存与数字格式化。
// 识别本身在后端 `probe_relay_billing`：先试 sub2api（/v1/sub2api/billing、/v1/usage），
// 不是再试 new-api（/api/usage/token/、/api/log/token、/api/status）。

export type RelayBillingPlatform = "sub2api" | "newapi";

export type RelayRateInfo = {
  endpoint: string;
  multiplier: number;
  /** sub2api：高峰时段叠加的倍率。 */
  appliedPeakMultiplier: number | null;
  /** new-api：倍率取自哪个分组的消费记录。 */
  group: string | null;
};

export type RelayRateLimitWindow = { window: string; limit: number; used: number; remaining: number };

export type RelayUsageInfo = {
  endpoint: string;
  /** keyQuota：Key 自带额度/限速；subscription：订阅分组；wallet：账户余额；unlimited：Key 不限额度且查不到账户余额。 */
  kind: "keyQuota" | "subscription" | "wallet" | "unlimited" | string;
  isValid: boolean;
  planName: string | null;
  remaining: number | null;
  quotaLimit: number | null;
  quotaUsed: number | null;
  unit: string;
  rateLimits: RelayRateLimitWindow[];
};

/**
 * 后端给的识别结论：
 * - recognized：识别出中转站
 * - notRelay：明确不是（404、返回结构不符等），长时间不再识别
 * - unavailable：这次请求没成功（断网、代理没起、超时、429 / 5xx、401/403），不能当成「不是」
 */
export type RelayBillingOutcome = "recognized" | "notRelay" | "unavailable";

/** new-api 访问令牌查账户余额的情况（#2793）：没配置、查到了（error 为空）或失败原因（不含令牌）。 */
export type NewApiAccessStatus = { configured: boolean; error: string | null };

export type RelayBillingProbe = {
  /** 识别出的中转站；null 表示没识别出，原因看 outcome。 */
  platform: RelayBillingPlatform | null;
  rate: RelayRateInfo | null;
  usage: RelayUsageInfo | null;
  detail: string;
  outcome: RelayBillingOutcome;
  /** new-api 才有；旧后端没有这个字段。 */
  newapiAccess?: NewApiAccessStatus | null;
};

export function relayPlatformLabel(platform: RelayBillingPlatform): string {
  return platform === "newapi" ? "new-api" : "sub2api";
}

export type RelayBillingProfile = {
  id: string;
  baseUrl: string;
  upstreamBaseUrl: string;
  apiKey: string;
  apiKeys?: { id: string; apiKey: string }[];
  activeApiKeyId?: string;
  relayMode: string;
  officialMixApiKey: boolean;
  noAuth: boolean;
};

/** 识别出中转站时余额会变，缓存 5 分钟；明确不是的记 1 天，免得每次打开都去敲门；
 *  请求失败（暂时不可用）只在内存里记 5 分钟，过后重试，不写长期负缓存。 */
export const RELAY_BILLING_POSITIVE_TTL_MS = 5 * 60_000;
export const RELAY_BILLING_NEGATIVE_TTL_MS = 24 * 60 * 60_000;
export const RELAY_BILLING_FAILURE_TTL_MS = 5 * 60_000;
export const RELAY_BILLING_NEGATIVE_STORAGE_KEY = "codex-plus-relay-billing-negative-probes";

/** 把命令返回的原始结果整理成 RelayBillingProbe；老后端没有 outcome 时按 platform 推断。 */
export function normalizeRelayBillingProbe(result: {
  platform?: string | null;
  rate?: RelayRateInfo | null;
  usage?: RelayUsageInfo | null;
  detail?: string | null;
  outcome?: string | null;
  newapiAccess?: NewApiAccessStatus | null;
}): RelayBillingProbe {
  const platform = result.platform === "sub2api" || result.platform === "newapi" ? result.platform : null;
  return {
    platform,
    rate: result.rate ?? null,
    usage: result.usage ?? null,
    detail: result.detail ?? "",
    outcome: platform ? "recognized" : result.outcome === "unavailable" ? "unavailable" : "notRelay",
    newapiAccess: platform === "newapi" ? result.newapiAccess ?? null : null,
  };
}

/**
 * 供应商池里 new-api 卡片的余额提示（#2793）：Key 不限额度、拿不到余额时，说明配访问令牌能显示余额，
 * 或者配了但查询失败的原因。查到余额、或不是 new-api 时为 null。只给供应商池用（直连模式没有这个设置）。
 */
export function newApiAccessHint(probe: RelayBillingProbe): { text: string; failed: boolean } | null {
  if (probe.platform !== "newapi" || probe.usage?.kind !== "unlimited") return null;
  const access = probe.newapiAccess;
  if (access?.configured && access.error) return { text: access.error, failed: true };
  if (access?.configured) return null;
  return { text: "", failed: false };
}

/** 内存里的识别缓存。 */
export type RelayBillingCacheEntry = {
  /** 最近一次识别成功的结果；没有则为 null。 */
  probe: RelayBillingProbe | null;
  /** probe 的取得时间。 */
  at: number;
  /** 最近一次请求失败的时间与原因；之后识别成功或明确不是时清掉。 */
  failure: { at: number; detail: string } | null;
};

/**
 * 把一次识别结果并入缓存。`entry` 为 null 表示删掉内存缓存；`negative` 是对长期负缓存
 * （localStorage）要做的事：remember 记下、clear 清掉、keep 不动。
 * 请求失败时保留上次成功的结果（标注暂时获取失败），也不写长期负缓存。
 */
export function mergeRelayBillingResult(
  previous: RelayBillingCacheEntry | undefined,
  probe: RelayBillingProbe,
  now: number,
): { entry: RelayBillingCacheEntry | null; negative: "remember" | "clear" | "keep" } {
  if (probe.outcome === "recognized") return { entry: { probe, at: now, failure: null }, negative: "clear" };
  if (probe.outcome === "notRelay") return { entry: null, negative: "remember" };
  return {
    entry: { probe: previous?.probe ?? null, at: previous?.at ?? 0, failure: { at: now, detail: probe.detail } },
    negative: "keep",
  };
}

/** 不强制刷新时能否直接用内存缓存：刚失败过的 5 分钟内不重试，识别成功的 5 分钟内不重查。 */
export function relayBillingCacheUsable(entry: RelayBillingCacheEntry | undefined, now: number): boolean {
  if (!entry) return false;
  if (entry.failure) return now - entry.failure.at < RELAY_BILLING_FAILURE_TTL_MS;
  return entry.probe !== null && now - entry.at < RELAY_BILLING_POSITIVE_TTL_MS;
}

function activeApiKey(profile: RelayBillingProfile): string {
  const keys = profile.apiKeys ?? [];
  const selected = keys.find((entry) => entry.id === profile.activeApiKeyId) ?? keys[0];
  return (selected?.apiKey ?? profile.apiKey).trim();
}

function upstreamUrl(profile: RelayBillingProfile): string {
  return (profile.upstreamBaseUrl.trim() || profile.baseUrl.trim()).replace(/\/+$/, "");
}

/** 官方登录、聚合、免鉴权或缺地址 / Key 的供应商不去识别。 */
export function relayBillingProbeEligible(profile: RelayBillingProfile): boolean {
  if (profile.relayMode === "aggregate" || profile.noAuth) return false;
  if (profile.relayMode === "official" && !profile.officialMixApiKey) return false;
  return Boolean(upstreamUrl(profile) && activeApiKey(profile));
}

// FNV-1a：缓存键里只放 Key 的指纹，不落明文。
function fingerprint(text: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(36);
}

/** 地址或当前 Key 一变，缓存键随之变化，自动重新识别。 */
export function relayBillingCacheKey(profile: RelayBillingProfile): string {
  return `${profile.id}|${fingerprint(`${upstreamUrl(profile)}\n${activeApiKey(profile)}`)}`;
}

/** 网关供应商池：前端看不到 Key，按地址、Key 数、代理和 new-api 访问令牌变化重新识别。
 *  订阅登录账号、官方登录，以及有余额接口的厂商（DeepSeek、Kimi 等，余额另走「用量」页那份）不识别。 */
export function relayBillingGatewayCacheKey(entry: {
  id: string;
  baseUrl: string;
  keyCount: number;
  official?: boolean;
  proxy?: string;
  account?: string | null;
  balanceApi?: boolean;
  newapiAccess?: { masked: string; userId: string } | null;
}): string {
  if (entry.account || entry.official || entry.balanceApi || entry.keyCount <= 0 || !entry.baseUrl.trim()) return "";
  const url = entry.baseUrl.trim().replace(/\/+$/, "");
  const access = entry.newapiAccess ? `\n${entry.newapiAccess.masked}\n${entry.newapiAccess.userId}` : "";
  return `gateway:${entry.id}|${fingerprint(`${url}\n${entry.keyCount}\n${entry.proxy ?? ""}${access}`)}`;
}

type StorageLike = Pick<Storage, "getItem" | "setItem">;

function readNegatives(storage: StorageLike | null): Record<string, number> {
  try {
    const parsed = JSON.parse(storage?.getItem(RELAY_BILLING_NEGATIVE_STORAGE_KEY) ?? "{}");
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

export function relayBillingNegativeCached(storage: StorageLike | null, key: string, now = Date.now()): boolean {
  const checkedAt = readNegatives(storage)[key];
  return typeof checkedAt === "number" && now - checkedAt < RELAY_BILLING_NEGATIVE_TTL_MS;
}

/** 记下或清掉「识别不出」，顺手丢掉过期条目，避免越攒越多。 */
export function rememberRelayBillingNegative(storage: StorageLike | null, key: string, negative: boolean, now = Date.now()) {
  const next: Record<string, number> = {};
  for (const [entryKey, checkedAt] of Object.entries(readNegatives(storage))) {
    if (entryKey !== key && typeof checkedAt === "number" && now - checkedAt < RELAY_BILLING_NEGATIVE_TTL_MS) {
      next[entryKey] = checkedAt;
    }
  }
  if (negative) next[key] = now;
  try {
    storage?.setItem(RELAY_BILLING_NEGATIVE_STORAGE_KEY, JSON.stringify(next));
  } catch {
    // 存不了只影响下次是否重新识别。
  }
}

export function formatRelayMultiplier(value: number): string {
  return `${Number(value.toFixed(4))}x`;
}

export function formatRelayAmount(value: number, unit: string): string {
  const amount = value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  const normalized = unit.trim().toUpperCase();
  if (!normalized || normalized === "USD") return `$${amount}`;
  if (normalized === "CNY" || normalized === "RMB") return `¥${amount}`;
  return `${amount} ${unit.trim()}`;
}

/** 高峰时段实际叠加的倍率；不在高峰或倍率为 1 时不单独提示。 */
export function relayAppliedPeak(rate: RelayRateInfo | null): number | null {
  const peak = rate?.appliedPeakMultiplier;
  return typeof peak === "number" && Number.isFinite(peak) && peak !== 1 ? peak : null;
}

/** 供应商池右侧徽章：剩余金额，以及有上限时的剩余比例（画额度条）。 */
export function relayBalanceBadge(usage: RelayUsageInfo): { amount: string; ratio: number | null; tone: "ok" | "mid" | "low" } | null {
  if (usage.remaining === null) return null;
  const ratio = usage.kind === "keyQuota" && usage.quotaLimit && usage.quotaLimit > 0
    ? Math.min(1, Math.max(0, usage.remaining / usage.quotaLimit))
    : null;
  const exhausted = usage.remaining <= 0 || !usage.isValid || usage.rateLimits.some((window) => window.remaining <= 0);
  const tone = exhausted ? "low" : ratio !== null && ratio <= 0.1 ? "low" : ratio !== null && ratio <= 0.3 ? "mid" : "ok";
  return { amount: formatRelayAmount(usage.remaining, usage.unit), ratio, tone };
}
