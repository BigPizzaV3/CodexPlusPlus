import readline from "node:readline";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

function resolveBridgeBinary() {
  if (process.env.XUAN_BRIDGE_BIN) return process.env.XUAN_BRIDGE_BIN;
  if (process.platform === "win32" && process.env.LOCALAPPDATA) {
    const installed = path.join(process.env.LOCALAPPDATA, "XuanPlusPlus", "bin", "xuan-bridge.exe");
    if (fs.existsSync(installed)) return installed;
  }
  return "xuan-bridge";
}

const bridgeBinary = resolveBridgeBinary();
const bridge = spawn(bridgeBinary, [], { stdio: ["pipe", "pipe", "inherit"], windowsHide: true });
const pending = new Map();
let nextBridgeId = 1;
let bridgeFailure = null;

function rejectPending(error) {
  for (const item of pending.values()) {
    clearTimeout(item.timer);
    item.reject(error);
  }
  pending.clear();
}

bridge.once("error", (error) => {
  bridgeFailure = error;
  rejectPending(error);
});
bridge.once("exit", (code) => {
  bridgeFailure = new Error(`xuan-bridge exited with code ${code}`);
  rejectPending(bridgeFailure);
});
process.once("exit", () => bridge.kill());

readline.createInterface({ input: bridge.stdout }).on("line", (line) => {
  try {
    const response = JSON.parse(line);
    const item = pending.get(response.id);
    if (!item) return;
    pending.delete(response.id);
    clearTimeout(item.timer);
    if (response.error) item.reject(new Error(response.error.message));
    else item.resolve(response.result);
  } catch {}
});

function callBridge(method, params) {
  if (bridgeFailure) return Promise.reject(bridgeFailure);
  return new Promise((resolve, reject) => {
    const id = `mcp-${nextBridgeId++}`;
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`xuan-bridge request timed out: ${method}`));
    }, 30_000);
    pending.set(id, { resolve, reject, timer });
    bridge.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
  });
}

function resultText(value) {
  return [{ type: "text", text: JSON.stringify(value) }];
}

export function createMcpServer({ name, version = "0.1.2", tools }) {
  const input = readline.createInterface({ input: process.stdin });
  input.once("close", () => bridge.stdin.end());
  input.on("line", async (line) => {
    let request;
    try { request = JSON.parse(line); } catch { return; }
    const response = { jsonrpc: "2.0", id: request.id };
    try {
      if (request.method === "initialize") {
        response.result = {
          protocolVersion: "2024-11-05",
          capabilities: { tools: {} },
          serverInfo: { name, version }
        };
      } else if (request.method === "notifications/initialized") {
        return;
      } else if (request.method === "tools/list") {
        response.result = { tools: tools.map(({ bridgeMethod, ...tool }) => tool) };
      } else if (request.method === "tools/call") {
        const tool = tools.find((item) => item.name === request.params?.name);
        if (!tool) throw new Error("unknown tool");
        const value = await callBridge(tool.bridgeMethod, request.params?.arguments || {});
        response.result = { content: resultText(value), isError: false };
      } else {
        throw new Error(`unsupported MCP method: ${request.method}`);
      }
    } catch (error) {
      if (request.method === "tools/call") {
        response.result = { content: resultText({ error: String(error?.message || error) }), isError: true };
      } else {
        response.error = { code: -32000, message: String(error?.message || error) };
      }
    }
    process.stdout.write(`${JSON.stringify(response)}\n`);
  });
}
