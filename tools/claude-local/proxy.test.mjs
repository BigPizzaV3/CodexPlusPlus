import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import path from "node:path";
import { after, before, test } from "node:test";
import tls from "node:tls";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const gatewayRequests = [];
let gateway;
let proxy;
let proxyClosed;
let proxyPort;

async function listen(server) {
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return server.address().port;
}

async function response(req, body) {
  const result = new Promise((resolve, reject) => {
    req.on("error", reject);
    req.on("response", (res) => {
      const chunks = [];
      res.on("error", reject);
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => resolve({
        status: res.statusCode,
        headers: res.headers,
        body: JSON.parse(Buffer.concat(chunks).toString("utf8")),
      }));
    });
  });
  req.end(body);
  return result;
}

async function request(pathname, { host, body } = {}) {
  const method = body === undefined ? "GET" : "POST";
  const raw = body === undefined ? undefined : JSON.stringify(body);
  const headers = raw === undefined ? {} : {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(raw),
  };
  if (!host) {
    return response(http.request({
      hostname: "127.0.0.1", port: proxyPort, path: pathname, method, headers,
    }), raw);
  }
  const connect = http.request({
    hostname: "127.0.0.1", port: proxyPort, method: "CONNECT", path: `${host}:443`,
  });
  connect.end();
  const [res, socket, head] = await once(connect, "connect");
  assert.equal(res.statusCode, 200);
  assert.equal(head.length, 0);
  const secure = tls.connect({
    socket, servername: host, ca: fs.readFileSync(path.join(here, "ca/ca.pem")),
  });
  await once(secure, "secureConnect");
  const agent = new https.Agent({ keepAlive: false });
  agent.createConnection = () => secure;
  try {
    return await response(https.request({
      hostname: host, path: pathname, method, headers, agent,
    }), raw);
  } finally {
    agent.destroy();
  }
}

before(async () => {
  gateway = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    gatewayRequests.push({
      path: req.url,
      body: JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}"),
    });
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ type: "message", content: [{ type: "text", text: "ok" }] }));
  });
  const gatewayPort = await listen(gateway);
  const reservation = net.createServer();
  proxyPort = await listen(reservation);
  await new Promise((resolve) => reservation.close(resolve));
  proxy = spawn(process.execPath, [
    path.join(here, "proxy.mjs"), "--port", String(proxyPort), "--mode", "mock", "--log", "0",
  ], {
    env: { ...process.env, CLAUDE_LOCAL_GATEWAY_URL: `http://127.0.0.1:${gatewayPort}` },
    stdio: ["ignore", "pipe", "pipe"],
  });
  proxyClosed = new Promise((resolve) => proxy.once("close", resolve));
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error("Proxy did not start")), 10000);
    const fail = (error) => {
      clearTimeout(timeout);
      reject(error);
    };
    proxy.once("error", fail);
    proxy.once("exit", (code) => fail(new Error(`Proxy exited with ${code}`)));
    proxy.stdout.on("data", (chunk) => {
      if (chunk.toString().includes("claude-local proxy:")) {
        clearTimeout(timeout);
        resolve();
      }
    });
  });
});

after(async () => {
  if (proxy?.pid && proxy.exitCode === null && proxy.signalCode === null) {
    proxy.kill("SIGTERM");
  }
  if (proxyClosed) await proxyClosed;
  if (gateway) await new Promise((resolve) => gateway.close(resolve));
});

for (const host of [undefined, "api.anthropic.com", "claude.ai"]) {
  test(`managed settings satisfy the CLI response contract over ${host || "direct HTTP"}`, async () => {
    const result = await request("/api/claude_code/settings", { host });
    assert.equal(result.status, 200);
    // Claude Code 2.1.317 requires all three fields; Desktop only checks settings.
    assert.equal(typeof result.body.uuid, "string");
    assert.notEqual(result.body.uuid, "");
    assert.equal(typeof result.body.checksum, "string");
    assert.deepEqual(result.body.settings, {});
    assert.equal(result.headers.etag, JSON.stringify(result.body.checksum));
    const again = await request("/api/claude_code/settings", { host });
    assert.equal(again.body.uuid, result.body.uuid);
  });
}

test("Code user settings keep the Desktop content.entries shape", async () => {
  const result = await request("/api/claude_code/organizations/local/user_settings", { host: "claude.ai" });
  assert.deepEqual(result.body.content.entries, {});
});

test("Code settings receive a top-level list of connected instances", async () => {
  const result = await request("/api/oauth/organizations/local/oauth_tokens?application_slug=claude-code", {
    host: "claude.ai",
  });
  assert.equal(result.status, 200);
  assert.deepEqual(result.body, []);
});

test("Cowork plugin metadata is a dictionary rather than a paginated list", async () => {
  const result = await request("/api/organizations/local/cowork_plugin_metadata", { host: "claude.ai" });
  assert.equal(result.status, 200);
  assert.deepEqual(result.body, {});
});

test("token counting stays local instead of reaching the model gateway", async () => {
  const before = gatewayRequests.length;
  const result = await request("/v1/messages/count_tokens", {
    body: { model: "local-model", messages: [{ role: "user", content: "hello" }] },
  });
  assert.equal(result.status, 200);
  assert.ok(Number.isInteger(result.body.input_tokens));
  assert.ok(result.body.input_tokens > 0);
  assert.equal(gatewayRequests.length, before);
});

test("model requests strip diagnostics and preserve the rest of the payload", async () => {
  const payload = {
    model: "local-model",
    messages: [{ role: "user", content: "hello" }],
    metadata: { user_id: "local-test" },
    max_tokens: 16,
  };
  const result = await request("/v1/messages?beta=true", {
    body: { ...payload, diagnostics: { source: "desktop" }, metadata: { ...payload.metadata, diagnostics: {} } },
  });
  assert.equal(result.status, 200);
  assert.deepEqual(gatewayRequests.at(-1), { path: "/v1/messages?beta=true", body: payload });
});
