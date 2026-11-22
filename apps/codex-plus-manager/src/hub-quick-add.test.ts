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

test("总览里点某家供应商：带上落地位置跳到供应商池，那边展开并滚到它", () => {
  const hub = read("HubOverview.tsx");
  assert.match(hub, /gatewayJump\.provider = id;\s*onNavigate\("gatewayProviders"\);/);
  assert.match(hub, /onClick=\{\(\) => openProvider\(provider\.id\)\}/);
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /takeGatewayJump\("provider"\)/);
  assert.match(providers, /scrollIntoView\(\{ block: "center" \}\)/);
});

test("总览里点某个 Agent：到「Agent 接入」展开它那一行；不在网关列表里的才去它自己的页面", () => {
  const hub = read("HubOverview.tsx");
  assert.match(hub, /gatewayJump\.agent = id;\s*onNavigate\("gateway"\);/);
  assert.match(hub, /onOpen=\{\(\) => openAgent\(agent\.id\)\}/);
  const agents = read("GatewayAgents.tsx");
  assert.match(agents, /const \[jumped\] = useState\(\(\) => \(embedded \? null : takeGatewayJump\("agent"\)\)\);/);
  assert.match(agents, /useGatewayViewState<string \| null>\(embedded \? null : "agents\.expanded", solo, jumped \?\? undefined\)/);
  assert.match(agents, /data-agent-row=\{agent\.agent\}/);
});
