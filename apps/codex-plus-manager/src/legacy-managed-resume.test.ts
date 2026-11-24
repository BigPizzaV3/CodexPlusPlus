import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = fs.readFileSync(new URL("../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const start = source.indexOf("  function codexConfigWithoutManagedProvider(");
const end = source.indexOf("  async function prepareCodexManagedSessionConnection(");
assert.ok(start >= 0 && end > start);

const sendResume = vm.runInNewContext(`${source.slice(start, end)}\nsendCodexManagedResumeWithFallback`) as (
  method: string, params: unknown, host: string, send: (params: unknown) => Promise<unknown>,
) => Promise<unknown>;

const rejected = "Cannot change this thread's model provider from \`openai\` to \`codex_plus_managed\` on resume";
const params = {
  threadId: "existing", model: "chosen", modelProvider: "codex_plus_managed",
  config: {
    model_provider: "codex_plus_managed",
    model_providers: { codex_plus_managed: { name: "managed" }, other: { name: "other" } },
    "model_providers.codex_plus_managed.base_url": "http://127.0.0.1",
    model_reasoning_effort: "high",
  },
};

test("explicit native refusal of a local resume retries once on the original provider", async () => {
  const calls: unknown[] = [];
  await sendResume("thread/resume", params, "local", async (args) => {
    calls.push(JSON.parse(JSON.stringify(args)));
    if (calls.length === 1) throw new Error(rejected);
    return { modelProvider: "openai" };
  });
  assert.deepEqual(calls, [params, {
    ...params, modelProvider: "openai",
    config: { model_providers: { other: { name: "other" } }, model_reasoning_effort: "high" },
  }]);
  assert.ok(params.config.model_providers.codex_plus_managed);
});

test("does not replay on timeout, different provider, remote host, or a turn", async () => {
  const cases = [
    ["thread/resume", params, "local", "request timed out"],
    ["thread/resume", params, "local", rejected.replace("openai", "another")],
    ["thread/resume", params, "remote", rejected],
    ["thread/resume", { ...params, hostId: "remote" }, "local", rejected],
    ["turn/start", params, "local", rejected],
  ] as const;
  for (const [method, args, host, reason] of cases) {
    let count = 0;
    await assert.rejects(sendResume(method, args, host, async () => { count++; throw new Error(reason); }), { message: reason });
    assert.equal(count, 1);
  }
});
