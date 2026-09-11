import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const source = fs.readFileSync(path.join(import.meta.dirname, "xuan-plus-remote-command.user.js"), "utf8");

test("remote command script is credential-free and feature-detected", () => {
  assert.match(source, /sendMessageFromView/);
  assert.match(source, /__codexPlusMobileRemoteCommand/);
  assert.doesNotMatch(source, /api[_-]?key|authorization|bearer/i);
});

test("remote command script maps send and stop to official renderer requests", async () => {
  const listeners = new Set();
  const calls = [];
  let requestSequence = 0;
  const window = {
    addEventListener(type, listener) { if (type === "message") listeners.add(listener); },
    removeEventListener(type, listener) { if (type === "message") listeners.delete(listener); },
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
    electronBridge: {
      sendMessageFromView(message) {
        const { id, method, params } = message.request;
        calls.push({ method, params, source: message.source });
        requestSequence += 1;
        const result = method === "turn/start" ? { turn: { id: `turn-${requestSequence}` } } : {};
        queueMicrotask(() => listeners.forEach((listener) => listener({
          data: { type: "mcp-response", hostId: "local", message: { id, result } },
        })));
      },
    },
  };
  vm.runInNewContext(source, { window, crypto: { randomUUID: () => `request-${requestSequence}` }, Date, Math, Promise, setTimeout, clearTimeout });
  const sent = await window.__codexPlusMobileRemoteCommand({
    commandType: "send_input",
    threadId: "thread-0000000001",
    clientRequestId: "request-000000001",
    text: "continue",
  });
  assert.equal(sent.status, "completed");
  assert.deepEqual(calls.map((call) => call.method), ["thread/resume", "turn/start"]);
  assert.ok(calls.every((call) => call.source === "xuan_plus_remote"));
  calls.length = 0;
  await window.__codexPlusMobileRemoteCommand({
    commandType: "stop_task",
    threadId: "thread-0000000001",
    turnId: "turn-00000000001",
  });
  assert.deepEqual(calls.map((call) => call.method), ["turn/interrupt"]);
});
