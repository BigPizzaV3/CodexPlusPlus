import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../assets/inject/user-scripts-runtime.js", import.meta.url), "utf8");

function page(scripts = {}, document = {}) {
  const timers = [];
  const window = {
    __codexPlusUserScripts: { scripts },
    __codexPlusUserScriptsBootstrap: true,
    setTimeout(fn) { timers.push(fn); },
    location: { reload() {} },
  };
  vm.runInNewContext(source, { window, document, console });
  return { runtime: window.__codexPlusUserScripts, timers, window };
}

test("two-phase operation is idempotent and bound to document and revision", () => {
  let cleanups = 0;
  let applies = 0;
  const { runtime } = page({ old: { cleanups: [() => { cleanups++; }] } });
  const doc = runtime.documentId;
  assert.equal(runtime.prepareOperation("one", doc, "r1", true, 1).phase, "stopped");
  assert.equal(runtime.prepareOperation("one", doc, "r1", true, 1).phase, "stopped");
  assert.equal(runtime.prepareOperation("one", doc, "r2", true, 1).reason, "operation_identity_conflict");
  assert.equal(cleanups, 1);
  assert.equal(runtime.applyOperation("one", "other", "r1", () => applies++).phase, "rejected");
  assert.equal(runtime.applyOperation("one", doc, "r2", () => applies++).phase, "rejected");
  assert.equal(runtime.applyOperation("one", doc, "r1", () => applies++).phase, "verifying");
  assert.equal(runtime.applyOperation("one", doc, "r1", () => applies++).phase, "verifying");
  assert.equal(applies, 1);
});

test("no-page policy rejects old scripts without touching them or scheduling a refresh", () => {
  let cleanups = 0;
  const { runtime, timers } = page({ legacy: {}, other: { cleanups: [() => cleanups++] } });
  const result = runtime.prepareOperation("one", runtime.documentId, "r1", false, 1);
  assert.equal(result.phase, "blocked");
  assert.equal(result.reason, "cleanup_missing");
  assert.equal(cleanups, 0);
  assert.equal(timers.length, 0);
});

for (const [name, cleanup] of [
  ["throws", () => { throw new Error("failed"); }],
  ["returns a promise", () => Promise.resolve()],
]) {
  test(`no-page cleanup that ${name} is degraded without injecting or refreshing`, () => {
    const { runtime, timers } = page({ old: { cleanups: [cleanup] } });
    const result = runtime.prepareOperation("one", runtime.documentId, "r1", false, 1);
    assert.equal(result.phase, "degraded");
    assert.equal(runtime.applyOperation("one", runtime.documentId, "r1", () => {
      throw new Error("must not apply");
    }).phase, "degraded");
    assert.equal(runtime.prepareOperation("two", runtime.documentId, "r2", false, 2).phase, "degraded");
    assert.equal(timers.length, 0);
  });
}

test("old scripts use one page refresh when page fallback is allowed", () => {
  const { runtime, timers } = page({ old: {} });
  assert.equal(runtime.prepareOperation("one", runtime.documentId, "r1", true, 1).phase, "pageRequested");
  assert.equal(runtime.prepareOperation("one", runtime.documentId, "r1", true, 1).phase, "pageRequested");
  assert.equal(timers.length, 1);
});

test("a later fenced operation supersedes a stopped one and rejects late commands", () => {
  let cleanups = 0;
  let applies = 0;
  const { runtime } = page({ old: { cleanups: [() => { cleanups++; }] } });
  const doc = runtime.documentId;
  assert.equal(runtime.prepareOperation("old", doc, "r1", true, 10).phase, "stopped");
  assert.equal(runtime.prepareOperation("new", doc, "r2", true, 11).phase, "stopped");
  assert.equal(runtime.applyOperation("old", doc, "r1", () => {}).phase, "rejected");
  assert.equal(runtime.prepareOperation("late", doc, "r1", true, 9).reason, "stale_operation");
  assert.equal(runtime.applyOperation("new", doc, "r2", () => { applies++; }).phase, "verifying");
  assert.equal(cleanups, 1);
  assert.equal(applies, 1);
});

test("backend source is withdrawn before cleanup and page fallback", () => {
  let duringCleanup;
  const first = page({ old: { cleanups: [() => { duringCleanup = first.window.__codexPlusBackendSource; }] } });
  first.window.__codexPlusBackendSource = { generation: 7 };
  first.window.__codexPlusBackendServices = [{ name: "old" }];
  first.runtime.prepareOperation("clean", first.runtime.documentId, "r1", true, 1);
  assert.equal(duringCleanup, null);
  assert.equal(first.window.__codexPlusBackendServices.length, 0);

  const legacy = page({ old: {} });
  const oldSource = { generation: 7 };
  legacy.window.__codexPlusBackendSource = oldSource;
  legacy.runtime.prepareOperation("blocked", legacy.runtime.documentId, "r1", false, 1);
  assert.equal(legacy.window.__codexPlusBackendSource, oldSource);
  legacy.runtime.prepareOperation("page", legacy.runtime.documentId, "r1", true, 2);
  assert.equal(legacy.window.__codexPlusBackendSource, null);
});

test("native readiness includes settings and rejects an empty or extension-only shell", () => {
  let composer = false;
  let navigation = false;
  let content = false;
  let extensionOnly = false;
  const element = { closest: selector => selector.includes("data-codex-plus-ext") && extensionOnly ? {} : null };
  const root = {
    querySelectorAll(selector) {
      const present = selector.includes("ProseMirror") ? composer
        : selector.includes("app-shell-sidebar") ? navigation : content;
      return present ? [element] : [];
    },
  };
  const document = { readyState: "complete", getElementById: () => root };
  const { runtime } = page({}, document);
  assert.equal(runtime.nativePageReadiness().ready, false);
  navigation = true;
  assert.equal(runtime.nativePageReadiness().ready, false);
  content = true;
  assert.equal(runtime.nativePageReadiness().view, "native_shell");
  extensionOnly = true;
  assert.equal(runtime.nativePageReadiness().ready, false);
  extensionOnly = false;
  composer = true;
  assert.equal(runtime.nativePageReadiness().view, "conversation");
  document.readyState = "loading";
  assert.equal(runtime.nativePageReadiness().ready, false);
});
