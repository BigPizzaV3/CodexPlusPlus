import assert from "node:assert/strict";
import { readFileSync, openSync, readSync, closeSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/35-local-project-continuity.js", import.meta.url), "utf8");
const create = new Function(`${source}\nreturn createCodexLocalProjectContinuity;`)();
const copy = (value: any) => value === undefined ? undefined : structuredClone(value);
const project = (id: string, name = id) => ({
  id, name, rootPaths: [`C:/work/${id}`], createdAt: 1, updatedAt: 2,
});
const assignment = (projectId: string) => ({ projectKind: "local", projectId });
const empty = () => ({
  "local-projects": {}, "project-order": [], "thread-project-assignments": {},
  "projectless-thread-ids": [], "thread-project-membership-host-ids": {},
});
function installedNativeState() {
  const path = process.env.CODEX_PLUS_TEST_APP_ASAR;
  if (!path) return null;
  const fd = openSync(path, "r");
  try {
    const prefix = Buffer.alloc(16);
    readSync(fd, prefix, 0, 16, 0);
    const header = Buffer.alloc(prefix.readUInt32LE(12));
    readSync(fd, header, 0, header.length, 16);
    const files = JSON.parse(header.toString()).files[".vite"].files.build.files;
    const name = Object.keys(files).find((name) => name.startsWith("early-bootstrap-133-") && name.endsWith(".js"));
    assert.ok(name, "Installed native state module must match the inspected contract");
    const entry = files[name!], buffer = Buffer.alloc(Number(entry.size));
    readSync(fd, buffer, 0, buffer.length, 8 + prefix.readUInt32LE(4) + Number(entry.offset));
    const code = buffer.toString();
    const methodStart = code.indexOf("selectRendererOwner(e){");
    const methodEnd = code.indexOf("getStateFilePath(){", methodStart);
    const loadStart = code.indexOf("function ed(e){");
    const serializeStart = code.indexOf("function ep(e){", loadStart);
    const serializeEnd = code.indexOf("async function eh(", serializeStart);
    assert.ok(methodStart >= 0 && methodEnd > methodStart && loadStart >= 0 && serializeEnd > serializeStart);
    const globals = Object.keys(empty());
    const select = new Function("g", "u", "k",
      `return ({${code.slice(methodStart, methodEnd)}}).selectRendererOwner;`)(
      { ac: "electron-account-renderer-state-v1", Iz: (id: string) => id, kg: new Set() },
      { nh0: globals }, "electron-persisted-atom-state");
    const serialize = new Function(`${code.slice(serializeStart, serializeEnd)}; return ep;`)();
    const load = (text: string) => new Function("n", "eu", "ec", "ee",
      `${code.slice(loadStart, serializeStart)}; return ed("synthetic-state");`)(
      { existsSync: () => true, readFileSync: () => text }, new Set(), {},
      () => ({ warning() { throw Error("Unexpected native state parse warning"); } }));
    return { select, serialize, load };
  } finally { closeSync(fd); }
}
const nativeState = installedNativeState();
function fixture(initial: any = {}, home = "C:/test-home", sharedLocks?: any) {
  const data: any = { ...empty(), "electron-account-renderer-state-v1": { owner: "A", inactive: {} }, ...copy(initial) };
  const calls: any[] = [];
  const errors: string[] = [];
  const causes: any[] = [];
  let active = true;
  let tail = Promise.resolve();
  const locks = sharedLocks || { request(_name: string, callback: () => any) {
    const result = tail.then(callback);
    tail = result.catch(() => {});
    return result;
  } };
  const nativeIds = new Set(Object.keys(data["local-projects"]));
  let runtime: any;
  const runtimes = new Set<any>();
  const notify = (message: any) => runtimes.forEach((runtime) => runtime.notify(message));
  const services: any = {
    executionPaths: { async readCodexHome(host: string) { assert.equal(host, "local"); return home; } },
    applicationState: {
      async read(key: string) { return { value: copy(data[key]) }; },
      async write(key: string, value: any) {
        calls.push(["write", key]);
        data[key] = copy(value);
        notify({ type: "global-state-updated", keys: [key] });
        return { success: true };
      },
    },
    projects: {
      async getLocalProjects() { return copy(data["local-projects"]); },
      async upsertLocal(value: any, mode: string) {
        assert.equal(mode, "restore");
        assert.deepEqual(Object.keys(value).sort(), ["name", "projectId", "sources"]);
        calls.push(["upsert", value.projectId]);
        const p = project(value.projectId, value.name);
        p.rootPaths = value.sources;
        nativeIds.add(value.projectId);
        data["local-projects"][value.projectId] = p;
        notify({ type: "global-state-updated", keys: ["local-projects"] });
        return true;
      },
      async createLocal(value: any) {
        const p = project(value.id);
        data["local-projects"][p.id] = p;
        data["project-order"].push(p.id);
        nativeIds.add(p.id);
        notify({ type: "global-state-updated", keys: ["local-projects", "project-order"] });
        return p;
      },
      async renameLocal(value: any) {
        if (!nativeIds.has(value.projectId)) throw Error("missing native project");
        data["local-projects"][value.projectId].name = value.name;
        notify({ type: "global-state-updated", keys: ["local-projects"] });
        return true;
      },
      async removeLocal(id: string) {
        calls.push(["remove", id]);
        delete data["local-projects"][id];
        data["project-order"] = data["project-order"].filter((value: string) => value !== id);
        for (const [threadId, a] of Object.entries<any>(data["thread-project-assignments"]))
          if (a.projectId === id) {
            delete data["thread-project-assignments"][threadId];
            data["projectless-thread-ids"].push(threadId);
          }
        notify({ type: "global-state-updated", keys: ["local-projects", "project-order", "pinned-project-ids"] });
        return { previousMemberships: [] };
      },
      async setProjectOrder(value: any) {
        calls.push(["order"]);
        data["project-order"] = copy(value);
        notify({ type: "global-state-updated", keys: ["project-order"] });
      },
    },
    threadProjectAssignments: {
      async readMemberships() { return {
        assignments: copy(data["thread-project-assignments"]),
        projectlessThreadIds: copy(data["projectless-thread-ids"]),
      }; },
      async restoreMemberships(records: any[]) {
        calls.push(["memberships", copy(records)]);
        for (const record of records) {
          assert.equal(record.hostId, "local");
          if (record.assignment && !nativeIds.has(record.assignment.projectId)) throw Error("missing native project mapping");
          if (record.assignment) data["thread-project-assignments"][record.threadId] = copy(record.assignment);
          else delete data["thread-project-assignments"][record.threadId];
          data["projectless-thread-ids"] = data["projectless-thread-ids"].filter((id: string) => id !== record.threadId);
          if (record.projectless) data["projectless-thread-ids"].push(record.threadId);
          data["thread-project-membership-host-ids"][record.threadId] = record.hostId;
        }
        notify({ type: "thread-project-memberships-updated", memberships: copy(records) });
      },
      async setMembership(record: any) { return services.threadProjectAssignments.restoreMemberships([record]); },
    },
  };
  // Native methods call each other in the host, not through the renderer wrapper.
  services.threadProjectAssignments.setMembership = async (record: any) => originalMemberships([record]);
  const originalMemberships = services.threadProjectAssignments.restoreMemberships;
  const originalProjects = { ...services.projects };
  const originalServices = { ...services };
  const start = (options: any = {}) => {
    runtime = create({ services, locks, enabled: () => active,
      onError: (e: string, cause: any) => { errors.push(e); causes.push(cause); }, ...options });
    runtimes.add(runtime);
    return runtime;
  };
  const reset = (owner: any, globals: any = {}) => {
    const container = data["electron-account-renderer-state-v1"];
    if (nativeState) {
      if (owner != null && Object.keys(globals).length)
        container.inactive[owner] = { globals: { ...empty(), ...copy(globals) }, atoms: {} };
      const state = new Map(Object.entries(copy(data)));
      nativeState.select.call({
        state, changeListeners: new Map(), rendererAccountGeneration: 0, storedOwnerGeneration: 0,
        get(key: string) { return state.get(key) ?? null; }, set(key: string, value: any) { state.set(key, value); },
      }, owner);
      const loaded = nativeState.load(nativeState.serialize(state));
      for (const key of Object.keys(data)) delete data[key];
      Object.assign(data, empty(), Object.fromEntries(loaded));
    } else {
      if (container.owner != null) container.inactive[container.owner] = { globals: {
      ...Object.fromEntries(Object.keys(empty()).map((key) => [key, copy(data[key])])),
      }, atoms: { privateAtom: "must-not-copy" } };
      container.owner = owner;
      Object.assign(data, empty(), copy(globals));
    }
    nativeIds.clear();
    for (const name of Object.keys(data["local-projects"])) nativeIds.add(name);
    notify({ type: "persisted-atom-sync", state: {} });
  };
  const drain = async () => {
    await new Promise((resolve) => setTimeout(resolve, 5));
    await runtime.reconcile();
  };
  return { data, services, calls, errors, causes, start, reset, drain, originalProjects, originalMemberships,
    newWindow() {
      const services = { ...originalServices };
      const runtime = create({ services, locks, enabled: () => active, onError: (e: string) => errors.push(e) });
      runtimes.add(runtime);
      return { services, runtime };
    },
    enable(value: boolean) { active = value; }, get runtime() { return runtime; } };
}
test("installed native serializer retains the unscoped continuity record", { skip: !nativeState }, () => {
  const key = "codex-plus-shared-local-projects-v1";
  const value = { version: 1, projects: { p: project("p") } };
  assert.deepEqual(nativeState!.load(nativeState!.serialize(new Map([[key, value]]))).get(key), value);
});
const baseline = () => ({
  "local-projects": { p: project("p") }, "project-order": ["p"],
  "thread-project-assignments": { t: assignment("p") },
});

test("default-off does not read or change native state", async () => {
  const f = fixture(baseline());
  f.enable(false);
  f.start();
  const before = copy(f.data);
  await f.runtime.reconcile();
  assert.deepEqual(f.data, before);
  assert.deepEqual(f.calls, []);
  f.runtime.dispose();
});
test("A -> B -> none -> A preserves local projects and memberships", async () => {
  const f = fixture(baseline());
  f.start();
  await f.runtime.reconcile();
  for (const owner of ["B", null, "A"]) {
    f.reset(owner);
    await f.drain();
    assert.equal(f.data["local-projects"].p.name, "p");
    assert.deepEqual(f.data["thread-project-assignments"].t, assignment("p"));
    assert.deepEqual(f.data["project-order"], ["p"]);
  }
  assert.deepEqual(f.errors, []);
  assert.ok(f.calls.some((call) => call[0] === "upsert"));
  assert.ok(!f.calls.some((call) => call[0] === "write" && call[1] === "local-projects"));
  f.runtime.dispose();
});
test("anonymous -> anonymous constructor reset survives a new renderer", async () => {
  const f = fixture({ ...baseline(), "electron-account-renderer-state-v1": { owner: null, inactive: {} } });
  f.start();
  await f.runtime.reconcile();
  const backup = copy(f.data[f.runtime.key].backup);
  f.runtime.dispose();
  f.reset(null);
  f.start();
  await f.drain();
  assert.equal(f.data["local-projects"].p.id, "p");
  assert.deepEqual(f.data[f.runtime.key].backup, backup);
  f.runtime.dispose();
});
test("first bootstrap imports inactive ordinary projects, not private/cloud/remote state", async () => {
  const cloud = { ...project("cloud"), chatGptBacking: { accountId: "private" } };
  const f = fixture({
    "electron-account-renderer-state-v1": { owner: null, inactive: { A: {
      globals: { ...baseline(), "local-projects": { p: project("p"), cloud },
        "thread-project-assignments": { t: assignment("p"), c: assignment("cloud") },
        "queued-follow-ups": ["private-message"], "remote-projects": [{ id: "remote" }] },
      atoms: { privateAtom: "must-not-copy" },
    } } },
  });
  const container = copy(f.data["electron-account-renderer-state-v1"]);
  f.start();
  await f.runtime.reconcile();
  assert.deepEqual(Object.keys(f.data["local-projects"]), ["p"]);
  assert.deepEqual(f.data["thread-project-assignments"], { t: assignment("p") });
  const text = JSON.stringify(f.data[f.runtime.key]);
  for (const privateValue of ["private-message", "must-not-copy", "accountId", "cloud", "remote"])
    assert.ok(!text.includes(privateValue), privateValue);
  assert.deepEqual(f.data["electron-account-renderer-state-v1"], container);
  f.runtime.dispose();
});
test("delete remains deleted in stale scopes and inactive is not reimported", async () => {
  const f = fixture(baseline());
  f.start();
  await f.runtime.reconcile();
  await f.services.projects.removeLocal("p");
  f.reset("B", baseline());
  await f.drain();
  assert.deepEqual(f.data["local-projects"], {});
  assert.equal(f.data["thread-project-assignments"].t, undefined);
  assert.ok(f.data["projectless-thread-ids"].includes("t"));
  f.runtime.dispose();
});
test("explicit projectless and explicit unassignment survive stale account caches", async () => {
  const f = fixture(baseline());
  f.start();
  await f.runtime.reconcile();
  for (const projectless of [true, false]) {
    await f.services.threadProjectAssignments.setMembership({
      threadId: "t", hostId: "local", assignment: null, projectless,
    });
    f.reset(projectless ? "B" : "A", baseline());
    await f.drain();
    assert.equal(f.data["thread-project-assignments"].t, undefined);
    assert.equal(f.data["projectless-thread-ids"].includes("t"), projectless);
  }
  f.runtime.dispose();
});
test("native background membership notifications become authoritative", async () => {
  const f = fixture(baseline());
  f.start();
  await f.runtime.reconcile();
  await f.originalMemberships([{ threadId: "new-thread", hostId: "local", assignment: assignment("p"), projectless: false }]);
  await f.drain();
  f.reset("B");
  await f.drain();
  assert.deepEqual(f.data["thread-project-assignments"]["new-thread"], assignment("p"));
  f.runtime.dispose();
});
test("cached project service references still capture native deletion notifications", async () => {
  const f = fixture(baseline());
  const remove = f.originalProjects.removeLocal;
  f.start();
  await f.runtime.reconcile();
  await remove("p");
  await f.drain();
  f.reset("B", baseline());
  await f.drain();
  assert.deepEqual(f.data["local-projects"], {});
  f.runtime.dispose();
});
test("rename and create use native APIs and are shared", async () => {
  const f = fixture(baseline());
  f.start();
  await f.runtime.reconcile();
  await f.services.projects.renameLocal({ projectId: "p", name: "renamed" });
  await f.services.projects.createLocal({ id: "q" });
  f.reset(null);
  await f.drain();
  assert.equal(f.data["local-projects"].p.name, "renamed");
  assert.ok(f.data["local-projects"].q);
  f.runtime.dispose();
});
test("current cloud collision, remote membership and linked order remain native", async () => {
  const f = fixture(baseline());
  f.start();
  await f.runtime.reconcile();
  const cloud = { ...project("p", "Cloud-owned"), chatGptBacking: { accountId: "B" } };
  const remote = { projectKind: "remote", projectId: "r" };
  f.reset("B", { "local-projects": { p: cloud }, "project-order": ["linked:cloud", "p"],
    "thread-project-assignments": { t: remote }, "thread-project-membership-host-ids": { t: "remote-host" } });
  await f.drain();
  assert.deepEqual(f.data["local-projects"].p, cloud);
  assert.deepEqual(f.data["thread-project-assignments"].t, remote);
  assert.ok(f.data["project-order"].includes("linked:cloud"));
  f.runtime.dispose();
});
test("unknown native schema, corrupted saved schema and missing Web Locks fail closed", async () => {
  for (const options of [
    { initial: { "electron-account-renderer-state-v1": { owner: 7, inactive: {} } } },
    { initial: { "codex-plus-shared-local-projects-v1": { version: 2 } } },
    { options: { locks: null } },
  ]) {
    const f = fixture({ ...baseline(), ...options.initial });
    const before = copy(f.data);
    f.start(options.options);
    await f.runtime.reconcile();
    assert.deepEqual(f.data, before);
    assert.ok(f.errors.length);
    f.runtime.dispose();
  }
});
test("disabled or disposed runtime leaves native commands usable", async () => {
  const f = fixture(baseline());
  f.start();
  await f.runtime.reconcile();
  f.enable(false);
  await f.services.projects.removeLocal("p");
  assert.ok(f.data[f.runtime.key].projects.p);
  f.runtime.dispose();
  await f.services.projects.createLocal({ id: "q" });
  assert.ok(f.data["local-projects"].q);
});
test("no-op notifications and repeated reconciliation do not write or loop", async () => {
  const f = fixture(baseline());
  f.start();
  await f.runtime.reconcile();
  const count = f.calls.length;
  for (let i = 0; i < 5; i++) {
    f.runtime.notify({ type: "global-state-updated", keys: [f.runtime.key] });
    await f.runtime.reconcile();
  }
  assert.equal(f.calls.length, count);
  f.runtime.dispose();
});
test("re-enable forces restoration after disabled same-owner reset", async () => {
  const f = fixture({ ...baseline(), "electron-account-renderer-state-v1": { owner: null, inactive: {} } });
  f.start();
  await f.runtime.reconcile();
  f.enable(false);
  f.reset(null);
  f.enable(true);
  await f.runtime.resume();
  assert.ok(f.data["local-projects"].p);
  f.runtime.dispose();
});
test("identity changes during restoration are retried without another notification", async () => {
  const f = fixture(baseline());
  const original = f.services.projects.upsertLocal;
  let reset = false;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    const result = await original(...args);
    if (!reset) { reset = true; f.reset("C"); }
    return result;
  };
  f.start();
  await f.runtime.reconcile();
  f.reset("B");
  await f.drain();
  await f.drain();
  assert.equal(f.data["electron-account-renderer-state-v1"].owner, "C");
  assert.ok(f.data["local-projects"].p);
  assert.deepEqual(f.data["thread-project-assignments"].t, assignment("p"));
  f.runtime.dispose();
});
test("cached deletion while restoration is busy becomes a persistent tombstone", async () => {
  const f = fixture(baseline());
  const upsert = f.services.projects.upsertLocal;
  let remove = false;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    const result = await upsert(...args);
    if (remove) { remove = false; await f.originalProjects.removeLocal("p"); }
    return result;
  };
  f.start();
  await f.runtime.reconcile();
  remove = true;
  f.reset("B");
  await f.drain();
  await f.drain();
  assert.equal(f.data["local-projects"].p, undefined);
  assert.equal(f.data[f.runtime.key].projects.p, undefined);
  f.reset("A", baseline());
  await f.drain();
  assert.equal(f.data["local-projects"].p, undefined);
  f.runtime.dispose();
});
test("cached explicit unassignment during restoration wins over the old snapshot", async () => {
  const f = fixture(baseline());
  const upsert = f.services.projects.upsertLocal;
  let unassign = false;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    const result = await upsert(...args);
    if (unassign) {
      unassign = false;
      await f.originalMemberships([{ threadId: "t", hostId: "local", assignment: null, projectless: true }]);
    }
    return result;
  };
  f.start();
  await f.runtime.reconcile();
  unassign = true;
  f.reset("B");
  await f.drain();
  await f.drain();
  assert.equal(f.data["thread-project-assignments"].t, undefined);
  assert.ok(f.data["projectless-thread-ids"].includes("t"));
  f.runtime.dispose();
});
test("pre-call identity failure runs the native command once, not zero or twice", async () => {
  const f = fixture(baseline());
  const rename = f.services.projects.renameLocal;
  let calls = 0, reads = 0;
  f.services.projects.renameLocal = async (value: any) => { calls++; return rename(value); };
  const read = f.services.applicationState.read;
  let armed = false;
  f.services.applicationState.read = async (key: string) => {
    if (armed && key === "electron-account-renderer-state-v1" && ++reads === 6)
      f.runtime.notify({ type: "persisted-atom-sync" });
    return read(key);
  };
  f.start();
  await f.runtime.reconcile();
  armed = true;
  await f.services.projects.renameLocal({ projectId: "p", name: "once" });
  assert.ok(reads >= 6);
  assert.deepEqual(f.errors, []);
  assert.equal(calls, 1);
  assert.equal(f.data["local-projects"].p.name, "once");
  f.runtime.dispose();
});
test("two windows serialize native mutations and do not lose shared changes", async () => {
  const f = fixture(baseline());
  f.start();
  const other = f.newWindow();
  await Promise.all([f.runtime.reconcile(), other.runtime.reconcile()]);
  await Promise.all([
    f.services.projects.createLocal({ id: "q" }),
    other.services.projects.createLocal({ id: "r" }),
  ]);
  await f.drain();
  assert.deepEqual(Object.keys(f.data[f.runtime.key].projects).sort(), ["p", "q", "r"]);
  assert.deepEqual(f.errors, []);
  other.runtime.dispose();
  f.runtime.dispose();
});
test("native command failures are not retried by the continuity layer", async () => {
  const f = fixture(baseline());
  let calls = 0;
  f.services.projects.renameLocal = async () => { calls++; throw Error("native command failure"); };
  f.start();
  await f.runtime.reconcile();
  await assert.rejects(f.services.projects.renameLocal({ projectId: "p", name: "x" }), /native command failure/);
  assert.equal(calls, 1);
  f.runtime.dispose();
});
test("snapshot storage failure after native success preserves the native result", async () => {
  const f = fixture(baseline());
  const write = f.services.applicationState.write;
  let armed = false;
  f.services.applicationState.write = async (key: string, value: any) => {
    if (armed && key === "codex-plus-shared-local-projects-v1") throw Error("storage unavailable");
    return write(key, value);
  };
  f.start();
  await f.runtime.reconcile();
  armed = true;
  assert.equal(await f.services.projects.renameLocal({ projectId: "p", name: "native succeeded" }), true);
  assert.equal(f.data["local-projects"].p.name, "native succeeded");
  assert.ok(f.errors.length);
  f.runtime.dispose();
});
test("native cwd inference cannot replace known memberships during project restore", async () => {
  const f = fixture(baseline());
  const upsert = f.services.projects.upsertLocal;
  const memberships = f.originalMemberships;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    const inferred: any[] = [];
    for (const threadId of ["t", "unrelated"]) {
      if (!f.data["thread-project-assignments"][threadId] && !f.data["projectless-thread-ids"].includes(threadId))
        inferred.push({ threadId, hostId: "local", assignment: null, projectless: true });
    }
    if (inferred.length) await memberships(inferred);
    return upsert(...args);
  };
  f.start();
  await f.runtime.reconcile();
  f.reset(null);
  await f.drain();
  assert.deepEqual(f.data["thread-project-assignments"].t, assignment("p"));
  assert.ok(f.data["projectless-thread-ids"].includes("unrelated"));
  assert.equal(f.data[f.runtime.key].memberships.unrelated, undefined);
  assert.deepEqual(f.errors, []);
  f.runtime.dispose();
});
test("a shared record for another Codex home is never applied", async () => {
  const a = fixture(baseline());
  a.start();
  await a.runtime.reconcile();
  const b = fixture({ [a.runtime.key]: copy(a.data[a.runtime.key]) }, "D:/another-home");
  b.start();
  await b.runtime.reconcile();
  assert.deepEqual(b.data["local-projects"], {});
  assert.ok(b.errors.length);
  a.runtime.dispose();
  b.runtime.dispose();
});
test("a cloud ID collision does not temporarily reclassify an unassigned thread", async () => {
  const f = fixture(baseline());
  f.start();
  await f.runtime.reconcile();
  const cloud = { ...project("p"), chatGptBacking: { accountId: "B" } };
  f.reset("B", { "local-projects": { p: cloud } });
  await f.drain();
  assert.deepEqual(f.data["projectless-thread-ids"], []);
  assert.deepEqual(f.data["thread-project-assignments"], {});
  assert.deepEqual(f.data["local-projects"].p, cloud);
  assert.deepEqual(f.errors, []);
  f.runtime.dispose();
});
test("failed native upsert rolls back temporary projectless markers", async () => {
  const f = fixture(baseline());
  f.services.projects.upsertLocal = async () => { throw Error("native upsert failure"); };
  f.start();
  await f.runtime.reconcile();
  f.reset("B");
  await f.drain();
  await f.drain();
  assert.deepEqual(f.data["thread-project-assignments"], {});
  assert.deepEqual(f.data["projectless-thread-ids"], []);
  assert.ok(f.errors.length);
  f.runtime.dispose();
});
test("explicit unassignment stays false when native cwd inference runs", async () => {
  const f = fixture(baseline());
  const upsert = f.services.projects.upsertLocal;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    if (!f.data["thread-project-assignments"].t && !f.data["projectless-thread-ids"].includes("t"))
      await f.originalMemberships([{ threadId: "t", assignment: null, projectless: true, hostId: "local" }]);
    return upsert(...args);
  };
  f.start();
  await f.runtime.reconcile();
  await f.services.threadProjectAssignments.setMembership({
    threadId: "t", hostId: "local", assignment: null, projectless: false,
  });
  f.reset("B");
  await f.drain();
  assert.deepEqual(f.data[f.runtime.key].memberships.t, {
    threadId: "t", hostId: "local", assignment: null, projectless: false,
  });
  assert.equal(f.data["projectless-thread-ids"].includes("t"), false);
  assert.deepEqual(f.causes, []);
  assert.deepEqual(f.errors, []);
  f.runtime.dispose();
});
