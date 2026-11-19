import assert from "node:assert/strict";
import test from "node:test";
import { SkillSourceScan, discardSkillSourceScan, getSkillSourceScan } from "./skill-source-scan.ts";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const repo = (name: string) => ({ owner: "owner", name, branch: "main", subdir: "", enabled: true });
const result = (name: string) => ({
  status: "ok", message: "", previewId: `preview-${name}`, warnings: [], manualCandidates: [],
  matches: [{ localId: name, localName: name, repoKey: `owner/${name}@main`, repoPath: name, remoteName: name, exact: true }],
});

test("pause retains completed results, waits for the current request, and resumes the remaining repositories", async () => {
  const started: string[] = [];
  const pending = new Map<string, (result: unknown) => void>();
  const scan = new SkillSourceScan(async (command, args) => {
    if (command === "discover_skill_source_evidence") return { status: "ok", evidence: [] };
    const key = String(args.repoKey);
    started.push(key);
    return new Promise((resolve) => pending.set(key, resolve));
  });
  scan.ensure([repo("one"), repo("two"), repo("three")], ["one"]);
  assert.deepEqual(started, ["owner/one@main"]);
  pending.get("owner/one@main")!(result("one"));
  await tick();
  assert.deepEqual(started, ["owner/one@main", "owner/two@main"]);
  scan.pause();
  assert.equal(scan.getSnapshot().status, "paused");
  assert.equal(scan.getSnapshot().currentRepo, "owner/two@main");
  assert.deepEqual(scan.getSnapshot().candidates.map((item) => item.localId), ["one"]);
  pending.get("owner/two@main")!(result("two"));
  await tick();
  assert.deepEqual(started, ["owner/one@main", "owner/two@main"]);
  assert.equal(scan.getSnapshot().progress, 2);
  assert.deepEqual(scan.getSnapshot().candidates.map((item) => item.localId), ["one", "two"]);
  scan.resume();
  assert.deepEqual(started, ["owner/one@main", "owner/two@main", "owner/three@main"]);
  pending.get("owner/three@main")!(result("three"));
  await tick();
  assert.equal(scan.getSnapshot().status, "complete");
  assert.equal(scan.getSnapshot().progress, 3);
});

test("closing the view leaves the same scan running and reopening sees its results", async () => {
  let resolveScan!: (value: unknown) => void;
  const request = async () => new Promise((resolve) => { resolveScan = resolve; });
  const scan = getSkillSourceScan(request, "background-fixture", "C:/fixture");
  let notices = 0;
  const unsubscribe = scan.subscribe(() => notices++);
  scan.ensure([repo("one")], []);
  assert.equal(scan.getSnapshot().status, "scanning");
  unsubscribe();
  const before = notices;
  resolveScan(result("one"));
  await tick();
  assert.equal(notices, before);
  const reopened = getSkillSourceScan(request, "background-fixture", "C:/fixture");
  assert.equal(reopened, scan);
  assert.equal(reopened.getSnapshot().status, "complete");
  assert.equal(reopened.getSnapshot().candidates[0].previewId, "preview-one");
  discardSkillSourceScan(scan, "background-fixture", "C:/fixture");
});

test("scan reports per-repository failures and continues, including repositories added while paused", async () => {
  const started: string[] = [];
  let resolveFirst!: (value: unknown) => void;
  const scan = new SkillSourceScan(async (command, args) => {
    if (command === "discover_skill_source_evidence") throw new Error("evidence denied");
    const key = String(args.repoKey);
    started.push(key);
    if (key === "owner/one@main") return new Promise((resolve) => { resolveFirst = resolve; });
    if (key === "owner/two@main") return { status: "failed", message: "offline", warnings: [], matches: [] };
    throw new Error("network down");
  });
  scan.ensure([repo("one"), repo("two")], ["alpha"]);
  scan.pause();
  scan.addRepo(repo("three"));
  resolveFirst(result("one"));
  await tick();
  assert.deepEqual(started, ["owner/one@main"]);
  assert.deepEqual(scan.getSnapshot().evidenceErrors, ["alpha：Error: evidence denied"]);
  scan.resume();
  await tick();
  assert.equal(scan.getSnapshot().status, "complete");
  assert.equal(scan.getSnapshot().progress, 3);
  assert.deepEqual(scan.getSnapshot().warnings, ["owner/two@main：offline", "owner/three@main：Error: network down"]);
});

test("restarting ignores an old pending response and scans the new request once it settles", async () => {
  const pending: Array<(value: unknown) => void> = [];
  const scan = new SkillSourceScan(async () => new Promise((resolve) => pending.push(resolve)));
  scan.ensure([repo("one")], []);
  scan.restart([repo("two")], []);
  assert.equal(pending.length, 1);
  pending[0](result("one"));
  await tick();
  assert.equal(pending.length, 2);
  assert.deepEqual(scan.getSnapshot().candidates, []);
  pending[1](result("two"));
  await tick();
  assert.deepEqual(scan.getSnapshot().candidates.map((item) => item.localId), ["two"]);
  assert.equal(scan.getSnapshot().status, "complete");
});

test("a partial association removes applied candidates even when the active request returns later", async () => {
  let resolveScan!: (value: unknown) => void;
  const scan = new SkillSourceScan(async () => new Promise((resolve) => { resolveScan = resolve; }));
  scan.ensure([repo("one")], []);
  scan.pause();
  scan.consume(["one"]);
  resolveScan(result("one"));
  await tick();
  assert.deepEqual(scan.getSnapshot().candidates, []);
  assert.equal(scan.getSnapshot().status, "paused");
});

test("pause stops both queues after their current requests and reopening preserves local evidence progress", async () => {
  const started: string[] = [];
  const pending = new Map<string, (value: unknown) => void>();
  const request = async (command: string, args: Record<string, unknown>) => {
    const key = command === "discover_skill_source_evidence" ? `local:${args.id}` : `repo:${args.repoKey}`;
    started.push(key);
    return new Promise((resolve) => pending.set(key, resolve));
  };
  const scan = getSkillSourceScan(request, "paused-evidence-fixture");
  const unsubscribe = scan.subscribe(() => {});
  scan.ensure([repo("one"), repo("two")], ["alpha", "beta"]);
  assert.deepEqual(started, ["local:alpha", "repo:owner/one@main"]);
  scan.pause();
  unsubscribe();
  pending.get("local:alpha")!({ status: "ok", message: "", evidence: [
    { repo: "owner/alpha", kind: "lock", path: "skills-lock.json" },
  ] });
  pending.get("repo:owner/one@main")!(result("one"));
  await tick();
  assert.deepEqual(started, ["local:alpha", "repo:owner/one@main"]);

  const reopened = getSkillSourceScan(request, "paused-evidence-fixture");
  assert.equal(reopened, scan);
  assert.equal(reopened.getSnapshot().status, "paused");
  assert.equal(reopened.getSnapshot().evidenceScanning, false);
  assert.deepEqual(reopened.getSnapshot().evidence.map((item) => item.localId), ["alpha"]);
  assert.equal(reopened.getSnapshot().progress, 1);
  reopened.resume();
  assert.deepEqual(started, ["local:alpha", "repo:owner/one@main", "local:beta", "repo:owner/two@main"]);
  pending.get("local:beta")!({ status: "ok", message: "", evidence: [
    { repo: "owner/beta", kind: "receipt", path: "receipt.json" },
  ] });
  pending.get("repo:owner/two@main")!(result("two"));
  await tick();
  assert.equal(reopened.getSnapshot().status, "complete");
  assert.deepEqual(reopened.getSnapshot().evidence.map((item) => item.localId), ["alpha", "beta"]);
  assert.equal(reopened.getSnapshot().progress, 2);
  discardSkillSourceScan(scan, "paused-evidence-fixture");
});

test("local evidence can be paused and resumed even with no repositories", async () => {
  const started: string[] = [];
  const pending: Array<(value: unknown) => void> = [];
  const scan = new SkillSourceScan(async (_command, args) => {
    started.push(String(args.id));
    return new Promise((resolve) => pending.push(resolve));
  });
  scan.ensure([], ["alpha", "beta"]);
  assert.equal(scan.getSnapshot().status, "scanning");
  scan.pause();
  pending[0]({ status: "ok", message: "", evidence: [] });
  await tick();
  assert.deepEqual(started, ["alpha"]);
  assert.equal(scan.getSnapshot().status, "paused");
  assert.equal(scan.getSnapshot().evidenceScanning, false);
  scan.resume();
  assert.deepEqual(started, ["alpha", "beta"]);
  pending[1]({ status: "ok", message: "", evidence: [] });
  await tick();
  assert.equal(scan.getSnapshot().status, "complete");
});

test("restarting local evidence ignores an old response and starts the new queue", async () => {
  const started: string[] = [];
  const pending: Array<(value: unknown) => void> = [];
  const scan = new SkillSourceScan(async (_command, args) => {
    started.push(String(args.id));
    return new Promise((resolve) => pending.push(resolve));
  });
  scan.ensure([], ["old"]);
  scan.restart([], ["new"]);
  assert.deepEqual(started, ["old"]);
  pending[0]({ status: "ok", message: "", evidence: [
    { repo: "owner/old", kind: "lock", path: "old.json" },
  ] });
  await tick();
  assert.deepEqual(started, ["old", "new"]);
  assert.deepEqual(scan.getSnapshot().evidence, []);
  pending[1]({ status: "ok", message: "", evidence: [
    { repo: "owner/new", kind: "lock", path: "new.json" },
  ] });
  await tick();
  assert.equal(scan.getSnapshot().status, "complete");
  assert.deepEqual(scan.getSnapshot().evidence.map((item) => item.localId), ["new"]);
});
