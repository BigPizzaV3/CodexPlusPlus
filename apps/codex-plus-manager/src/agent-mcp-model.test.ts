import assert from "node:assert/strict";
import test from "node:test";
import { mcpCount, mcpOn, mcpPresent, mergeMcpRows } from "./agent-mcp-model.ts";

const entry = (id: string, enabled = true, tomlBody = `command = "${id}"\n`) => ({ id, enabled, tomlBody });

test("按名称合并三家，Codex 的顺序在前，其余按名称排", () => {
  const rows = mergeMcpRows([entry("zeta"), entry("alpha")], { claude: [entry("alpha"), entry("beta")], grok: [entry("gamma")] });
  assert.deepEqual(rows.map((row) => row.id), ["zeta", "alpha", "beta", "gamma"]);
  const alpha = rows.find((row) => row.id === "alpha")!;
  assert.deepEqual(mcpPresent(alpha), ["codex", "claude"]);
});

test("展示用表体优先取 Codex，其次 Claude、Grok", () => {
  const rows = mergeMcpRows([], { claude: [entry("x", true, "url = \"https://c\"\n")], grok: [entry("x", true, "url = \"https://g\"\n")] });
  assert.equal(rows[0].body, "url = \"https://c\"\n");
});

test("停用的条目算「在」但不算「在用」，计数只数在用的", () => {
  const rows = mergeMcpRows([entry("a", false), entry("b")], { grok: [entry("a", false)] });
  const a = rows.find((row) => row.id === "a")!;
  assert.equal(mcpOn(a, "codex"), false);
  assert.deepEqual(mcpPresent(a), ["codex", "grok"]);
  assert.equal(mcpCount(rows, "codex"), 1);
  assert.equal(mcpCount(rows, "claude"), 0);
});

test("MCP 页按 Agent 三列管理，Claude / Grok 走后端命令且命令已注册", async () => {
  const { readFileSync } = await import("node:fs");
  const page = readFileSync(new URL("./ManagementPages.tsx", import.meta.url), "utf8");
  const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.match(page, /const agents: McpAgentId\[\] = kind === "mcp" \? MCP_AGENTS : \["codex"\];/);
  assert.match(page, /invoke<Result & \{ config\?: AgentMcpConfig; backupPath\?: string \}>\("save_agent_mcp"/);
  assert.match(page, /const base = enabledBody\(body, true\);/, "编辑时先统一成启用，避免写到 Claude 被当成移除");
  for (const command of ["agent_mcp_commands::load_agent_mcp", "agent_mcp_commands::save_agent_mcp"]) assert.ok(lib.includes(command), command);
});
