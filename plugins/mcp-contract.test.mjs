import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";
import { spawn } from "node:child_process";
import { once } from "node:events";
import http from "node:http";

const repoRoot = path.resolve(import.meta.dirname, "..");
const bridgeBinary = process.env.XUAN_BRIDGE_BIN || path.join(
  repoRoot,
  "tools",
  "xuan-bridge",
  "target",
  "debug",
  process.platform === "win32" ? "xuan-bridge.exe" : "xuan-bridge"
);
const pluginNames = ["xuan-workspace-search", "xuan-usage", "xuan-polish", "xuan-mobile"];

function startServer(pluginName, options = {}) {
  const pluginRoot = path.join(import.meta.dirname, pluginName);
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xuan-mcp-home-"));
  const environment = {
    ...process.env,
    XUAN_BRIDGE_BIN: bridgeBinary,
    XUAN_HOME: home,
    ...options.env
  };
  for (const key of options.unsetEnv || []) delete environment[key];
  const child = spawn(process.execPath, ["server.mjs"], {
    cwd: pluginRoot,
    env: environment,
    stdio: ["pipe", "pipe", "pipe"]
  });
  const waiters = new Map();
  const unsolicited = [];
  let stderr = "";
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  readline.createInterface({ input: child.stdout }).on("line", (line) => {
    const response = JSON.parse(line);
    const waiter = waiters.get(response.id);
    if (waiter) {
      waiters.delete(response.id);
      waiter.resolve(response);
    } else unsolicited.push(response);
  });
  child.once("exit", (code) => {
    for (const waiter of waiters.values()) {
      waiter.reject(new Error(`MCP server exited with code ${code}: ${stderr}`));
    }
    waiters.clear();
  });
  return {
    request(id, method, params = {}) {
      return new Promise((resolve, reject) => {
        waiters.set(id, { resolve, reject });
        child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
      });
    },
    notify(method, params = {}) {
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
    },
    unsolicited() { return unsolicited.slice(); },
    async close() {
      child.stdin.end();
      let timer;
      try {
        if (child.exitCode !== null) return;
        await Promise.race([
          once(child, "exit"),
          new Promise((_, reject) => {
            timer = setTimeout(() => reject(new Error("插件未按预期退出")), 5000);
          }),
        ]);
      } finally {
        clearTimeout(timer);
        if (child.exitCode === null && child.signalCode === null) {
          const exited = once(child, "exit");
          child.kill();
          await exited;
        }
        fs.rmSync(home, { recursive: true, force: true });
      }
    }
  };
}

test("all plugin MCP servers complete initialize and tools/list", async () => {
  assert.ok(fs.existsSync(bridgeBinary), `build xuan-bridge first: ${bridgeBinary}`);
  for (const pluginName of pluginNames) {
    const server = startServer(pluginName);
    try {
      const initialized = await server.request(1, "initialize", {
        protocolVersion: "2024-11-05",
        capabilities: {},
        clientInfo: { name: "contract-test", version: "1" }
      });
      assert.equal(initialized.result.serverInfo.name, pluginName);
      const listed = await server.request(2, "tools/list");
      const expectedToolCount = {
        "xuan-workspace-search": 2,
        "xuan-usage": 1,
        "xuan-polish": 1,
        "xuan-mobile": 4
      }[pluginName];
      assert.equal(listed.result.tools.length, expectedToolCount);
      const tool = listed.result.tools[0];
      assert.equal(tool.inputSchema.type, "object");
      assert.equal(tool.inputSchema.additionalProperties, false);
      assert.equal("bridgeMethod" in tool, false);
    } finally {
      await server.close();
    }
  }
});

test("mobile status MCP tool calls the bridge end to end", async () => {
  const server = startServer("xuan-mobile");
  try {
    await server.request(40, "initialize");
    const response = await server.request(41, "tools/call", {
      name: "xuan_mobile_status",
      arguments: {}
    });
    assert.equal(typeof response.result.isError, "boolean");
    const payload = JSON.parse(response.result.content[0].text);
    assert.equal(typeof payload, "object");
    assert.ok("state" in payload || "paired" in payload || "error" in payload || "message" in payload);
  } finally {
    await server.close();
  }
});

test("workspace search MCP tool calls the bridge end to end", async () => {
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "xuan-mcp-search-"));
  fs.writeFileSync(path.join(workspace, "sample.txt"), "alpha\nneedle\nomega\n");
  const server = startServer("xuan-workspace-search");
  try {
    await server.request(10, "initialize");
    const response = await server.request(11, "tools/call", {
      name: "workspace_search",
      arguments: { root: workspace, query: "needle", maxResults: 10 }
    });
    assert.equal(response.result.isError, false);
    const payload = JSON.parse(response.result.content[0].text);
    assert.equal(payload.state, "complete");
    assert.equal(payload.result.results[0].relativePath, "sample.txt");
    assert.equal(payload.result.results[0].line, 2);
  } finally {
    await server.close();
    fs.rmSync(workspace, { recursive: true, force: true });
  }
});

test("windows plugin server finds the installed bridge without refreshed user environment", { skip: process.platform !== "win32" }, async () => {
  const localAppData = fs.mkdtempSync(path.join(os.tmpdir(), "xuan-local-app-data-"));
  const installedBridge = path.join(localAppData, "XuanPlusPlus", "bin", "xuan-bridge.exe");
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "xuan-mcp-fallback-"));
  fs.mkdirSync(path.dirname(installedBridge), { recursive: true });
  fs.copyFileSync(bridgeBinary, installedBridge);
  fs.writeFileSync(path.join(workspace, "sample.txt"), "fallback works\n");
  const server = startServer("xuan-workspace-search", {
    env: { LOCALAPPDATA: localAppData },
    unsetEnv: ["XUAN_BRIDGE_BIN"]
  });
  try {
    await server.request(30, "initialize");
    const response = await server.request(31, "tools/call", {
      name: "workspace_search",
      arguments: { root: workspace, query: "fallback works", maxResults: 10 }
    });
    assert.equal(response.result.isError, false);
    const payload = JSON.parse(response.result.content[0].text);
    assert.equal(payload.result.results[0].relativePath, "sample.txt");
  } finally {
    await server.close();
    fs.rmSync(workspace, { recursive: true, force: true });
    await fs.promises.rm(localAppData, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
  }
});

test("installed-style MCP stdio exits after input closes", async () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xuan-mcp-exit-"));
  const pluginRoot = path.join(import.meta.dirname, "xuan-workspace-search");
  const child = spawn(process.execPath, ["server.mjs"], {
    cwd: pluginRoot,
    env: { ...process.env, XUAN_BRIDGE_BIN: bridgeBinary, XUAN_HOME: home },
    stdio: ["pipe", "pipe", "pipe"]
  });
  let stdout = "";
  child.stdout.on("data", (chunk) => { stdout += chunk; });
  child.stdin.end(`${JSON.stringify({ jsonrpc: "2.0", id: 20, method: "initialize", params: {} })}\n`);
  const [code] = await Promise.race([
    once(child, "exit"),
    new Promise((_, reject) => setTimeout(() => reject(new Error("MCP server did not exit")), 5_000))
  ]);
  assert.equal(code, 0);
  assert.equal(JSON.parse(stdout.trim()).result.serverInfo.name, "xuan-workspace-search");
  fs.rmSync(home, { recursive: true, force: true });
});

test("all plugin MCP servers ignore cancellation notifications and answer ping", async () => {
  assert.ok(fs.existsSync(bridgeBinary), `build xuan-bridge first: ${bridgeBinary}`);
  for (const pluginName of pluginNames) {
    const server = startServer(pluginName);
    try {
      await server.request(1, "initialize");
      server.notify("notifications/initialized");
      server.notify("notifications/cancelled", { requestId: 999, reason: "cancelled" });
      server.notify("notifications/progress", { progressToken: "request", progress: 1 });
      const ping = await server.request(2, "ping");
      assert.deepEqual(ping.result, {});
      await new Promise((resolve) => setTimeout(resolve, 25));
      assert.deepEqual(server.unsolicited(), []);
    } finally {
      await server.close();
    }
  }
});

test("版本化安装加载独立界面模块，兼容旧环境变量并随 MCP 退出", { skip: process.platform !== "win32" }, async () => {
  const localAppData = fs.mkdtempSync(path.join(os.tmpdir(), "xuan-versioned-runtime-"));
  const bin = path.join(localAppData, "XuanPlusPlus", "bin");
  const version = "a".repeat(64);
  const runtime = path.join(bin, "versions", version);
  fs.mkdirSync(runtime, { recursive: true });
  fs.copyFileSync(bridgeBinary, path.join(runtime, "xuan-bridge.exe"));
  fs.copyFileSync(path.join(repoRoot, "tools", "xuan-ui-bridge", "xuan-ui-bridge.mjs"), path.join(runtime, "xuan-ui-bridge.mjs"));
  fs.writeFileSync(path.join(bin, "current.json"), JSON.stringify({ version }));
  let probes = 0;
  const debug = http.createServer((request, response) => {
    probes++;
    response.writeHead(200);
    response.end("[]");
  });
  debug.listen(0, "127.0.0.1");
  await once(debug, "listening");
  const server = startServer("xuan-polish", {
    env: {
      LOCALAPPDATA: localAppData,
      XUAN_BRIDGE_BIN: path.join(bin, "xuan-bridge.exe"),
      XUAN_CODEX_DEBUG_PORT: String(debug.address().port),
      XUAN_UI_BRIDGE_DISABLE: "0",
    },
    unsetEnv: ["XUAN_UI_BRIDGE_MODULE"],
  });
  try {
    const initialized = await server.request(60, "initialize");
    assert.equal(initialized.result.serverInfo.name, "xuan-polish");
    for (let attempt = 0; attempt < 100 && probes === 0; attempt++) {
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    assert.ok(probes > 0, "已安装的独立界面模块未启动");
    const validation = await server.request(61, "tools/call", { name: "polish_text", arguments: { text: "" } });
    assert.equal(validation.result.isError, true);
  } finally {
    await server.close();
    await new Promise((resolve) => debug.close(resolve));
    fs.rmSync(localAppData, { recursive: true, force: true });
  }
});
