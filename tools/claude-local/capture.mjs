#!/usr/bin/env node
/**
 * 用真实 Chrome（已通过 Cloudflare）把 claude.ai 的静态资源录进本地缓存。
 * 录完后 proxy.mjs 就能在离线/被墙的情况下仍然提供真 UI。
 *
 * 用法：
 *   node capture.mjs --cdp http://127.0.0.1:9447 --out .cache
 */
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const arg = (n, d) => {
  const i = args.indexOf(`--${n}`);
  return i === -1 ? d : args[i + 1] ?? d;
};
const CDP = arg("cdp", "http://127.0.0.1:9447");
const OUT = path.resolve(arg("out", path.join(here, ".cache")));
fs.mkdirSync(OUT, { recursive: true });

const targets = await (await fetch(`${CDP}/json/list`)).json();
const page = targets.find((t) => t.type === "page" && /claude\.ai/.test(t.url));
if (!page) {
  console.error("没找到 claude.ai 页面，先在 Chrome 里打开 https://claude.ai/");
  process.exit(1);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const waiters = new Map();
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  if (m.id && waiters.has(m.id)) {
    waiters.get(m.id)(m);
    waiters.delete(m.id);
  } else if (m.method === "Network.responseReceived") {
    const r = m.params.response;
    if (r.url.startsWith("https://claude.ai/")) pending.set(m.params.requestId, r);
  } else if (m.method === "Network.loadingFinished") {
    const r = pending.get(m.params.requestId);
    if (r) {
      pending.delete(m.params.requestId);
      queue.push({ requestId: m.params.requestId, response: r });
    }
  }
};
await new Promise((r) => (ws.onopen = r));
const send = (method, params = {}) =>
  new Promise((res) => {
    const i = ++id;
    waiters.set(i, res);
    ws.send(JSON.stringify({ id: i, method, params }));
  });

const pending = new Map();
const queue = [];
let saved = 0;

async function drain() {
  while (queue.length) {
    const { requestId, response } = queue.shift();
    try {
      const bodyRes = await send("Network.getResponseBody", { requestId });
      const body = bodyRes.result?.body;
      if (body == null) continue;
      const buf = bodyRes.result.base64Encoded ? Buffer.from(body, "base64") : Buffer.from(body, "utf8");
      const url = new URL(response.url);
      const key = `${url.host}${url.pathname}${url.search}`;
      const file = path.join(OUT, crypto.createHash("sha1").update(key).digest("hex"));
      fs.writeFileSync(file, buf);
      fs.writeFileSync(`${file}.json`, JSON.stringify({ status: response.status, headers: response.headers, url: key }));
      saved++;
      console.log(`${response.status} ${buf.length}b ${url.pathname}`);
    } catch (e) {
      console.error("getResponseBody failed", e.message);
    }
  }
}

await send("Network.enable");
await send("Page.enable");
console.log("reloading page to capture…");
await send("Page.reload", { ignoreCache: true });

const deadline = Date.now() + 60000;
while (Date.now() < deadline) {
  await new Promise((r) => setTimeout(r, 300));
  await drain();
}
await drain();
console.log(`\n完成：${saved} 个资源 -> ${OUT}`);
ws.close();
