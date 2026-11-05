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

function createClient(postJson, loadUserScripts = async () => {}) {
  const context = vm.createContext({
    postJson, loadUserScripts, showToast() {},
    crypto: { randomUUID },
    setTimeout: (callback) => { queueMicrotask(callback); return 0; },
    Date, Promise,
  });
  vm.runInContext(`${client}\nglobalThis.reload = reloadUserScriptsAfterChange;`, context);
  return context.reload;
}

function applied(id, revision = "current", desired = revision) {
  return {
    reloadOperation: { operationId: id, ownerEpoch: epoch, phase: "applied", revision },
    desiredRevision: desired,
  };
}

test("page client resolves a lost submission ACK with the original ID", async () => {
  let id;
  let submissions = 0;
  let queries = 0;
  const reload = createClient(async (route, payload) => {
    if (payload.capabilities) return { capabilities: { operationProtocol: 1 }, ownerEpoch: epoch };
    if (route === "/user-scripts/reload") {
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
    if (payload.capabilities) return { capabilities: { operationProtocol: 1 }, ownerEpoch: epoch };
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
    if (payload.capabilities) return { capabilities: { operationProtocol: 1 }, ownerEpoch: epoch };
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
    if (payload.capabilities) return { capabilities: { operationProtocol: 1 }, ownerEpoch: epoch };
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
