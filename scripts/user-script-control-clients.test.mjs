import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = await readFile(new URL("../assets/inject/renderer-inject/40-backend-settings.js", import.meta.url), "utf8");
const start = source.indexOf("  let codexPlusPendingReload = null;");
const end = source.indexOf("  /** 左面板的导航项。", start);
assert.ok(start > 0 && end > start);
const client = source.slice(start, end);
const epoch = "a".repeat(32);
const storageKey = "__codexPlusPendingUserScriptReloadV1";

function createMemoryStorage() {
  const values = new Map();
  return {
    getItem(key) { return values.has(key) ? values.get(key) : null; },
    setItem(key, value) { values.set(key, String(value)); },
    removeItem(key) { values.delete(key); },
  };
}

function createClient(postJson, loadUserScripts = async () => {}) {
  const context = vm.createContext({
    postJson, loadUserScripts, showToast() {},
    window: { sessionStorage: createMemoryStorage(), __codexPlusUserScripts: { documentId: "document-a" } },
    crypto: { randomUUID }, TextEncoder,
    setTimeout: (callback) => { queueMicrotask(callback); return 0; },
    Date, Promise,
  });
  vm.runInContext(`${client}\nglobalThis.reload = reloadUserScriptsAfterChange;`, context);
  return context.reload;
}

function applied(id, revision = "current", desired = revision, sourceDocumentId = "document-a") {
  return {
    reloadOperation: { operationId: id, ownerEpoch: epoch, allowPage: true, sourceDocumentId, phase: "applied", active: false, revision },
    desiredRevision: desired,
  };
}

test("page client resolves a lost submission ACK with the original ID", async () => {
  let id;
  let submissions = 0;
  let queries = 0;
  const reload = createClient(async (route, payload) => {
    if (payload.capabilities) return { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch };
    if (route === "/user-scripts/reload-operation-v1") {
      id = payload.operationId;
      submissions++;
      throw new Error("ACK lost after owner accepted");
    }
    queries++;
    assert.equal(payload.operationId, id);
    return applied(id);
  });
  await reload();
  assert.equal(submissions, 1);
  assert.equal(queries, 1);
  assert.match(id, /^[a-f0-9]{32}-[a-f0-9]+-[a-f0-9]{32}$/);
});

test("page client applies current revision after a historical operation completes", async () => {
  const ids = [];
  const reload = createClient(async (_route, payload) => {
    if (payload.capabilities) return { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch };
    ids.push(payload.operationId);
    return applied(payload.operationId, ids.length === 1 ? "old" : "current", "current");
  });
  await reload();
  assert.equal(ids.length, 2);
  assert.notEqual(ids[0], ids[1]);
});

test("an intent arriving after desiredRevision was read still drains another reload", async () => {
  let releaseInventory;
  let inventoryStarted;
  const waitingInventory = new Promise((resolve) => { inventoryStarted = resolve; });
  const inventoryGate = new Promise((resolve) => { releaseInventory = resolve; });
  let inventories = 0;
  const ids = [];
  let revision = "old";
  const reload = createClient(async (_route, payload) => {
    if (payload.capabilities) return { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch };
    ids.push(payload.operationId);
    return applied(payload.operationId, revision);
  }, async () => {
    if (++inventories === 1) {
      inventoryStarted();
      await inventoryGate;
    }
  });
  const first = reload();
  await waitingInventory;
  revision = "new";
  const second = reload();
  assert.equal(first, second);
  releaseInventory();
  await second;
  assert.equal(ids.length, 2);
  assert.notEqual(ids[0], ids[1]);
});

test("pending desired revision polls the same operation and verification intent", async () => {
  const requests = [];
  const reload = createClient(async (_route, payload) => {
    if (payload.capabilities) return { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch };
    requests.push(payload);
    if (requests.length === 1) {
      return { ...applied(payload.operationId), desiredRevision: null, desiredRevisionPending: true };
    }
    return applied(payload.operationId);
  });
  await reload();
  assert.equal(requests.length, 2);
  assert.equal(requests[0].operationId, requests[1].operationId);
  assert.equal(requests[0].verificationId, requests[1].verificationId);
});

const bridgeSource = await readFile(new URL("../crates/codex-plus-core/src/bridge.rs", import.meta.url), "utf8");
const bridgeStart = bridgeSource.indexOf('r#"', bridgeSource.indexOf("pub fn build_bridge_script(")) + 3;
const bridgeEnd = bridgeSource.indexOf('"#', bridgeStart);
assert.ok(bridgeStart > 2 && bridgeEnd > bridgeStart);
const bridgeScript = bridgeSource.slice(bridgeStart, bridgeEnd)
  .replace("{binding_name}", "codexSessionDeleteV2")
  .replaceAll("{{", "{")
  .replaceAll("}}", "}");
const postSource = await readFile(new URL("../assets/inject/renderer-inject/60-plugin-marketplace.js", import.meta.url), "utf8");
const postStart = postSource.indexOf("  async function postJson(path, payload, options = {})");
const postEnd = postSource.indexOf("  function downloadMarkdownFallback(", postStart);
const loadStart = source.indexOf('  async function loadUserScripts(path = "/user-scripts/list"');
const loadEnd = source.indexOf("  /**\n   * 拉市场清单。", loadStart);
assert.ok(postStart >= 0 && postEnd > postStart && loadStart >= 0 && loadEnd > loadStart);
const post = postSource.slice(postStart, postEnd);
const load = source.slice(loadStart, loadEnd);

class FakeClock {
  now = 1700000000000;
  nextId = 1;
  timers = new Map();

  setTimeout = (callback, delay = 0) => {
    const id = this.nextId++;
    this.timers.set(id, { callback, at: this.now + Math.max(0, delay) });
    return id;
  };

  clearTimeout = (id) => { this.timers.delete(id); };

  async flush() {
    for (let i = 0; i < 30; i++) await Promise.resolve();
  }

  async advance(ms) {
    const end = this.now + ms;
    await this.flush();
    while (true) {
      const due = [...this.timers.entries()]
        .filter(([, timer]) => timer.at <= end)
        .sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];
      if (!due) break;
      const [id, timer] = due;
      this.timers.delete(id);
      this.now = timer.at;
      timer.callback();
      await this.flush();
    }
    this.now = end;
    await this.flush();
  }
}

function createRealBridgeHarness(onRequest = () => {}, { storage = createMemoryStorage(), documentId = "document-a" } = {}) {
  const clock = new FakeClock();
  const calls = [];
  const toasts = [];
  const window = { __testRenders: [], sessionStorage: storage, __codexPlusUserScripts: { documentId } };
  let clientGeneration = 0;
  class FakeDate extends Date {
    static now() { return clock.now; }
  }
  const context = vm.createContext({
    window,
    Date: FakeDate,
    crypto: { randomUUID }, TextEncoder,
    setTimeout: clock.setTimeout,
    clearTimeout: clock.clearTimeout,
    showToast: (message) => toasts.push(message),
    recordCodexPlusBridgeFailure() {},
    recordCodexPlusBridgeSuccess() {},
    recordCodexPlusBridgeAttempt() {},
    sendCodexPlusDiagnostic() {},
  });
  const reply = (request, result) => window.__codexSessionDeleteResolve(request.id, result);
  window.codexSessionDeleteV2 = (raw) => {
    const request = JSON.parse(raw);
    calls.push(request);
    onRequest(request, reply);
  };
  const installBridge = () => vm.runInContext(bridgeScript, context);
  const installClient = () => vm.runInContext(`(() => {
    let codexPlusUserScripts = {};
    let codexPlusUserScriptsLoaded = false;
    const renderUserScripts = () => window.__testRenders.push(${++clientGeneration});
    const syncCodexPlusBuiltinExtensionToggles = () => {};
    const codexPlusActiveEntry = () => "";
    ${post}
    ${client}
    ${load}
    window.__testClient = { reload: reloadUserScriptsAfterChange, post: postJson };
  })();`, context);
  installBridge();
  installClient();
  return { clock, calls, toasts, window, reply, installBridge, installClient };
}

test("real bridge cleans callbacks and timers on response, rejection, and synchronous binding failure", async () => {
  const h = createRealBridgeHarness();
  const first = h.window.__testClient.post("/first", {}, { timeoutMs: 1000 });
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 1);
  assert.equal(h.clock.timers.size, 1);
  h.reply(h.calls[0], { status: "ok" });
  assert.equal((await first).status, "ok");
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);

  const rejected = h.window.__testClient.post("/second", {}, { timeoutMs: 1000 });
  h.window.__codexSessionDeleteReject(h.calls[1].id, "owner rejected");
  assert.equal((await rejected).message, "owner rejected");
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);

  h.window.codexSessionDeleteV2 = () => { throw new Error("binding unavailable"); };
  await assert.rejects(h.window.__testClient.post("/third", {}, { timeoutMs: 1000 }), /binding unavailable/);
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);
});

test("real bridge times out and ignores late replies without affecting the next request", async () => {
  const h = createRealBridgeHarness();
  const first = h.window.__testClient.post("/first", {}, { timeoutMs: 1000 });
  const firstRequest = h.calls[0];
  await h.clock.advance(1000);
  assert.equal((await first).timeout, true);
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);

  let secondSettled = false;
  const second = h.window.__testClient.post("/second", {}, { timeoutMs: 1000 }).then((value) => {
    secondSettled = true;
    return value;
  });
  const secondRequest = h.calls[1];
  assert.notEqual(firstRequest.id, secondRequest.id);
  h.reply(firstRequest, { status: "late" });
  await h.clock.flush();
  assert.equal(secondSettled, false);
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 1);
  h.reply(secondRequest, { status: "ok" });
  assert.equal((await second).status, "ok");
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);
});

test("bridge reinjection settles old requests, clears timers, and preserves unique request IDs", async () => {
  const h = createRealBridgeHarness();
  const first = h.window.__testClient.post("/first", {}, { timeoutMs: 1000 });
  const firstRequest = h.calls[0];
  const oldCallbacks = h.window.__codexSessionDeleteCallbacks;
  h.installBridge();
  assert.match((await first).message, /桥接已重新连接/);
  assert.equal(oldCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);

  let secondSettled = false;
  const second = h.window.__testClient.post("/second", {}, { timeoutMs: 1000 }).then((result) => {
    secondSettled = true;
    return result;
  });
  assert.notEqual(h.calls[1].id, firstRequest.id);
  h.reply(firstRequest, { status: "late" });
  await h.clock.flush();
  assert.equal(secondSettled, false);
  h.reply(h.calls[1], { status: "ok" });
  await second;
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);
});

test("a late response from an earlier document cannot resolve a new document request", async () => {
  const previous = createRealBridgeHarness();
  const current = createRealBridgeHarness();
  const previousRequest = previous.window.__testClient.post("/previous", {}, { timeoutMs: 1000 });
  let currentSettled = false;
  const currentRequest = current.window.__testClient.post("/current", {}, { timeoutMs: 1000 }).then((result) => {
    currentSettled = true;
    return result;
  });
  assert.notEqual(previous.calls[0].id, current.calls[0].id);
  current.reply(previous.calls[0], { status: "late" });
  await current.clock.flush();
  assert.equal(currentSettled, false);
  current.reply(current.calls[0], { status: "ok" });
  assert.equal((await currentRequest).status, "ok");
  await previous.clock.advance(1000);
  assert.equal((await previousRequest).timeout, true);
  assert.equal(previous.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(current.window.__codexSessionDeleteCallbacks.size, 0);
});

test("unrelated bridge routes remain unbounded unless explicitly requested", async () => {
  const h = createRealBridgeHarness();
  let settled = false;
  const pending = h.window.__testClient.post("/other", {}).then((result) => {
    settled = true;
    return result;
  });
  await h.clock.advance(60000);
  assert.equal(settled, false);
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 1);
  assert.equal(h.clock.timers.size, 0);
  h.reply(h.calls[0], { status: "ok" });
  assert.equal((await pending).status, "ok");
});

test("real timeout after an accepted submission queries the original operation ID", async () => {
  let submission;
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-operation-v1") {
      submission = request;
      return;
    }
    if (request.path === "/user-scripts/reload-status") return reply(request, applied(request.payload.operationId));
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  });
  const pending = h.window.__testClient.reload();
  await h.clock.flush();
  assert.ok(submission);
  await h.clock.advance(3500);
  await pending;
  const queries = h.calls.filter((request) => request.path === "/user-scripts/reload-status" && !request.payload.capabilities);
  assert.equal(queries.length, 1);
  assert.equal(queries[0].payload.operationId, submission.payload.operationId);
  assert.equal(h.calls.filter((request) => request.path === "/user-scripts/reload-operation-v1").length, 1);
  assert.deepEqual(h.window.__testRenders, [1]);
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);
});

test("an explicitly unregistered operation is resubmitted with the same ID", async () => {
  let submissions = 0;
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-operation-v1") {
      if (++submissions === 2) return reply(request, applied(request.payload.operationId));
      return;
    }
    if (request.path === "/user-scripts/reload-status") {
      return reply(request, { reloadOperation: { operationId: request.payload.operationId, ownerEpoch: epoch, phase: "not_found", active: false } });
    }
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  });
  const pending = h.window.__testClient.reload();
  await h.clock.advance(4000);
  await pending;
  const controlCalls = h.calls.filter((request) => !request.payload.capabilities && request.path !== "/user-scripts/list");
  assert.deepEqual(controlCalls.map((request) => request.path), [
    "/user-scripts/reload-operation-v1", "/user-scripts/reload-status", "/user-scripts/reload-operation-v1",
  ]);
  assert.equal(new Set(controlCalls.map((request) => request.payload.operationId)).size, 1);
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);
});

test("same-document client and bridge reinjection retains the in-flight operation", async () => {
  let submission;
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-operation-v1") {
      submission = request;
      return;
    }
    if (request.path === "/user-scripts/reload-status") return reply(request, applied(request.payload.operationId));
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  });
  const pending = h.window.__testClient.reload();
  await h.clock.flush();
  assert.equal(h.window.__codexPlusReloadState.pending.operationId, submission.payload.operationId);
  assert.equal(h.window.__codexPlusReloadState.pending.phase, "unknown");
  h.installBridge();
  h.installClient();
  assert.equal(h.window.__codexPlusReloadState.task, pending);
  await h.clock.advance(500);
  await pending;
  const queries = h.calls.filter((request) => request.path === "/user-scripts/reload-status" && !request.payload.capabilities);
  assert.equal(queries.length, 1);
  assert.equal(queries[0].payload.operationId, submission.payload.operationId);
  assert.equal(h.calls.filter((request) => request.path === "/user-scripts/reload-operation-v1").length, 1);
  assert.deepEqual(h.window.__testRenders, [2]);
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);
});

test("an unconfirmed operation survives the deadline and a same-document client reinjection", async () => {
  let allowStatus = false;
  let submission;
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-operation-v1") {
      submission = request;
      return;
    }
    if (request.path === "/user-scripts/reload-status" && allowStatus) return reply(request, applied(request.payload.operationId));
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  });
  const first = h.window.__testClient.reload();
  await h.clock.advance(20000);
  await first;
  assert.equal(h.window.__codexPlusReloadState.pending.operationId, submission.payload.operationId);
  assert.equal(h.window.__codexPlusReloadState.pending.phase, "unknown");
  assert.ok(h.toasts.some((message) => message.includes("待确认")));
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);

  allowStatus = true;
  h.installClient();
  const before = h.calls.length;
  await h.window.__testClient.reload();
  const next = h.calls.slice(before);
  assert.equal(next[0].path, "/user-scripts/reload-status");
  assert.equal(next[0].payload.operationId, submission.payload.operationId);
  assert.equal(next.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.equal(h.window.__codexPlusReloadState.pending, null);
});

test("a missing inventory response does not keep a confirmed reload pending", async () => {
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-operation-v1") return reply(request, applied(request.payload.operationId));
  });
  let settled = false;
  const pending = h.window.__testClient.reload().then(() => { settled = true; });
  await h.clock.flush();
  assert.equal(settled, false);
  assert.equal(h.calls.at(-1).path, "/user-scripts/list");
  await h.clock.advance(3000);
  await pending;
  assert.equal(h.window.__codexPlusReloadState.pending, null);
  assert.ok(h.toasts.some((message) => message.includes("清单暂时无法刷新")));
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);
});

test("missing replies are reported as unconfirmed, not as unsupported", async () => {
  const h = createRealBridgeHarness();
  const pending = h.window.__testClient.reload();
  await h.clock.advance(6000);
  await pending;
  assert.ok(h.toasts.some((message) => message.includes("无法确认启动器重载能力")));
  assert.ok(h.toasts.every((message) => !message.includes("不支持文档隔离")));
  assert.equal(h.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.equal(h.window.__codexSessionDeleteCallbacks.size, 0);
  assert.equal(h.clock.timers.size, 0);
});

test("an explicitly unsupported owner falls back to legacy reload, but a missing reply still does not", async () => {
  const owner = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { status: "failed", message: "Unknown bridge path" });
    if (request.path === "/user-scripts/reload") return reply(request, { scripts: [] });
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  });
  await owner.window.__testClient.reload();
  assert.equal(owner.toasts.length, 0);
  assert.equal(owner.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.equal(owner.calls.filter((request) => request.path === "/user-scripts/reload").length, 1);
  assert.equal(owner.window.sessionStorage.getItem(storageKey), null);

  const oldBridge = createRealBridgeHarness();
  let oldBridgeCalls = 0;
  oldBridge.window.__codexSessionDeleteBridge = () => {
    oldBridgeCalls++;
    return new Promise(() => {});
  };
  await oldBridge.window.__testClient.reload();
  assert.ok(oldBridge.toasts.some((message) => message.includes("无法确认启动器重载能力")));
  assert.equal(oldBridgeCalls, 0);
});

test("a capability response without the document fence falls back to legacy reload", async () => {
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1 }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload") return reply(request, { scripts: [] });
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  });
  await h.window.__testClient.reload();
  assert.equal(h.toasts.length, 0);
  assert.equal(h.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.equal(h.calls.filter((request) => request.path === "/user-scripts/reload").length, 1);
  assert.equal(h.window.sessionStorage.getItem(storageKey), null);
});

test("a capability response without the versioned submit route falls back to legacy reload", async () => {
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, {
      capabilities: { operationProtocol: 1, documentFence: true }, ownerEpoch: epoch,
    });
    if (request.path === "/user-scripts/reload") return reply(request, { scripts: [] });
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  });
  await h.window.__testClient.reload();
  assert.equal(h.toasts.length, 0);
  assert.equal(h.calls.some((request) => request.path.startsWith("/user-scripts/reload-operation")), false);
  assert.equal(h.calls.filter((request) => request.path === "/user-scripts/reload").length, 1);
  assert.equal(h.window.sessionStorage.getItem(storageKey), null);
});

test("an old host after handshake cannot trigger a legacy reload or clear the reference", async () => {
  let legacyEffects = 0;
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, {
      capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch,
    });
    if (request.path === "/user-scripts/reload") {
      legacyEffects++;
      return reply(request, { status: "ok" });
    }
    if (request.path === "/user-scripts/reload-operation-v1" || request.path === "/user-scripts/reload-status") {
      return reply(request, { status: "failed", message: "Unknown bridge path" });
    }
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  });
  const task = h.window.__testClient.reload();
  await h.clock.advance(20000);
  await task;
  assert.equal(legacyEffects, 0);
  assert.equal(h.calls.filter((request) => request.path === "/user-scripts/reload-operation-v1").length, 1);
  assert.ok(h.window.sessionStorage.getItem(storageKey));
  assert.equal(h.window.__codexPlusReloadState.pending.phase, "unknown");
});

test("incomplete terminal receipts and ambiguous tombstones stay pending", async () => {
  const responses = [
    { phase: "applied", active: false, allowPage: true, revision: "current" },
    { phase: "applied", active: false, sourceDocumentId: "document-a", revision: "current" },
    { phase: "applied", allowPage: true, sourceDocumentId: "document-a", revision: "current" },
    { phase: "rejected", active: false },
    { phase: "not_found", allowPage: true, sourceDocumentId: "document-a" },
  ];
  for (const response of responses) {
    const h = createRealBridgeHarness((request, reply) => {
      if (request.payload.capabilities) return reply(request, {
        capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch,
      });
      if (request.path === "/user-scripts/reload-operation-v1" || request.path === "/user-scripts/reload-status") {
        return reply(request, { reloadOperation: {
          operationId: request.payload.operationId, ownerEpoch: epoch, ...response,
        }, desiredRevision: "current" });
      }
      if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
    });
    const task = h.window.__testClient.reload();
    await h.clock.advance(20000);
    await task;
    assert.equal(h.calls.filter((request) => request.path === "/user-scripts/reload-operation-v1").length, 1);
    assert.ok(h.window.sessionStorage.getItem(storageKey));
    assert.equal(h.window.__codexPlusReloadState.pending.phase, "unknown");
  }
});

test("applied receipts without a valid revision cannot start another operation", async () => {
  for (const revision of [undefined, null, 123, ""]) {
    let submissions = 0;
    const h = createRealBridgeHarness((request, reply) => {
      if (request.payload.capabilities) return reply(request, {
        capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch,
      });
      if (request.path === "/user-scripts/reload-operation-v1") {
        if (++submissions > 1) return;
        const response = applied(request.payload.operationId);
        response.reloadOperation.revision = revision;
        return reply(request, response);
      }
      if (request.path === "/user-scripts/reload-status") {
        const response = applied(request.payload.operationId);
        response.reloadOperation.revision = revision;
        return reply(request, response);
      }
      if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
    });
    const task = h.window.__testClient.reload();
    await h.clock.advance(20000);
    await task;
    assert.equal(submissions, 1, `invalid revision ${String(revision)} caused a new submission`);
    assert.ok(h.window.sessionStorage.getItem(storageKey));
    assert.equal(h.window.__codexPlusReloadState.pending.phase, "unknown");
  }
});

test("unregistered receipts with contradictory identity cannot resubmit", async () => {
  for (const fields of [
    { allowPage: false },
    { allowPage: null },
    { sourceDocumentId: "different-document" },
    { sourceDocumentId: null },
  ]) {
    let submissions = 0;
    const h = createRealBridgeHarness((request, reply) => {
      if (request.payload.capabilities) return reply(request, {
        capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch,
      });
      if (request.path === "/user-scripts/reload-operation-v1") {
        submissions++;
        return;
      }
      if (request.path === "/user-scripts/reload-status") return reply(request, { reloadOperation: {
        operationId: request.payload.operationId, ownerEpoch: epoch, phase: "not_found", active: false, ...fields,
      } });
      if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
    });
    const task = h.window.__testClient.reload();
    await h.clock.advance(20000);
    await task;
    assert.equal(submissions, 1, `contradictory identity ${JSON.stringify(fields)} caused a resubmission`);
    assert.ok(h.window.sessionStorage.getItem(storageKey));
    assert.equal(h.window.__codexPlusReloadState.pending.phase, "unknown");
  }
});

test("unavailable, rejected, or unverifiable session storage prevents submission", async () => {
  const cases = [
    { getItem() { throw new Error("access denied"); }, setItem() {}, removeItem() {} },
    { getItem() { return null; }, setItem() { throw new Error("quota exceeded"); }, removeItem() {} },
    { getItem() { return null; }, setItem() {}, removeItem() {} },
  ];
  for (const storage of cases) {
    const h = createRealBridgeHarness((request, reply) => {
      if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
      if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
    }, { storage });
    await h.window.__testClient.reload();
    assert.equal(h.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
    assert.ok(h.toasts.some((message) => message.includes("未提交重载")));
  }
});

test("invalid stored references or missing document identity do not submit", async () => {
  const storage = createMemoryStorage();
  storage.setItem(storageKey, "{not valid JSON");
  const corrupt = createRealBridgeHarness((request, reply) => {
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { storage });
  await corrupt.window.__testClient.reload();
  assert.equal(corrupt.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.equal(storage.getItem(storageKey), "{not valid JSON");
  assert.ok(corrupt.toasts.some((message) => message.includes("无法安全读取")));

  const noDocument = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { documentId: "" });
  await noDocument.window.__testClient.reload();
  assert.equal(noDocument.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.ok(noDocument.toasts.some((message) => message.includes("文档身份尚不可用")));

  const longDocument = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { documentId: "x".repeat(257) });
  await longDocument.window.__testClient.reload();
  assert.equal(longDocument.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.ok(longDocument.toasts.some((message) => message.includes("文档身份尚不可用")));
});

test("document identity uses the Rust UTF-8 byte limit and rejects malformed UTF-16", async () => {
  const validIds = ["x".repeat(256), "\u00e9".repeat(128), "\ud83d\ude00".repeat(64)];
  for (const documentId of validIds) {
    const h = createRealBridgeHarness((request, reply) => {
      if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
      if (request.path === "/user-scripts/reload-operation-v1") return reply(request, applied(request.payload.operationId, "current", "current", documentId));
      if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
    }, { documentId });
    await h.window.__testClient.reload();
    const submission = h.calls.find((request) => request.path === "/user-scripts/reload-operation-v1");
    assert.equal(submission?.payload.sourceDocumentId, documentId);
    assert.equal(h.window.sessionStorage.getItem(storageKey), null);
  }

  const invalidIds = ["\u00e9".repeat(129), "\ud83d\ude00".repeat(65), "\ud800", "\udc00"];
  for (const documentId of invalidIds) {
    const h = createRealBridgeHarness((request, reply) => {
      if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
      if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
    }, { documentId });
    await h.window.__testClient.reload();
    assert.equal(h.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
    assert.equal(h.window.sessionStorage.getItem(storageKey), null);
    assert.ok(h.toasts.some((message) => message.includes("文档身份尚不可用")));
  }
});

test("a stored document identity over the UTF-8 byte limit is not restored or submitted", async () => {
  const storage = createMemoryStorage();
  const reference = {
    operationId: `${epoch}-18bcfe56800-${"b".repeat(32)}`,
    ownerEpoch: epoch, allowPage: true, sourceDocumentId: "\u00e9".repeat(129),
  };
  storage.setItem(storageKey, JSON.stringify(reference));
  const h = createRealBridgeHarness((request, reply) => {
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { storage });
  await h.window.__testClient.reload();
  assert.equal(h.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.deepEqual(JSON.parse(storage.getItem(storageKey)), reference);
  assert.ok(h.toasts.some((message) => message.includes("无法安全读取")));
});

test("a new document automatically queries the original lost-ACK operation", async () => {
  const storage = createMemoryStorage();
  let submitted;
  const previous = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-operation-v1") submitted = request;
  }, { storage, documentId: "document-old" });
  void previous.window.__testClient.reload();
  await previous.clock.flush();
  const reference = JSON.parse(storage.getItem(storageKey));
  assert.deepEqual(reference, {
    operationId: submitted.payload.operationId, ownerEpoch: epoch,
    allowPage: true, sourceDocumentId: "document-old",
  });

  const current = createRealBridgeHarness((request, reply) => {
    if (request.path === "/user-scripts/reload-status") return reply(request, applied(request.payload.operationId, "current", "current", "document-old"));
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { storage, documentId: "document-new" });
  await current.clock.flush();
  const pending = current.window.__codexPlusReloadState.task;
  if (pending) await pending;
  assert.equal(current.calls[0].path, "/user-scripts/reload-status");
  assert.equal(current.calls[0].payload.operationId, submitted.payload.operationId);
  assert.equal(current.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.equal(storage.getItem(storageKey), null);
});

test("after navigation not_found is retried only with the original source document and operation", async () => {
  const storage = createMemoryStorage();
  let submitted;
  const previous = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-operation-v1") submitted = request;
  }, { storage, documentId: "document-old" });
  void previous.window.__testClient.reload();
  await previous.clock.flush();

  const current = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-status") return reply(request, {
      reloadOperation: { operationId: request.payload.operationId, ownerEpoch: epoch, phase: "not_found", active: false },
    });
    if (request.path === "/user-scripts/reload-operation-v1") return reply(request, {
      reloadOperation: { ...request.payload, phase: "rejected", active: false, reason: "source document changed" },
    });
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { storage, documentId: "document-new" });
  await current.clock.advance(500);
  const task = current.window.__codexPlusReloadState.task;
  if (task) await task;
  const controls = current.calls.filter((request) => request.path !== "/user-scripts/list");
  assert.equal(controls[0].path, "/user-scripts/reload-status");
  assert.equal(controls[0].payload.operationId, submitted.payload.operationId);
  const retry = controls.find((request) => request.path === "/user-scripts/reload-operation-v1");
  assert.equal(retry.payload.operationId, submitted.payload.operationId);
  assert.equal(retry.payload.ownerEpoch, epoch);
  assert.equal(retry.payload.allowPage, true);
  assert.equal(retry.payload.sourceDocumentId, "document-old");
  assert.ok(current.toasts.some((message) => message.includes("source document changed")));
  assert.equal(storage.getItem(storageKey), null);
});

test("a changed owner never receives an old operation and its reference is retained", async () => {
  const storage = createMemoryStorage();
  const reference = {
    operationId: `${epoch}-18bcfe56800-${"b".repeat(32)}`,
    ownerEpoch: epoch, allowPage: true, sourceDocumentId: "document-old",
  };
  storage.setItem(storageKey, JSON.stringify(reference));
  const h = createRealBridgeHarness((request, reply) => {
    if (request.path === "/user-scripts/reload-status") return reply(request, {
      reloadOperation: { operationId: reference.operationId, ownerEpoch: "c".repeat(32), phase: "rejected" },
    });
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { storage, documentId: "document-new" });
  await h.clock.flush();
  const task = h.window.__codexPlusReloadState.task;
  if (task) await task;
  assert.equal(h.calls[0].payload.operationId, reference.operationId);
  assert.equal(h.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
  assert.deepEqual(JSON.parse(storage.getItem(storageKey)), reference);
  assert.equal(h.window.__codexPlusReloadState.pending.phase, "unknown");
  assert.ok(h.toasts.some((message) => message.includes("原重载结果无法确认")));
});

test("a terminal response without owner identity is not trusted", async () => {
  const storage = createMemoryStorage();
  const reference = {
    operationId: `${epoch}-18bcfe56800-${"b".repeat(32)}`,
    ownerEpoch: epoch, allowPage: true, sourceDocumentId: "document-old",
  };
  storage.setItem(storageKey, JSON.stringify(reference));
  const h = createRealBridgeHarness((request, reply) => {
    if (request.path === "/user-scripts/reload-status") return reply(request, {
      reloadOperation: { operationId: reference.operationId, phase: "applied", revision: "current" },
      desiredRevision: "current",
    });
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { storage });
  await h.clock.advance(20000);
  assert.deepEqual(JSON.parse(storage.getItem(storageKey)), reference);
  assert.equal(h.window.__codexPlusReloadState.pending.phase, "unknown");
  assert.equal(h.calls.some((request) => request.path === "/user-scripts/reload-operation-v1"), false);
});

test("a confirmed older revision creates a new intent for the current document only afterward", async () => {
  const storage = createMemoryStorage();
  const reference = {
    operationId: `${epoch}-18bcfe56800-${"b".repeat(32)}`,
    ownerEpoch: epoch, allowPage: true, sourceDocumentId: "document-old",
  };
  storage.setItem(storageKey, JSON.stringify(reference));
  let storedAtSubmission;
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-status") return reply(request, applied(reference.operationId, "old", "new", "document-old"));
    if (request.path === "/user-scripts/reload-operation-v1") {
      storedAtSubmission = JSON.parse(storage.getItem(storageKey));
      return reply(request, applied(request.payload.operationId, "new", "new", "document-new"));
    }
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { storage, documentId: "document-new" });
  await h.clock.flush();
  const task = h.window.__codexPlusReloadState.task;
  if (task) await task;
  const request = h.calls.find((call) => call.path === "/user-scripts/reload-operation-v1");
  assert.equal(h.calls[0].payload.operationId, reference.operationId);
  assert.notEqual(request.payload.operationId, reference.operationId);
  assert.equal(request.payload.sourceDocumentId, "document-new");
  assert.equal(storedAtSubmission.operationId, request.payload.operationId);
  assert.equal(storedAtSubmission.sourceDocumentId, "document-new");
  assert.equal(storage.getItem(storageKey), null);
});

test("a completed operation never clears a different pending reference", async () => {
  const storage = createMemoryStorage();
  let submit;
  const h = createRealBridgeHarness((request, reply) => {
    if (request.payload.capabilities) return reply(request, { capabilities: { operationProtocol: 1, documentFence: true, fencedSubmitV1: true }, ownerEpoch: epoch });
    if (request.path === "/user-scripts/reload-operation-v1") { submit = { request, reply }; return; }
    if (request.path === "/user-scripts/list") return reply(request, { scripts: [] });
  }, { storage });
  const task = h.window.__testClient.reload();
  await h.clock.flush();
  const other = {
    operationId: `${epoch}-18bcfe56800-${"c".repeat(32)}`,
    ownerEpoch: epoch, allowPage: true, sourceDocumentId: "other-document",
  };
  storage.setItem(storageKey, JSON.stringify(other));
  submit.reply(submit.request, applied(submit.request.payload.operationId));
  await task;
  assert.deepEqual(JSON.parse(storage.getItem(storageKey)), other);
  assert.ok(h.toasts.some((message) => message.includes("无法安全清理")));
});
