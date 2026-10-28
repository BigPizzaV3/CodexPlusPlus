import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

test("总览里「接入 Agent」「添加供应商」就地弹窗，不跳页", () => {
  const hub = read("HubOverview.tsx");
  assert.match(hub, /onClick=\{\(\) => setAdding\("agent"\)\}/);
  assert.match(hub, /onClick=\{\(\) => setAdding\("provider"\)\}/);
  assert.match(hub, /<AgentConnectDialog/);
  assert.match(hub, /<ProviderAdder/);
  assert.doesNotMatch(hub, /hub-node-ghost" onClick=\{\(\) => onNavigate/);
});

test("供应商池和总览共用同一套添加流程", () => {
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /export function ProviderAdder/);
  assert.match(providers, /\{adding \? <ProviderAdder/);
});
