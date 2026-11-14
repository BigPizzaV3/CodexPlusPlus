import assert from "node:assert/strict";
import test from "node:test";
import { bodyEnabledState, editedEnabledState, mcpCount, mcpOn, mcpPresent, mergeMcpRows } from "./agent-mcp-model.ts";

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
  // 按使用模式去掉隐藏的 Agent 列（只管理 Codex 时只剩 Codex），隐藏的 Agent 也不读配置。
  assert.match(page, /const shownAgents = MCP_AGENTS\.filter\(\(agent\) => agentVisible\(focus, agent\)\);/);
  assert.match(page, /const agents: McpAgentId\[\] = kind === "mcp" \? shownAgents : \["codex"\];/);
  assert.match(page, /for \(const agent of \(\["claude", "grok"\] as const\)\.filter\(\(item\) => agentVisible\(focus, item\)\)\)/);
  assert.match(page, /invoke<Result & \{ config\?: AgentMcpConfig; backupPath\?: string \}>\("save_agent_mcp"/);
  assert.match(page, /const base = enabledBody\(body, true\);/, "编辑时先统一成启用，避免写到 Claude 被当成移除");
  for (const command of ["agent_mcp_commands::load_agent_mcp", "agent_mcp_commands::save_agent_mcp"]) assert.ok(lib.includes(command), command);
});

test("表体的启停只认根字段，子表里的 enabled 不算", () => {
  assert.equal(bodyEnabledState("enabled = false\n"), false);
  assert.equal(bodyEnabledState("command = \"x\"\ndisabled = true\n"), false);
  assert.equal(bodyEnabledState("enabled = true # 注释\n"), true);
  assert.equal(bodyEnabledState("command = \"x\"\n"), undefined);
  assert.equal(bodyEnabledState("command = \"x\"\n[env]\nenabled = false\n"), undefined);
});

test("#3115：编辑器里把插件改成 enabled = false 要生效，没动启停行时沿用原状态", () => {
  assert.equal(editedEnabledState("enabled = true\n", "enabled = false\n"), false);
  assert.equal(editedEnabledState("enabled = false\n", "enabled = true\n"), true);
  assert.equal(editedEnabledState("enabled = true\n", "enabled = true\nfoo = 1\n"), undefined);
  assert.equal(editedEnabledState("command = \"x\"\n", "command = \"y\"\n"), undefined);
  // 原表体没写 enabled 即默认启用：补一行 enabled = false 是停用，补 true 不算改动。
  assert.equal(editedEnabledState("command = \"x\"\n", "enabled = false\ncommand = \"x\"\n"), false);
  assert.equal(editedEnabledState(undefined, "enabled = true\n"), undefined);
});

test("#3115：保存时 Codex 以编辑器里改过的启停为准，不再一律改回原状态", async () => {
  const { readFileSync } = await import("node:fs");
  const page = readFileSync(new URL("./ManagementPages.tsx", import.meta.url), "utf8");
  assert.match(page, /const edited = editedEnabledState\(editor\.row\?\.body, body\);/);
  assert.match(page, /agent === "codex" && edited !== undefined\s*\? enabledBody\(base, edited\)/);
});

test("#3115：「切换时保留」在连接页对直连和网关都可见，不再藏在直连供应商编辑里", async () => {
  const { readFileSync } = await import("node:fs");
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  assert.equal(app.match(/relayPreserveNonproviderSettings: event\.currentTarget\.checked/g)?.length, 1);
  const row = app.indexOf('className="cx-preserve-row"');
  assert.ok(row > 0);
  // 只受配置切换总开关控制，不受「被网关接管」影响。
  const guard = app.lastIndexOf("{normalized.relayProfilesEnabled ? (", row);
  assert.ok(guard > 0 && row - guard < 200, "保留开关应只由 relayProfilesEnabled 控制");
});
