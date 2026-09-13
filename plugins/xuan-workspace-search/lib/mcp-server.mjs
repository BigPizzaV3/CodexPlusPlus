import readline from "node:readline";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

function resolveBridgeBinary() {
  if (process.platform === "win32" && process.env.LOCALAPPDATA) {
    const installed = path.join(process.env.LOCALAPPDATA, "XuanPlusPlus", "bin", "xuan-bridge.exe");
    if (process.env.XUAN_BRIDGE_BIN && path.resolve(process.env.XUAN_BRIDGE_BIN).toLowerCase() !== installed.toLowerCase()) return process.env.XUAN_BRIDGE_BIN;
    const pointer = path.join(path.dirname(installed), "current.json");
    if (fs.existsSync(pointer)) {
      const { version } = JSON.parse(fs.readFileSync(pointer, "utf8"));
      if (!/^[a-f0-9]{64}$/.test(version)) throw new Error("插件运行文件索引无效，请重新安装插件");
      const versioned = path.join(path.dirname(installed), "versions", version, "xuan-bridge.exe");
      if (!fs.existsSync(versioned)) throw new Error("插件运行文件缺失，请重新安装插件");
      return versioned;
    }
    if (fs.existsSync(installed)) return installed;
  }
  return process.env.XUAN_BRIDGE_BIN || "xuan-bridge";
}

const bridgeBinary = resolveBridgeBinary();
const bridge = spawn(bridgeBinary, [], {
  stdio: ["pipe", "pipe", "inherit"], windowsHide: true,
  env: { ...process.env, XUAN_MOBILE_BRIDGE_URL: process.env.XUAN_MOBILE_BRIDGE_URL || "http://127.0.0.1:17421" },
});
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
    }, method === "polish.generate" ? 75_000 : 30_000);
    pending.set(id, { resolve, reject, timer });
    try {
      bridge.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
    } catch (error) {
      pending.delete(id);
      clearTimeout(timer);
      reject(error);
    }
  });
}

function resultText(value) {
  return [{ type: "text", text: JSON.stringify(value) }];
}

export function createMcpServer({ name, version = "0.1.2", tools }) {
  let uiStarting;
  let closing = false;
  function startUi() {
    if (uiStarting || closing || process.env.XUAN_UI_BRIDGE_DISABLE === "1") return;
    const modulePath = process.env.XUAN_UI_BRIDGE_MODULE || path.join(path.dirname(bridgeBinary), "xuan-ui-bridge.mjs");
    if (!fs.existsSync(modulePath)) return;
    uiStarting = import(pathToFileURL(modulePath).href)
      .then(({ startPluginUi }) => startPluginUi({
        name, request: callBridge, remoteBinary: path.join(path.dirname(bridgeBinary), "xuan-plus-remote-bridge.exe"),
      }))
      .catch(() => { process.stderr.write("插件界面通道启动失败，请重新安装插件。\n"); });
  }
  const input = readline.createInterface({ input: process.stdin });
  input.once("close", async () => {
    closing = true;
    rejectPending(new Error("MCP 客户端已关闭"));
    const ui = await uiStarting;
    await ui?.close();
    if (!bridge.stdin.destroyed) bridge.stdin.end();
    if (!bridge.killed) bridge.kill();
  });
  input.on("line", async (line) => {
    let request;
    try { request = JSON.parse(line); } catch { return; }
    if (!request || typeof request !== "object" || Array.isArray(request)) return;
    const isNotification = !Object.hasOwn(request, "id");
    if (isNotification) {
      // 客户端通知没有响应；向 stdout 写入无 id 响应会破坏 JSON-RPC 会话。
      return;
    }
    const response = { jsonrpc: "2.0", id: request.id };
    try {
      if (request.method === "initialize") {
        startUi();
        response.result = {
          protocolVersion: "2024-11-05",
          capabilities: { tools: {} },
          serverInfo: { name, version }
        };
      } else if (request.method === "notifications/initialized") {
        return;
      } else if (request.method === "ping") {
        response.result = {};
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
