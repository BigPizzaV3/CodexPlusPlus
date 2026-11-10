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
    "restoreThreadScrollPosition", "scheduleThreadScrollRestore", "scheduleThreadScrollBottom", "syncThreadScrollState",
    "scheduleThreadScrollSyncAttempts", "captureThreadScrollNavigation", "syncPendingThreadScrollNavigation",
    "threadScrollRestoreSetting", "threadScrollBottomEnabled",
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
  for (const state of ["waiting", "bottom", "enhancements", "builtin"]) {
    const { context, scroller, flushTimers } = fixture(reading);
    if (state === "bottom") {
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
    if (state === "enhancements" || state === "builtin") {
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

test("restores the visible conversation when its sidebar row is absent", () => {
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
  assert.equal(scroller.scrollTop, 19000);
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

test("a disabled position setting goes to the bottom only when opening another session", () => {
  const { context, scroller, flushTimers, setSessionId } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  scroller.scrollTop = 2000;
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(scroller.scrollTop, 19000);

  scroller.scrollTop = 1500;
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(scroller.scrollTop, 1500, "ordinary rescans do not move a reader");

  setSessionId("thread-87654321");
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(scroller.scrollTop, 19000);
});

test("unloaded settings do not force the bottom", () => {
  const { context, scroller, flushTimers } = fixture(reading);
  scroller.scrollTop = 2000;
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(scroller.scrollTop, 2000);
});

test("a startup snapshot of disabled restoration works without the settings bridge", () => {
  for (const loaded of [false, true]) {
    const { context, scroller, flushTimers } = fixture(reading);
    context.window.__CODEX_PLUS_THREAD_SCROLL_RESTORE__ = false;
    context.codexPlusBackendSettingsLoaded = loaded;
    scroller.scrollTop = 2000;
    context.syncThreadScrollState();
    flushTimers();
    assert.equal(scroller.scrollTop, 19000, `partial settings loaded=${loaded}`);
  }
});

test("a live setting overrides the disabled startup snapshot", () => {
  const { context, scroller, flushTimers } = fixture(reading);
  context.window.__CODEX_PLUS_THREAD_SCROLL_RESTORE__ = false;
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  scroller.scrollTop = 19000;
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(scroller.scrollTop, reading.top);
});

test("a live enable cancels a pending bottom from the startup snapshot", () => {
  const { context, scroller, flushTimers } = fixture(reading);
  context.window.__CODEX_PLUS_THREAD_SCROLL_RESTORE__ = false;
  scroller.scrollTop = 2000;
  context.syncThreadScrollState();
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  flushTimers();
  assert.equal(scroller.scrollTop, 2000);
});

test("pending bottom navigation does not move a previous session after a route change", () => {
  const { context, scroller, flushTimers, setSessionId } = fixture(reading);
  context.window.__CODEX_PLUS_THREAD_SCROLL_RESTORE__ = false;
  scroller.scrollTop = 2000;
  context.syncThreadScrollState();
  setSessionId("thread-87654321");
  flushTimers();
  assert.equal(scroller.scrollTop, 2000);
});

test("disabled restoration tracks a new target for earlier bottom placement", () => {
  const { context } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  context.syncThreadScrollState();
  context.captureThreadScrollNavigation("thread-87654321");
  assert.equal(context.threadScrollRuntime().pendingBottomNavigation?.targetSessionId, "thread-87654321");
});

test("earlier bottom placement waits for the target composer and relevant content", () => {
  const { context, scroller, setSessionId } = fixture(reading);
  const target = "01a18420-78e4-79c1-8ef2-9e0ce9fc61ab";
  let composer = "01a18420-78e4-79c1-8ef2-9e0ce9fc61aa";
  context.document.querySelectorAll = () => [{
    getAttribute: () => composer,
    closest: () => ({ getBoundingClientRect: () => ({ width: 900 }) }),
  }];
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  context.syncThreadScrollState();
  context.captureThreadScrollNavigation(target);
  setSessionId(target);
  scroller.scrollTop = 2000;
  context.syncPendingThreadScrollNavigation([{ target: scroller, addedNodes: [] }]);
  assert.equal(scroller.scrollTop, 2000, "the old composer must not move");
  composer = target;
  context.syncPendingThreadScrollNavigation([{ target: {}, addedNodes: [] }]);
  assert.equal(scroller.scrollTop, 2000, "unrelated mutations must not move the conversation");
  context.syncPendingThreadScrollNavigation([{ target: scroller, addedNodes: [] }]);
  assert.equal(scroller.scrollTop, 19000);
  context.cancelThreadScrollRestoreForUserIntent();
  scroller.scrollTop = 1500;
  context.syncPendingThreadScrollNavigation([{ target: scroller, addedNodes: [] }]);
  assert.equal(scroller.scrollTop, 1500, "user input cancels early placement");
});

test("returning to the current row or letting the early placement expire cancels it", () => {
  const { context, setSessionId } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  context.syncThreadScrollState();
  context.captureThreadScrollNavigation("thread-87654321");
  context.captureThreadScrollNavigation(sessionId);
  assert.equal(context.threadScrollRuntime().pendingBottomNavigation, null);
  context.captureThreadScrollNavigation("thread-87654321");
  setSessionId("thread-87654321");
  context.threadScrollRuntime().pendingBottomNavigation.at -= 4000;
  context.syncPendingThreadScrollNavigation([]);
  assert.equal(context.threadScrollRuntime().pendingBottomNavigation, null);
});

test("earlier enabled restoration waits for the visible target and respects user input", () => {
  const { context, scroller, stored, setSessionId } = fixture(reading);
  const target = "01a18420-78e4-79c1-8ef2-9e0ce9fc61ab";
  const targetEntry = { top: 2300, scrollHeight: 6000, clientHeight: 1000, at: 1, atBottom: false };
  stored.set("codexThreadScroll", JSON.stringify({ version: "1", entries: {
    [sessionId]: reading, [target]: targetEntry,
  } }));
  let composer = "01a18420-78e4-79c1-8ef2-9e0ce9fc61aa";
  context.document.querySelectorAll = () => [{
    getAttribute: () => composer,
    closest: () => ({ getBoundingClientRect: () => ({ width: 900 }) }),
  }];
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  context.syncThreadScrollState();
  context.captureThreadScrollNavigation(target);
  setSessionId(target);
  scroller.scrollTop = 1800;
  context.syncPendingThreadScrollNavigation([{ target: scroller, addedNodes: [] }]);
  assert.equal(scroller.scrollTop, 1800, "the old composer must not move");
  composer = target;
  context.syncPendingThreadScrollNavigation([{ target: {}, addedNodes: [] }]);
  assert.equal(scroller.scrollTop, 1800, "unrelated mutations must not move the conversation");
  context.syncPendingThreadScrollNavigation([{ target: scroller, addedNodes: [] }]);
  assert.equal(scroller.scrollTop, targetEntry.top);
  context.cancelThreadScrollRestoreForUserIntent();
  scroller.scrollTop = 1500;
  context.syncPendingThreadScrollNavigation([{ target: scroller, addedNodes: [] }]);
  assert.equal(scroller.scrollTop, 1500, "user input cancels early restoration");
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

test("disabled enhancements or an inactive builtin do not force bottom navigation", () => {
  for (const inactive of [false, true]) {
    const { context, scroller, flushTimers } = fixture(reading);
    context.window.__CODEX_PLUS_THREAD_SCROLL_RESTORE__ = false;
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

test("a user's scroll cancels pending bottom navigation", () => {
  const { context, scroller, flushTimers, setSessionId } = fixture(reading);
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  scroller.scrollTop = 2000;
  context.syncThreadScrollState();
  context.eventTargetsActiveThreadScroller = () => true;
  vm.runInContext(extract("markThreadScrollUserIntent"), context);
  context.markThreadScrollUserIntent({ target: scroller });
  flushTimers();
  assert.equal(scroller.scrollTop, 2000);

  setSessionId("thread-87654321");
  context.syncThreadScrollState();
  flushTimers();
  assert.equal(scroller.scrollTop, 19000, "a later navigation can still go to the bottom");
});

test("a user's scroll also cancels a pending bottom from the startup snapshot", () => {
  const { context, scroller, flushTimers } = fixture(reading);
  context.window.__CODEX_PLUS_THREAD_SCROLL_RESTORE__ = false;
  scroller.scrollTop = 2000;
  context.syncThreadScrollState();
  context.eventTargetsActiveThreadScroller = () => true;
  vm.runInContext(extract("markThreadScrollUserIntent"), context);
  context.markThreadScrollUserIntent({ target: scroller });
  flushTimers();
  assert.equal(scroller.scrollTop, 2000);
});

test("user input does not activate scroll handling when enhancements or the builtin are inactive", () => {
  for (const setting of [false, undefined]) {
    for (const inactive of ["enhancements", "builtin"]) {
      const { context, scroller } = fixture(reading);
      context.window.__CODEX_PLUS_THREAD_SCROLL_RESTORE__ = setting;
      if (inactive === "enhancements") context.codexPlusBackendSettings.enhancementsEnabled = false;
      else context.__codexPlusBuiltinInactive = new Set(["thread-scroll-restore"]);
      context.eventTargetsActiveThreadScroller = () => true;
      vm.runInContext(extract("markThreadScrollUserIntent"), context);
      context.markThreadScrollUserIntent({ target: scroller });
      assert.equal(context.threadScrollRuntime().userScrollIntentUntil, 0);
    }
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
  assert.equal(starts, 2);
});

test("enabling restoration after a user scroll binds the listener without moving the reader", () => {
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
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = false;
  context.codexPlusThreadScrollRestoreExtension({
    onScan() {},
    on: (name: string, handler: () => void) => listeners.set(name, handler),
    effect: (fn: () => () => void) => fn(),
  });
  context.syncThreadScrollState();
  scroller.scrollTop = 1500;
  context.eventTargetsActiveThreadScroller = () => true;
  context.markThreadScrollUserIntent({ target: scroller });
  context.codexPlusBackendSettings.codexAppThreadScrollRestore = true;
  listeners.get("settings/loaded")!();
  flushTimers();
  assert.equal(scroller.scrollTop, 1500);
  assert.equal(scrollListeners.size, 1);
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
