import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { attachPage, isAppPage, readLoopbackJson, validateSocket } from "./xuan-ui-bridge.mjs";

async function evaluate(target, debugPort, expression) {
  const socket = new WebSocket(validateSocket(target.webSocketDebuggerUrl, debugPort));
  try {
    return await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("只读页面验证超时")), 10_000);
      socket.addEventListener("open", () => socket.send(JSON.stringify({
        id: 1, method: "Runtime.evaluate", params: { expression, awaitPromise: true, returnByValue: true },
      })));
      socket.addEventListener("message", ({ data }) => {
        const response = JSON.parse(data);
        if (response.id !== 1) return;
        clearTimeout(timer);
        if (response.error || response.result?.exceptionDetails) reject(new Error("只读页面验证失败"));
        else resolve(response.result?.result?.value);
      });
      socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("只读页面连接失败")); });
    });
  } finally { socket.close(); }
}

test("真实页面通过独立通道读取手机状态和润色设置", { skip: process.env.XUAN_LIVE_UI_TEST !== "1" }, async () => {
  const debugPort = Number(process.env.XUAN_CODEX_DEBUG_PORT || 9229);
  const target = (await readLoopbackJson(debugPort, "/json/list")).find(isAppPage);
  assert.ok(target, "未找到本机 Codex 主页面");
  const sessions = [];
  let calls = 0;
  try {
    for (const [name, route, method] of [
      ["xuan-polish", "/v1/polish/settings", "polish.settings.get"],
      ["xuan-mobile", "/v1/mobile/status", "mobile.status"],
    ]) {
      const occupied = await evaluate(target, debugPort, `Boolean(window.__xuanPluginBridge?.[${JSON.stringify(name)}])`);
      assert.equal(occupied, false, "已有独立插件通道，跳过对正在运行实例的替换");
      sessions.push(await attachPage(target, {
        name, debugPort, request: async (actual) => {
          assert.equal(actual, method);
          calls++;
          return readLoopbackJson(57324, route);
        },
      }));
      // 只返回结构校验结果，不把配置、绑定或任务数据带出页面。
      const result = await evaluate(target, debugPort, `(async () => {
        for (let i = 0; i < 30 && !window.__xuanPluginBridge?.[${JSON.stringify(name)}]; i++) {
          await new Promise(resolve => setTimeout(resolve, 50));
        }
        const api = window.__xuanPluginBridge?.[${JSON.stringify(name)}];
        if (!api) return { connected: false };
        const value = await api(${JSON.stringify(route)}, {});
        return { connected: true, valid: ${name === "xuan-polish"
          ? 'value?.status === "ok" && typeof value.settings === "object"'
          : 'typeof value?.enabled === "boolean"'} };
      })()`);
      assert.equal(result.connected, true, "独立页面通道未就绪");
      assert.equal(result.valid, true, "只读接口响应结构异常");
    }
    assert.equal(calls, 2);
  } finally {
    await Promise.all(sessions.map((session) => session.close()));
  }
});

test("真实页面通过插件 MCP 进程和标准输入输出完成只读请求", { skip: process.env.XUAN_LIVE_UI_TEST !== "1" }, async () => {
  const root = path.resolve(import.meta.dirname, "../..");
  const home = fs.mkdtempSync(path.join(root, "target", "xuan-live-ui-"));
  const debugPort = Number(process.env.XUAN_CODEX_DEBUG_PORT || 9229);
  const target = (await readLoopbackJson(debugPort, "/json/list")).find(isAppPage);
  const children = [];
  assert.ok(target, "未找到本机 Codex 主页面");
  try {
    for (const [name, route] of [["xuan-polish", "/v1/polish/settings"], ["xuan-mobile", "/v1/mobile/status"]]) {
      assert.equal(await evaluate(target, debugPort, `Boolean(window.__xuanPluginBridge?.[${JSON.stringify(name)}])`), false);
      const child = spawn(process.execPath, [path.join(root, "plugins", name, "server.mjs")], {
        windowsHide: true,
        stdio: ["pipe", "pipe", "pipe"],
        env: {
          ...process.env,
          XUAN_HOME: path.join(home, name),
          XUAN_BRIDGE_BIN: path.join(root, "tools", "xuan-bridge", "target", "release", "xuan-bridge.exe"),
          XUAN_UI_BRIDGE_MODULE: path.join(import.meta.dirname, "xuan-ui-bridge.mjs"),
          XUAN_UI_BRIDGE_DISABLE: "0",
          XUAN_CODEX_DEBUG_PORT: String(debugPort),
          // 只复用已运行的手机服务，测试不能恢复或创建新的真实绑定状态。
          XUAN_REMOTE_BRIDGE_BIN: path.join(home, "not-installed.exe"),
        },
      });
      children.push(child);
      child.stdout.resume();
      child.stderr.resume();
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} })}\n`);
      const result = await evaluate(target, debugPort, `(async () => {
        for (let i = 0; i < 100 && !window.__xuanPluginBridge?.[${JSON.stringify(name)}]; i++) {
          await new Promise(resolve => setTimeout(resolve, 50));
        }
        const api = window.__xuanPluginBridge?.[${JSON.stringify(name)}];
        if (!api) return false;
        const value = await api(${JSON.stringify(route)}, {});
        return ${name === "xuan-polish" ? 'value?.status === "ok" && typeof value.settings === "object"' : 'typeof value?.enabled === "boolean"'};
      })()`);
      assert.equal(result, true, "独立插件进程未完成页面只读请求");
    }
  } finally {
    for (const child of children) {
      let timer;
      try {
        const exited = once(child, "exit");
        child.stdin.end();
        await Promise.race([
          exited,
          new Promise((_, reject) => { timer = setTimeout(() => reject(new Error("测试插件退出超时")), 5000); }),
        ]);
      } finally {
        clearTimeout(timer);
        if (child.exitCode === null && child.signalCode === null) {
          const exited = once(child, "exit");
          child.kill();
          await exited;
        }
      }
    }
    fs.rmSync(home, { recursive: true, force: true });
  }
  for (const name of ["xuan-polish", "xuan-mobile"]) {
    assert.equal(await evaluate(target, debugPort, `Boolean(window.__xuanPluginBridge?.[${JSON.stringify(name)}])`), false, "插件退出后仍残留界面通道");
  }
});
