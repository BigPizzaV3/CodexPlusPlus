import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { importCandidates, pendingImportCount, type GatewayPayload } from "./gateway-model.ts";

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

test("导入弹窗多选、一次导完；官方登录账号直接去登录；空池时总览和供应商池给出导入入口", () => {
  const providers = read("GatewayProviders.tsx");
  assert.match(providers, /"import_gateway_providers", \{ items, skipAgents \}/);
  assert.match(providers, /new Set\(candidates\.filter\(\(item\) => !item\.imported\)\.map\(keyOf\)\)/, "默认勾上还没导入的");
  assert.match(providers, /onClick=\{onLogin\} type="button">\{t\("去登录"\)\}/);
  assert.match(providers, /setAdding\("import"\)/);
  assert.match(read("HubOverview.tsx"), /setAdding\("import"\)/);
  assert.doesNotMatch(read("../src-tauri/src/lib.rs"), /gateway_commands::import_gateway_provider,/, "单项导入命令已被批量导入取代");
});
