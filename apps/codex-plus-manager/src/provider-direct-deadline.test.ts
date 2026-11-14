import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
const provider = source.slice(0, source.indexOf("  function codexManagedSessionConnection("));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

async function settle() {
  for (let i = 0; i < 50; i++) await Promise.resolve();
}

function runtime(write: (payload: any) => Promise<any>) {
  const timers = new Map<number, () => void>();
  let nextTimer = 0;
  const settings = {
    providerSyncEnabled: true, relayProfilesEnabled: true, activeRelayId: "relay",
    relayProfiles: [{ id: "relay", relayMode: "pureApi", configContents: 'model_provider = "custom"' }],
  };
  const api = new Function("window", "codexPlusBackendSettings", "codexModelCatalog", "sendCodexPlusDiagnostic",
    "postJson", "readCodexAppServerPreparation", "codexAppServerPreparationTimeoutMs", "setTimeout", "clearTimeout",
    `${provider}\nreturn { prepare: prepareCodexPureApiResume, preparations: codexThreadProviderPreparations };`)(
    {}, settings, {}, () => {}, (_route: string, payload: any) => write(payload),
    async () => ({ status: "ok", active: false, generation: "" }), 2000,
    (callback: () => void) => { timers.set(++nextTimer, callback); return nextTimer; },
    (id: number) => timers.delete(id));
  return { api, timers, settings };
}

describe("direct provider preparation", () => {
  it("does not read a normal direct thread on turn/start when provider sync is disabled", async () => {
    const env = runtime(async () => assert.fail("no index write"));
    env.settings.providerSyncEnabled = false;
    const params = { threadId: "thread", input: [] };
    assert.equal(await env.api.prepare("turn/start", params, "local", false,
      async () => assert.fail("no provider preflight")), params);
  });

  it("allows a turn to proceed when the initial provider state read fails", async () => {
    const env = runtime(async () => assert.fail("no index write"));
    const params = { threadId: "thread", input: [] };
    const calls: string[] = [];
    const result = await env.api.prepare("turn/start", params, "local", false, async (method: string) => {
      calls.push(method);
      throw new Error("thread read unavailable");
    });
    assert.equal(result, params);
    assert.deepEqual(calls, ["thread/read"]);
  });

  it("allows a turn to proceed when the initial configuration read fails", async () => {
    const env = runtime(async () => assert.fail("no index write"));
    const params = { threadId: "thread", input: [] };
    const calls: string[] = [];
    const result = await env.api.prepare("turn/start", params, "local", false, async (method: string) => {
      calls.push(method);
      if (method === "thread/read") {
        return { thread: { modelProvider: "openai", status: { type: "idle" } } };
      }
      throw new Error("configuration unavailable");
    });
    assert.equal(result, params);
    assert.deepEqual(calls, ["thread/read", "config/read"]);
  });

  it("rejects a hung caller while the in-flight write retains the lock", async () => {
    const write = deferred<any>();
    const env = runtime(() => write.promise);
    const calls: string[] = [];
    let active = false;
    const send = (label: string) => async (method: string) => {
      calls.push(`${label}:${method}`);
      if (method === "config/read") return { config: { model_provider: "custom" } };
      assert.equal(method, "thread/read");
      return { thread: { status: { type: active ? "active" : "idle" }, modelProvider: "openai" } };
    };
    const first = env.api.prepare("turn/start", { threadId: "thread" }, "local", false, send("A"))
      .then(() => null, (error: Error) => error);
    await settle();
    const deadline = [...env.timers.values()][0];
    assert.ok(deadline);
    deadline();
    assert.match(String(await first), /会话准备超时/);
    const before = calls.length;
    const second = env.api.prepare("turn/start", { threadId: "thread" }, "local", false, send("B"));
    await settle();
    assert.equal(calls.length, before);
    assert.ok(env.api.preparations.get("thread").tail);

    active = true;
    write.resolve({ status: "ok" });
    await second;
    assert.equal(calls.at(-1), "B:thread/read");
    assert.equal(calls.some((call) => call.endsWith("thread/unsubscribe") || call.endsWith("thread/resume")), false);
  });

  it("holds the lock through the direct resume before another turn can prepare", async () => {
    const resume = deferred<any>();
    let sourceProvider = "openai";
    let status = "idle";
    const env = runtime(async () => { sourceProvider = "custom"; return { status: "ok" }; });
    const calls: string[] = [];
    const send = (label: string) => async (method: string) => {
      calls.push(`${label}:${method}`);
      if (method === "thread/read") return { thread: { status: { type: status }, modelProvider: sourceProvider } };
      if (method === "config/read") return { config: { model_provider: "custom" } };
      if (method === "thread/unsubscribe") { status = "notLoaded"; return {}; }
      if (method === "thread/resume") {
        await resume.promise;
        status = "idle";
        return { modelProvider: "custom" };
      }
      assert.fail(`unexpected ${method}`);
    };
    const first = env.api.prepare("turn/start", { threadId: "thread", model: "saved" }, "local", false, send("A"));
    await settle();
    assert.equal(calls.at(-1), "A:thread/resume");
    const second = env.api.prepare("turn/start", { threadId: "thread" }, "local", false, send("B"));
    await settle();
    assert.equal(calls.some((call) => call.startsWith("B:")), false);
    resume.resolve(null);
    await Promise.all([first, second]);
    assert.equal(calls.filter((call) => call === "A:thread/resume").length, 1);
    assert.equal(calls.at(-1), "B:config/read");
  });
});
