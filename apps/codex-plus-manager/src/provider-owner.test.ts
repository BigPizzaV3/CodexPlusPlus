import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { entriesForPage, otherAgentOwner, ownerBadgeKind, readShowOtherAgents, showOtherAgentsKey } from "./provider-owner.ts";

type Entry = { id: string; owner?: "codex" | "claude" | null; linkedCodexRelay?: string | null };
const pool: Entry[] = [
  { id: "relay-a", owner: "codex", linkedCodexRelay: "relay-a" },
  { id: "old-copy", owner: "codex" },
  { id: "anthropic", owner: "claude" },
  { id: "deepseek", owner: null },
  { id: "chatgpt" },
];

test("勾上时两页都列全部；不勾时各自去掉别的 Agent 的", () => {
  assert.deepEqual(entriesForPage(pool, "claude", true).map((entry) => entry.id), pool.map((entry) => entry.id));
  assert.deepEqual(entriesForPage(pool, "codex", true).map((entry) => entry.id), pool.map((entry) => entry.id));
  // Claude 页：Codex 的直连供应商（引用和旧版复制品）不列，本地服务页加的照列。
  assert.deepEqual(entriesForPage(pool, "claude", false).map((entry) => entry.id), ["anthropic", "deepseek", "chatgpt"]);
  // Codex 页「共享」分页：Claude 的不列，Codex 自己的照列。
  assert.deepEqual(entriesForPage(pool, "codex", false).map((entry) => entry.id), ["relay-a", "old-copy", "deepseek", "chatgpt"]);
});

test("来源标记：别的 Agent 的写来自哪个 Agent，Codex 页自己的直连供应商写来自直连", () => {
  assert.equal(ownerBadgeKind(pool[0], "claude"), "codex");
  assert.equal(ownerBadgeKind(pool[2], "codex"), "claude");
  assert.equal(ownerBadgeKind(pool[0], "codex"), "direct");
  assert.equal(ownerBadgeKind(pool[1], "codex"), "direct-copy");
  assert.equal(ownerBadgeKind(pool[2], "claude"), null, "Claude 页自己的不标");
  assert.equal(ownerBadgeKind(pool[3], "claude"), null, "没有归属的不标");
  assert.equal(otherAgentOwner(pool[4], "codex"), null, "旧版后端没有 owner 字段时当作没有归属");
});

test("复选框默认勾上，按页分开记", () => {
  assert.notEqual(showOtherAgentsKey("codex"), showOtherAgentsKey("claude"));
  assert.equal(readShowOtherAgents("codex", null), true);
  const stored = new Map([[showOtherAgentsKey("claude"), "0"]]);
  const storage = { getItem: (key: string) => stored.get(key) ?? null };
  assert.equal(readShowOtherAgents("claude", storage), false);
  assert.equal(readShowOtherAgents("codex", storage), true);
  assert.equal(readShowOtherAgents("codex", { getItem: () => { throw new Error("blocked"); } }), true, "存储不可用时按默认勾上");
});

test("Claude 页：按归属筛、数目跟着列出的、行上带来源标记，新加的记成 Claude 的", () => {
  const source = readFileSync(path.join(import.meta.dirname, "ClaudeProviders.tsx"), "utf8");
  assert.match(source, /const \[showOtherAgents, setShowOtherAgents\] = useShowOtherAgents\("claude"\);/);
  assert.match(source, /const pool = entriesForPage\(allPool, "claude", showOtherAgents\);/);
  // 分页数目、列表、「供应商 N」都从筛过的 pool 算。
  assert.match(source, /pool\.filter\(\(entry\) => viewOf\(entry\.protocol\) === view\)\.length/);
  assert.match(source, /count=\{pool\.length \|\| undefined\}/);
  assert.match(source, /<MenuItem checked=\{showOtherAgents\} onClick=\{\(\) => setShowOtherAgents\(!showOtherAgents\)\}/);
  assert.match(source, /<OwnerBadge entry=\{entry\} page="claude" \/>/);
  assert.match(source, /owner="claude"/);
  const adder = readFileSync(path.join(import.meta.dirname, "GatewayProviders.tsx"), "utf8");
  assert.match(adder, /"create_gateway_provider", \{ input, source: [\s\S]*?, skipAgents, \.\.\.\(owner \? \{ owner \} : \{\}\) \}/);
  assert.match(adder, /"import_gateway_providers", \{ items, skipAgents, \.\.\.\(owner \? \{ owner \} : \{\}\) \}/);
});

test("共享供应商卡片：引用 Codex 直连供应商的不给改连接资料、不给删", () => {
  const source = readFileSync(path.join(import.meta.dirname, "GatewayProviders.tsx"), "utf8");
  assert.match(source, /const linked = Boolean\(entry\.linkedCodexRelay\);/);
  assert.match(source, /\(entry\.account && entry\.account !== "chatgpt"\) \|\| linked \? \(\s*<span aria-hidden="true" className="gw-pool-edit-slot" \/>/);
  assert.match(source, /\{linked \? null : <span className="gw-detail-actions">\s*<EditIcon disabled=\{busy !== null\} label=\{t\("编辑模型列表"\)\}/);
  assert.match(source, /\{linked \? null : \(\s*<button className="gw-danger-link"/);
});
