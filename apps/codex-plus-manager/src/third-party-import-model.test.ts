/** 「从第三方导入」对话框的数据整理。 */
import assert from "node:assert/strict";
import test from "node:test";
import { MAGPIE_UPSTREAM_ID, defaultMagpieMode, defaultPicked, importPlan, initialSource, itemKey, thirdPartyItems, togglePoolPick, urlHost } from "./third-party-import-model.ts";

const ccs = {
  providers: [
    { sourceId: "a", name: "Alpha", baseUrl: "https://alpha.example.com/v1", apiKey: "sk-a", protocol: "responses" as const },
    { sourceId: "b", name: "Beta", baseUrl: "https://beta.example.com/v1", apiKey: "", protocol: "chatCompletions" as const },
  ],
  imported: ["a"],
};
const magpie = {
  providers: [
    { sourceId: "m1", name: "Relay", baseUrl: "https://relay.example.com/v1", protocol: "responses" as const, disabled: true, hasKey: true, imported: false },
    { sourceId: "m2", name: "Claude Relay", baseUrl: "https://claude.example.com", protocol: "anthropicMessages" as const, disabled: false, hasKey: true, imported: true },
  ],
};
/** 装了 Magpie：除了各家供应商，还能把 Magpie 本身当作一个上游。 */
const magpieWithGateway = { ...magpie, gateway: { sourceId: "@gateway", baseUrl: "http://127.0.0.1:3425/v1", imported: false } };

test("两个来源整理成同一种条目：已导入、已关闭、有没有 Key 都带上", () => {
  const items = thirdPartyItems(ccs, magpie);
  assert.deepEqual(items.map(itemKey), ["ccswitch:a", "ccswitch:b", "magpie:m1", "magpie:m2"]);
  assert.deepEqual(items.map((item) => item.imported), [true, false, false, true]);
  assert.deepEqual(items.map((item) => item.hasKey), [true, false, true, true]);
  assert.equal(items[2].disabled, true);
  // 旧版后端不带 imported：都当成没导入过。
  assert.equal(thirdPartyItems({ providers: ccs.providers }, null).every((item) => !item.imported), true);
  assert.deepEqual(thirdPartyItems(null, null), []);
});

test("默认勾上没导入的；导入时各来源只导勾着的、没导入过的", () => {
  const items = thirdPartyItems(ccs, magpie);
  const picked = defaultPicked(items);
  assert.deepEqual([...picked], ["ccswitch:b", "magpie:m1"]);
  assert.deepEqual(importPlan(items, picked, "copy"), { ccswitch: ["b"], magpie: ["m1"] });
  // 已导入的就算被塞进勾选集合也不会再导。
  assert.deepEqual(importPlan(items, new Set(["ccswitch:a", "magpie:m2", "magpie:m1"]), "copy"), { ccswitch: [], magpie: ["m1"] });
  assert.deepEqual(importPlan(items, new Set(), "copy"), { ccswitch: [], magpie: [] });
});

test("打开时停在有东西可导的来源上", () => {
  assert.equal(initialSource(thirdPartyItems(ccs, magpie)), "ccswitch");
  // cc-switch 都导过了，Magpie 还有：停在 Magpie。
  assert.equal(initialSource(thirdPartyItems({ providers: ccs.providers, imported: ["a", "b"] }, magpie)), "magpie");
  // 只装了 Magpie。
  assert.equal(initialSource(thirdPartyItems(null, magpie)), "magpie");
  // 都导过了：停在第一个有条目的；什么都没有就是 cc-switch。
  assert.equal(initialSource(thirdPartyItems(null, { providers: [magpie.providers[1]] })), "magpie");
  assert.equal(initialSource([]), "ccswitch");
});

test("Magpie 的两种导入方式二选一，默认当作上游：只导那一条；选复制才导勾着的供应商，不会两种一起", () => {
  const items = thirdPartyItems(ccs, magpieWithGateway);
  assert.deepEqual(items.map(itemKey), ["ccswitch:a", "ccswitch:b", "magpie:@gateway", "magpie:m1", "magpie:m2"]);
  const upstream = items[2];
  assert.equal(upstream.upstream, true);
  assert.equal(upstream.id, MAGPIE_UPSTREAM_ID);
  assert.equal(upstream.name, "Magpie");
  assert.equal(upstream.baseUrl, "http://127.0.0.1:3425/v1");
  const picked = defaultPicked(items);
  assert.equal(picked.has("magpie:@gateway"), false);
  // 默认方式是当作上游：只导那一条，勾着的供应商不导；别的来源照常。
  assert.equal(defaultMagpieMode(items), "upstream");
  assert.deepEqual(importPlan(items, picked, defaultMagpieMode(items)), { ccswitch: ["b"], magpie: ["@gateway"] });
  assert.deepEqual(importPlan(items, new Set(), "upstream").magpie, ["@gateway"]);
  // 改选复制：只导勾着的供应商，没有「当作上游」那一条，就算它混进了勾选集合也不会导。
  assert.deepEqual(importPlan(items, picked, "copy"), { ccswitch: ["b"], magpie: ["m1"] });
  assert.deepEqual(importPlan(items, new Set([...picked, "magpie:@gateway"]), "copy").magpie, ["m1"]);
  // 已经当作上游导入过：默认方式回到复制；硬选当作上游也没有可导的。
  const done = thirdPartyItems(null, { ...magpie, gateway: { sourceId: "@gateway", baseUrl: "http://127.0.0.1:3425/v1", imported: true } });
  assert.equal(defaultMagpieMode(done), "copy");
  assert.deepEqual(importPlan(done, defaultPicked(done), "upstream").magpie, []);
  // 没装 Magpie 或旧版后端：没有这一条，默认就是复制。
  const plain = thirdPartyItems(null, magpie);
  assert.equal(plain.some((item) => item.upstream), false);
  assert.equal(defaultMagpieMode(plain), "copy");
  assert.deepEqual(importPlan(plain, new Set(["magpie:m1"]), "upstream").magpie, []);
  // 「当作上游」还没导入也算这个来源有东西可导：cc-switch 都导过了就停在 Magpie。
  assert.equal(initialSource(thirdPartyItems({ providers: ccs.providers, imported: ["a", "b"] }, { providers: [magpie.providers[1]], gateway: magpieWithGateway.gateway })), "magpie");
});

test("共享供应商的导入里 Magpie 那一组也是二选一：默认只勾当作上游，勾了一种会取消另一种", () => {
  const candidates = [
    { source: "codex", id: "relay-b", imported: false },
    { source: "magpie", id: MAGPIE_UPSTREAM_ID, imported: false },
    { source: "magpie", id: "m1", imported: false },
    { source: "magpie", id: "m2", imported: false },
    { source: "magpie", id: "m3", imported: true },
  ];
  const initial = new Set(["codex:relay-b", "magpie:@gateway"]);
  // 勾某一家：「当作上游」被取消，别的来源不动。
  const copy = togglePoolPick(initial, candidates[2], candidates);
  assert.deepEqual([...copy], ["codex:relay-b", "magpie:m1"]);
  const more = togglePoolPick(copy, candidates[3], candidates);
  assert.deepEqual([...more], ["codex:relay-b", "magpie:m1", "magpie:m2"]);
  // 再勾回「当作上游」：Magpie 的复制条目都取消。
  assert.deepEqual([...togglePoolPick(more, candidates[1], candidates)], ["codex:relay-b", "magpie:@gateway"]);
  // 取消勾选不影响别的；别的来源的勾选和 Magpie 互不影响。
  assert.deepEqual([...togglePoolPick(copy, candidates[2], candidates)], ["codex:relay-b"]);
  assert.deepEqual([...togglePoolPick(initial, candidates[0], candidates)], ["magpie:@gateway"]);
});

test("地址只显示主机名", () => {
  assert.equal(urlHost("https://relay.example.com/v1"), "relay.example.com");
  assert.equal(urlHost("relay.example.com/v1"), "relay.example.com");
  assert.equal(urlHost(""), "");
  assert.equal(urlHost(undefined), "");
});
