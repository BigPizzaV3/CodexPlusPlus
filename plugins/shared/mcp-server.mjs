import readline from "node:readline";
import { spawn } from "node:child_process";

const bridgeBinary = process.env.XUAN_BRIDGE_BIN || "xuan-bridge";
const bridge = spawn(bridgeBinary, [], { stdio: ["pipe", "pipe", "inherit"] });
const pending = new Map();
let nextBridgeId = 1;

const bridgeLines = readline.createInterface({ input: bridge.stdout });
bridgeLines.on("line", (line) => {
  try {
    const response = JSON.parse(line);
    const resolve = pending.get(response.id);
    if (resolve) { pending.delete(response.id); resolve(response); }
  } catch {}
});

function callBridge(method, params) {
  return new Promise((resolve, reject) => {
    const id = `mcp-${nextBridgeId++}`;
    pending.set(id, (response) => response.error ? reject(new Error(response.error.message)) : resolve(response.result));
    bridge.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
  });
}

function resultText(value) {
  return [{ type: "text", text: JSON.stringify(value) }];
}

export function createMcpServer({ name, tools }) {
  const input = readline.createInterface({ input: process.stdin });
  input.on("line", async (line) => {
    let request;
    try { request = JSON.parse(line); } catch { return; }
    const response = { jsonrpc: "2.0", id: request.id };
    try {
      if (request.method === "initialize") {
        response.result = {
          protocolVersion: "2024-11-05",
          capabilities: { tools: {} },
          serverInfo: { name, version: "0.1.0" }
        };
      } else if (request.method === "notifications/initialized") {
        return;
      } else if (request.method === "tools/list") {
        response.result = { tools };
      } else if (request.method === "tools/call") {
        const tool = tools.find((item) => item.name === request.params?.name);
        if (!tool) throw new Error("unknown tool");
        const value = await callBridge(tool.bridgeMethod, request.params?.arguments || {});
        response.result = { content: resultText(value), isError: false };
      } else {
        throw new Error(`unsupported MCP method: ${request.method}`);
      }
    } catch (error) {
      response.error = { code: -32000, message: String(error?.message || error) };
    }
    process.stdout.write(`${JSON.stringify(response)}\n`);
  });
}
