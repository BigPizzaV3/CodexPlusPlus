import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { AGENT_PET_EXTENSION_KEY, agentPetExtensionConfig, agentPetStateFromExtension } from "./agent-pet-extension.ts";

const manifest = JSON.parse(readFileSync(new URL("../../../assets/extensions/agent-pet/codexpp.json", import.meta.url), "utf8"));
const main = readFileSync(new URL("../../../assets/extensions/agent-pet/main.js", import.meta.url), "utf8");
const manifestFragments = JSON.parse(readFileSync(new URL("../../../assets/inject/renderer-inject/manifest.json", import.meta.url), "utf8"));

test("拓展配置与页面设置互相换算，键都在清单 config 里声明过", () => {
  const state = agentPetStateFromExtension({ kind: "whaleWidget", image: "fox", size: 999, volume: 0.3, codexAppWhaleBalanceProtocol: "custom", codexAppWhaleBalancePath: "/b" }, true);
  assert.equal(state.pet.enabled, true);
  assert.equal(state.pet.kind, "whaleWidget");
  assert.equal(state.pet.image, "fox");
  assert.equal(state.pet.size, 240, "按原来的规则夹在范围内");
  assert.deepEqual(state.pet.agents, ["codex"]);
  assert.equal(state.whale.codexAppWhaleBalanceProtocol, "custom");
  const config = agentPetExtensionConfig(state);
  for (const key of Object.keys(config)) assert.ok(key in manifest.config, `${key} 没在清单 config 里声明`);
  assert.equal(config.size, 240);
  assert.equal(config.codexAppWhaleBalancePath, "/b");
  assert.equal("enabled" in config, false, "启停是拓展开关，不进配置");
  assert.equal(agentPetStateFromExtension(undefined, false).pet.enabled, false);
});

test("桌宠是官方拓展包：入口只用公开 ctx 接口，内置注入层不再带桌宠分片", () => {
  assert.equal(AGENT_PET_EXTENSION_KEY, `bundled:${manifest.id}`);
  assert.deepEqual(manifest.renderer.at(-1), "main.js");
  for (const call of ["ctx.registerSlot(", "ctx.onScan(", "ctx.onConfigUpdate(", "ctx.listen(", "ctx.effect("]) assert.ok(main.includes(call), call);
  const fragments = manifestFragments.fragments.map((fragment: { name: string }) => fragment.name);
  for (const removed of ["97-agent-pet.js", "96-whale-widget.js", "96-whale-compat.js"]) assert.ok(!fragments.includes(removed), removed);
});
