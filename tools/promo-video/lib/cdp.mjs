// 最小的无头 Chrome 驱动：启动 Chrome、开一个页面、收发 CDP 命令。
// 只用 Node 自带的 WebSocket / fetch，不引第三方依赖。
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import http from "node:http";
import { readFile, stat } from "node:fs/promises";

const CHROME_CANDIDATES = [
  process.env.CHROME_PATH,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Chromium.app/Contents/MacOS/Chromium",
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
  "/usr/bin/google-chrome",
].filter(Boolean);

async function findChrome() {
  for (const candidate of CHROME_CANDIDATES) {
    try { await stat(candidate); return candidate; } catch {}
  }
  throw new Error("找不到 Chrome，可用 CHROME_PATH 指定");
}

export async function launchChrome({ width = 1920, height = 1080, scale = 1 } = {}) {
  const chrome = await findChrome();
  const profile = mkdtempSync(path.join(tmpdir(), "cpp-promo-chrome-"));
  const proc = spawn(chrome, [
    "--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--no-first-run", "--no-default-browser-check", "--hide-scrollbars", "--mute-audio",
    "--force-color-profile=srgb", "--font-render-hinting=none", "--disable-background-timer-throttling",
    "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows",
    `--window-size=${width},${height}`, "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  const port = await new Promise((resolve, reject) => {
    let buffer = "";
    const timer = setTimeout(() => reject(new Error("Chrome 启动超时")), 20_000);
    proc.stderr.on("data", chunk => {
      buffer += chunk;
      const match = buffer.match(/DevTools listening on ws:\/\/[^:]+:(\d+)\//);
      if (match) { clearTimeout(timer); resolve(Number(match[1])); }
    });
    proc.on("exit", code => reject(new Error(`Chrome 退出：${code}`)));
  });
  const target = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" })).json();
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let nextId = 1;
  const pending = new Map();
  const listeners = new Map();
  ws.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(`${message.error.message} ${message.error.data ?? ""}`));
      else resolve(message.result);
    } else if (message.method) {
      for (const listener of listeners.get(message.method) ?? []) listener(message.params);
    }
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
  const on = (method, listener) => {
    if (!listeners.has(method)) listeners.set(method, []);
    listeners.get(method).push(listener);
  };
  const once = method => new Promise(resolve => {
    const listener = params => {
      listeners.set(method, (listeners.get(method) ?? []).filter(item => item !== listener));
      resolve(params);
    };
    on(method, listener);
  });
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: scale, mobile: false });
  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
  const page = {
    send, on, once,
    async goto(url) {
      // 先回到空白页，保证只改 hash 的地址也会整页重新加载。
      let loaded = once("Page.loadEventFired");
      await send("Page.navigate", { url: "about:blank" });
      await loaded;
      loaded = once("Page.loadEventFired");
      await send("Page.navigate", { url });
      await loaded;
    },
    async evaluate(expression) {
      const result = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
      return result.result.value;
    },
    async screenshot({ format = "png", quality, clip } = {}) {
      const params = { format, captureBeyondViewport: false, optimizeForSpeed: format === "jpeg" };
      if (quality) params.quality = quality;
      if (clip) params.clip = { ...clip, scale: 1 };
      const { data } = await send("Page.captureScreenshot", params);
      return Buffer.from(data, "base64");
    },
    async setViewport(w, h, s = scale) {
      await send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: s, mobile: false });
    },
    async close() {
      try { ws.close(); } catch {}
      proc.kill();
    },
  };
  return page;
}

const MIME = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".json": "application/json", ".png": "image/png", ".jpg": "image/jpeg", ".webp": "image/webp", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".woff": "font/woff", ".ico": "image/x-icon", ".wav": "audio/wav" };

/** 把若干目录挂成静态站点：mounts = { "/": dir, "/shots/": dir2 }，返回 { url, close }。 */
export async function serveStatic(mounts) {
  const entries = Object.entries(mounts).sort((a, b) => b[0].length - a[0].length);
  const server = http.createServer(async (req, res) => {
    const pathname = decodeURIComponent(new URL(req.url, "http://x").pathname);
    const mount = entries.find(([prefix]) => pathname.startsWith(prefix));
    if (!mount) { res.writeHead(404); res.end(); return; }
    let file = path.join(mount[1], pathname.slice(mount[0].length));
    if (file.endsWith(path.sep) || pathname.endsWith("/")) file = path.join(file, "index.html");
    if (!path.resolve(file).startsWith(path.resolve(mount[1]))) { res.writeHead(403); res.end(); return; }
    try {
      const body = await readFile(file);
      res.writeHead(200, { "content-type": MIME[path.extname(file)] ?? "application/octet-stream", "cache-control": "no-store" });
      res.end(body);
    } catch { res.writeHead(404); res.end(); }
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  return { url: `http://127.0.0.1:${server.address().port}`, close: () => server.close() };
}

export const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
