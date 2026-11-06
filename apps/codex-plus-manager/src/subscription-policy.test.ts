import assert from "node:assert/strict";
import test from "node:test";
import { defaultSubscriptionPolicy, normalizeSubscriptionPolicy, subscriptionPercentLimit, withSubscriptionWindowLimit } from "./subscription-policy.ts";
import { providerDraftFromEntry, providerInput, quotaWindowFresh, subscriptionBlockReason, type GatewayPoolEntry, type ProviderQuota } from "./gateway-model.ts";

test("保存账号和空策略不需要额外授权，未知用量默认允许请求", () => {
  const policy = defaultSubscriptionPolicy();
  assert.equal("permitted" in policy, false);
  assert.equal(policy.unknownUsage, "allow");
  assert.equal(policy.onLimit, "stop");
  const entry: GatewayPoolEntry = { id: "account-a", name: "Account A", baseUrl: "https://example.test", protocol: "responses", official: true, models: ["model-a"], keyCount: 1, account: "chatgpt" };
  assert.equal("subscriptionPolicy" in providerInput(providerDraftFromEntry(entry)), false);
});

test("后端省略空窗口表时补齐编辑值，丢弃旧授权字段并保留额度策略", () => {
  const wire = { onLimit: "apiFallback" as const, unknownUsage: "allow" as const, maxAgeSeconds: 120 };
  assert.deepEqual(normalizeSubscriptionPolicy(wire), {
    ...wire, defaultUsedPercentLimit: null, windowUsedPercentLimits: {},
  });
  for (const value of [undefined, null, {}, { windowUsedPercentLimits: null }]) {
    assert.deepEqual(normalizeSubscriptionPolicy(value), defaultSubscriptionPolicy());
  }
  for (const permitted of [false, true]) {
    const legacy = { ...wire, permitted };
    assert.deepEqual(normalizeSubscriptionPolicy(legacy), normalizeSubscriptionPolicy(wire));
    assert.equal("permitted" in normalizeSubscriptionPolicy(legacy), false);
  }
  const entry: GatewayPoolEntry = {
    id: "account-a", name: "Account A", baseUrl: "https://example.test", protocol: "responses",
    official: true, models: ["model-a"], keyCount: 0, account: "chatgpt", subscriptionPolicy: wire,
  };
  const input = providerInput(providerDraftFromEntry(entry));
  assert.deepEqual(input.subscriptionPolicy, normalizeSubscriptionPolicy(wire));
  assert.equal("windowUsedPercentLimits" in wire, false, "补齐草稿不能修改原始后端快照");
});

test("阈值按官方已用比例，空或100不设置上限，零表示不许继续消耗", () => {
  assert.equal(subscriptionPercentLimit(""), null);
  assert.equal(subscriptionPercentLimit("100"), null);
  assert.equal(subscriptionPercentLimit("0"), 0);
  assert.equal(subscriptionPercentLimit("80"), 80);
  for (const value of ["-1", "101", "80.5", "NaN", "Infinity"]) assert.equal(subscriptionPercentLimit(value), undefined);
});

test("无效或小数窗口阈值不会清空已有整数上限", () => {
  const policy = withSubscriptionWindowLimit(defaultSubscriptionPolicy(), "300", "80");
  for (const value of ["80.5", "-1", "101", "NaN"]) {
    assert.equal(withSubscriptionWindowLimit(policy, "300", value), policy);
  }
  assert.deepEqual(policy.windowUsedPercentLimits, { "300": 80 });
});

test("各官方窗口独立保存，清除一个窗口保留其余策略", () => {
  let policy = withSubscriptionWindowLimit(defaultSubscriptionPolicy(), "300", "80");
  policy = withSubscriptionWindowLimit(policy, "10080", "90");
  policy = { ...policy, onLimit: "nextPermitted", unknownUsage: "allow" };
  const cleared = withSubscriptionWindowLimit(policy, "300", "100");
  assert.deepEqual(cleared.windowUsedPercentLimits, { "10080": 90 });
  assert.deepEqual(policy.windowUsedPercentLimits, { "300": 80, "10080": 90 });
  assert.equal("permitted" in cleared, false);
  assert.equal(cleared.onLimit, "nextPermitted");
});

test("供应商编辑往返保留额度策略和未知窗口阈值", () => {
  const subscriptionPolicy = { ...defaultSubscriptionPolicy(), onLimit: "apiFallback" as const, windowUsedPercentLimits: { "300": 81, "43200": 91 }, maxAgeSeconds: 120 };
  const entry: GatewayPoolEntry = { id: "account-a", name: "Account A", baseUrl: "https://example.test", protocol: "responses", official: true, models: ["model-a"], keyCount: 1, account: "chatgpt", subscriptionPolicy };
  const input = providerInput({ ...providerDraftFromEntry(entry), name: "Alias A" });
  assert.deepEqual(input.subscriptionPolicy, subscriptionPolicy);
  assert.equal("permitted" in input.subscriptionPolicy!, false);
  assert.equal(input.keys.length, 0);
});

function account(quota?: ProviderQuota): GatewayPoolEntry {
  return {
    id: "subscription", name: "Plus", account: "chatgpt", baseUrl: "", protocol: "responses",
    official: true, keyCount: 0, models: ["model"], quota,
    subscriptionPolicy: defaultSubscriptionPolicy(),
  };
}

test("按实际窗口阻止5h/7d耗尽和本地阈值，不根据套餐限制无5h的Plus", () => {
  const now = 1000;
  const quota: ProviderQuota = { updatedAtMs: now, windowsComplete: true, officialAllowed: true,
    windows: [{ windowMinutes: 10080, usedPercent: 20, resetsAtMs: 1000000 }] };
  const entry = account(quota);
  entry.subscriptionPolicy!.windowUsedPercentLimits = { "300": 0, "10080": 80 };
  for (const plan of ["ChatGPT Plus", "ChatGPT Pro", "Team"]) {
    entry.plan = plan;
    assert.equal(subscriptionBlockReason(entry, now), null);
  }
  quota.windows.push({ windowMinutes: 300, usedPercent: 100, resetsAtMs: 2000 });
  assert.match(subscriptionBlockReason(entry, now)!, /5h/);
  quota.windows.pop();
  quota.windows[0].usedPercent = 80;
  assert.match(subscriptionBlockReason(entry, now)!, /7d.*80/);
});

test("部分快照缺窗口、过期、未来或已重置读数均未知；新鲜耗尽不能由允许未知绕过", () => {
  const quota: ProviderQuota = { updatedAtMs: 1000,
    windows: [{ windowMinutes: 10080, usedPercent: 20, resetsAtMs: 1000000 }] };
  const entry = account(quota);
  entry.subscriptionPolicy!.unknownUsage = "block";
  entry.subscriptionPolicy!.windowUsedPercentLimits = { "300": 80 };
  assert.match(subscriptionBlockReason(entry, 1000)!, /刷新/);
  quota.windowsComplete = true;
  assert.equal(subscriptionBlockReason(entry, 1000), null);
  for (const now of [999, 700000, 1000001]) assert.match(subscriptionBlockReason(entry, now)!, /刷新/);
  assert.equal(quotaWindowFresh(quota, quota.windows[0], 1000001), false);
  quota.windows.push({ windowMinutes: 300, usedPercent: 100, resetsAtMs: 1000000, observedAtMs: 700000 });
  entry.subscriptionPolicy!.unknownUsage = "allow";
  assert.match(subscriptionBlockReason(entry, 700000)!, /5h/);
});

test("旧账号没有策略或旧授权为false时仍可请求，未知用量仅在显式block时阻止", () => {
  const entry = account();
  entry.subscriptionPolicy = undefined;
  assert.equal(subscriptionBlockReason(entry, 1000), null);
  entry.quota = { updatedAtMs: 1000, officialAllowed: false,
    windows: [{ windowMinutes: 300, usedPercent: 100, resetsAtMs: 1000000 }] };
  entry.subscriptionBlockReason = "stale policy response";
  assert.equal(subscriptionBlockReason(entry, 1000), null);
  entry.quota = undefined;
  entry.subscriptionBlockReason = undefined;
  for (const permitted of [false, true]) {
    const legacy = { ...defaultSubscriptionPolicy(), permitted };
    entry.subscriptionPolicy = legacy;
    assert.equal(subscriptionBlockReason(entry, 1000), null);
  }
  entry.subscriptionPolicy = { ...defaultSubscriptionPolicy(), unknownUsage: "block" };
  assert.match(subscriptionBlockReason(entry, 1000)!, /刷新/);
});

test("官方受限和积分分离，空窗口需官方明确允许，后端别名准入仍生效", () => {
  const quota: ProviderQuota = { windows: [], updatedAtMs: 1000, windowsComplete: true, officialAllowed: false, creditsAvailable: true };
  const entry = account(quota);
  entry.subscriptionPolicy!.unknownUsage = "allow";
  assert.match(subscriptionBlockReason(entry, 1000)!, /受限/);
  quota.officialAllowed = true;
  assert.equal(subscriptionBlockReason(entry, 1000), null);
  quota.officialAllowed = undefined;
  entry.subscriptionPolicy!.unknownUsage = "block";
  assert.match(subscriptionBlockReason(entry, 1000)!, /刷新/);
  quota.officialAllowed = true;
  entry.subscriptionBlockReason = "同账号另一供应商上限已达";
  assert.equal(subscriptionBlockReason(entry, 1000), entry.subscriptionBlockReason);
});
