import assert from "node:assert/strict";
import { test } from "node:test";
import {
  contextEntriesFromConfig,
  contextEntryToTomlSection,
  splitContextConfigText,
  parseContextEntries,
  stripContextEntriesFromConfig,
} from "./context-config-text.ts";

// #2887：后端 toml_edit 规范化后，子表不再紧跟父表——`.env` 会被挪到文档末尾、
// 中间隔着别的 server。旧解析只在「子表紧邻父表」时才当子表，否则用同一 id
// 新开一条空 entry 把完整的那条覆盖掉，command/args 全丢。
const shuffledConfig = `model = "gpt-5"

[mcp_servers.premiere-pro.env]
ADOBE_HOME = "/opt/adobe"

[mcp_servers.vivado]
command = "python"
args = ["-m", "vivado_mcp"]

[mcp_servers.node_repl]
command = "node"
args = ["repl.js"]

[mcp_servers.vivado.env]
VIVADO_PATH = "/opt/vivado"
`;

test("parseContextEntries keeps root keys and non-adjacent subtables together", () => {
  const entries = parseContextEntries(shuffledConfig, "mcp", "mcp_servers");
  const vivado = entries.find((entry) => entry.id === "vivado");
  assert.ok(vivado, "vivado 条目必须存在");
  assert.match(vivado.tomlBody, /command = "python"/);
  assert.match(vivado.tomlBody, /args = \["-m", "vivado_mcp"\]/);
  assert.match(vivado.tomlBody, /\[env\]/);
  assert.match(vivado.tomlBody, /VIVADO_PATH = "\/opt\/vivado"/);
  // env 键必须落在 [env] 子表下面，不能被压平进父表。
  const envIndex = vivado.tomlBody.indexOf("[env]");
  assert.ok(vivado.tomlBody.indexOf("VIVADO_PATH") > envIndex, vivado.tomlBody);
  // root 键排在子表之前，否则序列化回去会落进子表。
  assert.ok(vivado.tomlBody.indexOf("command") < envIndex, vivado.tomlBody);
  const nodeRepl = entries.find((entry) => entry.id === "node_repl");
  assert.ok(nodeRepl);
  assert.match(nodeRepl.tomlBody, /command = "node"/);
});

test("parseContextEntries creates an entry for a subtable without a parent header", () => {
  const entries = parseContextEntries(shuffledConfig, "mcp", "mcp_servers");
  const premiere = entries.find((entry) => entry.id === "premiere-pro");
  assert.ok(premiere, "只有 [mcp_servers.premiere-pro.env] 也要建成条目");
  assert.match(premiere.tomlBody, /\[env\]/);
  assert.match(premiere.tomlBody, /ADOBE_HOME = "\/opt\/adobe"/);
});

test("parseContextEntries merges a repeated parent header instead of overwriting", () => {
  const config = `[mcp_servers.vivado]
command = "python"

[mcp_servers.other]
command = "other"

[mcp_servers.vivado]
args = ["-m", "vivado_mcp"]
`;
  const entries = parseContextEntries(config, "mcp", "mcp_servers");
  const vivado = entries.filter((entry) => entry.id === "vivado");
  assert.equal(vivado.length, 1);
  assert.match(vivado[0].tomlBody, /command = "python"/);
  assert.match(vivado[0].tomlBody, /args = \["-m", "vivado_mcp"\]/);
});

test("stripContextEntriesFromConfig removes non-adjacent subtables of known ids only", () => {
  const entries = contextEntriesFromConfig(shuffledConfig);
  const kept = stripContextEntriesFromConfig(shuffledConfig, entries);
  // 三个已知 id 的父表和子表都剥掉。
  assert.ok(!kept.includes("vivado"), kept);
  assert.ok(!kept.includes("node_repl"), kept);
  assert.ok(!kept.includes("premiere-pro"), kept);
  assert.ok(!kept.includes("VIVADO_PATH"), kept);
  assert.ok(kept.includes(`model = "gpt-5"`), kept);
});

test("stripContextEntriesFromConfig keeps subtables of unknown ids", () => {
  // managed 列表里只有 vivado/node_repl，没有 premiere-pro。
  const entries = contextEntriesFromConfig(`[mcp_servers.vivado]
command = "python"

[mcp_servers.node_repl]
command = "node"
`);
  const kept = stripContextEntriesFromConfig(shuffledConfig, entries);
  assert.ok(!kept.includes("[mcp_servers.vivado]"), kept);
  assert.ok(!kept.includes("VIVADO_PATH"), kept);
  assert.ok(kept.includes("[mcp_servers.premiere-pro.env]"), kept);
  assert.ok(kept.includes("ADOBE_HOME"), kept);
});

test("multi-level subtables round-trip under their own server", () => {
  const config = `[mcp_servers.github]
command = "gh-mcp"

[mcp_servers.github.tools.search]
approval_mode = "approve"
`;
  const split = splitContextConfigText(config);
  assert.match(split.context, /\[mcp_servers\.github\.tools\.search\]/);
  assert.doesNotMatch(split.context, /^\[tools\.search\]/m);
  const entry = parseContextEntries(split.context, "mcp", "mcp_servers")[0];
  assert.equal(contextEntryToTomlSection("mcp_servers", entry), `[mcp_servers.github]
command = "gh-mcp"

[mcp_servers.github.tools.search]
approval_mode = "approve"`);
});
