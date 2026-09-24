const assert = require("node:assert/strict");
const { createRecovery } = require("./composer-readiness.js");

const flush = () => new Promise((resolve) => setImmediate(resolve));
function fixture({ hostId = "local", responds = true, cancels = true } = {}) {
  let now = 0;
  const recovery = createRecovery({ now: () => now });
  const home = {
    queryKey: ["vscode", "codex-home", JSON.stringify({ hostId })],
    state: { status: "pending", fetchStatus: "fetching", data: undefined },
    isActive: () => true,
  };
  const workspace = { queryKey: ["git", hostId, "managed-worktree-state", "codex-home-loading", "/project"], getObserversCount: () => 1 };
  const queries = [home, workspace];
  const calls = [];
  const client = {
    getQueryCache: () => ({ getAll: () => queries }),
    cancelQueries: async (filter) => {
      assert.equal(filter.exact, true);
      assert.deepEqual(filter.queryKey, home.queryKey);
      calls.push("cancel");
      if (!cancels) throw new Error("cancel failed");
      home.state.fetchStatus = "idle";
    },
    refetchQueries: (filter) => {
      assert.equal(filter.exact, true);
      assert.deepEqual(filter.queryKey, home.queryKey);
      calls.push("refetch");
      home.state.fetchStatus = "fetching";
      if (!responds) return new Promise(() => {});
      home.state = { status: "success", fetchStatus: "idle", data: { codexHome: "/real-home" } };
      return Promise.resolve();
    },
  };
  return { recovery, home, queries, calls, client, tick: (time, enabled = true) => { now = time; recovery.tick(client, enabled); } };
}

(async () => {
  const stalled = fixture();
  stalled.tick(0);
  stalled.tick(9999);
  await flush();
  assert.deepEqual(stalled.calls, [], "ordinary requests get time to complete");
  stalled.tick(10000);
  stalled.tick(10000);
  await flush();
  assert.deepEqual(stalled.calls, ["cancel", "refetch"], "cancel the orphaned promise before refetching exactly once");
  assert.equal(stalled.home.state.data.codexHome, "/real-home", "use the native response, never fabricate workspace readiness");
  stalled.tick(90000);
  assert.equal(stalled.calls.length, 2, "successful requests need no further recovery");

  const offline = fixture({ responds: false });
  for (const time of [0, 10000, 20000, 30000, 90000]) { offline.tick(time); await flush(); }
  assert.deepEqual(offline.calls, ["cancel", "refetch", "cancel", "refetch"], "permanently unavailable native IPC must not cause endless retries");
  assert.equal(offline.home.state.data, undefined, "failure must not unlock workspace checks with fake data");

  for (const change of [
    f => { f.home.queryKey = ["vscode", "codex-home", "invalid json"]; },
    f => { f.home.queryKey = ["vscode", "codex-home", JSON.stringify({ hostId: "remote" })]; },
    f => { f.home.queryKey = ["vscode", "get-global-state"]; },
    f => { f.home.isActive = () => false; },
    f => { f.home.state.data = { codexHome: "/cached-home" }; },
    f => { f.home.state.fetchStatus = "paused"; },
    f => { f.queries.splice(1); },
    f => { f.queries[1].getObserversCount = () => 0; },
    f => { f.client.cancelQueries = undefined; },
  ]) {
    const f = fixture(); change(f); f.tick(0); f.tick(20000); await flush();
    assert.deepEqual(f.calls, [], "unrelated, remote, inactive and healthy queries stay untouched");
  }
  const off = fixture(); off.tick(0, false); off.tick(20000, false); await flush();
  assert.deepEqual(off.calls, [], "only official mixed-key mode enables recovery");
  const switched = fixture(); switched.tick(0); switched.tick(10000); switched.tick(10001, false); await flush();
  assert.deepEqual(switched.calls, [], "do not cancel a request after leaving mixed-key mode");
  const slowCancel = fixture({ responds: false });
  let finishCancel;
  slowCancel.client.cancelQueries = () => {
    slowCancel.calls.push("cancel");
    slowCancel.home.state.fetchStatus = "idle";
    return new Promise(resolve => { finishCancel = resolve; });
  };
  slowCancel.tick(0); slowCancel.tick(10000); await flush();
  slowCancel.tick(20000); await flush();
  assert.deepEqual(slowCancel.calls, ["cancel"], "an idle snapshot during cancellation must not reset the retry budget");
  finishCancel(); await flush();
  slowCancel.tick(30000); await flush(); finishCancel(); await flush();
  slowCancel.tick(50000); await flush();
  assert.deepEqual(slowCancel.calls, ["cancel", "refetch", "cancel", "refetch"]);
  const switchDuringCancel = fixture();
  switchDuringCancel.client.cancelQueries = () => new Promise(resolve => { finishCancel = resolve; });
  switchDuringCancel.tick(0); switchDuringCancel.tick(10000); await flush();
  switchDuringCancel.tick(10001, false); finishCancel(); await flush();
  assert.deepEqual(switchDuringCancel.calls, ["refetch"], "an already cancelled native read must be restored even if the mode changes");
  const failedCancel = fixture({ cancels: false }); failedCancel.tick(0); failedCancel.tick(10000); await flush();
  assert.deepEqual(failedCancel.calls, ["cancel"], "a failed cancellation must not create a concurrent request");
  const bare = fixture(); bare.home.queryKey = ["vscode", "codex-home"]; bare.tick(0); bare.tick(10000); await flush();
  assert.deepEqual(bare.calls, ["cancel", "refetch"], "the default local home query also recovers");
  console.log("composer readiness recovery contracts passed");
})().catch((error) => { console.error(error); process.exitCode = 1; });
