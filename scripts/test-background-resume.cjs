const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { execFileSync } = require("node:child_process");

const repo = path.resolve(__dirname, "..");
const source = process.env.CODEX_OVERLAY_BASELINE
  ? execFileSync("git", ["show", `${process.env.CODEX_OVERLAY_BASELINE}:assets/inject/renderer-inject.js`],
    { cwd: repo, encoding: "utf8", maxBuffer: 10 * 1024 * 1024 })
  : fs.readFileSync(path.join(repo, "assets/inject/renderer-inject.js"), "utf8");
const scheduling = source.slice(source.indexOf("  function isExtensionUiNode("),
  source.indexOf("  /**\n   * 侧边栏入口的启动补扫"));
const scan = source.match(/^  function scan\(\) \{[^]*?^  \}/m)[0];
const runStep = source.match(/^  function runScanStep\(step\) \{[^]*?^  \}/m)[0];
const initStart = source.indexOf("  window.__codexPlusScanScheduleCleanup?.();");
const init = initStart < 0 ? "" : source.slice(initStart, source.indexOf("  /**", initStart));
const counters = { light: 0, deferred: 0, dictation: 0 };
const timers = new Map(), frames = new Map(), listeners = new Set();
let nextId = 0;
const document = {
  hidden: false,
  addEventListener(name, handler) { if (name === "visibilitychange") listeners.add(handler); },
  removeEventListener(name, handler) { if (name === "visibilitychange") listeners.delete(handler); },
};
const context = vm.createContext({
  document, window: {}, console,
  setTimeout(fn) { const id = ++nextId; timers.set(id, fn); return id; },
  clearTimeout(id) { timers.delete(id); },
  requestAnimationFrame(fn) { const id = ++nextId; frames.set(id, fn); return id; },
  cancelAnimationFrame(id) { frames.delete(id); },
  pruneSessionMoreMenus() {},
  pruneRemovedSessionActions() { return false; },
  pruneRemovedCodexPlusUiSlots() { return false; },
  installDictationSupportPatch() { counters.dictation++; return Promise.resolve(); },
  scanLightweight() { counters.light++; },
  scanDeferred() { counters.deferred++; },
});
vm.runInContext(`${init}\n${runStep}\n${scan}\n${scheduling}`, context);
const flush = queue => {
  const pending = [...queue.values()];
  queue.clear();
  pending.forEach(fn => fn());
};
const visibility = hidden => {
  document.hidden = hidden;
  [...listeners].forEach(fn => fn());
};
const checks = [];
function check(name, fn) {
  try { fn(); checks.push({ name, passed: true }); console.log(`PASS ${name}`); }
  catch (error) { checks.push({ name, passed: false }); console.log(`FAIL ${name}: ${error.message}`); }
}
check("foreground-scans-share-one-deferred-frame", () => {
  for (let i = 0; i < 300; i++) context.scan();
  assert.equal(frames.size, 1, "RAF backlog must stay bounded even before visibility notification");
  flush(frames);
  assert.equal(counters.deferred, 1);
});
timers.clear(); frames.clear();
check("hidden-scans-do-no-layout-or-dictation-work", () => {
  visibility(true);
  const before = { ...counters };
  for (let i = 0; i < 300; i++) context.scan();
  assert.equal(counters.light, before.light);
  assert.equal(counters.dictation, before.dictation);
  assert.equal(frames.size, 0);
  visibility(false);
  flush(timers);
  flush(frames);
  assert.equal(counters.light, before.light + 1);
  assert.equal(counters.deferred, before.deferred + 1);
});
check("hide-cancels-pending-timer-and-frame-without-losing-work", () => {
  context.scan();
  context.window.__codexSessionDeleteScanPending = true;
  context.window.__codexSessionDeleteScanTimer = context.setTimeout(context.runScheduledScan);
  visibility(true);
  assert.equal(timers.size, 0);
  assert.equal(frames.size, 0);
  const before = counters.deferred;
  visibility(false);
  flush(timers);
  flush(frames);
  assert.equal(counters.deferred, before + 1);
});
check("timer-fired-after-hidden-notification-keeps-one-pending-scan", () => {
  visibility(true);
  const before = counters.light;
  for (let i = 0; i < 100; i++) context.runScheduledScan();
  assert.equal(counters.light, before);
  visibility(false);
  flush(timers);
  flush(frames);
  assert.equal(counters.light, before + 1);
});
check("hidden-dom-mutations-keep-latest-state-and-one-scan", () => {
  let relevanceChecks = 0;
  context.shouldScheduleScan = () => { relevanceChecks++; return true; };
  visibility(true);
  const before = { ...counters };
  let latest;
  for (let i = 0; i < 300; i++) {
    latest = [{ index: i }];
    context.scheduleScan(latest);
  }
  assert.equal(context.window.__codexSessionDeleteLastMutations, latest);
  assert.equal(timers.size + frames.size, 0);
  visibility(false);
  assert.equal(timers.size, 1, "one scan without retired menu refresh work");
  flush(timers);
  flush(frames);
  assert.equal(counters.light - before.light, 1);
  assert.equal(counters.deferred - before.deferred, 1);
  assert.equal(relevanceChecks, 1, "pending scan does not reclassify each mutation batch");
});
check("late-deferred-frame-before-visibility-event-does-no-hidden-work", () => {
  context.scan();
  document.hidden = true;
  const before = counters.deferred;
  flush(frames);
  assert.equal(counters.deferred, before);
  visibility(false);
  flush(frames);
  assert.equal(counters.deferred, before + 1);
});
check("deferred-only-resume-and-idle-visibility-do-not-full-scan", () => {
  context.scan();
  visibility(true);
  const before = counters.light;
  visibility(false);
  flush(timers);
  flush(frames);
  assert.equal(counters.light, before);
  const deferred = counters.deferred;
  visibility(true);
  visibility(false);
  flush(timers);
  flush(frames);
  assert.equal(counters.deferred, deferred);
});
check("virtualized turn mounting and shell output do not schedule a whole-window scan", () => {
  const original = context.shouldScheduleScan;
  context.shouldScheduleScan = vm.runInContext(`(${scheduling.match(/^  function shouldScheduleScan\([^]*?^  \}/m)[0]})`, context);
  const refs = { turn: null, scroller: null, side: null };
  const make = (tokens, parent) => ({
    nodeType: 1, matches: (selector) => selector.split(/,\s*/).some((token) => tokens.includes(token)),
    closest(selector) { return this.matches(selector) ? this : parent?.closest(selector) || null; },
    querySelector() { return null; },
  });
  refs.scroller = make([".thread-scroll-container"]);
  refs.turn = make(["[data-turn-key]", '[data-testid="conversation-turn"]'], refs.scroller);
  refs.side = make(["[data-sidebar-fixture]"]);
  Object.assign(context, {
    selectors: { sidebarThread: "[data-sidebar-fixture]", appHeader: "[data-header-fixture]" },
    codexPluginMarketplacePatchEnabled: () => false,
    nodeOrAncestorLooksLikeCodexUserBubble: () => false,
    nodeLooksLikeCodexUserBubble: () => false,
    isExtensionUiNode: () => false,
  });
  const mutate = (target, addedNodes = [], removedNodes = []) => ({ type: "childList", target, addedNodes, removedNodes });
  try {
    const shell = mutate(refs.turn, [make(['[data-message-author-role]'], refs.turn)]);
    const mount = mutate(refs.scroller, [refs.turn], [make(["[data-measured-turn]"])]);
    const nativeWrapper = mutate(refs.scroller, [make(["[data-virtualized-turn]", '[data-testid="conversation-turn"]'])]);
    const wrappedApproval = {
      ...make(["[data-virtualized-turn]"]),
      querySelector: selector => selector.includes("[data-codex-approval-surface]") ? {} : null,
    };
    const navigation = mutate(refs.scroller, [make([".composer-footer"], refs.scroller)]);
    const recent = mutate(make([]), [refs.side]);
    const approval = mutate(refs.turn, [make(["[data-codex-approval-surface]"], refs.turn)]);
    assert.equal(context.shouldScheduleScan([shell, mount]), false);
    assert.equal(context.shouldScheduleScan([nativeWrapper]), false);
    assert.equal(context.shouldScheduleScan([mutate(refs.scroller, [wrappedApproval])]), true);
    assert.equal(context.shouldScheduleScan([shell, approval]), true);
    assert.equal(context.shouldScheduleScan([mount, recent]), true);
    assert.equal(context.shouldScheduleScan([shell, navigation]), true);
    assert.equal(context.shouldScheduleScan([mutate(refs.scroller, [refs.turn, make([".composer-footer"])])]), true);
  } finally {
    context.shouldScheduleScan = original;
  }
});
check("repeated-background-round-trips-stay-bounded", () => {
  const before = { ...counters };
  for (let round = 0; round < 50; round++) {
    visibility(true);
    for (let i = 0; i < 40; i++) context.scan();
    visibility(false);
    flush(timers);
    flush(frames);
    assert.equal(frames.size + timers.size, 0);
    assert.equal(listeners.size, 1);
  }
  assert.equal(counters.light - before.light, 50);
  assert.equal(counters.deferred - before.deferred, 50);
});
check("reinjection-retires-old-pending-work-and-visibility-listener", () => {
  context.scan();
  context.window.__codexSessionDeleteScanPending = true;
  context.window.__codexSessionDeleteScanTimer = context.setTimeout(context.runScheduledScan);
  vm.runInContext(`(() => {${init}\n${scheduling}})();`, context);
  assert.equal(frames.size + timers.size, 0);
  assert.equal(listeners.size, 1);
  const before = counters.deferred;
  visibility(true);
  visibility(false);
  flush(timers);
  flush(frames);
  assert.equal(counters.deferred, before);
});
console.log(JSON.stringify({ checks, counters }, null, 2));
if (checks.some(result => !result.passed)) process.exitCode = 1;
