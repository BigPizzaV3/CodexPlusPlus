import assert from "node:assert/strict";
import { test } from "node:test";
import {
  formatRelayAmount,
  formatRelayMultiplier,
  rememberRelayBillingNegative,
  relayAppliedPeak,
  relayBalanceBadge,
  relayPlatformLabel,
  relayBillingGatewayCacheKey,
  relayBillingNegativeCached,
  relayBillingCacheKey,
  relayBillingProbeEligible,
  mergeRelayBillingResult,
  newApiAccessHint,
  normalizeRelayBillingProbe,
  relayBillingCacheUsable,
  RELAY_BILLING_FAILURE_TTL_MS,
  RELAY_BILLING_NEGATIVE_TTL_MS,
  RELAY_BILLING_POSITIVE_TTL_MS,
  type RelayBillingProbe,
  type RelayRateInfo,
  type RelayBillingProfile,
  type RelayUsageInfo,
} from "./relay-billing.ts";

const profile = (patch: Partial<RelayBillingProfile> = {}): RelayBillingProfile => ({
  id: "relay-1", baseUrl: "https://relay.example.com/v1", upstreamBaseUrl: "", apiKey: "sk-a",
  relayMode: "mixedApi", officialMixApiKey: false, noAuth: false, ...patch,
});

class MemoryStorage {
  data = new Map<string, string>();
  getItem(key: string) { return this.data.get(key) ?? null; }
  setItem(key: string, value: string) { this.data.set(key, value); }
}

test("only API-key relays with a URL and key are probed", () => {
  assert.equal(relayBillingProbeEligible(profile()), true);
  assert.equal(relayBillingProbeEligible(profile({ relayMode: "pureApi" })), true);
  assert.equal(relayBillingProbeEligible(profile({ relayMode: "official" })), false);
  assert.equal(relayBillingProbeEligible(profile({ relayMode: "official", officialMixApiKey: true })), true);
  assert.equal(relayBillingProbeEligible(profile({ relayMode: "aggregate" })), false);
  assert.equal(relayBillingProbeEligible(profile({ noAuth: true })), false);
  assert.equal(relayBillingProbeEligible(profile({ apiKey: " " })), false);
  assert.equal(relayBillingProbeEligible(profile({ baseUrl: "" })), false);
  assert.equal(relayBillingProbeEligible(profile({ baseUrl: "", upstreamBaseUrl: "https://up.example.com" })), true);
});

test("cache key follows the active key and upstream URL without leaking the key", () => {
  const base = relayBillingCacheKey(profile());
  assert.ok(!base.includes("sk-a"));
  assert.equal(relayBillingCacheKey(profile({ baseUrl: "https://relay.example.com/v1/" })), base);
  assert.notEqual(relayBillingCacheKey(profile({ apiKey: "sk-b" })), base);
  assert.notEqual(relayBillingCacheKey(profile({ upstreamBaseUrl: "https://other.example.com" })), base);
  const multiKey = profile({ apiKeys: [{ id: "a", apiKey: "sk-a" }, { id: "b", apiKey: "sk-b" }], activeApiKeyId: "b" });
  assert.equal(relayBillingCacheKey(multiKey), relayBillingCacheKey(profile({ apiKey: "sk-b" })));
});

test("negative probes expire and can be cleared", () => {
  const storage = new MemoryStorage();
  rememberRelayBillingNegative(storage, "k1", true, 1000);
  rememberRelayBillingNegative(storage, "k2", true, 1000);
  assert.equal(relayBillingNegativeCached(storage, "k1", 2000), true);
  assert.equal(relayBillingNegativeCached(storage, "k1", 1000 + RELAY_BILLING_NEGATIVE_TTL_MS), false);
  rememberRelayBillingNegative(storage, "k1", false, 2000);
  assert.equal(relayBillingNegativeCached(storage, "k1", 2000), false);
  assert.equal(relayBillingNegativeCached(storage, "k2", 2000), true);
  // 过期条目在下次写入时被丢掉
  rememberRelayBillingNegative(storage, "k3", true, 1000 + RELAY_BILLING_NEGATIVE_TTL_MS);
  assert.deepEqual(Object.keys(JSON.parse(storage.getItem("codex-plus-relay-billing-negative-probes")!)), ["k3"]);
  assert.equal(relayBillingNegativeCached(null, "k3"), false);
});

test("formats multipliers, amounts and peak hints", () => {
  assert.equal(formatRelayMultiplier(0.9), "0.9x");
  assert.equal(formatRelayMultiplier(1.234567), "1.2346x");
  assert.equal(formatRelayAmount(12.345, "USD"), "$12.35");
  assert.equal(formatRelayAmount(97505834.07, "USD"), "$97,505,834.07");
  assert.equal(formatRelayAmount(3, "CNY"), "¥3.00");
  assert.equal(formatRelayAmount(3, "credits"), "3.00 credits");
  const billing = { appliedPeakMultiplier: 1.5 } as RelayRateInfo;
  assert.equal(relayAppliedPeak(billing), 1.5);
  assert.equal(relayAppliedPeak({ appliedPeakMultiplier: 1 } as RelayRateInfo), null);
  assert.equal(relayAppliedPeak({ appliedPeakMultiplier: null } as RelayRateInfo), null);
  assert.equal(relayAppliedPeak(null), null);
});

test("gateway pool entries are probed by id unless they are accounts or official vendors", () => {
  const entry = { id: "pool-1", baseUrl: "https://relay.example.com", keyCount: 1, official: false };
  const key = relayBillingGatewayCacheKey(entry);
  assert.ok(key.startsWith("gateway:pool-1|"));
  assert.equal(relayBillingGatewayCacheKey({ ...entry, baseUrl: "https://relay.example.com/" }), key);
  assert.notEqual(relayBillingGatewayCacheKey({ ...entry, keyCount: 2 }), key);
  assert.notEqual(relayBillingGatewayCacheKey({ ...entry, proxy: "http://127.0.0.1:7890" }), key);
  assert.equal(relayBillingGatewayCacheKey({ ...entry, official: true }), "");
  assert.equal(relayBillingGatewayCacheKey({ ...entry, account: "chatgpt" }), "");
  assert.equal(relayBillingGatewayCacheKey({ ...entry, keyCount: 0 }), "");
  assert.equal(relayBillingGatewayCacheKey({ ...entry, baseUrl: " " }), "");
  // 有余额接口的厂商（DeepSeek 等）余额另走「用量」页那份，不当中转站识别（#2793）。
  assert.equal(relayBillingGatewayCacheKey({ ...entry, balanceApi: true }), "");
  // 配上 / 换掉 new-api 访问令牌后重新识别，才能带上账户余额。
  const withAccess = relayBillingGatewayCacheKey({ ...entry, newapiAccess: { masked: "nap_ab…wxyz", userId: "42" } });
  assert.notEqual(withAccess, key);
  assert.notEqual(relayBillingGatewayCacheKey({ ...entry, newapiAccess: { masked: "nap_ab…wxyz", userId: "7" } }), withAccess);
  assert.equal(relayBillingGatewayCacheKey({ ...entry, newapiAccess: null }), key);
});

test("new-api 卡片：Key 不限额度时提示配置访问令牌，配了但失败给出原因（#2793）", () => {
  const usage = (kind: string): RelayUsageInfo => ({
    endpoint: "", kind, isValid: true, planName: null, remaining: kind === "wallet" ? 8.23 : null,
    quotaLimit: null, quotaUsed: null, unit: "USD", rateLimits: [],
  });
  const probe = (kind: string, newapiAccess: unknown, platform = "newapi") => normalizeRelayBillingProbe({
    platform, rate: null, usage: usage(kind), detail: "", outcome: "recognized",
    newapiAccess: newapiAccess as RelayBillingProbe["newapiAccess"],
  });
  assert.deepEqual(newApiAccessHint(probe("unlimited", { configured: false, error: null })), { text: "", failed: false });
  // 旧后端没有这个字段时也按「没配置」提示。
  assert.deepEqual(newApiAccessHint(probe("unlimited", undefined)), { text: "", failed: false });
  assert.deepEqual(
    newApiAccessHint(probe("unlimited", { configured: true, error: "HTTP 401：无权进行此操作" })),
    { text: "HTTP 401：无权进行此操作", failed: true },
  );
  // 查到了余额、Key 自己有额度、或者不是 new-api：不提示。
  assert.equal(newApiAccessHint(probe("wallet", { configured: true, error: null })), null);
  assert.equal(newApiAccessHint(probe("keyQuota", { configured: false, error: null })), null);
  assert.equal(newApiAccessHint(probe("unlimited", null, "sub2api")), null);
  assert.equal(probe("unlimited", { configured: true, error: null }, "sub2api").newapiAccess, null);
});

test("pool badge shows remaining amount, quota ratio and exhausted tone", () => {
  const usage = (patch: Partial<RelayUsageInfo>): RelayUsageInfo => ({
    endpoint: "", kind: "wallet", isValid: true, planName: null, remaining: 12, quotaLimit: null, quotaUsed: null,
    unit: "USD", rateLimits: [], ...patch,
  });
  assert.deepEqual(relayBalanceBadge(usage({})), { amount: "$12.00", ratio: null, tone: "ok" });
  assert.deepEqual(relayBalanceBadge(usage({ kind: "keyQuota", remaining: 10, quotaLimit: 50 })), { amount: "$10.00", ratio: 0.2, tone: "mid" });
  assert.equal(relayBalanceBadge(usage({ remaining: 0 }))?.tone, "low");
  assert.equal(relayBalanceBadge(usage({ rateLimits: [{ window: "5h", limit: 10, used: 10, remaining: 0 }] }))?.tone, "low");
  assert.equal(relayBalanceBadge(usage({ remaining: null })), null);
});

test("platform labels and unlimited keys", () => {
  assert.equal(relayPlatformLabel("sub2api"), "sub2api");
  assert.equal(relayPlatformLabel("newapi"), "new-api");
  const unlimited: RelayUsageInfo = {
    endpoint: "", kind: "unlimited", isValid: true, planName: null, remaining: null, quotaLimit: null, quotaUsed: null,
    unit: "USD", rateLimits: [],
  };
  assert.equal(relayBalanceBadge(unlimited), null);
});

const recognized = (patch: Partial<RelayBillingProbe> = {}): RelayBillingProbe => ({
  platform: "sub2api", rate: null, usage: null, detail: "", outcome: "recognized", ...patch,
});

test("probe outcome distinguishes not-a-relay from temporary failures", () => {
  assert.equal(normalizeRelayBillingProbe({ platform: "newapi" }).outcome, "recognized");
  assert.equal(normalizeRelayBillingProbe({ platform: null, outcome: "unavailable", detail: "HTTP 503" }).outcome, "unavailable");
  assert.equal(normalizeRelayBillingProbe({ platform: null, outcome: "notRelay" }).outcome, "notRelay");
  // 老后端没有 outcome：按 platform 推断。
  assert.equal(normalizeRelayBillingProbe({ platform: null }).outcome, "notRelay");
  assert.equal(normalizeRelayBillingProbe({ platform: "other", outcome: "recognized" }).platform, null);
});

test("only a definite not-a-relay answer writes the long negative cache", () => {
  const now = 1_000_000;
  const positive = { probe: recognized(), at: now - 1000, failure: null };

  const ok = mergeRelayBillingResult(undefined, recognized(), now);
  assert.equal(ok.negative, "clear");
  assert.deepEqual(ok.entry, { probe: recognized(), at: now, failure: null });

  const notRelay = mergeRelayBillingResult(positive, normalizeRelayBillingProbe({ platform: null, outcome: "notRelay" }), now);
  assert.equal(notRelay.negative, "remember");
  assert.equal(notRelay.entry, null);

  // 请求失败：保留上次成功的结果并标注失败，不碰长期负缓存。
  const failed = mergeRelayBillingResult(positive, normalizeRelayBillingProbe({ platform: null, outcome: "unavailable", detail: "超时" }), now);
  assert.equal(failed.negative, "keep");
  assert.deepEqual(failed.entry, { probe: positive.probe, at: positive.at, failure: { at: now, detail: "超时" } });

  // 从没识别成功过时请求失败：只记失败，没有旧值。
  const firstFailure = mergeRelayBillingResult(undefined, normalizeRelayBillingProbe({ platform: null, outcome: "unavailable" }), now);
  assert.equal(firstFailure.negative, "keep");
  assert.equal(firstFailure.entry?.probe, null);
});

test("failed probes are retried after a few minutes, recognized ones after the positive TTL", () => {
  const now = 10_000_000;
  assert.equal(relayBillingCacheUsable(undefined, now), false);
  assert.equal(relayBillingCacheUsable({ probe: recognized(), at: now - 1000, failure: null }, now), true);
  assert.equal(relayBillingCacheUsable({ probe: recognized(), at: now - RELAY_BILLING_POSITIVE_TTL_MS, failure: null }, now), false);
  const failure = (at: number) => ({ probe: recognized(), at: 0, failure: { at, detail: "" } });
  assert.equal(relayBillingCacheUsable(failure(now - 1000), now), true);
  assert.equal(relayBillingCacheUsable(failure(now - RELAY_BILLING_FAILURE_TTL_MS), now), false);
  assert.ok(RELAY_BILLING_FAILURE_TTL_MS < RELAY_BILLING_NEGATIVE_TTL_MS);
});
