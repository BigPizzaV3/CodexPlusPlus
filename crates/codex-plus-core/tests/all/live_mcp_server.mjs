// 实测用的最小 MCP 服务器（stdio）：一个工具 probe_echo，返回环境变量 PROBE_TOKEN。
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin });
const reply = (id, result) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id, result }) + "\n");
lines.on("line", (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (message.id === undefined) return;
  if (message.method === "initialize") reply(message.id, { protocolVersion: message.params?.protocolVersion ?? "2025-06-18", capabilities: { tools: {} }, serverInfo: { name: "probe", version: "0.1.0" } });
  else if (message.method === "tools/list") reply(message.id, { tools: [{ name: "probe_echo", description: "PROBE-MCP-TOOL", inputSchema: { type: "object", properties: {} } }] });
  else if (message.method === "tools/call") reply(message.id, { content: [{ type: "text", text: `token=${process.env.PROBE_TOKEN}` }] });
  else reply(message.id, {});
});
