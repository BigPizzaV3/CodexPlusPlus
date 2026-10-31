"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const { test } = require("node:test");
const path = require("node:path");
const source = fs.readFileSync(path.join(__dirname, "../../assets/inject/renderer-inject/85-weixin-desktop-sync.js"), "utf8");
const id = "00000000-0000-4000-8000-000000000001";
const revision = "00000000-0000-4000-8000-000000000002";
const secondId = "00000000-0000-4000-8000-000000000003";
const nextRevision = "00000000-0000-4000-8000-000000000004";

function setup(notifyThread, extra = {}) {
  let now = 1000000;
  const timers = [];
  const schedule = (...args) => timers.push(args);
  const catalog = { notifyThread, readEntries() {} };
  const state = extra.state || { projectsByHome: {}, assignments: {} };
  const keys = {
    APP_SERVER_PROJECT_ID_BY_LEGACY_PROJECT_ID_BY_HOST: "projectsByHome",
    THREAD_PROJECT_ASSIGNMENTS: "assignments",
  };
  const services = {
    localThreadCatalog: catalog,
    projects: { getLocalProjects: async () => ({}) },
    executionPaths: { readCodexHome: async () => "C:/codex-home" },
    threadProjectAssignments: { setProjectless: async () => {}, assignIfUnassigned: async () => {} },
    ...extra,
  };
  class NativeClient {
    static getInstance() { return new NativeClient(); }
    async post(route, body) {
      assert.equal(route, "vscode://codex/get-global-state");
      return { body: { value: state[JSON.parse(body).key] ?? null } };
    }
    sendRequest() {}
    onFetchResponse() {}
    cancelStream() {}
  }
  const context = vm.createContext({
    window: { setTimeout: schedule, setInterval: schedule },
    Date: { now: () => now }, setTimeout: schedule, setInterval: schedule,
    codexPlusBackendSettingsLoaded: true,
    codexPlusBackendSettings: { weixinDesktopSync: [{ threadId: id, revision }] },
    loadOptionalCodexAppModule: async () => ({
      arbitraryExport: services, arbitraryConstants: keys, arbitraryClient: NativeClient,
      misleadingStateUpdater: () => { throw Error("get-global-state set-global-state params"); },
      permissiveRpc: new Proxy(() => {}, {
        get: (_, key) => key === "prototype" ? NativeClient.prototype : () => {},
      }),
    }),
    getCodexGlobalState: () => assert.fail("Do not use the ambiguous state-function matcher"),
  });
  const reinject = () => vm.runInContext(`(() => { ${source}\nwindow.syncWeixinDesktopThreads = syncWeixinDesktopThreads; })()`, context);
  reinject();
  return {
    context, state, timers, reinject,
    sync: context.window.__codexPlusWeixinDesktopSync,
    run: () => context.window.syncWeixinDesktopThreads(),
    advance: (milliseconds) => { now += milliseconds; },
  };
}

test("notifies only local IDs, deduplicates revisions and survives reinjection", async () => {
  const calls = [];
  const { context, sync, timers, reinject, run } = setup(async (...args) => calls.push(args));
  await run();
  await run();
  reinject();
  await run();
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0].hostId, "local");
  assert.equal(calls[0][0].threadId, id);
  assert.equal(calls[0][1], "update");
  assert.equal(context.window.__codexPlusWeixinDesktopSync, sync);
  context.codexPlusBackendSettings.weixinDesktopSync[0].revision = id;
  await run();
  assert.equal(calls.length, 2);
  assert.equal(timers.length, 0);
});

test("failures are retried without acknowledging or parallel notification", async () => {
  let calls = 0, release;
  const { sync, timers, reinject, advance, run } = setup(() => {
    calls += 1;
    if (calls > 1) return Promise.resolve();
    return new Promise((_, reject) => { release = () => reject(Error("not active")); });
  });
  const first = run();
  await new Promise((resolve) => setImmediate(resolve));
  const flight = sync.flight;
  reinject();
  await run();
  assert.equal(calls, 1);
  assert.equal(sync.flight, flight);
  release();
  await first;
  const retry = sync.retries.get(id);
  reinject();
  await run();
  assert.equal(calls, 1);
  assert.equal(sync.revisions.size, 0);
  assert.equal(sync.retries.get(id), retry);
  assert.equal(retry.revision, revision);
  assert.equal(retry.retryAt, 1030000);
  assert.equal(sync.retryAt, 0);
  advance(29999);
  await run();
  assert.equal(calls, 1);
  advance(1);
  await run();
  await run();
  assert.equal(calls, 2);
  assert.equal(sync.revisions.get(id), revision);
  assert.equal(sync.retries.size, 0);
  assert.equal(sync.flight, null);
  assert.equal(timers.length, 0);
});

test("reinjection upgrades old state without losing acknowledged revisions", async () => {
  let calls = 0;
  const { context, sync, timers, reinject, run } = setup(async () => { calls += 1; });
  await run();
  delete sync.retries;
  reinject();
  await run();
  assert.equal(context.window.__codexPlusWeixinDesktopSync, sync);
  assert.equal(sync.revisions.get(id), revision);
  assert.equal(sync.retries.size, 0);
  assert.equal(calls, 1);
  assert.equal(timers.length, 0);
});

test("a broken selected project does not block good entries or their new revisions", async () => {
  const notified = [], assigned = [];
  let projectReads = 0;
  const { context, state, sync, advance, run } = setup(async ({ threadId }) => notified.push(threadId), {
    state: { projectsByHome: { "local:C:\\codex-home": { desktop: project.id } }, assignments: {} },
    projects: {
      getLocalProjects: async () => {
        projectReads += 1;
        return { desktop: {}, repaired: {} };
      },
    },
    threadProjectAssignments: {
      setProjectless: async () => {},
      assignIfUnassigned: async ([request]) => assigned.push(request),
    },
  });
  context.codexPlusBackendSettings.weixinDesktopSync = [
    { threadId: id, revision, project: { ...project, id: nextRevision } },
    { threadId: secondId, revision, project },
  ];
  await run();
  assert.deepEqual(notified, [secondId]);
  assert.equal(sync.revisions.has(id), false);
  assert.equal(sync.revisions.get(secondId), revision);
  assert.equal(sync.retries.get(id).revision, revision);
  assert.equal(sync.retryAt, 0);
  assert.equal(assigned[0].assignment.projectId, "desktop");
  assert.equal(projectReads, 2);
  await run();
  context.codexPlusBackendSettings.weixinDesktopSync[1].revision = nextRevision;
  await run();
  assert.deepEqual(notified, [secondId, secondId]);
  assert.equal(projectReads, 3);
  assert.equal(sync.revisions.get(secondId), nextRevision);
  state.projectsByHome["local:C:\\codex-home"].repaired = nextRevision;
  advance(29999);
  await run();
  assert.equal(projectReads, 3);
  advance(1);
  await run();
  await run();
  assert.deepEqual(notified, [secondId, secondId, id]);
  assert.equal(assigned[2].assignment.projectId, "repaired");
  assert.equal(projectReads, 4);
  assert.equal(sync.revisions.get(id), revision);
  assert.equal(sync.retries.size, 0);
});

test("a failed notification leaves only its own revision pending", async () => {
  const notified = [];
  const { context, sync, advance, run } = setup(async ({ threadId }) => {
    notified.push(threadId);
    if (threadId === id && notified.length === 1) throw Error("catalog unavailable for this thread");
  });
  context.codexPlusBackendSettings.weixinDesktopSync.push({ threadId: secondId, revision });
  await run();
  await run();
  assert.deepEqual(notified, [id, secondId]);
  assert.equal(sync.revisions.has(id), false);
  assert.equal(sync.revisions.get(secondId), revision);
  advance(30000);
  await run();
  await run();
  assert.deepEqual(notified, [id, secondId, id]);
  assert.equal(sync.revisions.get(id), revision);
  assert.equal(sync.retries.size, 0);
});

test("a changed failed revision can retry immediately", async () => {
  let calls = 0;
  const { context, sync, advance, run } = setup(async () => {
    calls += 1;
    if (calls === 1) throw Error("old revision failed");
  });
  await run();
  context.codexPlusBackendSettings.weixinDesktopSync[0].revision = nextRevision;
  await run();
  assert.equal(calls, 2);
  assert.equal(sync.revisions.get(id), nextRevision);
  assert.equal(sync.retries.size, 0);
  advance(30000);
  await run();
  assert.equal(calls, 2);
});

test("an in-flight notification acknowledges only its captured revision", async () => {
  let calls = 0, release;
  const { context, sync, run } = setup(() => {
    calls += 1;
    if (calls > 1) return Promise.resolve();
    return new Promise((resolve) => { release = resolve; });
  });
  const first = run();
  await new Promise((resolve) => setImmediate(resolve));
  context.codexPlusBackendSettings.weixinDesktopSync[0].revision = nextRevision;
  release();
  await first;
  assert.equal(sync.revisions.get(id), revision);
  await run();
  await run();
  assert.equal(calls, 2);
  assert.equal(sync.revisions.get(id), nextRevision);
});

test("shared module discovery failures back off without acknowledging any entry", async () => {
  let loads = 0;
  const notified = [];
  const { context, sync, advance, run } = setup(async ({ threadId }) => notified.push(threadId));
  const load = context.loadOptionalCodexAppModule;
  context.loadOptionalCodexAppModule = async () => {
    loads += 1;
    if (loads === 1) throw Error("desktop is starting");
    return load();
  };
  context.codexPlusBackendSettings.weixinDesktopSync.push({ threadId: secondId, revision });
  await run();
  await run();
  assert.equal(loads, 1);
  assert.equal(sync.revisions.size, 0);
  assert.equal(sync.retries.size, 0);
  assert.equal(sync.retryAt, 1030000);
  advance(30000);
  await run();
  assert.equal(loads, 2);
  assert.deepEqual(notified, [id, secondId]);
});

test("removed entries release both acknowledged and failed state", async () => {
  const { context, sync, run } = setup(async ({ threadId }) => {
    if (threadId === id) throw Error("pending");
  });
  context.codexPlusBackendSettings.weixinDesktopSync.push({ threadId: secondId, revision });
  await run();
  assert.equal(sync.retries.size, 1);
  assert.equal(sync.revisions.size, 1);
  context.codexPlusBackendSettings.weixinDesktopSync = [];
  await run();
  assert.equal(sync.retries.size, 0);
  assert.equal(sync.revisions.size, 0);
});

test("rejects malformed or unbounded snapshots", async () => {
  let calls = 0;
  const { context, run } = setup(async () => { calls += 1; });
  for (const entries of [null, {}, [null], [{ threadId: "remote:any", revision }],
    [{ threadId: id, revision: "invalid" }], Array.from({ length: 129 }, () => ({ threadId: id, revision }))]) {
    context.codexPlusBackendSettings.weixinDesktopSync = entries;
    await run();
  }
  assert.equal(calls, 0);
});

const reserved = "codex-plus-weixin-reserved-v1";
const project = {
  id: revision, name: "Renamed Weixin", rootPaths: ["C:\\work\\weixin"],
};

test("never registers an unrecognized project", async () => {
  for (const registrationKey of [undefined, reserved]) {
    let notifications = 0, upserts = 0, writes = 0;
    const { context, sync, run } = setup(async () => { notifications += 1; }, {
      projects: {
        getLocalProjects: async () => ({}),
        upsertLocal: async () => { upserts += 1; return true; },
      },
      threadProjectAssignments: {
        setProjectless: async () => { writes += 1; },
        assignIfUnassigned: async () => { writes += 1; },
      },
    });
    context.codexPlusBackendSettings.weixinDesktopSync[0].project = { ...project, registrationKey };
    await run();
    assert.equal(notifications, 0);
    assert.equal(upserts, 0);
    assert.equal(writes, 0);
    assert.equal(sync.revisions.size, 0);
    assert.equal(sync.retries.get(id).revision, revision);
  }
});
test("uses the existing desktop project ID instead of duplicating the server project", async () => {
  for (const registrationKey of [undefined, reserved]) {
    let assigned, upserts = 0;
    const { context, run } = setup(async () => {}, {
      state: { projectsByHome: { "local:C:\\codex-home": { desktop: project.id } }, assignments: {} },
      projects: {
        getLocalProjects: async () => ({ desktop: { id: "desktop", rootPaths: ["c:/WORK/weixin/"] } }),
        upsertLocal: async () => { upserts += 1; },
      },
      threadProjectAssignments: {
        setProjectless: async () => {},
        assignIfUnassigned: async ([request]) => { assigned = request; },
      },
    });
    context.codexPlusBackendSettings.weixinDesktopSync[0].project = { ...project, registrationKey };
    await run();
    assert.equal(assigned.assignment.projectId, "desktop");
    assert.equal(upserts, 0);
    assert.equal(context.window.__codexPlusWeixinDesktopSync.revisions.size, 1);
  }
});

test("missing or ambiguous existing project identities leave the revision pending", async () => {
  for (const mode of ["missing-project", "unmapped-roots", "ambiguous-project", "ambiguous-home"]) {
    let notifications = 0, upserts = 0, writes = 0;
    const mappings = mode === "unmapped-roots" ? {} : { first: project.id };
    if (mode === "ambiguous-project") mappings.second = project.id;
    const projectsByHome = { "local:C:\\codex-home": mappings };
    if (mode === "ambiguous-home") projectsByHome["local:c:/CODEX-home/"] = mappings;
    const { context, sync, run } = setup(async () => { notifications += 1; }, {
      state: { projectsByHome, assignments: {} },
      projects: {
        getLocalProjects: async () => mode === "missing-project" ? {} : {
          first: { id: "first", rootPaths: project.rootPaths },
          second: { id: "second", rootPaths: project.rootPaths },
        },
        upsertLocal: async () => { upserts += 1; },
      },
      threadProjectAssignments: {
        setProjectless: async () => { writes += 1; },
        assignIfUnassigned: async () => { writes += 1; },
      },
    });
    context.codexPlusBackendSettings.weixinDesktopSync[0].project = project;
    await run();
    assert.equal(notifications, 0, mode);
    assert.equal(upserts, 0, mode);
    assert.equal(writes, 0, mode);
    assert.equal(sync.revisions.size, 0, mode);
    assert.equal(sync.retries.get(id).retryAt, 1030000, mode);
  }
});

test("never infers identity from shared roots, other homes or overwrites a saved assignment", async () => {
  for (const mode of ["shared-root", "other-home", "remote-host", "already-assigned"]) {
    let notified = false, upserts = 0, writes = 0;
    const mappings = mode === "shared-root" ? { desktop: id } : { desktop: project.id };
    const { context, run } = setup(async () => { notified = true; }, {
      state: {
        projectsByHome: { [mode === "other-home" ? "local:C:\\other-home"
          : mode === "remote-host" ? "remote:C:\\codex-home" : "local:C:\\codex-home"]: mappings },
        assignments: mode === "already-assigned" ? { [id]: { projectKind: "remote", projectId: "user-choice" } } : {},
      },
      projects: {
        getLocalProjects: async () => ({ desktop: { id: "desktop", rootPaths: project.rootPaths } }),
        upsertLocal: async () => { upserts += 1; },
      },
      threadProjectAssignments: {
        setProjectless: async () => { writes += 1; },
        assignIfUnassigned: async () => { writes += 1; },
      },
    });
    context.codexPlusBackendSettings.weixinDesktopSync[0].project = { ...project, registrationKey: undefined };
    await run();
    assert.equal(notified, mode === "already-assigned");
    assert.equal(upserts, 0, mode);
    assert.equal(writes, 0, mode);
  }
});

test("an unreadable native state response cannot be treated as an unassigned thread", async () => {
  const { context, run } = setup(async () => assert.fail("Must leave unreadable state pending"));
  const module = await context.loadOptionalCodexAppModule();
  class UnreadableClient {
    static getInstance() { return new UnreadableClient(); }
    async post() { return { body: {} }; }
    sendRequest() {}
    onFetchResponse() {}
    cancelStream() {}
  }
  context.loadOptionalCodexAppModule = async () => ({ ...module, arbitraryClient: UnreadableClient });
  context.codexPlusBackendSettings.weixinDesktopSync[0].project = project;
  await run();
  assert.equal(context.window.__codexPlusWeixinDesktopSync.revisions.size, 0);
  assert.equal(context.window.__codexPlusWeixinDesktopSync.retries.get(id).retryAt, 1030000);
});
