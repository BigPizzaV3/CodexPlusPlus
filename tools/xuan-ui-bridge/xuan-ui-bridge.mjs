import http from "node:http";
import net from "node:net";
import path from "node:path";
import fs from "node:fs";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";

export const contracts = {
  "xuan-workspace-search": {
    port: 57401,
    routes: {
      "/v1/search/roots": "workspace.roots",
      "/v1/search/projects": "workspace.projects",
      "/v1/search/current-root": "workspace.current_root",
      "/v1/search/start": "workspace.search.start",
      "/v1/search/poll": "workspace.search.poll",
      "/v1/search/cancel": "workspace.search.cancel",
      "/v1/search/preview": "workspace.search.preview",
    },
  },
  "xuan-usage": { port: 57402, routes: { "/v1/usage": "usage.query" } },
  "xuan-polish": {
    port: 57403,
    routes: {
      "/v1/polish/settings": "polish.settings.get",
      "/v1/polish/settings/set": "polish.settings.set",
      "/v1/polish": "polish.generate",
    },
  },
  "xuan-mobile": {
    port: 57404,
    routes: {
      "/v1/mobile/status": "mobile.status",
      "/v1/mobile/pair": "mobile.pair",
      "/v1/mobile/enable": "mobile.enable",
      "/v1/mobile/confirm": "mobile.confirm",
      "/v1/mobile/auto-sync": "mobile.auto_sync",
      "/v1/mobile/select": "mobile.select",
      "/v1/mobile/tasks": "mobile.tasks",
    },
  },
};

function pageAdapter(name, binding, owner) {
  if (window.top !== window || window.location.origin !== "app://-") return;
  const registry = window.__xuanPluginBridge ||= {};
  if (registry[name]?.owner === owner) return;
  registry[name]?.dispose?.();
  const pending = new Map();
  let sequence = 0;
  const api = (route, payload = {}) => new Promise((resolve, reject) => {
    if (pending.size >= 32) return reject(new Error("插件请求过多，请稍后重试"));
    const id = `${owner}:${++sequence}`;
    const timer = window.setTimeout(() => {
      pending.delete(id);
      reject(new Error("插件请求超时，请稍后重试"));
    }, 75_000);
    pending.set(id, { resolve, reject, timer });
    try {
      window[binding](JSON.stringify({ id, route, payload }));
    } catch {
      window.clearTimeout(timer);
      pending.delete(id);
      reject(new Error("插件连接已断开，请重新打开任务后重试"));
    }
  });
  api.owner = owner;
  api.resolve = (id, value) => {
    const item = pending.get(id);
    if (!item) return;
    pending.delete(id);
    window.clearTimeout(item.timer);
    item.resolve(value);
  };
  api.dispose = () => {
    for (const item of pending.values()) {
      window.clearTimeout(item.timer);
      item.reject(new Error("插件连接已更新，请重试"));
    }
    pending.clear();
    if (registry[name] === api) delete registry[name];
  };
  registry[name] = api;
}

export function adapterScript(name, binding, owner) {
  return `(${pageAdapter.toString()})(${JSON.stringify(name)},${JSON.stringify(binding)},${JSON.stringify(owner)})`;
}

export function isAppPage(target) {
  return target?.type === "page" && /^app:\/\/-\/(?:index\.html)?(?:[?#]|$)/.test(target.url || "");
}

export function validateSocket(raw, port) {
  const url = new URL(raw);
  if (url.protocol !== "ws:" || url.hostname !== "127.0.0.1" || Number(url.port) !== port
      || url.username || url.password || url.search || url.hash || !url.pathname.startsWith("/devtools/page/")) {
    throw new Error("插件调试通道地址不受支持");
  }
  return url.href;
}

export function readLoopbackJson(port, resource) {
  return new Promise((resolve, reject) => {
    const request = http.get({ hostname: "127.0.0.1", port, path: resource, agent: false }, (response) => {
      let bytes = 0;
      const chunks = [];
      response.on("data", (chunk) => {
        bytes += chunk.length;
        if (bytes > 4 * 1024 * 1024) request.destroy(new Error("插件响应过大"));
        else chunks.push(chunk);
      });
      response.on("error", reject);
      response.on("end", () => {
        if (response.statusCode !== 200) return reject(new Error("插件调试端口未就绪"));
        try { resolve(JSON.parse(Buffer.concat(chunks).toString("utf8"))); } catch {
          reject(new Error("插件调试响应无效"));
        }
      });
    });
    request.setTimeout(2500, () => request.destroy(new Error("插件调试连接超时")));
    request.once("error", reject);
  });
}

export async function dispatchUi(name, raw, request) {
  if (typeof raw !== "string" || Buffer.byteLength(raw) > 1_048_576) throw new Error("插件请求过大");
  const value = JSON.parse(raw);
  const routes = contracts[name]?.routes;
  if (!routes || !Object.hasOwn(routes, value.route) || typeof value.id !== "string" || value.id.length > 100
      || !value.payload || typeof value.payload !== "object" || Array.isArray(value.payload)) {
    throw new Error("插件请求不受支持");
  }
  const result = await request(routes[value.route], value.payload);
  if (Buffer.byteLength(JSON.stringify(result) ?? "") > 4 * 1024 * 1024) throw new Error("插件响应过大");
  return result;
}

export async function attachPage(target, { name, debugPort, request }) {
  const socket = new WebSocket(validateSocket(target.webSocketDebuggerUrl, debugPort));
  const owner = randomUUID();
  const binding = `xuan_${name.replaceAll("-", "_")}_v1`;
  const pending = new Map();
  const contexts = new Set();
  let sequence = 0;
  let frameId = "";
  let closed = false;
  let active = 0;
  function command(method, params = {}) {
    return new Promise((resolve, reject) => {
      if (closed || socket.readyState !== WebSocket.OPEN) return reject(new Error("插件连接已关闭"));
      const id = ++sequence;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error("插件调试请求超时"));
      }, 5000);
      pending.set(id, { resolve, reject, timer });
      socket.send(JSON.stringify({ id, method, params }));
    });
  }
  async function install(context) {
    if (context.origin !== "app://-" || !context.auxData?.isDefault || context.auxData.frameId !== frameId) return;
    const id = context.id;
    contexts.add(id);
    await command("Runtime.addBinding", { name: binding, executionContextId: id });
    if (!contexts.has(id)) return;
    await command("Runtime.evaluate", {
      expression: adapterScript(name, binding, owner), contextId: id, returnByValue: true,
    });
  }
  async function onBinding(params) {
    if (params.name !== binding || !contexts.has(params.executionContextId) || active >= 32) return;
    let envelope;
    try {
      if (Buffer.byteLength(params.payload) > 1_048_576) return;
      envelope = JSON.parse(params.payload);
      if (typeof envelope.id !== "string" || !envelope.id.startsWith(`${owner}:`)) return;
    } catch { return; }
    active += 1;
    try {
      let value;
      try { value = await dispatchUi(name, params.payload, request); } catch (error) {
        const message = typeof error?.message === "string" && /\p{Script=Han}/u.test(error.message)
          ? error.message : "插件请求失败，请检查插件设置或重新打开任务";
        value = { status: "failed", error: message, message };
      }
      if (closed || !contexts.has(params.executionContextId)) return;
      await command("Runtime.evaluate", {
        contextId: params.executionContextId,
        expression: `window.__xuanPluginBridge?.[${JSON.stringify(name)}]?.resolve(${JSON.stringify(envelope.id)},${JSON.stringify(value)})`,
        returnByValue: true,
      });
    } finally { active -= 1; }
  }
  socket.addEventListener("message", ({ data }) => {
    let message;
    try { message = JSON.parse(data); } catch { return; }
    if (message.id) {
      const item = pending.get(message.id);
      if (!item) return;
      pending.delete(message.id);
      clearTimeout(item.timer);
      if (message.error) item.reject(new Error("插件调试接口不可用"));
      else item.resolve(message.result);
    } else if (message.method === "Runtime.executionContextCreated") {
      install(message.params.context).catch(() => {});
    } else if (message.method === "Runtime.executionContextDestroyed") {
      contexts.delete(message.params.executionContextId);
    } else if (message.method === "Runtime.executionContextsCleared") {
      contexts.clear();
    } else if (message.method === "Runtime.bindingCalled") {
      onBinding(message.params).catch(() => {});
    }
  });
  socket.addEventListener("close", () => {
    closed = true;
    contexts.clear();
    for (const item of pending.values()) {
      clearTimeout(item.timer);
      item.reject(new Error("插件连接已关闭"));
    }
    pending.clear();
  });
  socket.addEventListener("error", () => {});
  try {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("插件调试连接超时")), 5000);
      socket.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
      socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("插件调试连接失败")); }, { once: true });
    });
    const tree = await command("Page.getFrameTree");
    frameId = tree.frameTree.frame.id;
    await command("Runtime.enable");
  } catch (error) {
    socket.close();
    throw error;
  }
  return {
    get closed() { return closed; },
    async close() {
      await Promise.all([...contexts].map((contextId) => command("Runtime.evaluate", {
        contextId,
        expression: `(() => { const api = window.__xuanPluginBridge?.[${JSON.stringify(name)}]; if (api?.owner === ${JSON.stringify(owner)}) api.dispose(); })()`,
      }).catch(() => {})));
      await command("Runtime.removeBinding", { name: binding }).catch(() => {});
      socket.close();
    },
  };
}

function portOpen(port) {
  return new Promise((resolve) => {
    const socket = net.connect({ host: "127.0.0.1", port });
    const finish = (result) => { socket.destroy(); resolve(result); };
    socket.setTimeout(1000, () => finish(false));
    socket.once("connect", () => finish(true));
    socket.once("error", () => finish(false));
  });
}

export function startPluginUi({ name, request, debugPort = Number(process.env.XUAN_CODEX_DEBUG_PORT || 9229),
  lockPort = contracts[name]?.port, autoStartMobile = true, intervalMs = 2000, remoteBinary }) {
  if (!Object.hasOwn(contracts, name) || !Number.isInteger(debugPort) || debugPort < 1 || debugPort > 65535) {
    throw new Error("插件调试端口配置无效");
  }
  let stopped = false;
  let running;
  let lease;
  let mobileChild;
  const sessions = new Map();
  async function tick() {
    if (!lease) {
      const candidate = net.createServer((socket) => socket.destroy());
      const acquired = await new Promise((resolve) => {
        candidate.once("error", () => resolve(false));
        candidate.listen({ host: "127.0.0.1", port: lockPort, exclusive: true }, () => resolve(true));
      });
      if (!acquired) return;
      lease = candidate;
    }
    if (stopped) return;
    // 端口租约只选举插件实例，不接收操作；重复打开任务不会重复执行绑定或润色。
    if (name === "xuan-mobile" && autoStartMobile && !mobileChild && !await portOpen(17421)) {
      const legacy = process.env.LOCALAPPDATA
        ? path.join(process.env.LOCALAPPDATA, "XuanPlusPlus", "bin", "xuan-plus-remote-bridge.exe") : "";
      const override = process.env.XUAN_REMOTE_BRIDGE_BIN;
      const binary = override && path.resolve(override).toLowerCase() !== legacy.toLowerCase()
        ? override : remoteBinary || legacy;
      if (binary && fs.existsSync(binary) && !stopped) {
        mobileChild = spawn(binary, [], { stdio: "ignore", windowsHide: true });
        mobileChild.once("error", () => { mobileChild = undefined; });
        mobileChild.once("exit", () => { mobileChild = undefined; });
      }
    }
    const targets = await readLoopbackJson(debugPort, "/json/list");
    if (stopped || !Array.isArray(targets)) return;
    const allowed = targets.filter(isAppPage);
    for (const [id, session] of sessions) {
      if (session.closed || !allowed.some((target) => target.id === id)) {
        await session.close();
        sessions.delete(id);
      }
    }
    for (const target of allowed) {
      if (stopped || sessions.has(target.id)) continue;
      const session = await attachPage(target, { name, debugPort, request });
      if (stopped) await session.close();
      else sessions.set(target.id, session);
    }
  }
  const poll = () => {
    if (!stopped && !running) {
      running = tick().catch(() => {}).finally(() => { running = undefined; });
    }
  };
  const timer = setInterval(poll, intervalMs);
  timer.unref();
  poll();
  return {
    get leader() { return Boolean(lease) && !stopped; },
    async close() {
      stopped = true;
      clearInterval(timer);
      await running;
      await Promise.all([...sessions.values()].map((session) => session.close()));
      sessions.clear();
      if (mobileChild) {
        const child = mobileChild;
        await new Promise((resolve) => {
          child.once("exit", resolve);
          child.once("error", resolve);
          if (!child.kill()) resolve();
        });
      }
      if (lease) await new Promise((resolve) => lease.close(resolve));
    },
  };
}
