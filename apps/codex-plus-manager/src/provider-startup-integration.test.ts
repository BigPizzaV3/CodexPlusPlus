import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const bundle = readFileSync(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

test("generated renderer installs provider RPC preparation and completes startup", async () => {
  const sent: any[] = [];
  const requests: string[] = [];
  const preparations: any[] = [];
  const store = new Map<string, string>();
  let timerId = 0;
  class Port {
    postMessage(frame: any) { sent.push(frame); }
  }
  const node = () => ({
    dataset: {}, style: { setProperty() {}, removeProperty() {} }, children: [],
    classList: { add() {}, remove() {}, toggle() {}, contains() { return false; } },
    appendChild() {}, append() {}, prepend() {}, remove() {}, setAttribute() {}, removeAttribute() {}, toggleAttribute() {},
    addEventListener() {}, removeEventListener() {}, querySelector() { return null; },
    querySelectorAll() { return []; }, closest() { return null; }, matches() { return false; },
  });
  const client = {
    async sendRequest(method: string) {
      requests.push(method);
      if (method === "thread/read") return { thread: { status: { type: "notLoaded" }, modelProvider: "openai" } };
      if (method === "config/read") return { config: { model_provider: "custom" } };
      throw new Error(`unexpected request: ${method}`);
    },
    async getConversation() { return null; },
    subscribe() {
      return Object.assign(Promise.resolve({ [Symbol.dispose]() {} }), { [Symbol.dispose]() {} });
    },
  };
  const scope = { node: {}, get: () => ({ forHost: () => client }) };
  const scopeNode = { __reactFiber$fixture: { memoizedState: { memoizedState: { current: scope } } } };
  const root = node();
  const body = node();
  const head = node();
  const context: any = {
    process, console, Promise, URL, JSON, AbortController, queueMicrotask, MessagePort: Port,
    setTimeout: () => ++timerId, clearTimeout() {}, setInterval: () => ++timerId, clearInterval() {},
    requestAnimationFrame: () => ++timerId, cancelAnimationFrame() {},
    addEventListener() {}, removeEventListener() {}, dispatchEvent() {},
    document: {
      scripts: [{ src: "data:text/javascript,export%20const%20atom%3D%7B%7D%3B%2F%2F/assets/app-initial-fixture.js" }],
      documentElement: root, body, head, activeElement: null, hidden: false,
      createElement: () => node(), createElementNS: () => node(), getElementById: () => null,
      querySelector: () => null,
      querySelectorAll: (selector: string) => selector === "[data-app-action-sidebar-thread-id], [data-codex-composer-root], main, aside"
        ? [scopeNode] : [],
      addEventListener() {}, removeEventListener() {},
    },
    localStorage: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: unknown) => store.set(key, String(value)),
      removeItem: (key: string) => store.delete(key),
    },
    location: { href: "app://-/index.html", pathname: "/", search: "", hash: "" },
    navigator: { userAgent: "node-test", language: "en" }, performance: { getEntriesByType: () => [] },
    MutationObserver: class { observe() {} disconnect() {} },
    fetch: async () => ({ ok: false, text: async () => "" }),
    __codexSessionDeleteBridge: async (path: string, payload: any) => {
      if (path === "/settings/get") return { enhancementsEnabled: true, providerSyncEnabled: true };
      if (path === "/thread-provider/prepare") {
        preparations.push(payload);
        return { status: "ok" };
      }
      return { status: "ok" };
    },
  };
  context.window = context;
  context.globalThis = context;

  new Function(...Object.keys(context), bundle)(...Object.values(context));
  assert.equal(context.__codexPlusRpcFrameInterceptor?.version, 19);
  assert.ok(context.__codexSessionDeleteObserver, "startup must install its observer after the RPC fragment");
  assert.ok(context.codexPlus, "startup must expose the extension API after the RPC fragment");

  // Let the real startup settings read settle before the host sends a frame.
  await new Promise((resolve) => setTimeout(resolve, 0));
  const port = new Port();
  port.postMessage(["push", ["pipeline", 1, ["subscribe"], [{ key: { hostId: "local" } }]]]);
  port.postMessage(["push", ["pipeline", 1, ["resumeThread"], [{ threadId: "thread-1" }]]]);
  for (let attempt = 0; attempt < 100 && !sent.some((frame) => frame[0] === "resolve" && frame[1] === -0.25); attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.deepEqual(requests, ["thread/read", "config/read"]);
  assert.deepEqual(preparations, [{
    host_id: "local", thread_id: "thread-1", source_provider: "openai", target_provider: "custom",
  }]);
  assert.deepEqual(sent.at(-1), ["resolve", -0.25, { threadId: "thread-1" }]);
});
