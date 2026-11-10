import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import http from "node:http";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import tls from "node:tls";
import { fileURLToPath } from "node:url";
import { after, before, test } from "node:test";
import { decode, get, asMsg, asStr, ld, vi } from "./rpc.mjs";
import { createOfficialAssets } from "./official-assets.mjs";
import { createStateStore, dropUnroutedModels, gatewayModel, gatewayTarget, modelLabel, modelSelectors, modelWindow, planProfile, runtimeConfig } from "./runtime.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const modelIds = ["alpha/claude-opus-4-6", "beta/gpt-6", "gamma/grok-4", "alpha/shared", "beta/shared"];
const modelWindows = {
  [modelIds[0]]: 1_000_000, [modelIds[1]]: 1_100_000, [modelIds[2]]: 256_000,
  [modelIds[3]]: 128_000, [modelIds[4]]: 512_000,
};
let root;
let gateway;
let gatewayPort;
let service;
const received = [];

function writeAsar(resourcesDir, members) {
  const tree = { files: {} };
  const bodies = [];
  let offset = 0;
  for (const [member, source] of Object.entries(members)) {
    const body = Buffer.from(source);
    const parts = member.split("/");
    let directory = tree.files;
    for (const part of parts.slice(0, -1)) {
      directory[part] ??= { files: {} };
      directory = directory[part].files;
    }
    directory[parts.at(-1)] = { size: body.length, offset: String(offset) };
    bodies.push(body);
    offset += body.length;
  }
  const json = Buffer.from(JSON.stringify(tree));
  const paddedJsonSize = Math.ceil(json.length / 4) * 4;
  const headerSize = 8 + paddedJsonSize;
  const payloadStart = 8 + headerSize;
  const prefix = Buffer.alloc(payloadStart);
  prefix.writeUInt32LE(4, 0);
  prefix.writeUInt32LE(headerSize, 4);
  prefix.writeUInt32LE(4 + paddedJsonSize, 8);
  prefix.writeUInt32LE(json.length, 12);
  json.copy(prefix, 16);
  fs.mkdirSync(resourcesDir, { recursive: true });
  fs.writeFileSync(path.join(resourcesDir, "ion-dist.asar"), Buffer.concat([prefix, ...bodies]));
}

async function request(instance, pathname, { host, body, type = "application/json", method = body === undefined ? "GET" : "POST", extraHeaders = {}, auth = true } = {}) {
  const raw = body === undefined ? null : Buffer.isBuffer(body) ? body : Buffer.from(JSON.stringify(body));
  const headers = {
    ...(raw ? { "content-type": type, "content-length": raw.length } : {}),
    ...(!host && auth && pathname !== "/__codex_claude_local/health" ? { authorization: "Bearer test-local-gateway-token" } : {}),
    ...extraHeaders,
  };
  let agent;
  if (host) {
    const connect = http.request({ host: "127.0.0.1", port: instance.port, method: "CONNECT", path: `${host}:443` });
    connect.end();
    const [response, socket] = await once(connect, "connect");
    assert.equal(response.statusCode, 200);
    const secure = tls.connect({ socket, servername: host, ca: fs.readFileSync(path.join(here, "ca/ca.pem")) });
    await once(secure, "secureConnect");
    agent = new https.Agent({ keepAlive: false });
    agent.createConnection = () => secure;
  }
  try {
    return await new Promise((resolve, reject) => {
      const req = (host ? https : http).request({
        hostname: host || "127.0.0.1", port: host ? 443 : instance.port,
        path: pathname, method, headers, ...(agent ? { agent } : {}),
      }, (res) => {
        const chunks = [];
        res.on("error", reject);
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => {
          const bytes = Buffer.concat(chunks);
          resolve({
            status: res.statusCode, bytes,
            headers: res.headers,
            json: String(res.headers["content-type"]).includes("json") ? JSON.parse(bytes.toString("utf8")) : null,
          });
        });
      });
      req.on("error", (error) => reject(new Error(`${method} ${pathname}: ${error.message}\n${service?.output?.() ?? ""}`)));
      req.end(raw);
    });
  } finally {
    agent?.destroy();
  }
}

async function start(extra = {}) {
  const ready = path.join(root, "ready.json");
  const instanceId = `managed-test-${Date.now()}`;
  const child = spawn(process.execPath, [path.join(root, "runtime", "proxy.mjs"), "--port", "0", "--mode", "mock", "--log", "0", "--ca", path.join(here, "ca")], {
    env: {
      ...process.env,
      CLAUDE_LOCAL_MANAGED: "1",
      CLAUDE_LOCAL_DESKTOP_RESOURCES: path.join(root, "desktop-resources"),
      CLAUDE_LOCAL_READY_FILE: ready,
      CLAUDE_LOCAL_INSTANCE_ID: instanceId,
      CLAUDE_LOCAL_STATE_FILE: path.join(root, "state.json"),
      CLAUDE_LOCAL_CACHE_DIR: path.join(root, "cache"),
      CLAUDE_LOCAL_WEB_I18N_DIR: path.join(root, "catalogs"),
      CLAUDE_LOCAL_GATEWAY_URL: `http://127.0.0.1:${gatewayPort}/gw/desktop`,
      CLAUDE_LOCAL_GATEWAY_KEY: "test-local-gateway-token",
      CLAUDE_LOCAL_GATEWAY_MODEL: modelIds[0],
      CLAUDE_LOCAL_GATEWAY_MODELS: JSON.stringify(modelIds),
      CLAUDE_LOCAL_GATEWAY_MODEL_WINDOWS: JSON.stringify(modelWindows),
      CLAUDE_LOCAL_LOCALIZATION: "1",
      CLAUDE_LOCAL_VOICE_INPUT: "0",
      CLAUDE_LOCAL_COMPUTER_USE: "0",
      CLAUDE_LOCAL_BACKGROUND_MODE: "0",
      ...extra,
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const closed = once(child, "exit");
  let output = "";
  child.stdout.on("data", (chunk) => { output += chunk.toString("utf8"); });
  child.stderr.on("data", (chunk) => { output += chunk.toString("utf8"); });
  for (let count = 0; count < 500; count++) {
    if (child.exitCode !== null) throw new Error(`Managed fixture exited: ${output}`);
    try {
      const receipt = JSON.parse(fs.readFileSync(ready, "utf8"));
      if (receipt.instanceId === instanceId) {
        assert.equal(receipt.pid, child.pid);
        assert.ok(receipt.port > 0);
        return { ...receipt, child, closed, output: () => output };
      }
    } catch {}
    await delay(10);
  }
  child.kill("SIGTERM");
  await closed;
  throw new Error("Managed fixture did not become ready");
}

async function stop(instance, signal = false) {
  if (!instance || instance.child.exitCode !== null) return;
  if (signal) instance.child.kill("SIGTERM");
  else instance.child.stdin.end();
  const result = await Promise.race([instance.closed, delay(5000).then(() => { throw new Error("Managed fixture did not exit"); })]);
  assert.equal(result[0], 0);
}

const header = (conv) => ld(1, Buffer.concat([ld(1, Buffer.concat([ld(1, "test-session"), vi(2, 1)])), ld(2, conv)]));
const send = (conv, prompt, model) => Buffer.concat([
  header(conv),
  ld(2, Buffer.concat([
    ld(1, `user-${crypto.randomUUID()}`), ld(2, `assistant-${crypto.randomUUID()}`), ld(3, prompt),
    ...(model ? [ld(7, ld(2, model))] : []),
  ])),
]);

before(async () => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), "claude-managed-"));
  writeAsar(path.join(root, "desktop-resources"), {
    "index.html": '<!doctype html><html data-build-id="fixture-build"><head><meta charset="UTF-8"><link rel="preload" href="/assets/v1/main.js"><script>window.fixtureAppStarted = true;</script><script type="module" src="/assets/v1/main.js"></script></head><body><div id="root"></div></body></html>',
    "assets/v1/main.js": "export const localOfficialAsset = true;",
    "i18n/en-US.json": JSON.stringify({ greeting: "Hello" }),
    "i18n/dynamic/en-US.json": JSON.stringify({ dynamic: "Hello" }),
    "images/icon.png": Buffer.from([0x89, 0x50, 0x4e, 0x47]),
    "audio/tone.mp3": Buffer.from([0x49, 0x44, 0x33]),
  });
  fs.mkdirSync(path.join(root, "runtime", "scripts"), { recursive: true });
  for (const file of ["proxy.mjs", "conversation-service.mjs", "rpc.mjs", "runtime.mjs", "official-assets.mjs", "scripts/patch-dictation-start.mjs"]) {
    fs.copyFileSync(path.join(here, file), path.join(root, "runtime", file));
  }
  fs.copyFileSync(
    path.join(here, "../../assets/claude-localization/page-shim.js"),
    path.join(root, "runtime", "page-shim.js"),
  );
  fs.mkdirSync(path.join(root, "runtime", "mock", "api"), { recursive: true });
  fs.writeFileSync(path.join(root, "runtime", "mock", "bootstrap.json"), JSON.stringify({ privateFixture: "must-not-leak" }));
  fs.writeFileSync(path.join(root, "runtime", "mock", "api", "account_profile.json"), JSON.stringify({ privateFixture: "must-not-leak" }));
  fs.mkdirSync(path.join(root, "catalogs"), { recursive: true });
  fs.writeFileSync(path.join(root, "catalogs", "zh-Hans.json"), JSON.stringify({ message: "translated" }));
  fs.mkdirSync(path.join(root, "cache"), { recursive: true });
  fs.writeFileSync(path.join(root, "cache", "private-recorded.json"), JSON.stringify({ url: "claude.ai/edge-api/bootstrap", status: 200 }));
  fs.writeFileSync(path.join(root, "cache", "private-recorded"), JSON.stringify({ privateFixture: "must-not-leak" }));
  const oldLoginKey = crypto.createHash("sha1").update("claude.ai/login").digest("hex");
  fs.writeFileSync(path.join(root, "cache", oldLoginKey), "<html>stale private login shell</html>");
  fs.writeFileSync(path.join(root, "cache", `${oldLoginKey}.json`), JSON.stringify({ status: 200, headers: { "content-type": "text/html" }, url: "claude.ai/login" }));
  gateway = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const raw = Buffer.concat(chunks).toString("utf8");
    received.push({ path: req.url, headers: req.headers, body: raw ? JSON.parse(raw) : null });
    res.writeHead(200, { "content-type": "text/event-stream" });
    res.end('data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"gateway reply"}}\n\ndata: {"type":"message_stop"}\n\n');
  });
  gateway.listen(0, "127.0.0.1");
  await once(gateway, "listening");
  gatewayPort = gateway.address().port;
  service = await start();
});

after(async () => {
  await stop(service);
  gateway?.closeAllConnections();
  await new Promise((resolve) => gateway?.close(resolve));
});

test("managed readiness reports an assigned port and health polling is not client traffic", async () => {
  const first = await request(service, "/__codex_claude_local/health");
  const second = await request(service, "/__codex_claude_local/health");
  assert.equal(first.json.instanceId, service.instanceId);
  assert.equal(first.json.pid, service.child.pid);
  assert.equal(first.json.port, service.port);
  assert.equal(first.json.lastClientAt, null);
  assert.equal(second.json.lastClientAt, null);
  assert.equal(Object.hasOwn(first.json, "token"), false);
  assert.equal(fs.statSync(path.join(root, "ready.json")).mode & 0o777, 0o600);
  await request(service, "/api/bootstrap", { host: "claude.ai" });
  const active = await request(service, "/__codex_claude_local/health");
  assert.equal(typeof active.json.lastClientAt, "string");
});

test("fresh managed profile serves the installed official shell and bounded ASAR resources", async () => {
  const resources = createOfficialAssets(path.join(root, "desktop-resources"));
  assert.equal(resources.buildId, "fixture-build");
  assert.deepEqual(resources.i18nLocales, ["en-US"]);
  assert.equal(resources.read("/assets/%2e%2e%2fi18n/s/zh-Hans.json"), null);
  assert.equal(resources.read("/assets%2fv1/main.js"), null);
  resources.close();

  const shell = await request(service, "/new", { host: "claude.ai" });
  assert.equal(shell.status, 200);
  const shellHtml = shell.bytes.toString("utf8");
  assert.match(shellHtml, /data-build-id="fixture-build"/);
  assert.doesNotMatch(shellHtml, /stale private login shell/);
  assert.match(shellHtml, /<meta name="i18n-catalogs" data-i18n-catalogs="en-US:1\.1\.1,zh-Hans:1\.1\.1">/);
  const shimIndex = shellHtml.indexOf('<script src="/__codex_plus/claude-localization.js"></script>');
  const firstAppScript = shellHtml.indexOf("<script>");
  assert.ok(shellHtml.indexOf('name="i18n-catalogs"') < firstAppScript);
  assert.ok(shimIndex > shellHtml.indexOf('<meta charset="UTF-8">'));
  assert.ok(shimIndex < firstAppScript);
  assert.equal(shell.headers["content-security-policy"], undefined);

  const shim = await request(service, "/__codex_plus/claude-localization.js", { host: "claude.ai" });
  assert.equal(shim.status, 200);
  assert.equal(shim.headers["content-type"], "text/javascript; charset=utf-8");
  assert.equal(shim.headers["x-content-type-options"], "nosniff");
  assert.equal(shim.headers["cache-control"], "no-store");
  assert.deepEqual(shim.bytes, fs.readFileSync(path.join(root, "runtime", "page-shim.js")));
  assert.match(shim.bytes.toString("utf8"), /spa:locale/);
  assert.equal((await request(service, "/__codex_plus/claude-localization.js", {
    host: "claude.ai", method: "HEAD",
  })).status, 200);
  assert.equal((await request(service, "/__codex_plus/other.js", { host: "claude.ai" })).status, 404);
  assert.equal((await request(service, "/__codex_plus/claude-localization.js", {
    host: "claude.ai", method: "POST", body: {},
  })).status, 404);

  const route = await request(service, "/settings/general", { host: "claude.ai" });
  assert.equal(route.status, 200);
  const routeHtml = route.bytes.toString("utf8");
  assert.match(routeHtml, /data-build-id="fixture-build"/);
  assert.equal([...routeHtml.matchAll(/<script src="\/__codex_plus\/claude-localization\.js"><\/script>/g)].length, 1);

  for (const [host, pathname] of [
    ["claude.ai", "/i18n/zh-Hans.json?v=1.1"],
    ["claude.ai", "/i18n/dynamic/zh-Hans.json?v=1.1"],
    ["assets-proxy.anthropic.com", "/claude-ai/v2/i18n/zh-Hans.json?v=1.1"],
    ["assets-proxy.anthropic.com", "/claude-ai/v2/i18n/dynamic/zh-Hans.json?v=1.1"],
  ]) {
    const catalog = await request(service, pathname, { host });
    assert.equal(catalog.status, 200, `${host}${pathname}: ${catalog.bytes.toString("utf8").slice(0, 160)}`);
    assert.deepEqual(catalog.json, host === "claude.ai" && pathname.includes("/dynamic/")
      || host === "assets-proxy.anthropic.com" && pathname.includes("/dynamic/")
      ? {}
      : { message: "translated" });
    assert.equal(host === "assets-proxy.anthropic.com" ? catalog.headers["access-control-allow-origin"] : undefined,
      host === "assets-proxy.anthropic.com" ? "*" : undefined);
  }

  const asset = await request(service, "/assets/v1/main.js", { host: "claude.ai" });
  assert.equal(asset.status, 200);
  assert.equal(asset.bytes.toString("utf8"), "export const localOfficialAsset = true;");
  assert.match(asset.bytes.toString("utf8"), /localOfficialAsset/);
  const cdnAsset = await request(service, "/claude-ai/v2/assets/v1/main.js", { host: "assets-proxy.anthropic.com" });
  assert.equal(cdnAsset.status, 200);
  assert.equal(cdnAsset.bytes.toString("utf8"), asset.bytes.toString("utf8"));
  const missing = await request(service, "/assets/v1/missing.js", { host: "claude.ai" });
  assert.equal(missing.status, 404);

  const bootstrap = await request(service, "/api/bootstrap", { host: "claude.ai" });
  assert.equal(bootstrap.json.gated_imports_build_id, "fixture-build");
});

test("clean managed runtime uses synthetic bootstrap and configured catalogs without private recordings", async () => {
  for (const pathname of ["/edge-api/bootstrap", "/api/bootstrap", "/api/bootstrap/local/app_start"]) {
    const result = await request(service, pathname, { host: "claude.ai" });
    assert.equal(result.status, 200);
    assert.equal(result.json.privateFixture, undefined);
    assert.equal(result.json.account.email_address, "local@claude.local");
    assert.equal(result.json.locale, "zh-Hans");
    assert.deepEqual(result.json.gated_imports, {});
    assert.deepEqual(result.json.current_user_access.features, []);
    assert.equal(result.json.activeOrganization.uuid, result.json.resolved_org_uuid);
    assert.equal(result.json.activeOrganization.uuid, result.json.account.organization_uuid);
    assert.equal(result.json.account.memberships[0].organization.uuid, result.json.activeOrganization.uuid);
    assert.equal(result.json.account.memberships[0].status, "active");
    assert.equal(result.json.account.memberships[0].organization.capabilities.includes("chat"), true);
    assert.equal(result.json.account.workspace_memberships.length, 1);
    assert.equal(result.json.user.memberships[0].organization.uuid, result.json.activeOrganization.uuid);
    assert.ok(result.headers["set-cookie"].some((cookie) => cookie.startsWith(`lastActiveOrg=${result.json.activeOrganization.uuid};`)));
    assert.deepEqual(result.json.model_selector_config.find((entry) => entry.id === "chat").models.map((model) => model.id), modelIds);
    assert.ok(result.json.model_selector_config[0].models.every((model) => model.capabilities.mm_images === false));
  }
  const profile = await request(service, "/api/account_profile", { host: "claude.ai" });
  assert.equal(profile.json.privateFixture, undefined);
  assert.equal(profile.json.conversation_preferences, null);
  assert.equal(profile.json.display_name, "Local User");
  assert.equal(profile.json.full_name, "Local User");
  assert.equal(profile.json.organization_uuid, "e0e877d3-8475-44a1-a501-4fc431b085ab");
  assert.equal(profile.json.membership.organization.uuid, profile.json.organization_uuid);
  assert.ok(profile.headers["set-cookie"].some((cookie) => cookie.startsWith(`lastActiveOrg=${profile.json.organization_uuid};`)));
  const catalog = await request(service, "/i18n/s/zh-Hans.json", { host: "claude.ai" });
  assert.deepEqual(catalog.json, { message: "translated" });
  const chromeLocaleCatalog = await request(service, "/i18n/s/zh-CN.json", { host: "claude.ai" });
  assert.deepEqual(chromeLocaleCatalog.json, { message: "translated" });
  const defaultLocaleCatalog = await request(service, "/i18n/s/en-US.json", { host: "claude.ai" });
  assert.deepEqual(defaultLocaleCatalog.json, { message: "translated" });
});

test("managed organization discovery stays on the already joined local organization", async () => {
  const discoverable = await request(service, "/api/organizations/discoverable", { host: "claude.ai" });
  assert.equal(discoverable.status, 200);
  assert.deepEqual(discoverable.json.organizations, []);
  assert.deepEqual(discoverable.json.data, []);
  assert.deepEqual(discoverable.json.items, []);
  assert.ok(discoverable.headers["set-cookie"].some((cookie) => cookie.startsWith("lastActiveOrg=e0e877d3-8475-44a1-a501-4fc431b085ab;")));

  const joined = await request(service, "/api/organizations/e0e877d3-8475-44a1-a501-4fc431b085ab/join", {
    host: "claude.ai", method: "POST", body: {},
  });
  assert.equal(joined.status, 200);
  assert.equal(joined.json.joined, true);
  assert.equal(joined.json.is_member, true);
  assert.equal(joined.json.membership.organization.uuid, "e0e877d3-8475-44a1-a501-4fc431b085ab");
  assert.equal(joined.json.membership.status, "active");
});

test("managed feature flags are independently reflected without granting system permissions", async () => {
  const result = await request(service, "/api/desktop/features", { host: "claude.ai" });
  assert.deepEqual(result.json.features["1291166712"].value, { enabled: false, appScopedEnabled: false });
  assert.equal(result.json.chicagoEnabled, undefined);
  const bootstrap = await request(service, "/api/bootstrap", { host: "claude.ai" });
  assert.equal(bootstrap.json.feature_flags.computer_use, false);
  assert.equal(bootstrap.json.account.capabilities.includes("computer_use"), false);
});

test("managed usage returns the frontend limits contract without invented subscription usage", async () => {
  const usage = await request(service, "/api/organizations/local/usage?skip_spend=1", { host: "claude.ai" });
  assert.equal(usage.status, 200);
  assert.deepEqual(usage.json.limits, []);
  assert.equal(usage.json.five_hour, null);
  assert.equal(usage.json.extra_usage.used_credits, null);
  assert.equal(usage.json.windows, undefined);
  const bootstrap = await request(service, "/edge-api/bootstrap", { host: "claude.ai" });
  assert.equal(bootstrap.json.served_copy["web.plan_name.max_20x"], "Codex++ 本地网关");
});

test("managed model selector responses include per-model runtime windows", async () => {
  for (const host of ["claude.ai", "api.anthropic.com"]) {
    for (const surface of ["cc", "ccd", "ccr"]) {
      for (const prefix of ["/api", "/api/organizations/local"]) {
        const result = await request(service, `${prefix}/model_selector/${surface}`, { host });
        assert.equal(result.status, 200);
        const [config] = result.json.model_selector_config;
        assert.equal(config.id, surface);
        assert.deepEqual(config.models.map((entry) => [entry.id, entry.context_window]), Object.entries(modelWindows));
        assert.deepEqual(config.models.map((entry) => [entry.id, entry.runtime.max_input_tokens]), Object.entries(modelWindows));
      }
    }
  }
});

test("model window metadata validates configured values", () => {
  const config = runtimeConfig({
    CLAUDE_LOCAL_MANAGED: "1",
    CLAUDE_LOCAL_GATEWAY_URL: "https://localhost/gw/test/",
    CLAUDE_LOCAL_GATEWAY_MODEL: "alpha/claude-opus-4-6",
    CLAUDE_LOCAL_GATEWAY_KEY: "test",
    CLAUDE_LOCAL_GATEWAY_MODEL_WINDOWS: JSON.stringify({ "alpha/claude-opus-4-6": 1_000_000 }),
  });
  assert.equal(modelWindow(config, "alpha/claude-opus-4-6"), 1_000_000);
  assert.equal(modelWindow(config, "unknown"), 200_000);
  assert.equal(modelWindow(config, "ALPHA/claude-opus-4-6"), 1_000_000);
  assert.equal(modelWindow(config, "claude-opus-4-6"), 1_000_000, "裸模型名也能命中带前缀的声明");
  for (const value of ["[]", '{"m":0}', '{"m":-1}', '{"m":"1100000"}', '{"m":1.1}']) {
    assert.throws(() => runtimeConfig({ CLAUDE_LOCAL_GATEWAY_MODEL_WINDOWS: value }), /model windows/);
  }
});

test("selected model falls back to the gateway default window instead of 200k", () => {
  const config = runtimeConfig({
    CLAUDE_LOCAL_MANAGED: "1",
    CLAUDE_LOCAL_GATEWAY_URL: "https://localhost/gw/test/",
    CLAUDE_LOCAL_GATEWAY_MODEL: "jojocodemax/gpt-6.1-sol",
    CLAUDE_LOCAL_GATEWAY_KEY: "test",
    CLAUDE_LOCAL_GATEWAY_MODELS: JSON.stringify(["jojocodemax/gpt-6.1-sol", "chatgpt/gpt-6.1-terra"]),
    CLAUDE_LOCAL_GATEWAY_MODEL_WINDOWS: JSON.stringify({}),
    CLAUDE_LOCAL_GATEWAY_DEFAULT_WINDOW: "1100000",
  });
  assert.equal(config.defaultWindow, 1_100_000);
  assert.equal(modelWindow(config, "jojocodemax/gpt-6.1-sol"), 1_100_000, "选中的模型用网关窗口");
  assert.equal(modelWindow(config, "chatgpt/gpt-6.1-terra"), 200_000, "没声明的其它模型仍按 200k 兜底");
  // 写坏了就当没给：不因为一个环境变量让启动失败
  for (const value of ["", "abc", "0", "-1", "1.5"]) {
    const broken = runtimeConfig({
      CLAUDE_LOCAL_MANAGED: "1",
      CLAUDE_LOCAL_GATEWAY_URL: "https://localhost/gw/test/",
      CLAUDE_LOCAL_GATEWAY_MODEL: "jojocodemax/gpt-6.1-sol",
      CLAUDE_LOCAL_GATEWAY_KEY: "test",
      CLAUDE_LOCAL_GATEWAY_DEFAULT_WINDOW: value,
    });
    assert.equal(broken.defaultWindow, 200_000);
  }
});

test("gateway forwarding preserves the configured root, authentication and selected provider/model", async () => {
  for (const host of [undefined, "api.anthropic.com"]) {
    const result = await request(service, "/v1/messages?choice=1", {
      host, body: { model: modelIds[2], messages: [{ role: "user", content: "test prompt" }], diagnostics: {}, metadata: { diagnostics: {}, user_id: "preserved" } },
    });
    assert.equal(result.status, 200);
    const call = received.at(-1);
    assert.equal(call.path, "/gw/desktop/v1/messages?choice=1");
    assert.equal(call.headers.authorization, "Bearer test-local-gateway-token");
    assert.equal(call.headers["x-api-key"], "test-local-gateway-token");
    assert.equal(call.body.model, modelIds[2]);
    assert.equal(call.body.diagnostics, undefined);
    assert.deepEqual(call.body.metadata, { user_id: "preserved" });
  }
  const result = await request(service, "/v1/models");
  assert.deepEqual(result.json.data.map((entry) => entry.id), modelIds);
});

test("managed loopback routes reject cross-site and unauthenticated requests before attaching gateway credentials", async () => {
  const before = received.length;
  for (const options of [
    { auth: false },
    { extraHeaders: { authorization: "Bearer invalid" } },
    { extraHeaders: { origin: "https://untrusted.example" } },
    { extraHeaders: { origin: "null" } },
  ]) {
    const response = await request(service, "/v1/messages", { ...options, body: { model: modelIds[1], messages: [] } });
    assert.equal(response.status, 403);
  }
  assert.equal(received.length, before);
  const health = await request(service, "/__codex_claude_local/health", { extraHeaders: { origin: "null" } });
  assert.equal(health.status, 403);
});

test("RPC sends preserve model choice, switching, complete history and restart state", async () => {
  const conv = "a0a0a0a0-b1b1-42c2-83d3-e4e4e4e4e4e4";
  const rpc = (name, body) => request(service, `/claudeai-rpc/anthropic.bard.api.v1alpha.ConversationService/${name}`, {
    host: "claude.ai", body, type: "application/proto",
  });
  await rpc("SendAndSettle", send(conv, "first test", modelIds[1]));
  assert.equal(received.at(-1).body.model, modelIds[1]);
  const switchModel = Buffer.concat([header(conv), ld(27, ld(1, ld(2, modelIds[2])))]);
  await rpc("PerformAction", switchModel);
  await rpc("SendAndSettle", send(conv, "second test"));
  assert.equal(received.at(-1).body.model, modelIds[2]);
  assert.deepEqual(received.at(-1).body.messages, [
    { role: "user", content: "first test" },
    { role: "assistant", content: "gateway reply" },
    { role: "user", content: "second test" },
  ]);
  await request(service, "/api/organizations/local/notification/preferences", {
    host: "claude.ai", method: "PATCH",
    body: { preferences: { feature_preference: { complete: { enable_email: false } } } },
  });
  await stop(service);
  const saved = JSON.parse(fs.readFileSync(path.join(root, "state.json"), "utf8"));
  assert.equal(saved.conversations[0].turns[0].model, modelIds[1]);
  assert.equal(saved.conversations[0].model, modelIds[2]);
  assert.equal(fs.statSync(path.join(root, "state.json")).mode & 0o777, 0o600);
  service = await start();
  await rpc("SendAndSettle", send(conv, "third test"));
  assert.equal(received.at(-1).body.model, modelIds[2]);
  assert.equal(received.at(-1).body.messages.length, 5);
  const prefs = await request(service, "/api/organizations/local/notification/preferences", { host: "claude.ai" });
  assert.equal(prefs.json.preferences.feature_preference.complete.enable_email, false);
  assert.equal(service.output().includes("first test"), false);
  assert.equal(service.output().includes("test-local-gateway-token"), false);
});

test("managed SIGTERM and stdin closure flush state and release the listener", async () => {
  await stop(service, true);
  await assert.rejects(request(service, "/__codex_claude_local/health"));
  service = await start({ CLAUDE_LOCAL_COMPUTER_USE: "1", CLAUDE_LOCAL_BACKGROUND_MODE: "0", CLAUDE_LOCAL_LOCALIZATION: "0" });
  const flags = await request(service, "/api/desktop/features", { host: "claude.ai" });
  assert.deepEqual(flags.json.features["1291166712"].value, { enabled: true, appScopedEnabled: false });
  const bootstrap = await request(service, "/edge-api/bootstrap", { host: "claude.ai" });
  assert.equal(bootstrap.json.locale, "en-US");
  const shell = await request(service, "/new", { host: "claude.ai" });
  assert.doesNotMatch(shell.bytes.toString("utf8"), /__codex_plus\/claude-localization\.js/);
  assert.doesNotMatch(shell.bytes.toString("utf8"), /name="i18n-catalogs"/);
  assert.equal((await request(service, "/__codex_plus/claude-localization.js", { host: "claude.ai" })).status, 404);
});

test("state loader validates contracts, preserves corrupt files and settles interrupted turns", () => {
  const invalid = path.join(root, "invalid.json");
  fs.writeFileSync(invalid, JSON.stringify({ version: 1, conversations: [{}], notifications: [] }));
  const original = fs.readFileSync(invalid);
  const warnings = [];
  const store = createStateStore(invalid, new Map(), new Map(), (message) => warnings.push(message));
  store.flush();
  assert.deepEqual(fs.readFileSync(invalid), original);
  assert.equal(warnings.length, 1);
  const oversized = path.join(root, "oversized.json");
  const fd = fs.openSync(oversized, "w", 0o600);
  fs.ftruncateSync(fd, 32 * 1024 * 1024 + 1);
  fs.closeSync(fd);
  const oversizedWarnings = [];
  createStateStore(oversized, new Map(), new Map(), (message) => oversizedWarnings.push(message));
  assert.equal(oversizedWarnings.length, 1);
  const state = JSON.parse(fs.readFileSync(path.join(root, "state.json"), "utf8"));
  state.conversations[0].turns[0].complete = false;
  fs.writeFileSync(path.join(root, "interrupted.json"), JSON.stringify(state));
  const conversations = new Map();
  createStateStore(path.join(root, "interrupted.json"), conversations, new Map());
  assert.equal(conversations.values().next().value.turns[0].stopReason, "interrupted");
  assert.equal(conversations.values().next().value.turns[0].complete, true);
});

test("gateway contracts keep duplicate model names distinct and old sessions use the default", () => {
  const config = runtimeConfig({
    CLAUDE_LOCAL_MANAGED: "1", CLAUDE_LOCAL_GATEWAY_URL: "https://localhost/gw/test/",
    CLAUDE_LOCAL_GATEWAY_MODEL: modelIds[0], CLAUDE_LOCAL_GATEWAY_KEY: "test",
    CLAUDE_LOCAL_GATEWAY_SMALL_MODEL: modelIds[2],
    CLAUDE_LOCAL_GATEWAY_MODELS: JSON.stringify(modelIds),
  });
  assert.equal(gatewayTarget(config, "/v1/messages?x=1").href, "https://localhost/gw/test/v1/messages?x=1");
  assert.equal(gatewayModel(config, modelIds[3]), modelIds[3]);
  assert.equal(gatewayModel(config, modelIds[4]), modelIds[4]);
  assert.equal(gatewayModel(config, undefined), modelIds[0]);
  assert.equal(gatewayModel(config, "claude-native-default"), modelIds[0]);
  assert.equal(gatewayModel(config, "claude-haiku-4-5"), modelIds[2]);
});

test("native bootstrap models without a provider are dropped from every picker list", () => {
  const base = {
    CLAUDE_LOCAL_MANAGED: "1", CLAUDE_LOCAL_GATEWAY_URL: "https://localhost/gw/test/",
    CLAUDE_LOCAL_GATEWAY_MODEL: modelIds[0], CLAUDE_LOCAL_GATEWAY_KEY: "test",
    CLAUDE_LOCAL_GATEWAY_MODELS: JSON.stringify(modelIds),
  };
  // 默认开；显式写 0 才关
  assert.equal(runtimeConfig(base).hideUnroutedModels, true);
  assert.equal(runtimeConfig({ ...base, CLAUDE_LOCAL_HIDE_UNROUTED_MODELS: "0" }).hideUnroutedModels, false);

  const config = runtimeConfig(base);
  const org = () => ({ claude_ai_bootstrap_models_config: [{ model: "claude-opus-5-5" }, { model: modelIds[1] }] });
  const growthbookModels = [{ model_id: "claude-sonnet-5-5" }];
  const bootstrap = {
    activeOrganization: org(),
    account: { memberships: [{ organization: org() }, {}] },
    claude_ai_available_models: { models: [{ model_id: "claude-haiku-5-5" }, { model_id: modelIds[0] }] },
    model_selector_config: [
      { id: "chat", models: [{ id: "claude-fable-5-1" }, { id: modelIds[2] }, { id: "beta/shared" }] },
      { id: "empty" },
    ],
    org_growthbook: { features: { list: { defaultValue: { models: growthbookModels } } } },
  };
  dropUnroutedModels(bootstrap, config);
  assert.deepEqual(bootstrap.activeOrganization.claude_ai_bootstrap_models_config.map((m) => m.model), [modelIds[1]]);
  assert.deepEqual(bootstrap.account.memberships[0].organization.claude_ai_bootstrap_models_config.map((m) => m.model), [modelIds[1]]);
  assert.deepEqual(bootstrap.claude_ai_available_models.models.map((m) => m.model_id), [modelIds[0]]);
  assert.deepEqual(bootstrap.model_selector_config[0].models.map((m) => m.id), [modelIds[2], "beta/shared"]);
  assert.equal(bootstrap.model_selector_config[1].models, undefined);
  // growthbook 不动
  assert.deepEqual(bootstrap.org_growthbook.features.list.defaultValue.models, growthbookModels);
});

test("recorded data dir opts managed mode into the PoC data path", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "claude-recorded-"));
  const env = {
    CLAUDE_LOCAL_MANAGED: "1", CLAUDE_LOCAL_GATEWAY_URL: "https://localhost/gw/test/",
    CLAUDE_LOCAL_GATEWAY_MODEL: "claude-test", CLAUDE_LOCAL_GATEWAY_KEY: "test",
    CLAUDE_LOCAL_RECORDED_DIR: dir,
  };
  // 未就绪（缺 .cache 和 mock/bootstrap.json）时保持合成数据路径
  assert.equal(runtimeConfig(env).recordedDir, null);
  fs.mkdirSync(path.join(dir, ".cache"));
  assert.equal(runtimeConfig(env).recordedDir, null);
  fs.mkdirSync(path.join(dir, "mock"));
  fs.writeFileSync(path.join(dir, "mock", "bootstrap.json"), "{}");
  assert.equal(runtimeConfig(env).recordedDir, dir);
  // 非托管模式下忽略该变量
  assert.equal(runtimeConfig({ CLAUDE_LOCAL_RECORDED_DIR: dir }).recordedDir, null);
  // 未提供时同样保持合成数据路径
  delete env.CLAUDE_LOCAL_RECORDED_DIR;
  assert.equal(runtimeConfig(env).recordedDir, null);
});

test("runtime config parses display name and plan with safe defaults", () => {
  assert.equal(runtimeConfig({}).displayName, "Local User");
  assert.equal(runtimeConfig({}).plan, "ultra");
  assert.equal(runtimeConfig({ CLAUDE_LOCAL_DISPLAY_NAME: "  张三  " }).displayName, "张三");
  assert.equal(runtimeConfig({ CLAUDE_LOCAL_DISPLAY_NAME: "a\nb\t" }).displayName, "ab");
  assert.equal(runtimeConfig({ CLAUDE_LOCAL_DISPLAY_NAME: "x".repeat(80) }).displayName.length, 64);
  assert.equal(runtimeConfig({ CLAUDE_LOCAL_DISPLAY_NAME: " \t\n " }).displayName, "Local User");
  assert.equal(runtimeConfig({}).email, "local@claude.local");
  assert.equal(runtimeConfig({ CLAUDE_LOCAL_EMAIL: "  me@example.com  " }).email, "me@example.com");
  for (const bad of ["", "no-at", "@b.c", "a@", "a b@c.d", "a@b\nc", `${"a".repeat(260)}@b.c`]) {
    assert.equal(runtimeConfig({ CLAUDE_LOCAL_EMAIL: bad }).email, "local@claude.local", bad);
  }
  for (const [raw, expected] of [["free", "free"], ["pro", "pro"], ["max", "max"], ["ultra", "ultra"], ["vip", "ultra"], ["", "ultra"]]) {
    assert.equal(runtimeConfig({ CLAUDE_LOCAL_PLAN: raw }).plan, expected, raw);
  }
});

test("plan profile covers all four tiers", () => {
  assert.equal(planProfile("free").plan, "free");
  assert.equal(planProfile("free").rateLimitTier, "default_claude_ai");
  assert.deepEqual(planProfile("pro").capabilities, ["claude_pro"]);
  assert.equal(planProfile("max").rateLimitTier, "default_claude_max_5x");
  assert.equal(planProfile("ultra").rateLimitTier, "default_claude_max_20x");
  assert.equal(planProfile("ultra").label, "Ultra");
  assert.equal(planProfile("nope").label, "Ultra");
});

test("runtime config parses model names and bad data falls back", () => {
  assert.deepEqual(runtimeConfig({}).modelNames, {});
  const ok = runtimeConfig({ CLAUDE_LOCAL_GATEWAY_MODEL_NAMES: '{"jojo/gpt-6.1-sol":{"name":"gpt-6.1-sol","description":"Jojo Codemax"}}' });
  assert.equal(ok.modelNames["jojo/gpt-6.1-sol"].name, "gpt-6.1-sol");
  assert.deepEqual(runtimeConfig({ CLAUDE_LOCAL_GATEWAY_MODEL_NAMES: "not json" }).modelNames, {});
  assert.deepEqual(runtimeConfig({ CLAUDE_LOCAL_GATEWAY_MODEL_NAMES: '{"a/b":{"name":5}}' }).modelNames, {});
});

test("model selectors use friendly names and fall back to ids", () => {
  const config = runtimeConfig({
    CLAUDE_LOCAL_GATEWAY_MODEL: "jojo/gpt-6.1-sol",
    CLAUDE_LOCAL_GATEWAY_MODELS: '["jojo/gpt-6.1-sol","other/raw-model"]',
    CLAUDE_LOCAL_GATEWAY_MODEL_NAMES: '{"jojo/gpt-6.1-sol":{"name":"gpt-6.1-sol","description":"Jojo Codemax"}}',
  });
  const models = modelSelectors(config).model_selector_config[0].models;
  const named = models.find((model) => model.id === "jojo/gpt-6.1-sol");
  assert.equal(named.name, "gpt-6.1-sol");
  assert.equal(named.short_name, "gpt-6.1-sol");
  assert.equal(named.description, "Jojo Codemax");
  assert.deepEqual(modelLabel(config, "jojo/gpt-6.1-sol"), { name: "gpt-6.1-sol", description: "Jojo Codemax" });
  assert.deepEqual(modelLabel(config, "other/raw-model"), { name: "other/raw-model", description: "" });
  const raw = models.find((model) => model.id === "other/raw-model");
  assert.equal(raw.name, "other/raw-model");
  assert.equal(raw.description, "");
});
