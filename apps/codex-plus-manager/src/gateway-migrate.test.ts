import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { importCandidates, pendingImportCount, pendingImports, type GatewayPayload } from "./gateway-model.ts";

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

function payload(): GatewayPayload {
  return {
    gateway: { version: 1, bindings: {}, keyPolicy: { mode: "sticky" }, restoreOnExit: false },
    port: 1,
    helperRunning: true,
    pool: [{ id: "relay-a", name: "A", baseUrl: "https://a.test/v1", protocol: "responses", official: false, models: [], keyCount: 1, enabled: true, source: "codex:relay-a" }],
    groups: [],
    agents: [],
    health: { keys: [], providers: [] },
    codexProviders: [
      { id: "relay-a", name: "A", baseUrl: "https://a.test/v1", protocol: "responses", official: false, models: [], keyCount: 1 },
      { id: "relay-b", name: "B", baseUrl: "https://b.test/v1", protocol: "responses", official: false, models: [], keyCount: 1 },
    ],
    codexAggregates: [{ id: "agg", name: "日常", members: 2, mode: "inOrder", sticky: "cache", imported: false }],
    codexOfficial: [{ id: "official", name: "ChatGPT Pro · 26****34@qq.com" }],
    claudeProviders: [{ id: "kimi", name: "Kimi" }],
    grokProfiles: [],
    opencodeKeys: [],
  };
}

test("可导入的供应商：直连 Codex / 聚合 / Claude / Grok / OpenCode，按来源标出已导入的；官方登录账号不算", () => {
  const items = importCandidates(payload());
  assert.deepEqual(items.map((item) => `${item.source}:${item.id}:${item.imported}`), [
    "codex:relay-a:true",
    "codex:relay-b:false",
    "aggregate:agg:false",
    "claude:kimi:false",
  ]);
  assert.equal(pendingImportCount(payload()), 3);
});

test("Magpie 的供应商也是可导入的来源：排在 ClaudeMom 后面，已在池里的按来源标记认出来；旧版后端没有这一项时不出现", () => {
  const withMagpie = { ...payload(), magpieProviders: [{ id: "relay", name: "Relay" }, { id: "claude-relay", name: "Claude Relay" }] };
  withMagpie.pool = [...withMagpie.pool, { id: "relay", name: "Relay", baseUrl: "https://relay.test/v1", protocol: "responses", official: false, models: [], keyCount: 1, enabled: true, source: "magpie:relay" }];
  const magpie = importCandidates(withMagpie).filter((item) => item.source === "magpie");
  assert.deepEqual(magpie.map((item) => `${item.id}:${item.imported}`), ["relay:true", "claude-relay:false"]);
  assert.equal(importCandidates(payload()).some((item) => item.source === "magpie"), false);
  // 两种方式二选一、默认当作上游：有「当作上游」那一条时，待导入的只算它一条，可复制的各家不算。
  const both = { ...payload(), magpieProviders: [{ id: "@gateway", name: "Magpie" }, { id: "relay", name: "Relay" }, { id: "claude-relay", name: "Claude Relay" }] };
  assert.deepEqual(pendingImports(both).filter((item) => item.source === "magpie").map((item) => item.id), ["@gateway"]);
  assert.equal(pendingImportCount(both), pendingImportCount(payload()) + 1);
  // Magpie 里已经导过任何一条（不管哪种方式）：不再算待导入，免得「还有 N 个没导」一直亮着。
  both.pool = [...both.pool, { id: "relay", name: "Relay", baseUrl: "https://relay.test/v1", protocol: "responses", official: false, models: [], keyCount: 1, enabled: true, source: "magpie:relay" }];
  assert.deepEqual(pendingImports(both).filter((item) => item.source === "magpie"), []);
  // 旧版后端没有「当作上游」那一条：待导入的就是可复制的各家。
  assert.deepEqual(pendingImports({ ...payload(), magpieProviders: [{ id: "relay", name: "Relay" }] }).filter((item) => item.source === "magpie").map((item) => item.id), ["relay"]);
  // 导入弹窗里有它这一组；Claude 的「供应商」页把它算进「导入 · N」的待导入数。
  assert.match(read("GatewayProviders.tsx"), /\{ key: "magpie", title: t\("Magpie 的供应商"\) \}/);
  assert.match(read("ClaudeProviders.tsx"), /new Set\(\["claude", "ccswitch", "claudemom", "magpie"\]\)/);
  // `@gateway` 这一条不是复制，而是把 Magpie 本身当作一个上游：说明单独一句。
  assert.match(read("GatewayProviders.tsx"), /return item\.id === MAGPIE_UPSTREAM_ID\n\s*\? t\("把 Magpie 当作一个上游：请求经 Magpie 转发，用它里面的账号和路由，Magpie 要开着；和下面的复制二选一"\)\n\s*: t\("Magpie 的供应商，复制一份进本地服务，模型导入后自动获取；和上面的当作上游二选一"\)/);
  assert.match(read("ClaudeProviders.tsx"), /pendingImports\(payload\)\.filter\(\(item\) => CLAUDE_IMPORT_SOURCES\.has\(item\.source\)\)\.length/);
  // 和复制二选一：勾了一种取消另一种，「全选未导入的」不含它。
  assert.match(read("GatewayProviders.tsx"), /const toggle = \(item: ImportCandidate\) => setPicked\(togglePoolPick\(picked, item, candidates\)\);/);
  assert.match(read("GatewayProviders.tsx"), /setPicked\(allPicked \? new Set\(\) : pendingKeys\(\)\)/);
});

test("导入弹窗多选、一次导完；官方登录账号直接去登录；空池时总览和共享供应商给出导入入口", () => {
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /"import_gateway_providers", \{ items, skipAgents \}/);
  assert.match(providers, /const pendingKeys = \(\) => new Set\(pendingImports\(payload\)\.map\(keyOf\)\);\n\s*const \[picked, setPicked\] = useState<Set<string>>\(pendingKeys\);/, "默认勾上还没导入的");
  assert.match(providers, /onClick=\{onLogin\} type="button">\{t\("去登录"\)\}/);
  assert.match(providers, /setAdding\("import"\)/);
  assert.match(read("HubOverview.tsx"), /setAdding\("import"\)/);
  assert.doesNotMatch(read("../src-tauri/src/lib.rs"), /gateway_commands::import_gateway_provider,/, "单项导入命令已被批量导入取代");
});
