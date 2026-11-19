import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

const fragment = (name: string) => readFileSync(
  new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");
const service = fragment("30-service-tier.js");
const provider = service.slice(0, service.indexOf("  function codexManagedSessionConnection("));
const rpc = fragment("91-extension-rpc.js");
type Frame = any[];

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

async function settle() {
  for (let i = 0; i < 50; i++) await Promise.resolve();
}

class Clock {
  now = 0;
  private nextId = 0;
  private timers = new Map<number, { at: number; run: () => void }>();

  setTimeout = (callback: (...args: any[]) => void, delay = 0, ...args: any[]) => {
    const id = ++this.nextId;
    this.timers.set(id, { at: this.now + delay, run: () => callback(...args) });
    return id;
  };

  clearTimeout = (id: number) => { this.timers.delete(id); };

  async advance(duration: number) {
    const target = this.now + duration;
    for (;;) {
      const next = [...this.timers.entries()]
        .filter(([, timer]) => timer.at <= target)
        .sort((left, right) => left[1].at - right[1].at || left[0] - right[0])[0];
      if (!next) break;
      this.now = next[1].at;
      this.timers.delete(next[0]);
      next[1].run();
      await settle();
    }
    this.now = target;
    await settle();
  }
}

function providerRuntime(clock: Clock, postJson: (...args: any[]) => any = () => assert.fail("an active thread must not be changed")) {
  const settings = {
    providerSyncEnabled: true,
    relayProfilesEnabled: true,
    activeRelayId: "relay",
    relayProfiles: [{ id: "relay", relayMode: "pureApi", configContents: 'model_provider = "custom"' }],
  };
  class FakeDate extends Date {
    static now() { return clock.now; }
  }
  const window = { setTimeout: clock.setTimeout, clearTimeout: clock.clearTimeout };
  return new Function("window", "codexPlusBackendSettings", "codexModelCatalog",
    "sendCodexPlusDiagnostic", "postJson", "codexAppServerPreparationTimeoutMs",
    "setTimeout", "clearTimeout", "Date", `${provider}
      return { align: prepareCodexThreadProviderResume, preparations: codexThreadProviderPreparations };`)(
    window, settings, {}, () => {}, postJson,
    2000, clock.setTimeout, clock.clearTimeout, FakeDate);
}

function rpcRuntime(clock: Clock, discovery: Promise<any>, prepare: (...args: any[]) => Promise<any>,
  options: {
    preparations?: Map<string, any>;
    context?: boolean;
    refresh?: (...args: any[]) => Promise<void>;
  } = {}) {
  const sent: Frame[] = [];
  const received: Frame[] = [];
  const failures: string[] = [];
  class PortMessageEvent extends MessageEvent<any> {
    get data() { return structuredClone(super.data); }
  }
  class MessagePort {
    postMessage(frame: Frame) { sent.push(structuredClone(frame)); }
    dispatchEvent(event: MessageEvent) {
      Object.defineProperty(event, "currentTarget", { value: this });
      received.push(event.data);
      return true;
    }
  }
  const window = {
    MessagePort, MessageEvent: PortMessageEvent,
    setTimeout: clock.setTimeout, clearTimeout: clock.clearTimeout,
  } as any;
  class FakeDate extends Date {
    static now() { return clock.now; }
  }
  new Function("window", "codexPlusBackendSettings", "prepareCodexThreadProviderResume",
    "codexThreadProviderPreparations", "codexPlusAppServerClient",
    "codexPlusExtensionListeners", "codexPlusExtensionInterceptors",
    "codexPlusRunWaterfall", "codexPlusMarkExtensionFailure", "showToast",
    "codexPerModelContextEnabled", "refreshCodexThreadModelBeforeTurn",
    "setTimeout", "clearTimeout", "Date", rpc)(
    window, { providerSyncEnabled: true }, prepare, options.preparations ?? new Map(),
    () => discovery, new Map(), new Map(),
    (_name: string, value: any) => value,
    (_key: string, message: string) => failures.push(message), () => {},
    () => options.context === true, options.refresh ?? (async () => {}),
    clock.setTimeout, clock.clearTimeout, FakeDate);
  const port = new MessagePort();
  port.postMessage(["push", ["pipeline", -19, ["subscribe"], [{
    type: "notification", key: { hostId: "local" }, methods: [["thread/tokenUsage/updated"]],
    listener: ["export", -3],
  }]]]);
  return { port, window, sent, received, failures };
}

describe("provider preparation timeout", () => {
  it("times out a waiter without releasing its owner or running it later", async () => {
    const clock = new Clock();
    const env = providerRuntime(clock);
    const heldRead = deferred<any>();
    const calls: string[] = [];
    const activeThread = { thread: { id: "thread", status: { type: "active" }, modelProvider: "custom" } };
    const send = (label: string) => async (method: string) => {
      calls.push(`${label}:${method}`);
      assert.equal(method, "thread/read");
      return label === "A" ? heldRead.promise : activeThread;
    };
    let firstOutcome: string | undefined;
    let secondOutcome: string | undefined;
    let thirdOutcome: string | undefined;
    void env.align({ threadId: "thread" }, "local", send("A"))
      .then(() => { firstOutcome = "resolved"; }, () => { firstOutcome = "rejected"; });
    await settle();
    void env.align({ threadId: "thread" }, "local", send("B"))
      .then(() => { secondOutcome = "resolved"; }, () => { secondOutcome = "rejected"; });
    await settle();
    assert.deepEqual(calls, ["A:thread/read"]);

    await clock.advance(70_000);
    assert.equal(secondOutcome, undefined, "a legitimate 70-second owner must be allowed to finish");
    await clock.advance(60_000);
    assert.equal(secondOutcome, "rejected", "the waiter must have a bounded wait");
    assert.deepEqual(calls, ["A:thread/read"]);

    void env.align({ threadId: "thread" }, "local", send("C"))
      .then(() => { thirdOutcome = "resolved"; }, () => { thirdOutcome = "rejected"; });
    await settle();
    assert.equal(thirdOutcome, undefined);
    assert.deepEqual(calls, ["A:thread/read"], "C must not enter while A still holds the lock");

    heldRead.resolve(activeThread);
    await settle();
    assert.ok(firstOutcome, "the original owner must finish");
    assert.equal(thirdOutcome, "resolved");
    assert.deepEqual(calls, ["A:thread/read", "C:thread/read"], "the timed-out B must never execute");
    assert.equal(env.preparations.has("thread"), false);
  });

  it("does not unload or resume after a timed-out index write completes", async () => {
    const clock = new Clock();
    const write = deferred<any>();
    const env = providerRuntime(clock, () => write.promise);
    const controller = new AbortController();
    const calls: string[] = [];
    const send = async (method: string) => {
      calls.push(method);
      if (method === "thread/read") {
        return { thread: { id: "thread", status: { type: "idle" }, modelProvider: "openai" } };
      }
      assert.equal(method, "config/read");
      return { config: { model_provider: "custom" } };
    };
    const outcome = env.align({ threadId: "thread" }, "local", send, null, null, controller.signal)
      .then(() => null, (error: Error) => error);
    await settle();
    assert.deepEqual(calls, ["thread/read", "config/read", "thread/read"]);
    controller.abort(new Error("fixture timeout"));
    write.resolve({ status: "ok" });
    assert.match(String(await outcome), /fixture timeout/);
    assert.deepEqual(calls, ["thread/read", "config/read", "thread/read"]);
    assert.equal(env.preparations.get("thread").pending.targetProvider, "custom");
  });

  for (const blocked of ["discovery", "preparation"] as const) {
    it(`rejects the RPC gate when ${blocked} is late and ignores its eventual result`, async () => {
      const clock = new Clock();
      const lateDiscovery = deferred<any>();
      const latePreparation = deferred<any>();
      const client = { sendRequest: async () => ({}) };
      let prepares = 0;
      const env = rpcRuntime(clock, blocked === "discovery" ? lateDiscovery.promise : Promise.resolve(client),
        async () => {
          prepares++;
          return blocked === "preparation" ? latePreparation.promise : null;
        });
      const params = { threadId: "thread", model: null };
      const callbacks = { onSettled: ["export", -7] };
      env.port.postMessage(["push", ["pipeline", -19, ["resumeThread"], [params, callbacks]]]);
      await settle();
      const request = env.sent.find((frame) => frame[0] === "push" && frame[1][2][0] === "resumeThread");
      assert.deepEqual(request?.[1][3], [["promise", -0.25], callbacks]);
      assert.equal(prepares, blocked === "preparation" ? 1 : 0);

      await clock.advance(70_000);
      assert.equal(env.sent.some((frame) => frame[0] === "reject" && frame[1] === -0.25), false);
      await clock.advance(60_000);
      const rejections = env.sent.filter((frame) => frame[0] === "reject" && frame[1] === -0.25);
      assert.equal(rejections.length, 1, "the original request must receive a timeout rejection");
      assert.equal(env.sent.some((frame) => frame[0] === "resolve" && frame[1] === -0.25), false);

      lateDiscovery.resolve(client);
      latePreparation.resolve(null);
      await settle();
      assert.equal(prepares, blocked === "preparation" ? 1 : 0, "late discovery must not start preparation");
      assert.equal(env.sent.filter((frame) => frame[0] === "reject" && frame[1] === -0.25).length, 1);
      assert.equal(env.sent.some((frame) => frame[0] === "resolve" && frame[1] === -0.25), false,
        "a late result must not allow the native request to execute");
      env.port.dispatchEvent(new env.window.MessageEvent("message", { data: ["release", -0.25, 1] }));
      assert.equal(env.window.__codexPlusRpcResumeGates.get(env.port).pending.size, 0);
      assert.deepEqual(env.failures, []);
    });
  }

  it("removes an internal resume marker from direct and envelope RPC frames", async () => {
    for (const receiver of [-19, -20]) {
      for (const envelope of [false, true]) {
        const clock = new Clock();
        let prepares = 0;
        const env = rpcRuntime(clock, Promise.resolve({}), async () => { prepares++; return null; }, {
          preparations: new Map([["thread", { resuming: 7 }]]),
        });
        const options = { timeoutMs: 2000, priority: "normal", __codexPlusProviderResume: 7 };
        const params = envelope
          ? { hostId: "local", method: "thread/resume", params: { threadId: "thread" } }
          : { threadId: "thread" };
        const frame: Frame = ["push", ["pipeline", receiver, ["sendRequest"],
          [envelope ? "send-cli-request-for-host" : "thread/resume", params, options]]];
        env.port.postMessage(frame);
        await settle();
        const sent = env.sent.find((item) => item[0] === "push" && item[1][2][0] === "sendRequest");
        assert.deepEqual(sent?.[1][3], [frame[1][3][0], params, { timeoutMs: 2000, priority: "normal" }]);
        assert.equal(options.__codexPlusProviderResume, 7, "the caller's options must remain unchanged");
        assert.equal(prepares, 0);
      }
    }
  });

  for (const event of ["release", "disconnect"] as const) {
    for (const blocked of ["discovery", "preparation"] as const) {
      it(`aborts RPC cancellation by ${event} during ${blocked}`, async () => {
        const clock = new Clock();
        const discovery = deferred<any>();
        const preparation = deferred<void>();
        const client = { sendRequest: async () => ({}) };
        const calls: string[] = [];
        let signal: AbortSignal | undefined;
        const env = rpcRuntime(clock, blocked === "discovery" ? discovery.promise : Promise.resolve(client),
          async (_params, _host, _send, _client, _internal, currentSignal) => {
            calls.push("prepare");
            signal = currentSignal;
            await preparation.promise;
            signal?.throwIfAborted();
            calls.push("write");
            return null;
          });
        env.port.postMessage(["push", ["pipeline", -19, ["resumeThread"], [{ threadId: "thread" }]]]);
        await settle();
        assert.deepEqual(calls, blocked === "preparation" ? ["prepare"] : []);
        env.port.dispatchEvent(new env.window.MessageEvent("message", {
          data: event === "release" ? ["release", -0.25, 1] : null,
        }));
        if (blocked === "preparation") {
          assert.ok(signal, "preparation must receive a cancellation signal");
          assert.equal(signal.aborted, true);
        }
        discovery.resolve(client);
        preparation.resolve();
        await settle();
        assert.deepEqual(calls, blocked === "preparation" ? ["prepare"] : []);
        assert.equal(env.sent.some((frame) => ["resolve", "reject"].includes(frame[0]) && frame[1] === -0.25), false);
        assert.equal(env.window.__codexPlusRpcResumeGates.get(env.port).pending.size, 0);
        assert.deepEqual(env.failures, []);
      });
    }
  }

  it("times out RPC context without releasing its serialization before an in-flight unsubscribe settles", async () => {
    const clock = new Clock();
    const unsubscribe = deferred<void>();
    const calls: string[] = [];
    let prepares = 0;
    const client = {
      async unsubscribeInactiveConversation() { calls.push("unsubscribe"); await unsubscribe.promise; },
      async sendRequest(method: string) { calls.push(method); return {}; },
      async resumeThread() { calls.push("resumeThread"); return {}; },
    };
    const env = rpcRuntime(clock, Promise.resolve(client), async () => { prepares++; return null; }, {
      context: true,
      refresh: async (_host, send, _method, params) => {
        calls.push(`context:${params.fixture}`);
        if (params.fixture === "A") {
          await send("thread/unsubscribe", { threadId: "thread" });
          await send("thread/resume", { threadId: "thread", config: {} });
        }
      },
    });
    const candidate = (id: number, fixture: string): Frame => ["resolve", id, {
      executionHostId: "local", conversationId: "thread", resume: { conversationId: "thread" },
      start: { request: { threadId: "thread", input: [[]], fixture } },
    }];
    env.port.postMessage(candidate(1, "A"));
    await settle();
    assert.equal(prepares, 1);
    assert.deepEqual(calls, ["context:A", "unsubscribe"]);
    await clock.advance(70_000);
    assert.equal(env.sent.some((frame) => frame[0] === "reject" && frame[1] === -0.25), false);
    await clock.advance(60_000);
    assert.equal(env.sent.filter((frame) => frame[0] === "reject" && frame[1] === -0.25).length, 1);

    env.port.postMessage(candidate(2, "B"));
    await settle();
    assert.equal(prepares, 1, "the next turn must wait for the first in-flight operation");
    unsubscribe.resolve();
    await settle();
    assert.equal(prepares, 2);
    assert.deepEqual(calls, ["context:A", "unsubscribe", "context:B"]);
    assert.equal(env.sent.some((frame) => frame[0] === "resolve" && frame[1] === -0.25), false);
    assert.equal(env.sent.some((frame) => frame[0] === "resolve" && frame[1] === -1.25), true);
    assert.deepEqual(env.failures, []);
  });

  for (const event of ["release", "disconnect"] as const) {
    it(`stops RPC context after ${event} during an in-flight unsubscribe`, async () => {
      const clock = new Clock();
      const unsubscribe = deferred<void>();
      const calls: string[] = [];
      const client = {
        async unsubscribeInactiveConversation() { calls.push("unsubscribe"); await unsubscribe.promise; },
        async sendRequest(method: string) { calls.push(method); return {}; },
      };
      const env = rpcRuntime(clock, Promise.resolve(client), async () => null, {
        context: true,
        refresh: async (_host, send) => {
          await send("thread/unsubscribe", { threadId: "thread" });
          await send("thread/resume", { threadId: "thread" });
        },
      });
      env.port.postMessage(["resolve", 1, {
        executionHostId: "local", conversationId: "thread", resume: { conversationId: "thread" },
        start: { request: { threadId: "thread", input: [[]] } },
      }]);
      await settle();
      assert.deepEqual(calls, ["unsubscribe"]);
      env.port.dispatchEvent(new env.window.MessageEvent("message", {
        data: event === "release" ? ["release", -0.25, 1] : null,
      }));
      unsubscribe.resolve();
      await settle();
      assert.deepEqual(calls, ["unsubscribe"]);
      assert.equal(env.sent.some((frame) => ["resolve", "reject"].includes(frame[0]) && frame[1] === -0.25), false);
      assert.equal(env.window.__codexPlusRpcResumeGates.get(env.port).pending.size, 0);
      assert.deepEqual(env.failures, []);
    });
  }
});
