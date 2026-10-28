import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

test("Key 数量用图标 + 数字，不写「N 个 Key」这种句子", () => {
  for (const name of ["GatewayProviders.tsx", "GatewayPage.tsx", "HubOverview.tsx"]) {
    assert.doesNotMatch(read(name), /tf\("\{0\} 个 Key/, name);
  }
  assert.match(read("gateway-ui.tsx"), /export function KeyCount[\s\S]*?<KeyRound/);
});

test("订阅账号显示套餐", () => {
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /entry\.account && entry\.plan \? entry\.plan/);
  assert.match(providers, /entry\.account \? t\("套餐"\) : "Key"/);
});
