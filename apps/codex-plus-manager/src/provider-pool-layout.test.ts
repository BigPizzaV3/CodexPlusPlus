import assert from "node:assert/strict";
import test from "node:test";
import { providerPoolSections, reorderProviderSlots } from "./provider-pool-layout.ts";
import type { GatewayPoolEntry } from "./gateway-model.ts";

const entry = (id: string, enabled?: boolean): GatewayPoolEntry => ({
  id, name: id, enabled, baseUrl: `https://${id}.example.com/v1`,
  protocol: "responses", official: false, models: [`${id}-model`], keyCount: 1,
});
const pool = [entry("alpha"), entry("paused", false), entry("beta", true), entry("offline", false)];

test("供应商分组只做展示投影：旧数据默认开启，不更改原始优先级", () => {
  const { active, inactive } = providerPoolSections(pool, "");
  assert.deepEqual(active.map((e) => e.id), ["alpha", "beta"]);
  assert.deepEqual(inactive.map((e) => e.id), ["paused", "offline"]);
  assert.deepEqual(pool.map((e) => e.id), ["alpha", "paused", "beta", "offline"]);
});

test("搜索名称、域名和模型：停用项也返回，空结果不误报启用列表", () => {
  for (const query of [" PAUSED ", "paused.example.com", "paused-model"]) {
    const { active, inactive } = providerPoolSections(pool, query);
    assert.equal(active.length, 0);
    assert.deepEqual(inactive.map((e) => e.id), ["paused"]);
  }
  assert.deepEqual(providerPoolSections(pool, "not-found"), { active: [], inactive: [] });
  assert.deepEqual(providerPoolSections([], ""), { active: [], inactive: [] });
});

test("启用组内拖动回填原槽位：关闭项不丢失、不改位置", () => {
  const order = pool.map((e) => e.id);
  assert.deepEqual(reorderProviderSlots(order, ["beta", "alpha"]), ["beta", "paused", "alpha", "offline"]);
  assert.deepEqual(order, ["alpha", "paused", "beta", "offline"]);
});

test("停用组内拖动与全组排序，始终保留完整 ID", () => {
  const order = pool.map((e) => e.id);
  assert.deepEqual(reorderProviderSlots(order, ["offline", "paused"]), ["alpha", "offline", "beta", "paused"]);
  assert.deepEqual(reorderProviderSlots(order, [...order].reverse()), [...order].reverse());
  assert.deepEqual(reorderProviderSlots(order, []), order);
  assert.deepEqual(reorderProviderSlots(order, ["alpha"]), order);
  assert.deepEqual(reorderProviderSlots(order, ["alpha", "alpha"]), order);
  assert.deepEqual(reorderProviderSlots(order, ["missing"]), order);
});
