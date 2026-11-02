import assert from "node:assert/strict";
import test from "node:test";
import { assignProviderSlot } from "./flow-provider-colors.ts";

test("供应商配色不随排序、方向和新供应商变化", () => {
  const registry = new Map<string, number>();
  assert.equal(assignProviderSlot(registry, "a"), 0);
  assert.equal(assignProviderSlot(registry, "b"), 1);
  assert.equal(assignProviderSlot(registry, "b"), 1);
  assert.equal(assignProviderSlot(registry, "c"), 2);
  assert.equal(assignProviderSlot(registry, "a"), 0);
  assert.equal(assignProviderSlot(new Map(registry), "a"), 0);
});

test("无路由保持中性，超出配色容量不循环冒充前三家", () => {
  const registry = new Map<string, number>();
  assert.equal(assignProviderSlot(registry, ""), -1);
  for (const id of ["a", "b", "c"]) assignProviderSlot(registry, id);
  assert.equal(assignProviderSlot(registry, "d"), 3);
  assert.equal(assignProviderSlot(registry, "e"), 3);
  assert.equal(registry.size, 5);
});
