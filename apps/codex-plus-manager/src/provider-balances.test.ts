import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  PROVIDER_BALANCE_TTL_MS,
  cachedProviderBalances,
  loadProviderBalances,
  providerBalanceFor,
  providerBalancesFresh,
  resetProviderBalancesForTest,
  subscribeProviderBalances,
} from "./provider-balances.ts";

const deepseek = { providerId: "deepseek", provider: "DeepSeek", baseUrl: "https://api.deepseek.com", amount: 14.6, currency: "CNY", error: null };

test("厂商余额：几张卡片同时要只查一次，之后在有效期内用缓存（#2793）", async () => {
  resetProviderBalancesForTest();
  let calls = 0;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const loader = async () => {
    calls += 1;
    await gate;
    return { status: "ok", balances: [deepseek] };
  };
  const first = loadProviderBalances(loader);
  const second = loadProviderBalances(loader);
  release();
  assert.deepEqual(await first, [deepseek]);
  assert.deepEqual(await second, [deepseek]);
  assert.equal(calls, 1);

  await loadProviderBalances(loader);
  assert.equal(calls, 1, "有效期内不重查");
  await loadProviderBalances(loader, { force: true });
  assert.equal(calls, 2, "点刷新强制重查");
});

test("厂商余额：「用量」页只要查过就用缓存，查不到时保留上次的结果，并通知订阅者", async () => {
  resetProviderBalancesForTest();
  const seen: number[] = [];
  const unsubscribe = subscribeProviderBalances((balances) => seen.push(balances.length));
  await loadProviderBalances(async () => ({ balances: [deepseek] }));
  const cache = cachedProviderBalances();
  cache.at -= PROVIDER_BALANCE_TTL_MS * 10;
  let calls = 0;
  await loadProviderBalances(async () => { calls += 1; return { balances: [] }; }, { maxAge: null });
  assert.equal(calls, 0);
  const kept = await loadProviderBalances(async () => { throw new Error("offline"); }, { force: true });
  assert.deepEqual(kept, [deepseek]);
  assert.deepEqual(seen, [1, 1]);
  unsubscribe();
});

test("厂商余额：按供应商 id 取，缓存新鲜度按时间算", () => {
  assert.equal(providerBalanceFor([deepseek], "deepseek"), deepseek);
  assert.equal(providerBalanceFor([deepseek], "kimi"), null);
  assert.equal(providerBalanceFor(null, "deepseek"), null);
  assert.equal(providerBalancesFresh({ balances: [], at: 1000 }, 1000 + PROVIDER_BALANCE_TTL_MS - 1), true);
  assert.equal(providerBalancesFresh({ balances: [], at: 1000 }, 1000 + PROVIDER_BALANCE_TTL_MS), false);
  assert.equal(providerBalancesFresh({ balances: null, at: 1000 }, 1000), false);
});

const providers = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
const billingLine = readFileSync(new URL("./RelayBillingLine.tsx", import.meta.url), "utf8");
const usage = readFileSync(new URL("./GatewayUsage.tsx", import.meta.url), "utf8");
const extras = readFileSync(new URL("./ProviderExtrasDialog.tsx", import.meta.url), "utf8");
const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
const commands = readFileSync(new URL("../src-tauri/src/gateway_commands.rs", import.meta.url), "utf8");

test("DeepSeek 等厂商的卡片余额和「用量」页共用一份查询，样式与中转站余额一致", () => {
  assert.match(providers, /useProviderBalance\(entry\.id, enabled && Boolean\(entry\.balanceApi\)\)/);
  assert.match(providers, /<ProviderBalancePoolBadge balance=\{vendorBalance\.balance\}/);
  // 卡片和「用量」页都走共享缓存，不各自直接调后端。
  assert.match(billingLine, /loadProviderBalances\(\(\) => invoke<[^>]+>\("load_gateway_balances"\), \{ force \}\)/);
  assert.match(usage, /loadProviderBalances\(\r?\n?\s*\(\) => invoke<[^>]+>\("load_gateway_balances"\),\r?\n?\s*\{ force, maxAge: null \},/);
  assert.doesNotMatch(usage, /memo\.balances/);
  // 同一个徽章和刷新按钮。
  assert.match(billingLine, /export function ProviderBalancePoolBadge[\s\S]*?className="gw-billing-status"[\s\S]*?className="gw-quota-badge"[\s\S]*?RelayBillingRefreshButton/);
  assert.match(commands, /balance_api: codex_plus_core::gateway::balance::has_balance_api\(relay\),/);
});

test("new-api 访问令牌：详情里可设置，弹窗用密码框、只显示打码，命令已注册", () => {
  assert.match(providers, /billing\.probe\?\.platform === "newapi" \|\| Boolean\(entry\.newapiAccess\)/);
  assert.match(providers, /"save_gateway_provider_newapi_access", \{ id: entry\.id, token, userId, clear \}/);
  assert.match(providers, /<code className="gw-chip">\{entry\.newapiAccess\.masked\}<\/code>/);
  assert.match(extras, /export function ProviderNewApiAccessDialog[\s\S]*?className="gw-modal-backdrop gw-portal"[\s\S]*?type="password"/);
  assert.match(lib, /gateway_commands::save_gateway_provider_newapi_access,/);
  // 没配置时悬停提示说明配置后可显示余额。
  assert.match(billingLine, /配置访问令牌后可显示余额/);
});
