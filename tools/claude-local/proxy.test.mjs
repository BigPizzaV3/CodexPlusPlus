import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import os from "node:os";
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
let dictationFixture;
let dictationFixturePort;
let dictationStateDir;
let dictationCacheDir;
let dictationAssetPath;
let dictationUpgrade;
let dictationFrame;

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

async function request(pathname, { host, body, method = body === undefined ? "GET" : "POST", beforeBody } = {}) {
  const raw = body === undefined ? undefined : JSON.stringify(body);
  const headers = raw === undefined ? {} : {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(raw),
  };
  const send = async (req) => {
    if (beforeBody) {
      req.setHeader("expect", "100-continue");
      const ready = once(req, "continue");
      req.flushHeaders();
      await ready;
      await beforeBody();
    }
    return response(req, raw);
  };
  if (!host) {
    return send(http.request({
      hostname: "127.0.0.1", port: proxyPort, path: pathname, method, headers,
    }));
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
    return await send(https.request({
      hostname: host, path: pathname, method, headers, agent,
    }));
  } finally {
    agent.destroy();
  }
}

async function requestText(host, pathname, headers = {}) {
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
    const result = await new Promise((resolve, reject) => {
      const req = https.request({
        hostname: host, path: pathname, method: "GET", headers: { ...headers, connection: "close" }, agent,
      });
      req.on("error", reject);
      req.on("response", (response) => {
        const chunks = [];
        response.on("error", reject);
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () => resolve({
          status: response.statusCode,
          headers: response.headers,
          body: Buffer.concat(chunks).toString("utf8"),
        }));
      });
      req.end();
    });
    return result;
  } finally {
    agent.destroy();
  }
}

function websocketAccept(key) {
  return crypto.createHash("sha1")
    .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
    .digest("base64");
}

function websocketTextFrame(text) {
  const payload = Buffer.from(text);
  assert.ok(payload.length < 126);
  return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
}

function websocketCloseFrame(code = 1000) {
  return Buffer.from([0x88, 0x02, (code >> 8) & 0xff, code & 0xff]);
}

function websocketClientFrame(payload) {
  const data = Buffer.from(payload);
  assert.ok(data.length < 126);
  const mask = crypto.randomBytes(4);
  const frame = Buffer.alloc(2 + mask.length + data.length);
  frame[0] = 0x82;
  frame[1] = 0x80 | data.length;
  mask.copy(frame, 2);
  for (let index = 0; index < data.length; index += 1) {
    frame[6 + index] = data[index] ^ mask[index % 4];
  }
  return frame;
}

async function openTlsUpgrade({
  host = "claude.ai",
  pathname = "/api/ws/speech_to_text/voice_stream",
  origin = "https://claude.ai",
  extraHeaders = {},
} = {}) {
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
  const key = crypto.randomBytes(16).toString("base64");
  const headers = {
    Host: host,
    Upgrade: "websocket",
    Connection: "Upgrade",
    "Sec-WebSocket-Key": key,
    "Sec-WebSocket-Version": "13",
    ...(origin ? { Origin: origin } : {}),
    ...extraHeaders,
  };
  const rawHeaders = Object.entries(headers).map(([name, value]) => `${name}: ${value}\r\n`).join("");
  const responseBytes = new Promise((resolve, reject) => {
    let bytes = Buffer.alloc(0);
    const timeout = setTimeout(() => reject(new Error("WebSocket upgrade timed out")), 5000);
    const onData = (chunk) => {
      bytes = Buffer.concat([bytes, chunk]);
      if (bytes.includes(Buffer.from("\r\n\r\n"))) {
        clearTimeout(timeout);
        secure.off("error", onError);
        secure.off("data", onData);
        resolve(bytes);
      }
    };
    const onError = (error) => {
      clearTimeout(timeout);
      secure.off("data", onData);
      reject(error);
    };
    secure.on("data", onData);
    secure.once("error", onError);
  });
  secure.write(`GET ${pathname} HTTP/1.1\r\n${rawHeaders}\r\n`);
  const response = await responseBytes;
  const headerEnd = response.indexOf(Buffer.from("\r\n\r\n")) + 4;
  return {
    secure,
    key,
    status: response.subarray(0, headerEnd).toString("latin1"),
    remainder: response.subarray(headerEnd),
  };
}

before(async () => {
  dictationStateDir = fs.mkdtempSync(path.join(os.tmpdir(), "claude-local-dictation-"));
  dictationCacheDir = fs.mkdtempSync(path.join(os.tmpdir(), "claude-local-cache-"));
  const assetHost = "assets-proxy.anthropic.com";
  dictationAssetPath = "/claude-ai/v2/assets/v1/c342ae4ef-ROR3Yba4.js";
  const assetCacheKey = `${assetHost}${dictationAssetPath}`;
  const assetCacheName = crypto.createHash("sha1").update(assetCacheKey).digest("hex");
  const assetBody = Buffer.from('import{ph as e}from"./preload-helper-test.js";const D=125e3;const A=6e4;');
  fs.writeFileSync(path.join(dictationCacheDir, assetCacheName), assetBody, { mode: 0o600 });
  fs.writeFileSync(
    path.join(dictationCacheDir, `${assetCacheName}.json`),
    JSON.stringify({
      status: 200,
      headers: { "content-type": "text/javascript; charset=utf-8" },
      url: assetCacheKey,
    }),
    { mode: 0o600 },
  );
  dictationFixture = http.createServer();
  dictationFixture.on("upgrade", (req, socket, head) => {
    dictationUpgrade = {
      path: req.url,
      token: req.headers["x-codex-plus-dictation-token"],
      cookie: req.headers.cookie,
      authorization: req.headers.authorization,
      host: req.headers.host,
    };
    socket.write(
      "HTTP/1.1 101 Switching Protocols\r\n" +
      "Upgrade: websocket\r\n" +
      "Connection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${websocketAccept(req.headers["sec-websocket-key"])}\r\n\r\n`,
    );
    if (head.length) socket.write(head);
    let replied = false;
    socket.on("data", (chunk) => {
      dictationFrame = Buffer.concat([dictationFrame || Buffer.alloc(0), chunk]);
      if (!replied) {
        replied = true;
        socket.end(Buffer.concat([websocketTextFrame("helper-ok"), websocketCloseFrame()]));
      }
    });
  });
  dictationFixturePort = await listen(dictationFixture);
  fs.writeFileSync(
    path.join(dictationStateDir, "dictation-helper.json"),
    JSON.stringify({ port: dictationFixturePort, token: "123e4567-e89b-42d3-a456-426614174000" }),
    { mode: 0o600 },
  );

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
    env: {
      ...process.env,
      CLAUDE_LOCAL_GATEWAY_URL: `http://127.0.0.1:${gatewayPort}`,
      CLAUDE_LOCAL_CACHE_DIR: dictationCacheDir,
      CODEX_PLUS_STATE_DIR: dictationStateDir,
    },
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
  if (dictationFixture) await new Promise((resolve) => dictationFixture.close(resolve));
  if (dictationStateDir) {
    fs.unlinkSync(path.join(dictationStateDir, "dictation-helper.json"));
    fs.rmdirSync(dictationStateDir);
  }
  if (dictationCacheDir) {
    for (const file of fs.readdirSync(dictationCacheDir)) {
      fs.unlinkSync(path.join(dictationCacheDir, file));
    }
    fs.rmdirSync(dictationCacheDir);
  }
});

test("mock mode serves the patched dictation module with the CDN cross-origin contract", async () => {
  const result = await requestText("assets-proxy.anthropic.com", dictationAssetPath, {
    origin: "https://claude.ai",
  });
  assert.equal(result.status, 200);
  assert.equal(result.headers["access-control-allow-origin"], "*");
  assert.match(result.headers["content-type"], /^text\/javascript\b/);
  assert.equal(result.headers["cache-control"], "no-store");
  assert.equal(Number(result.headers["content-length"]), Buffer.byteLength(result.body));
  assert.match(result.body, /D=125e3/);
  assert.doesNotMatch(result.body, /D=2e3/);
});

test("Claude dictation websocket is authorized locally and forwards only bounded metadata", async () => {
  const clientFrame = websocketClientFrame("synthetic-pcm-frame");
  const result = await openTlsUpgrade({
    pathname: "/api/ws/speech_to_text/voice_stream?language=zh&stt_provider=deepgram-nova3&organization_uuid=private",
    extraHeaders: { Cookie: "session=private", Authorization: "Bearer private" },
  });
  assert.match(result.status, /^HTTP\/1\.1 101 /);
  assert.equal(result.status.includes("123e4567-e89b-42d3-a456-426614174000"), false);
  assert.deepEqual(dictationUpgrade, {
    path: "/dictation/claude-stream?language=zh",
    token: "123e4567-e89b-42d3-a456-426614174000",
    cookie: undefined,
    authorization: undefined,
    host: `127.0.0.1:${dictationFixturePort}`,
  });
  dictationFrame = undefined;
  const clientResponse = new Promise((resolve, reject) => {
    let bytes = result.remainder;
    const timeout = setTimeout(() => reject(new Error("helper response did not close cleanly")), 3000);
    const onData = (chunk) => {
      bytes = Buffer.concat([bytes, chunk]);
    };
    const onClose = () => {
      clearTimeout(timeout);
      result.secure.off("error", onError);
      result.secure.off("data", onData);
      resolve(bytes);
    };
    const onError = (error) => {
      clearTimeout(timeout);
      result.secure.off("data", onData);
      result.secure.off("close", onClose);
      reject(error);
    };
    result.secure.on("data", onData);
    result.secure.once("error", onError);
    result.secure.once("close", onClose);
  });
  result.secure.write(clientFrame);
  const responseBytes = await clientResponse;
  assert.ok(dictationFrame.includes(clientFrame));
  assert.ok(responseBytes.includes(websocketTextFrame("helper-ok")));
  assert.ok(responseBytes.includes(websocketCloseFrame()));
});

test("Claude dictation websocket rejects untrusted origins before reaching the helper", async () => {
  const previous = dictationUpgrade;
  const result = await openTlsUpgrade({ origin: "https://attacker.example" });
  assert.match(result.status, /^HTTP\/1\.1 403 /);
  assert.equal(dictationUpgrade, previous);
  result.secure.destroy();
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

test("local mock accounts do not request identity verification", async () => {
  const result = await request("/api/organizations/local/kyc_status", { host: "claude.ai" });
  assert.equal(result.status, 200);
  assert.deepEqual(result.body, { status: "not_required", should_poll: false });
});

test("Desktop receives the native Computer Use rollout without enabling permissions", async () => {
  const result = await request("/api/desktop/features", { host: "claude.ai" });
  assert.equal(result.status, 200);
  assert.equal(result.body.features["1291166712"].value.enabled, true);
  assert.equal(result.body.features["1291166712"].on, true);
  assert.equal(result.body.features["1291166712"].off, false);
  assert.equal(result.body.features["1291166712"].source, "force");
  assert.deepEqual(Object.keys(result.body.features), ["1291166712"]);
  assert.equal(result.body.chicagoEnabled, undefined);
});

test("Desktop exposes the separate background-mode capability without selecting a user mode", async () => {
  const result = await request("/api/desktop/features", { host: "claude.ai" });
  assert.equal(result.status, 200);
  assert.deepEqual(result.body.features["1291166712"].value, {
    enabled: true, appScopedEnabled: true,
  });
  assert.equal(result.body.chicagoPreferredMode, undefined);
});

test("account settings receive deletion eligibility and trusted-device lists", async () => {
  const eligibility = await request("/api/account/deletion-allowed", { host: "claude.ai" });
  assert.equal(eligibility.status, 200);
  assert.equal(eligibility.body.deletable, false);
  assert.deepEqual(eligibility.body.reasons, []);
  const devices = await request("/api/auth/trusted_devices", { host: "claude.ai" });
  assert.deepEqual(devices.body.devices, []);
});

test("account security receives an explicit list of local MFA methods", async () => {
  const result = await request("/api/auth/mfa", { host: "claude.ai" });
  assert.equal(result.status, 200);
  assert.deepEqual(result.body.methods, []);
  assert.equal(result.body.methods.find((entry) => entry.method === "sms"), undefined);
  assert.equal(result.body.mfa_enrolled, false);
});

test("account sessions receive a valid empty paginated response", async () => {
  const result = await request("/api/auth/sessions/list-active", { host: "claude.ai" });
  assert.equal(result.status, 200);
  assert.deepEqual(result.body, { data: [], pagination: { has_more: false, total: 0 } });
});

test("billing settings receive an empty invoice page without undefined rows", async () => {
  const result = await request("/api/stripe/local/invoices?limit=12&page=", { host: "claude.ai" });
  assert.equal(result.status, 200);
  assert.deepEqual(result.body, { invoices: [], next_page: null });
  assert.deepEqual([result.body].flatMap((page) => page.invoices), []);
});

test("Artifacts receives a concrete artifact list and count", async () => {
  const result = await request("/api/organizations/local/user_artifacts?limit=30&offset=0&include_latest_published_artifact_uuid=true", {
    host: "claude.ai",
  });
  assert.equal(result.status, 200);
  assert.deepEqual([...result.body.artifacts], []);
  const count = await request("/api/organizations/local/user_artifacts/count", { host: "claude.ai" });
  assert.equal(count.body.count, 0);
});

test("Artifacts shared galleries receive frame arrays instead of load-error states", async () => {
  for (const endpoint of ["/api/frame/frames", "/api/frame/frames/external"]) {
    const result = await request(endpoint, { host: "claude.ai" });
    assert.equal(result.status, 200);
    assert.deepEqual(result.body.frames, []);
  }
});

test("general settings can read and update notification preferences without losing other channels", async () => {
  const endpoint = "/api/organizations/notification-test/notification/preferences";
  const initial = await request(endpoint, { host: "claude.ai" });
  assert.deepEqual(initial.body.preferences.feature_preference, {});
  assert.deepEqual(initial.body.effective_preferences, {});
  const patch = (feature_preference) => request(endpoint, {
    host: "claude.ai", method: "PATCH", body: { preferences: { feature_preference } },
  });
  await patch({ completion: { enable_push: true }, compass: { enable_push: true } });
  await patch({ completion: { enable_email: true } });
  const updated = await patch({ completion: { enable_push: false } });
  assert.equal(updated.status, 200);
  assert.deepEqual(updated.body.preferences.feature_preference, {
    completion: { enable_push: false, enable_email: true }, compass: { enable_push: true },
  });
  assert.deepEqual(updated.body.effective_preferences.completion, {
    enable_push: false, push_source: "explicit", enable_email: true, email_source: "explicit",
  });
  const again = await request(endpoint, { host: "claude.ai" });
  assert.deepEqual(again.body, updated.body);
  const other = await request("/api/organizations/other/notification/preferences", { host: "claude.ai" });
  assert.deepEqual(other.body.preferences.feature_preference, {});
});

test("concurrent first notification updates preserve both features", async () => {
  const endpoint = "/api/organizations/concurrent-notifications/notification/preferences";
  // 先确认服务端已接收首请求，再让第二个 PATCH 抢先完成。
  await request(endpoint, {
    host: "claude.ai", method: "PATCH",
    body: { preferences: { feature_preference: { completion: { enable_push: true } } } },
    beforeBody: () => request(endpoint, {
      host: "claude.ai", method: "PATCH",
      body: { preferences: { feature_preference: { compass: { enable_email: true } } } },
    }),
  });
  const result = await request(endpoint, { host: "claude.ai" });
  assert.deepEqual(result.body.preferences.feature_preference, {
    completion: { enable_push: true }, compass: { enable_email: true },
  });
});

test("notification updates reject malformed envelopes and preserve safe dictionaries", async () => {
  const endpoint = "/api/organizations/notification-validation/notification/preferences";
  const invalid = await request(endpoint, { host: "claude.ai", method: "PATCH", body: { preferences: { feature_preference: [] } } });
  assert.equal(invalid.status, 400);
  const result = await request(endpoint, {
    host: "claude.ai", method: "PATCH",
    body: { preferences: { feature_preference: {
      ["__proto__"]: { enable_push: true },
      constructor: { enable_email: false },
      completion: { enable_push: "true", enable_email: false },
    } } },
  });
  assert.deepEqual(result.body.preferences.feature_preference.completion, { enable_email: false });
  assert.deepEqual(result.body.preferences.feature_preference.__proto__, { enable_push: true });
  const clean = await request("/api/organizations/notification-clean/notification/preferences", { host: "claude.ai" });
  assert.deepEqual(clean.body.preferences.feature_preference, {});
});

test("Chinese web catalogs cover Code settings with both long and short message IDs", async () => {
  const result = await request("/i18n/s/zh-Hans.json?v=local-test", { host: "claude.ai" });
  assert.equal(result.status, 200);
  assert.equal(result.body.mnLinrjnq9, "没有已连接的 Claude Code 实例");
  assert.equal(result.body.mnLinr, result.body.mnLinrjnq9);
  assert.equal(result.body.jqhhMI, "设置");
  assert.equal(result.body.iTM1PGTIqQ, "授权令牌");
  assert.ok(Object.keys(result.body).length > 30000);
});

test("Chinese full catalogs preserve navigation translations under long message IDs", async () => {
  const result = await request("/i18n/zh-Hans.json", { host: "claude.ai" });
  assert.equal(result.body.jqhhMImrPt, "设置");
  assert.equal(result.body.y1Z3orIe9Z, "语言");
});

test("dynamic catalogs contain dynamic translations rather than Desktop shell messages", async () => {
  const result = await request("/i18n/dynamic/zh-Hans.json", { host: "claude.ai" });
  assert.equal(result.body["0gtp5o1nEE"], "复杂、细致的工作");
  assert.equal(result.body["+/cwsayrqk"], undefined);
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
