import assert from "node:assert/strict";
import { closeSync, openSync, readFileSync, readSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/35-shared-local-projects.js", import.meta.url), "utf8");
const create = new Function(`${source}\nreturn createCodexSharedLocalProjects;`)();
const loadServices = new Function(`${source}\nreturn loadCodexLocalProjectServices;`)();
const copy = (value: any) => value === undefined ? undefined : structuredClone(value);
const key = "codex-plus-shared-local-projects-v1";
const accounts = "electron-account-renderer-state-v1";
const project = (id: string) => ({ id, name: id, rootPaths: [`C:/work/${id}`], createdAt: 1, updatedAt: 2 });
const assignment = (projectId: string) => ({ projectKind: "local", projectId });
const empty = () => ({
  "local-projects": {}, "project-order": [], "thread-project-assignments": {},
  "projectless-thread-ids": [], "thread-project-membership-host-ids": {},
});
const baseline = () => ({
  ...empty(), "local-projects": { p: project("p") }, "project-order": ["p"],
  "thread-project-assignments": { t: assignment("p") },
  "thread-project-membership-host-ids": { t: "local" },
});
function installedSelector() {
  const path = process.env.CODEX_PLUS_TEST_APP_ASAR;
  if (!path) return null;
  const fd = openSync(path, "r");
  try {
    const prefix = Buffer.alloc(16);
    readSync(fd, prefix, 0, prefix.length, 0);
    const header = Buffer.alloc(prefix.readUInt32LE(12));
    readSync(fd, header, 0, header.length, 16);
    const files = JSON.parse(header.toString()).files[".vite"].files.build.files;
    const name = Object.keys(files).find((name) => name.startsWith("early-bootstrap-133-") && name.endsWith(".js"));
    assert.ok(name, "Native selector module must match the inspected contract");
    const entry = files[name!], body = Buffer.alloc(Number(entry.size));
    readSync(fd, body, 0, body.length, 8 + prefix.readUInt32LE(4) + Number(entry.offset));
    const text = body.toString();
    const start = text.indexOf("selectRendererOwner(e){");
    const end = text.indexOf("getStateFilePath(){", start);
    assert.ok(start >= 0 && end > start);
    return new Function("g", "u", "k", `return ({${text.slice(start, end)}}).selectRendererOwner;`)(
      { ac: accounts, Iz: (id: string) => id, kg: new Set() },
      { nh0: Object.keys(empty()) }, "electron-persisted-atom-state");
  } finally { closeSync(fd); }
}
const nativeSelector = installedSelector();

function fixture(initial: any = baseline(), sharedLocks?: any, home = "C:/test-home") {
  const data: any = { ...empty(), [accounts]: { owner: "A", inactive: {} }, ...copy(initial) };
  const errors: string[] = [], calls: any[] = [];
  let runtime: any, active = true, tail = Promise.resolve();
  const runtimes = new Set<any>();
  const locks = sharedLocks || { request(_name: string, operation: () => any) {
    const result = tail.then(operation);
    tail = result.catch(() => {});
    return result;
  } };
  const notify = (message: any) => runtimes.forEach((runtime) => runtime.notify(copy(message)));
  const services: any = {
    executionPaths: { async readCodexHome() { return home; } },
    applicationState: {
      async read(name: string) { return { value: copy(data[name]) }; },
      async write(name: string, value: any) {
        assert.equal(name, key, "No direct writes to native project tables or account scopes");
        data[name] = copy(value);
        notify({ type: "global-state-updated", keys: [name] });
        return { success: true };
      },
    },
    projects: {
      async getLocalProjects() { return copy(data["local-projects"]); },
      async upsertLocal(value: any, mode: string) {
        assert.equal(mode, "restore");
        calls.push(["upsert", value.projectId]);
        data["local-projects"][value.projectId] = { ...project(value.projectId), name: value.name, rootPaths: value.sources };
        notify({ type: "global-state-updated", keys: ["local-projects"] });
        return true;
      },
      async removeLocal(id: string) {
        calls.push(["delete", id]);
        delete data["local-projects"][id];
        data["project-order"] = data["project-order"].filter((value: string) => value !== id);
        const memberships: any = {};
        for (const [threadId, a] of Object.entries<any>(data["thread-project-assignments"])) {
          if (a.projectId !== id) continue;
          delete data["thread-project-assignments"][threadId];
          if (!data["projectless-thread-ids"].includes(threadId)) data["projectless-thread-ids"].push(threadId);
          memberships[threadId] = { assignment: null, projectless: true };
        }
        notify({ type: "thread-project-memberships-updated", memberships });
        notify({ type: "global-state-updated", keys: ["local-projects", "project-order", "pinned-project-ids"] });
        return { previousMemberships: [] };
      },
      async setProjectOrder(order: string[]) {
        data["project-order"] = copy(order);
        notify({ type: "global-state-updated", keys: ["project-order"] });
      },
    },
    threadProjectAssignments: {
      async readMemberships() {
        return { assignments: copy(data["thread-project-assignments"]), projectlessThreadIds: copy(data["projectless-thread-ids"]) };
      },
      async restoreMemberships(records: any[]) {
        calls.push(["memberships", copy(records)]);
        const memberships: any = {};
        for (const entry of records) {
          assert.equal(entry.hostId, "local");
          if (entry.assignment) {
            assert.ok(data["local-projects"][entry.assignment.projectId], "Native project mapping must exist first");
            data["thread-project-assignments"][entry.threadId] = copy(entry.assignment);
          } else delete data["thread-project-assignments"][entry.threadId];
          data["projectless-thread-ids"] = data["projectless-thread-ids"].filter((id: string) => id !== entry.threadId);
          if (entry.projectless) data["projectless-thread-ids"].push(entry.threadId);
          data["thread-project-membership-host-ids"][entry.threadId] = "local";
          memberships[entry.threadId] = { assignment: copy(entry.assignment), projectless: entry.projectless };
        }
        notify({ type: "thread-project-memberships-updated", memberships });
      },
    },
  };
  function start() {
    runtime = create({ services, locks, enabled: () => active, onError: (error: Error) => errors.push(error.message) });
    runtimes.add(runtime);
    return runtime;
  }
  function reset(owner: string | null, selected?: any) {
    data[accounts] ||= { owner: null, inactive: {} };
    if (nativeSelector) {
      const state = new Map<string, any>(Object.entries(data));
      nativeSelector.call({
        state, get: (key: string) => state.get(key),
        set: (key: string, value: any) => state.set(key, value),
        rendererAccountGeneration: 0, storedOwnerGeneration: 0, changeListeners: new Map(),
      }, owner);
      for (const name of Object.keys(data)) delete data[name];
      Object.assign(data, Object.fromEntries(state));
    } else {
      const container = data[accounts];
      if (container.owner !== owner || owner === null) {
        if (container.owner != null)
          container.inactive[container.owner] = { globals: Object.fromEntries(Object.keys(empty()).map((name) => [name, copy(data[name])])), atoms: {} };
        const saved = owner == null ? {} : container.inactive[owner]?.globals || {};
        for (const name of Object.keys(empty())) delete data[name];
        Object.assign(data, copy(saved));
        if (owner != null) delete container.inactive[owner];
        container.owner = owner;
      }
    }
    Object.assign(data, { ...empty(), ...Object.fromEntries(Object.keys(empty()).filter((name) => data[name] != null).map((name) => [name, data[name]])) });
    if (selected) Object.assign(data, copy(selected));
    notify({ type: "persisted-atom-sync", state: {} });
  }
  return {
    data, services, errors, calls, notify, reset, locks, start,
    get runtime() { return runtime; },
    setEnabled(value: boolean) { active = value; },
    async drain() {
      for (let i = 0; i < 6; i++) {
        await new Promise((resolve) => setTimeout(resolve, 0));
        await runtime.reconcile();
      }
    },
    async document(owner: string | null, selected?: any) {
      runtime.dispose(); runtimes.delete(runtime);
      reset(owner, selected); start(); await runtime.reconcile(); await this.drain();
    },
    dispose() { runtimes.forEach((runtime) => runtime.dispose()); },
  };
}

test("existing opt-in is reused and native service objects and methods are not replaced", async () => {
  const f = fixture(), original = { ...f.services }, remove = f.services.projects.removeLocal;
  f.start(); await f.drain();
  for (const name of Object.keys(original)) assert.equal(f.services[name], original[name]);
  assert.equal(f.services.projects.removeLocal, remove);
  assert.match(source, /keepProjectsWithoutAccount === true/);
  assert.doesNotMatch(source, /new Proxy/);
  assert.deepEqual(f.errors, []);
  f.dispose();
});
test("A B and anonymous documents share projects and classifications in every direction", async () => {
  const f = fixture();
  f.start(); await f.drain();
  for (const owner of ["B", null, "A", null, "B", "A", null, null]) {
    await f.document(owner);
    assert.equal(f.data["local-projects"].p.name, "p");
    assert.deepEqual(f.data["thread-project-assignments"].t, assignment("p"));
  }
  assert.deepEqual(f.errors, []);
  f.dispose();
});
test("the actual installed selector clears null-to-null and the companion restores it", async () => {
  const f = fixture({ ...baseline(), [accounts]: { owner: null, inactive: {} } });
  f.start(); await f.drain();
  f.reset(null);
  assert.equal(f.data["local-projects"].p, undefined);
  await f.drain();
  assert.ok(f.data["local-projects"].p);
  f.dispose();
});
test("first enable combines all inactive local accounts once, without private atoms", async () => {
  const f = fixture({ ...baseline(), [accounts]: { owner: "A", inactive: {
    B: { globals: { ...empty(), "local-projects": { q: project("q") },
      "thread-project-assignments": { u: assignment("q") }, "project-order": ["q"], "queued-follow-ups": { u: ["never replay"] } },
    atoms: { private: true } },
  } } });
  f.start(); await f.drain();
  assert.deepEqual(Object.keys(f.data[key].projects).sort(), ["p", "q"]);
  assert.deepEqual(f.data["thread-project-assignments"].u, assignment("q"));
  assert.equal(f.data["queued-follow-ups"], undefined);
  assert.equal(f.data["electron-persisted-atom-state"], undefined);
  f.dispose();
});
test("real object-shaped membership notifications preserve explicit projectless changes", async () => {
  const f = fixture();
  f.start(); await f.drain();
  await f.services.threadProjectAssignments.restoreMemberships([{ threadId: "t", hostId: "local", assignment: null, projectless: true }]);
  await f.drain(); await f.document("B");
  assert.equal(f.data["thread-project-assignments"].t, undefined);
  assert.ok(f.data["projectless-thread-ids"].includes("t"));
  assert.deepEqual(f.errors, []);
  f.dispose();
});
test("native standalone order notifications and renamed projects are shared", async () => {
  const f = fixture({ ...baseline(), "local-projects": { p: project("p"), q: project("q") } });
  f.start(); await f.drain();
  await f.services.projects.setProjectOrder(["q", "p"]);
  f.data["local-projects"].p.name = "renamed";
  f.notify({ type: "global-state-updated", keys: ["local-projects"] });
  await f.drain(); await f.document(null);
  assert.deepEqual(f.data["project-order"], ["q", "p"]);
  assert.equal(f.data["local-projects"].p.name, "renamed");
  f.dispose();
});
test("non-account full atom synchronization does not roll back pending project edits", async () => {
  const f = fixture({ ...baseline(), "local-projects": { p: project("p"), q: project("q") }, "project-order": ["p", "q"] });
  f.start(); await f.drain();
  f.data["local-projects"].p.name = "renamed";
  f.notify({ type: "global-state-updated", keys: ["local-projects"] });
  await f.services.projects.setProjectOrder(["q", "p"]);
  f.notify({ type: "persisted-atom-sync", state: { "sidebar-width": 300 } });
  await f.drain();
  assert.equal(f.data["local-projects"].p.name, "renamed");
  assert.equal(f.data[key].projects.p.name, "renamed");
  assert.deepEqual(f.data["project-order"], ["q", "p"]);
  assert.deepEqual(f.data[key].order, ["q", "p"]);
  f.dispose();
});
test("deleting the final project and an unrelated atom sync are not mistaken for a scope reset", async () => {
  const f = fixture({ ...empty(), "local-projects": { p: project("p") }, "project-order": ["p"] });
  f.start(); await f.drain();
  await f.services.projects.removeLocal("p");
  f.notify({ type: "persisted-atom-sync", state: {} });
  await f.drain();
  assert.equal(f.data["local-projects"].p, undefined);
  assert.equal(f.data[key].projects.p, undefined);
  assert.deepEqual(f.errors, []);
  f.dispose();
});
test("native deletion remains a tombstone across accounts and repeated anonymous restarts", async () => {
  const f = fixture();
  f.start(); await f.drain();
  await f.services.projects.removeLocal("p");
  await f.drain();
  for (const owner of ["B", "A", null, null]) {
    await f.document(owner);
    assert.equal(f.data["local-projects"].p, undefined);
    assert.equal(f.data[key].projects.p, undefined);
  }
  assert.ok(f.data[key].knownProjects.includes("p"));
  f.dispose();
});
test("a retired logged-in document recovers its final deletion without importing old scopes again", async () => {
  const f = fixture();
  f.start(); await f.drain();
  await f.services.projects.removeLocal("p");
  await f.document("B");
  assert.equal(f.data["local-projects"].p, undefined);
  assert.equal(f.data[key].projects.p, undefined);
  f.dispose();
});
test("cloud ID collisions and remote memberships are never overwritten", async () => {
  const f = fixture();
  f.start(); await f.drain();
  const cloud = { ...project("p"), chatgptProjectId: "cloud" };
  await f.document("B", { "local-projects": { p: cloud },
    "thread-project-assignments": { t: { projectKind: "remote", projectId: "remote" } },
    "thread-project-membership-host-ids": { t: "remote-host" } });
  assert.deepEqual(f.data["local-projects"].p, cloud);
  assert.equal(f.data["thread-project-assignments"].t.projectKind, "remote");
  assert.ok(f.data[key].projects.p, "Another account's cloud collision must not erase the ordinary local canonical");
  await f.document(null);
  assert.deepEqual(f.data["local-projects"].p, project("p"));
  assert.deepEqual(f.data["thread-project-assignments"].t, assignment("p"));
  f.dispose();
});
test("existing R2 record and immutable backup retain deletion tombstones", async () => {
  const f = fixture();
  f.start(); await f.drain();
  const backup = copy(f.data[key].backup);
  await f.services.projects.removeLocal("p"); await f.drain();
  delete f.data[key].lastScope; delete f.data[key].enabled;
  await f.document("B", baseline());
  assert.equal(f.data["local-projects"].p, undefined);
  assert.deepEqual(f.data[key].backup, backup);
  f.dispose();
});
test("JSON property order from Rust does not invalidate a shared record", async () => {
  const f = fixture();
  f.start(); await f.drain();
  const p = f.data[key].projects.p;
  f.data[key].projects.p = { createdAt: p.createdAt, id: p.id, name: p.name, rootPaths: p.rootPaths, updatedAt: p.updatedAt };
  await f.document("B");
  assert.deepEqual(f.errors, []);
  assert.ok(f.data["local-projects"].p);
  f.dispose();
});
test("disabled sharing performs no native state writes", async () => {
  const f = fixture();
  f.setEnabled(false);
  f.start(); await f.runtime.reconcile();
  f.reset(null); await f.drain();
  assert.equal(f.data[key], undefined);
  assert.deepEqual(f.calls, []);
  f.dispose();
});
test("unknown shared record fails closed without replacing it", async () => {
  const f = fixture({ ...baseline(), [key]: { version: 99 } });
  f.start(); await f.drain();
  assert.deepEqual(f.data[key], { version: 99 });
  assert.ok(f.errors.length);
  assert.deepEqual(f.calls, []);
  f.dispose();
});
test("a transient native startup failure retries without requiring a new native notification", async () => {
  const f = fixture();
  const upsert = f.services.projects.upsertLocal;
  let fail = true;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    if (fail) { fail = false; throw Error("Backend starting"); }
    return upsert(...args);
  };
  f.start(); await f.runtime.reconcile();
  await new Promise((resolve) => setTimeout(resolve, 260));
  assert.ok(f.data[key].lastScope);
  assert.equal(f.errors.length, 1);
  f.dispose();
});
test("user sorting during native restoration wins over the previous shared order", async () => {
  const f = fixture({ ...baseline(), "local-projects": { p: project("p"), q: project("q") }, "project-order": ["p", "q"] });
  f.start(); await f.drain();
  const upsert = f.services.projects.upsertLocal;
  let armed = true;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    const result = await upsert(...args);
    if (armed) { armed = false; await f.services.projects.setProjectOrder(["q", "p"]); }
    return result;
  };
  await f.document("B");
  assert.deepEqual(f.data["project-order"], ["q", "p"]);
  assert.deepEqual(f.data[key].order, ["q", "p"]);
  f.dispose();
});
test("restoration does not overwrite a concurrent native rename", async () => {
  const f = fixture({ ...baseline(), "local-projects": { p: project("p"), q: project("q") } });
  f.start(); await f.drain();
  const upsert = f.services.projects.upsertLocal;
  let armed = true;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    const result = await upsert(...args);
    if (armed) {
      armed = false;
      f.data["local-projects"].q = { ...project("q"), name: "concurrent" };
      f.notify({ type: "global-state-updated", keys: ["local-projects"] });
    }
    return result;
  };
  await f.document("B");
  assert.equal(f.data["local-projects"].q.name, "concurrent");
  assert.equal(f.data[key].projects.q.name, "concurrent");
  f.dispose();
});
test("a deletion during restoration is captured before later upserts can resurrect it", async () => {
  const f = fixture({ ...baseline(), "local-projects": { p: project("p"), q: project("q") } });
  f.start(); await f.drain();
  const upsert = f.services.projects.upsertLocal;
  let armed = true;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    const result = await upsert(...args);
    if (armed) { armed = false; await f.services.projects.removeLocal("p"); }
    return result;
  };
  await f.document("B");
  assert.equal(f.data[key].projects.p, undefined);
  await f.document(null);
  assert.equal(f.data["local-projects"].p, undefined);
  assert.ok(f.data["local-projects"].q);
  f.dispose();
});
test("an explicit unassignment during restoration is replayed after temporary inference protection", async () => {
  const f = fixture();
  f.start(); await f.drain();
  const upsert = f.services.projects.upsertLocal;
  let armed = true;
  f.services.projects.upsertLocal = async (...args: any[]) => {
    const result = await upsert(...args);
    if (armed) {
      armed = false;
      await f.services.threadProjectAssignments.restoreMemberships([{ threadId: "t", hostId: "local", assignment: null, projectless: false }]);
    }
    return result;
  };
  await f.document("B");
  assert.equal(f.data["thread-project-assignments"].t, undefined);
  assert.equal(f.data["projectless-thread-ids"].includes("t"), false);
  assert.equal(f.data[key].memberships.t.assignment, null);
  f.dispose();
});
test("multiple windows coordinate snapshots through the same per-home native lock", async () => {
  const f = fixture();
  const first = f.start(), second = f.start();
  await Promise.all([first.reconcile(), second.reconcile()]);
  f.data["local-projects"].q = project("q");
  f.notify({ type: "global-state-updated", keys: ["local-projects"] });
  await Promise.all([first.reconcile(), second.reconcile()]);
  await f.drain();
  assert.deepEqual(Object.keys(f.data[key].projects).sort(), ["p", "q"]);
  assert.deepEqual(f.errors, []);
  f.dispose();
});
test("late self notifications do not swallow subsequent explicit membership updates", async () => {
  const f = fixture();
  f.start(); await f.drain();
  await f.services.threadProjectAssignments.restoreMemberships([{ threadId: "t", hostId: "local", assignment: null, projectless: false }]);
  await f.drain();
  assert.equal(f.data[key].memberships.t.assignment, null);
  assert.equal(f.data[key].memberships.t.projectless, false);
  f.dispose();
});
test("numeric Rspack discovery inspects executed cache only and fails on ambiguity", async () => {
  const f = fixture();
  let factories = 0;
  const runtime = { c: { ready: { exports: { appServices: f.services } } },
    m: { dormant() { factories++; } } };
  assert.equal(await loadServices({
    urls: ["app://-/assets/12.abcdef.js"], importModule: async () => ({ __webpack_require__: runtime }),
    legacy: () => { throw Error("Legacy discovery must not run"); },
  }), f.services);
  assert.equal(factories, 0);
  await assert.rejects(loadServices({
    urls: ["app://-/assets/12.abcdef.js"],
    importModule: async () => ({ __webpack_require__: { c: {
      a: { exports: { appServices: f.services } }, b: { exports: { appServices: fixture().services } },
    } } }),
  }), /Ambiguous/);
});
