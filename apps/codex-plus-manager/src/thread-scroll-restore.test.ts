import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const renderer = readFileSync(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
const extract = (name: string) => {
  const body = renderer.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"))?.[0];
  assert.ok(body, name);
  return body;
};

const sessionId = "thread-12345678";
type Entry = { top: number; scrollHeight: number; clientHeight: number; at: number; atBottom?: boolean };

function fixture(entry?: Entry) {
  const stored = new Map<string, string>();
  if (entry) stored.set("codexThreadScroll", JSON.stringify({ version: "1", entries: { [sessionId]: entry } }));
  stored.set("settings", JSON.stringify({ threadScrollRestore: true }));
  const timers = new Map<number, { delay: number; run: () => void }>();
  const scrollListeners = new Set<() => void>();
  let nextTimer = 0;
  let currentSessionId = sessionId;
  const scroller = {
    scrollHeight: 20000, clientHeight: 1000, scrollTop: 19000, direction: "column",
    scrollTo({ top }: { top: number }) { this.scrollTop = top; },
    querySelector(_selector: string): object | null { return {}; },
    closest(_selector: string): object | null { return null; },
    contains(node: unknown) { return node === this; },
    addEventListener(_name: string, listener: () => void) { scrollListeners.add(listener); },
    removeEventListener(_name: string, listener: () => void) { scrollListeners.delete(listener); },
  };
  const context = vm.createContext({
    window: {},
    document: { scrollingElement: {}, documentElement: {}, body: {} },
    localStorage: {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => stored.set(key, value),
      removeItem: (key: string) => stored.delete(key),
    },
    codexPlusBackendSettings: { enhancementsEnabled: true } as Record<string, unknown>,
    codexPlusBackendSettingsLoaded: false,
    codexPlusSettingsKey: "settings", conversationViewDefaultWidth: 900,
    codexThreadScrollKey: "codexThreadScroll", codexThreadScrollVersion: "1",
    codexThreadScrollMaxEntries: 120, codexThreadScrollSaveThrottleMs: 120,
    codexThreadScrollRestoreWindowMs: 3200, codexThreadScrollRestoreDelaysMs: [0, 80, 220],
    codexThreadScrollUserIntentWindowMs: 1200, codexThreadScrollListenerVersion: "4",
    sessionKey: (id: string) => id,
    currentSessionRef: () => ({ session_id: currentSessionId }),
    currentThreadScroller: () => scroller,
    getComputedStyle: () => ({ flexDirection: scroller.direction }),
    setTimeout: (run: () => void, delay: number) => {
      const id = ++nextTimer;
      timers.set(id, { delay, run });
      return id;
    },
    clearTimeout: (id: number) => timers.delete(id),
  });
  const settingsStart = renderer.indexOf("  function defaultCodexPlusSettings()");
  const settingsEnd = renderer.indexOf("  // Dream skin runtime", settingsStart);
  assert.ok(settingsStart >= 0 && settingsEnd > settingsStart);
  const names = [
    "finiteNonNegativeNumber", "finiteScrollNumber", "validThreadScrollSessionKey",
    "normalizedCodexThreadUuid", "currentThreadScrollSessionId",
    "readThreadScrollEntries", "writeThreadScrollEntries", "threadScrollRuntime",
    "clearThreadScrollRestoreTimers", "clearThreadScrollSyncTimers", "clearThreadScrollRestoreLock",
    "cancelThreadScrollRestoreForUserIntent", "userScrollIntentActive", "threadScrollRestoreCancelledForSession",
    "activeThreadScrollRestoreLock", "currentThreadScrollRestoreLock", "threadScrollIsReversed", "threadScrollRange",
    "startThreadScrollRestoreLock", "prepareThreadScrollRestoreLock", "threadScrollTargetTop",
    "threadScrollNearBottom", "threadScrollGuardScroller", "shouldBlockThreadScrollAutobottom",
    "bindThreadScrollListener", "saveThreadScrollPositionNow", "scheduleThreadScrollSave",
    "restoreThreadScrollPosition", "scheduleThreadScrollRestore", "syncThreadScrollState",
    "scheduleThreadScrollSyncAttempts", "captureThreadScrollNavigation",
  ];
  if (renderer.includes("  function threadScrollRestoreEnabled(")) names.push("threadScrollRestoreEnabled");
  vm.runInContext([renderer.slice(settingsStart, settingsEnd), ...names.map(extract)].join("\n"), context);
  const flushTimers = () => {
    while (timers.size) {
      const [id, timer] = [...timers.entries()].sort((a, b) => a[1].delay - b[1].delay || a[0] - b[0])[0];
      timers.delete(id);
      timer.run();
    }
  };
  return {
    context, scroller, stored, timers, scrollListeners, flushTimers,
    setSessionId: (value: string) => { currentSessionId = value; },
  };
}

const reading: Entry = { top: 2000, scrollHeight: 6000, clientHeight: 1000, at: 1 };
const bottom: Entry = { top: 5000, scrollHeight: 6000, clientHeight: 1000, at: 1 };

function scrollClickFixture(context: vm.Context, scroller: ReturnType<typeof fixture>["scroller"]) {
  const listeners = new Map<string, (event: object) => void>();
  const root = {};
  const otherRoot = {};
  const footer = {};
  scroller.closest = (selector) => selector === "[data-request-input-activity-root]" ? root : null;
  context.document.addEventListener = (name: string, handler: (event: object) => void) => listeners.set(name, handler);
  context.document.removeEventListener = () => {};
  context.selectors = { sidebarThread: "[data-sidebar-thread]" };
  vm.runInContext([
    extract("eventTargetsActiveThreadScroller"), extract("markThreadScrollUserIntent"),
    extract("installThreadScrollNavigationCapture"),
  ].join("\n"), context);
  context.window.__codexThreadScrollHandlers = {
    markUserIntent: context.markThreadScrollUserIntent,
    captureNavigation: () => {},
  };
  context.installThreadScrollNavigationCapture();
  const target = ({ sameRoot = true, inFooter = true, arrow = true, dialog = false, icon = false } = {}) => {
    const button = {
      classList: { contains: (name: string) => arrow && name === "group/scroll-to-bottom" },
      closest: (selector: string) => {
        if (selector === "[data-request-input-activity-root]") return sameRoot ? root : otherRoot;
        if (selector === "[data-thread-scroll-footer], .conversation-footer") return inFooter ? footer : null;
        if (selector === "dialog, [role='dialog']") return dialog ? {} : null;
        if (selector === "button") return button;
        return null;
      },
    };
    return icon ? { closest: (selector: string) => selector === "button" ? button : null } : button;
  };
  return {
    click: (element: object) => listeners.get("click")!({ type: "click", target: element }),
    target,
  };
}

test("native bottom-button clicks cancel pending restores and persist the bottom", () => {
  for (const icon of [false, true]) {
    const { context, scroller, stored, scrollListeners, flushTimers } = fixture(reading);
    context.codexPlusBackendSettingsLoaded = true;
    context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
    const { click, target } = scrollClickFixture(context, scroller);
    context.syncThreadScrollState();
    assert.ok(context.threadScrollRuntime().restoreLock);
    click(target({ icon }));
    assert.equal(context.threadScrollRuntime().restoreLock, null);
    assert.equal(context.shouldBlockThreadScrollAutobottom(scroller, 19000), false);
    scroller.scrollTo({ top: 19000 });
    for (const listener of scrollListeners) listener();
    flushTimers();
    context.syncThreadScrollState(true);
    flushTimers();
    assert.equal(scroller.scrollTop, 19000);
    assert.equal(JSON.parse(stored.get("codexThreadScroll")!).entries[sessionId].atBottom, true);
  }
});

test("clicks on unrelated controls or another pane leave the pending restore intact", () => {
  for (const options of [{ arrow: false }, { inFooter: false }, { sameRoot: false }, { dialog: true }]) {
    const { context, scroller, flushTimers } = fixture(reading);
    context.codexPlusBackendSettingsLoaded = true;
    context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
    const { click, target } = scrollClickFixture(context, scroller);
    context.syncThreadScrollState();
    click(target(options));
    assert.ok(context.threadScrollRuntime().restoreLock, JSON.stringify(options));
    flushTimers();
    assert.equal(scroller.scrollTop, reading.top, JSON.stringify(options));
  }
});

test("a bottom-button click cancels the visible session even while the previous session is active", () => {
  const nextId = "thread-87654321";
  const { context, scroller, stored, setSessionId, flushTimers } = fixture(reading);
  stored.set("codexThreadScroll", JSON.stringify({ version: "1", entries: {
    [sessionId]: reading, [nextId]: reading,
  } }));
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  const { click, target } = scrollClickFixture(context, scroller);
  context.syncThreadScrollState();
  setSessionId(nextId);
  click(target({ icon: true }));
  scroller.scrollTo({ top: 19000 });
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(scroller.scrollTop, 19000);
  assert.equal(context.threadScrollRuntime().userCancelledRestoreSessionId, nextId);
});

test("bottom-button clicks respect unloaded settings and inactive enhancements", () => {
  for (const state of ["waiting", "disabled", "enhancements", "builtin"]) {
    const { context, scroller, flushTimers } = fixture(reading);
    if (state === "disabled") {
      context.codexPlusBackendSettingsLoaded = true;
      context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
    } else if (state === "enhancements") {
      context.codexPlusBackendSettings.enhancementsEnabled = false;
    } else if (state === "builtin") {
      context.__codexPlusBuiltinInactive = new Set(["thread-scroll-restore"]);
    }
    const { click, target } = scrollClickFixture(context, scroller);
    context.syncThreadScrollState();
    click(target({ icon: true }));
    if (state === "disabled" || state === "enhancements" || state === "builtin") {
      assert.equal(context.threadScrollRuntime().userScrollIntentUntil, 0, state);
      continue;
    }
    assert.equal(context.threadScrollRuntime().userCancelledRestoreSessionId, sessionId, state);
    scroller.scrollTo({ top: 19000 });
    if (state === "waiting") {
      context.codexPlusBackendSettingsLoaded = true;
      context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
    }
    context.syncThreadScrollState(true);
    flushTimers();
    assert.equal(scroller.scrollTop, 19000, state);
  }
});

test("selects the visible thread scroller when a hidden conversation remains mounted", () => {
  const hidden = { isConnected: true, clientWidth: 0, clientHeight: 0 };
  const visible = { isConnected: true, clientWidth: 900, clientHeight: 720 };
  const document = {
    querySelector: () => hidden,
    querySelectorAll: () => [hidden, visible],
  };
  const context = vm.createContext({ document });
  vm.runInContext(extract("currentThreadScroller"), context);
  assert.equal(context.currentThreadScroller(), visible);
  document.querySelectorAll = () => [hidden];
  assert.equal(context.currentThreadScroller(), null);
});

test("identifies the visible conversation when its sidebar row is absent", () => {
  const { context, scroller, flushTimers, setSessionId } = fixture(reading);
  const visibleId = "01a18420-78e4-79c1-8ef2-9e0ce9fc61aa";
  const marker = (id: string, width: number) => ({
    getAttribute: () => id,
    closest: () => ({ getBoundingClientRect: () => ({ width }) }),
  });
  context.document.querySelectorAll = () => [
    marker("01a18420-78e4-79c1-8ef2-9e0ce9fc61ab", 0),
    marker(visibleId, 900),
  ];
  vm.runInContext([extract("normalizedCodexThreadUuid"), extract("codexPlusCurrentSessionId")].join("\n"), context);
  setSessionId("");
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  scroller.scrollTop = 2000;
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(context.threadScrollRuntime().activeSessionId, visibleId);
  assert.equal(scroller.scrollTop, 2000);
});

test("restores a saved reading position when the visible composer identifies the conversation", () => {
  const { context, scroller, stored, flushTimers, setSessionId } = fixture();
  const visibleId = "01a18420-78e4-79c1-8ef2-9e0ce9fc61aa";
  stored.set("codexThreadScroll", JSON.stringify({ version: "1", entries: { [visibleId]: reading } }));
  context.document.querySelectorAll = () => [{
    getAttribute: () => visibleId,
    closest: () => ({ getBoundingClientRect: () => ({ width: 900 }) }),
  }];
  vm.runInContext([extract("normalizedCodexThreadUuid"), extract("codexPlusCurrentSessionId")].join("\n"), context);
  setSessionId("");
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(context.threadScrollRuntime().activeSessionId, visibleId);
  assert.equal(scroller.scrollTop, reading.top);
});

test("does not use a hidden composer to identify the current conversation", () => {
  const { context, setSessionId } = fixture(reading);
  context.document.querySelectorAll = () => [{
    getAttribute: () => "01a18420-78e4-79c1-8ef2-9e0ce9fc61aa",
    closest: () => ({ getBoundingClientRect: () => ({ width: 0 }) }),
  }];
  vm.runInContext([extract("normalizedCodexThreadUuid"), extract("codexPlusCurrentSessionId")].join("\n"), context);
  setSessionId("");
  context.syncThreadScrollState();
  assert.equal(context.threadScrollRuntime().activeSessionId, "");
});

test("missing, partial, or disabled backend settings cannot restore, save, or block native scrolling", () => {
  for (const [loaded, setting] of [[false, undefined], [true, undefined], [true, false]] as const) {
    const { context, scroller, stored, flushTimers } = fixture(reading);
    context.codexPlusBackendSettingsLoaded = loaded;
    if (setting !== undefined) context.codexPlusBackendSettings.codexAppThreadScrollRestore = setting;
    context.syncThreadScrollState(true);
    flushTimers();
    assert.equal(scroller.scrollTop, 19000, `loaded=${loaded} setting=${setting}`);
    context.startThreadScrollRestoreLock(sessionId, reading);
    assert.equal(context.shouldBlockThreadScrollAutobottom(scroller, 19000), false);
    scroller.scrollTop = 1234;
    context.saveThreadScrollPositionNow(sessionId, scroller);
    assert.equal(JSON.parse(stored.get("codexThreadScroll")!).entries[sessionId].top, reading.top);
  }
});

test("disabled restoration leaves positions unchanged when switching conversations", () => {
  const { context, scroller, flushTimers, setSessionId } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  scroller.scrollTop = 2000;
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(scroller.scrollTop, 2000);

  context.captureThreadScrollNavigation("thread-87654321");
  setSessionId("thread-87654321");
  scroller.scrollTop = 1500;
  context.syncThreadScrollState(true);
  flushTimers();
  assert.equal(scroller.scrollTop, 1500);
});

test("disabling restoration cancels a pending restore", () => {
  const { context, scroller, flushTimers } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  context.syncThreadScrollState();
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  context.syncThreadScrollState();
  scroller.scrollTop = 1500;
  flushTimers();
  assert.equal(scroller.scrollTop, 1500);
  assert.equal(context.shouldBlockThreadScrollAutobottom(scroller, 19000), false);
});

test("a delayed restore cannot move the previous session's position into a new conversation", () => {
  const { context, scroller, setSessionId } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  context.syncThreadScrollState();
  setSessionId("thread-87654321");
  scroller.scrollTop = 1500;
  context.restoreThreadScrollPosition(sessionId);
  assert.equal(scroller.scrollTop, 1500);
});

test("disabled enhancements or an inactive builtin do not restore a position", () => {
  for (const inactive of [false, true]) {
    const { context, scroller, flushTimers } = fixture(reading);
    context.codexPlusBackendSettingsLoaded = true;
    context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
    if (inactive) {
      context.__codexPlusBuiltinInactive = new Set(["thread-scroll-restore"]);
    } else {
      context.codexPlusBackendSettings.enhancementsEnabled = false;
    }
    scroller.scrollTop = 2000;
    context.syncThreadScrollState();
    flushTimers();
    assert.equal(scroller.scrollTop, 2000);
  }
});

test("user input does not activate scroll handling when restoration is disabled", () => {
  for (const inactive of ["setting", "enhancements", "builtin"]) {
    const { context, scroller } = fixture(reading);
    context.codexPlusBackendSettingsLoaded = true;
    context.codexPlusBackendSettings.codexAppThreadScrollRestore = inactive !== "setting";
    if (inactive === "enhancements") context.codexPlusBackendSettings.enhancementsEnabled = false;
    if (inactive === "builtin") context.__codexPlusBuiltinInactive = new Set(["thread-scroll-restore"]);
    context.eventTargetsActiveThreadScroller = () => true;
    vm.runInContext(extract("markThreadScrollUserIntent"), context);
    context.markThreadScrollUserIntent({ target: scroller });
    assert.equal(context.threadScrollRuntime().userScrollIntentUntil, 0);
  }
});

test("enabled restoration preserves an actual reading position", () => {
  const { context, scroller, flushTimers } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  context.syncThreadScrollState(true);
  flushTimers();
  assert.equal(scroller.scrollTop, 2000);
  assert.equal(context.shouldBlockThreadScrollAutobottom(scroller, 19000), true);
});

test("a formerly bottom-aligned legacy or new record follows a growing bottom", () => {
  for (const entry of [bottom, { ...bottom, atBottom: true }]) {
    const { context, scroller, flushTimers } = fixture(entry);
    context.codexPlusBackendSettingsLoaded = true;
    context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
    context.syncThreadScrollState(true);
    flushTimers();
    assert.equal(scroller.scrollTop, 19000);
    assert.equal(context.shouldBlockThreadScrollAutobottom(scroller, 19000), false);
    scroller.scrollHeight = 24000;
    context.restoreThreadScrollPosition(sessionId);
    assert.equal(scroller.scrollTop, 23000);
  }
});

test("a reversed scroller records and restores its bottom", () => {
  const { context, scroller, stored, flushTimers } = fixture();
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  scroller.direction = "column-reverse";
  scroller.scrollTop = 0;
  context.saveThreadScrollPositionNow(sessionId, scroller);
  assert.equal(JSON.parse(stored.get("codexThreadScroll")!).entries[sessionId].atBottom, true);
  scroller.scrollTop = -5000;
  context.syncThreadScrollState(true);
  flushTimers();
  assert.equal(scroller.scrollTop, 0);
});

test("zero-size measurements neither replace good entries nor restore stale ones", () => {
  const { context, scroller, stored } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  scroller.scrollHeight = 0;
  scroller.clientHeight = 0;
  scroller.scrollTop = 0;
  context.saveThreadScrollPositionNow(sessionId, scroller);
  assert.equal(JSON.parse(stored.get("codexThreadScroll")!).entries[sessionId].top, reading.top);
  const invalid = { top: 0, scrollHeight: 0, clientHeight: 0, at: 2 };
  context.window.__codexThreadScrollEntries = { [sessionId]: invalid };
  assert.equal(context.readThreadScrollEntries()[sessionId], undefined, "cached entries survive reinjection");
  context.window.__codexThreadScrollEntries = null;
  stored.set("codexThreadScroll", JSON.stringify({ version: "1", entries: { [sessionId]: invalid } }));
  assert.equal(context.readThreadScrollEntries()[sessionId], undefined, "persisted entries are also ignored");
});

test("an unloaded non-scrollable conversation does not overwrite a saved reading position", () => {
  const { context, scroller, stored } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  scroller.scrollHeight = 1000;
  scroller.clientHeight = 1000;
  scroller.scrollTop = 0;
  scroller.querySelector = () => null;
  context.saveThreadScrollPositionNow(sessionId, scroller);
  assert.deepEqual(JSON.parse(stored.get("codexThreadScroll")!).entries[sessionId], reading);
});

test("a loaded short conversation can replace an older saved position", () => {
  const { context, scroller, stored } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  scroller.scrollHeight = 1000;
  scroller.clientHeight = 1000;
  scroller.scrollTop = 0;
  context.saveThreadScrollPositionNow(sessionId, scroller);
  const entry = JSON.parse(stored.get("codexThreadScroll")!).entries[sessionId];
  assert.equal(entry.top, 0);
  assert.equal(entry.atBottom, true);
});

test("disabling restoration removes the active scroll listener", () => {
  const { context, scrollListeners } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  context.syncThreadScrollState(true);
  assert.equal(scrollListeners.size, 1);
  vm.runInContext(extract("stopThreadScrollRestore"), context);
  context.stopThreadScrollRestore();
  assert.equal(scrollListeners.size, 0);
});

test("a late settings load starts restoration once, without repeating it on each heartbeat", () => {
  const { context } = fixture(reading);
  const listeners = new Map<string, () => void>();
  let starts = 0;
  let stops = 0;
  Object.assign(context, {
    runScanStep: (fn: () => void) => fn(),
    scheduleDeferredScan: () => {},
    stopThreadScrollRestore: () => { stops++; },
    scheduleThreadScrollSyncAttempts: () => { starts++; },
  });
  vm.runInContext([extract("codexPlusAlignWith"), extract("codexPlusThreadScrollRestoreExtension")].join("\n"), context);
  context.codexPlusThreadScrollRestoreExtension({
    onScan() {},
    on: (name: string, handler: () => void) => listeners.set(name, handler),
    effect: (fn: () => () => void) => fn(),
  });
  assert.equal(starts, 0);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  listeners.get("settings/loaded")!();
  listeners.get("settings/loaded")!();
  assert.equal(starts, 1);
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  listeners.get("settings/loaded")!();
  assert.ok(stops > 0);
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  listeners.get("settings/changed")!();
  assert.equal(starts, 1, "re-enabling does not restore over the current reading position");
});

test("enabling restoration after it was disabled does not move the reader", () => {
  for (const scanned of [false, true]) {
    const { context, scroller, scrollListeners, flushTimers } = fixture(reading);
    const listeners = new Map<string, () => void>();
    Object.assign(context, {
      runScanStep: (fn: () => void) => fn(),
      scheduleDeferredScan: () => {},
    });
    vm.runInContext([
      extract("stopThreadScrollRestore"), extract("scheduleThreadScrollSync"),
      extract("codexPlusAlignWith"), extract("codexPlusThreadScrollRestoreExtension"),
    ].join("\n"), context);
    context.codexPlusBackendSettingsLoaded = true;
    context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
    context.codexPlusThreadScrollRestoreExtension({
      onScan() {},
      on: (name: string, handler: () => void) => listeners.set(name, handler),
      effect: (fn: () => () => void) => fn(),
    });
    if (scanned) context.syncThreadScrollState();
    scroller.scrollTop = 1500;
    context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
    listeners.get("settings/changed")!();
    flushTimers();
    assert.equal(scroller.scrollTop, 1500);
    assert.equal(scrollListeners.size, 1);
  }
});

test("a user scroll before settings load is not overwritten by late restoration", () => {
  const { context, scroller, scrollListeners, flushTimers } = fixture(reading);
  const listeners = new Map<string, () => void>();
  Object.assign(context, {
    runScanStep: (fn: () => void) => fn(),
    scheduleDeferredScan: () => {},
  });
  vm.runInContext([
    extract("stopThreadScrollRestore"), extract("scheduleThreadScrollSync"),
    extract("markThreadScrollUserIntent"), extract("codexPlusAlignWith"),
    extract("codexPlusThreadScrollRestoreExtension"),
  ].join("\n"), context);
  context.codexPlusThreadScrollRestoreExtension({
    onScan() {},
    on: (name: string, handler: () => void) => listeners.set(name, handler),
    effect: (fn: () => () => void) => fn(),
  });
  context.syncThreadScrollState();
  scroller.scrollTop = 1500;
  context.eventTargetsActiveThreadScroller = () => true;
  context.markThreadScrollUserIntent({ target: scroller });
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  listeners.get("settings/loaded")!();
  flushTimers();
  assert.equal(scroller.scrollTop, 1500);
  assert.equal(scrollListeners.size, 1);
});
