import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

const runtimeSource = readFileSync(
  new URL("../../../assets/inject/renderer-inject/91-extension-runtime.js", import.meta.url), "utf8");
const source = runtimeSource.slice(runtimeSource.indexOf("  function installCodexThreadContextObserver()"),
  runtimeSource.indexOf("  /** 把 append"));

function runtime() {
  const window = {} as any;
  const calls: any[] = [];
  const diagnostics: any[] = [];
  const state = {
    enabled: true, now: 10000, failure: "", pendingClient: null as Promise<any> | null,
    listener: null as ((notification: any) => void) | null,
    broken: null as (() => void) | null,
    disposals: 0,
  };
  const handle = {
    onRpcBroken(callback: () => void) { state.broken = callback; },
    [Symbol.dispose]() { state.disposals++; },
  };
  const client = {
    subscribe(params: any) {
      calls.push({ method: "subscribe", params });
      state.listener = params.listener;
      return Object.assign(Promise.resolve(handle), { [Symbol.dispose]() { state.disposals++; } });
    },
  };
  const create = () => new Function("window", "codexPerModelContextEnabled", "codexPlusAppServerClient",
    "sendCodexPlusDiagnostic", "Date", `${source}\nreturn installCodexThreadContextObserver;`)(
    window, () => state.enabled, async (hostId: string) => {
      calls.push({ method: "client", hostId });
      if (state.failure) throw new Error(state.failure);
      return state.pendingClient ?? client;
    }, (event: string, detail: any) => diagnostics.push({ event, detail }), { now: () => state.now },
  );
  return { window, calls, diagnostics, state, client, create, install: create() };
}

describe("local runtime context observer", () => {
  it("subscribes once with the real host notification shape across scans and reinjections", async () => {
    const env = runtime();
    const pending = env.install();
    assert.equal(env.install(), pending);
    await pending;
    await env.install();
    await env.create()();
    assert.deepEqual(env.calls.map((call) => call.method), ["client", "subscribe"]);
    const params = env.calls[1].params;
    assert.equal(params.type, "notification");
    assert.deepEqual(params.key, { hostId: "local" });
    assert.deepEqual(params.methods, ["thread/tokenUsage/updated"]);
    assert.equal(env.window.__codexPlusThreadContextObserver.handle !== null, true);
  });

  it("does not request a client or subscribe when context support is disabled", async () => {
    const env = runtime();
    env.state.enabled = false;
    await env.install();
    assert.equal(env.calls.length, 0);
    assert.equal(env.window.__codexPlusThreadContextObserver, undefined);
  });

  it("checks activation again if the selected provider changes during client discovery", async () => {
    const env = runtime();
    let resolve!: (client: any) => void;
    env.state.pendingClient = new Promise((done) => { resolve = done; });
    const pending = env.install();
    env.state.enabled = false;
    resolve(env.client);
    await pending;
    assert.deepEqual(env.calls.map((call) => call.method), ["client"]);
  });

  it("records actual windows without estimating model-specific percentages and bounds storage", async () => {
    const env = runtime();
    await env.install();
    for (let index = 0; index < 300; index++) {
      env.state.listener!({ method: "thread/tokenUsage/updated",
        params: { threadId: `thread-${index}`, tokenUsage: { modelContextWindow: 123456 + index } } });
    }
    const observed = env.window.__codexPlusObservedContextWindows;
    assert.equal(observed.size, 256);
    assert.equal(observed.get("thread-299"), 123755);
    assert.equal(observed.has("thread-0"), false);
    env.state.listener!({ params: { threadId: "bad", tokenUsage: { modelContextWindow: null } } });
    assert.equal(observed.has("bad"), false);
  });

  it("logs discovery failures once per cooldown and retries after the scope is ready", async () => {
    const env = runtime();
    env.state.failure = "scope unavailable";
    await env.install();
    for (let index = 0; index < 30; index++) await env.install();
    assert.equal(env.calls.length, 1);
    assert.equal(env.diagnostics.length, 1);
    assert.equal(env.diagnostics[0].event, "thread_context_observer_failed");
    assert.equal(env.diagnostics[0].detail.errorMessage, "scope unavailable");
    env.state.now += 5000;
    env.state.failure = "";
    await env.install();
    assert.deepEqual(env.calls.map((call) => call.method), ["client", "client", "subscribe"]);
  });

  it("releases a broken connection and discovers a replacement after cooldown", async () => {
    const env = runtime();
    await env.install();
    env.state.broken!();
    assert.equal(env.state.disposals, 2);
    assert.equal(env.window.__codexPlusThreadContextObserver.handle, null);
    assert.equal(env.diagnostics[0].event, "thread_context_observer_disconnected");
    await env.install();
    assert.equal(env.calls.length, 2);
    env.state.now += 5000;
    await env.install();
    assert.equal(env.calls.filter((call) => call.method === "subscribe").length, 2);
  });
});
