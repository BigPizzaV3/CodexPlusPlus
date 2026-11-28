import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { buildCodexUnifiedProviders, codexItemSupportsAggregate, codexItemSupportsDirect } from "./codex-unified-providers.ts";

/**
 * Codex 统一供应商列表的规则：管理器「连接与供应商」页和 Codex 里的供应商页用同一份
 * （后者经 42 分片打包，挂在 codexPlusProviderEditing.unified 下）。
 */
const profile = (id: string, extra: Record<string, unknown> = {}) => ({ id, name: id.toUpperCase(), relayMode: "pureApi", officialMixApiKey: false, baseUrl: `https://${id}.example/v1`, ...extra }) as any;
const entry = (id: string, extra: Record<string, unknown> = {}) => ({ id, name: id.toUpperCase(), baseUrl: `https://${id}.example/v1`, protocol: "responses", models: [], ...extra }) as any;

test("直连供应商在前，连着它的共享供应商并进同一行；其余共享供应商排在后面", () => {
  const items = buildCodexUnifiedProviders(
    [profile("a"), profile("b")],
    [entry("gw-b", { linkedCodexRelay: "b" }), entry("gw-c"), entry("gw-a", { source: "codex:a" })],
  );
  assert.deepEqual(items.map((item) => item.key), ["relay:a", "relay:b", "pool:gw-c"]);
  assert.equal(items[0].poolEntry?.id, "gw-a", "source 为 codex:<id> 也算连着");
  assert.equal(items[1].poolEntry?.id, "gw-b");
});

test("有 ChatGPT 账号时不再列纯官方空壳，除非正以池外的登录用着它", () => {
  const profiles = [profile("official", { relayMode: "official", baseUrl: "" }), profile("a")];
  const entries = [entry("chatgpt-1", { account: "chatgpt", baseUrl: "" })];
  assert.deepEqual(buildCodexUnifiedProviders(profiles, entries).map((item) => item.key), ["relay:a", "pool:chatgpt-1"]);
  assert.deepEqual(
    buildCodexUnifiedProviders(profiles, entries, { activeOfficialExternalId: "official" }).map((item) => item.key),
    ["relay:official", "relay:a", "pool:chatgpt-1"],
  );
  assert.deepEqual(buildCodexUnifiedProviders(profiles, []).map((item) => item.key), ["relay:official", "relay:a"], "没有账号时照常列");
});

test("能不能在聚合 / 直连里用", () => {
  const [direct, linked, linkedNoUrl, pool, chatgpt] = [
    { key: "relay:a", name: "A", baseUrl: "", profile: profile("a"), poolEntry: null },
    { key: "relay:b", name: "B", baseUrl: "", profile: profile("b"), poolEntry: entry("gw-b", { linkedCodexRelay: "b" }) },
    { key: "relay:c", name: "C", baseUrl: "", profile: profile("c"), poolEntry: entry("gw-c", { linkedCodexRelay: "c", baseUrl: " " }) },
    { key: "pool:d", name: "D", baseUrl: "", profile: null, poolEntry: entry("d") },
    { key: "pool:e", name: "E", baseUrl: "", profile: null, poolEntry: entry("e", { account: "chatgpt", baseUrl: "" }) },
  ];
  assert.deepEqual([direct, linked, linkedNoUrl, pool, chatgpt].map(codexItemSupportsAggregate), [false, true, false, true, true]);
  assert.deepEqual([direct, linked, linkedNoUrl, pool, chatgpt].map(codexItemSupportsDirect), [true, true, true, false, true]);
});

test("Codex 里的供应商页用的是同一份规则（打包进 42 分片）", async () => {
  const script = await readFile(new URL("../../../scripts/assemble-provider-editing.mjs", import.meta.url), "utf8");
  assert.match(script, /export \* as unified from \\"\.\/codex-unified-providers\.ts\\"/);
  const page = await readFile(new URL("../../../assets/inject/renderer-inject/43-provider-page.js", import.meta.url), "utf8");
  assert.match(page, /codexPlusProviderEditingApi\.unified\.buildCodexUnifiedProviders\(/);
  assert.match(page, /codexPlusProviderEditingApi\.unified\.codexItemSupportsAggregate/);
  assert.match(page, /codexPlusProviderEditingApi\.unified\.codexItemSupportsDirect/);
});
