import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  createRevisionPoller,
  getRevisionPollingInterval,
} from "../web/src/revisionPolling.mjs";

function createTimerHarness() {
  const intervals = [];
  const cleared = [];

  return {
    setInterval(callback, delay) {
      const timer = { callback, delay };
      intervals.push(timer);
      return timer;
    },
    clearInterval(timer) {
      cleared.push(timer);
    },
    fire(index = 0) {
      return intervals[index].callback();
    },
    intervals,
    cleared,
  };
}

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
}

async function runFirstPoll(poller, timer, calls) {
  poller.start();
  await flush();
  if (calls.length === 0) {
    await timer.fire();
    await flush();
  }
}

test("polls the current revision every 2000ms by default and ignores unchanged responses", async () => {
  const timer = createTimerHarness();
  const calls = [];
  let invalidations = 0;
  const poller = createRevisionPoller({
    fetchRevision: async (since) => {
      calls.push(since);
      return { changed: false, revision: 0 };
    },
    onInvalidate: () => {
      invalidations += 1;
    },
    setInterval: timer.setInterval,
    clearInterval: timer.clearInterval,
  });

  await runFirstPoll(poller, timer, calls);

  assert.equal(timer.intervals.length, 1);
  assert.equal(timer.intervals[0].delay, 2000);
  assert.deepEqual(calls, [0]);
  assert.equal(invalidations, 0);
});

test("advances the revision on change and invalidates once", async () => {
  const timer = createTimerHarness();
  const calls = [];
  let invalidations = 0;
  const responses = [
    { changed: false, revision: 0 },
    { changed: true, revision: 7 },
    { changed: false, revision: 7 },
  ];
  const poller = createRevisionPoller({
    fetchRevision: async (since) => {
      calls.push(since);
      return responses.shift();
    },
    onInvalidate: () => {
      invalidations += 1;
    },
    setInterval: timer.setInterval,
    clearInterval: timer.clearInterval,
  });

  await runFirstPoll(poller, timer, calls);
  await timer.fire();
  await flush();
  await timer.fire();
  await flush();

  assert.deepEqual(calls, [0, 0, 7]);
  assert.equal(invalidations, 1);
});

test("does not start another request while a revision request is pending", async () => {
  const timer = createTimerHarness();
  const calls = [];
  let invalidations = 0;
  let resolveRequest;
  const request = new Promise((resolve) => {
    resolveRequest = resolve;
  });
  const poller = createRevisionPoller({
    fetchRevision: (since) => {
      calls.push(since);
      return request;
    },
    onInvalidate: () => {
      invalidations += 1;
    },
    setInterval: timer.setInterval,
    clearInterval: timer.clearInterval,
  });

  poller.start();
  await flush();
  if (calls.length === 0) timer.fire();
  await flush();
  timer.fire();
  timer.fire();
  await flush();

  assert.deepEqual(calls, [0]);

  resolveRequest({ changed: true, revision: 9 });
  await flush();
  await timer.fire();
  await flush();

  assert.deepEqual(calls, [0, 9]);
  assert.equal(invalidations, 1);
});

test("keeps the old revision after an error and retries it on the next tick", async () => {
  const timer = createTimerHarness();
  const calls = [];
  let attempt = 0;
  let invalidations = 0;
  const poller = createRevisionPoller({
    fetchRevision: async (since) => {
      calls.push(since);
      attempt += 1;
      if (attempt === 1) throw new Error("temporarily unavailable");
      return { changed: true, revision: 4 };
    },
    onInvalidate: () => {
      invalidations += 1;
    },
    setInterval: timer.setInterval,
    clearInterval: timer.clearInterval,
  });

  await runFirstPoll(poller, timer, calls);
  await timer.fire();
  await flush();

  assert.deepEqual(calls, [0, 0]);
  assert.equal(invalidations, 1);
});

test("uses an explicit interval and clears its timer when stopped", async () => {
  const timer = createTimerHarness();
  const poller = createRevisionPoller({
    fetchRevision: async () => ({ changed: false, revision: 0 }),
    onInvalidate: () => {},
    intervalMs: 750,
    setInterval: timer.setInterval,
    clearInterval: timer.clearInterval,
  });

  poller.start();
  poller.stop();

  assert.equal(timer.intervals.length, 1);
  assert.equal(timer.intervals[0].delay, 750);
  assert.deepEqual(timer.cleared, [timer.intervals[0]]);
});

test("does not invalidate after being stopped while a request is pending", async () => {
  const timer = createTimerHarness();
  let invalidations = 0;
  let resolveRequest;
  const poller = createRevisionPoller({
    fetchRevision: () => new Promise((resolve) => {
      resolveRequest = resolve;
    }),
    onInvalidate: () => {
      invalidations += 1;
    },
    setInterval: timer.setInterval,
    clearInterval: timer.clearInterval,
  });

  poller.start();
  await flush();
  poller.stop();
  resolveRequest({ changed: true, revision: 1 });
  await flush();

  assert.equal(invalidations, 0);
});

test("cloud metadata selects polling and local metadata does not", () => {
  assert.equal(getRevisionPollingInterval({
    mode: "cloud",
    realtime: { transport: "poll", intervalMs: 2000 },
  }), 2000);
  assert.equal(getRevisionPollingInterval({
    mode: "local",
    realtime: { transport: "poll", intervalMs: 2000 },
  }), null);
  assert.equal(getRevisionPollingInterval({ mode: "cloud" }), null);
});

test("the app connects the selected realtime transport without reloading the page", async () => {
  const [appSource, apiSource] = await Promise.all([
    readFile(new URL("../web/src/App.tsx", import.meta.url), "utf8"),
    readFile(new URL("../web/src/api.ts", import.meta.url), "utf8"),
  ]);

  assert.match(apiSource, /\/api\/revisions\?\$\{query\}/);
  assert.match(appSource, /getRevisionPollingInterval\(taskboardMetadata\)/);
  assert.match(appSource, /createRevisionPoller\(\{/);
  assert.match(appSource, /controller\.abort\(\);\s*poller\.stop\(\)/);
  assert.match(appSource, /new EventSource\("\/api\/events"\)/);
  assert.doesNotMatch(appSource, /location\.reload\(/);

  const pollingEffect = appSource.slice(
    appSource.indexOf("if (revisionPollingInterval === null) return;"),
    appSource.indexOf("function pushUndo"),
  );
  assert.doesNotMatch(pollingEffect, /\bdetailTaskId\b|\btaskboardMetadata\b|\bselectedProjectId\b/);
  assert.match(pollingEffect, /selectedProjectIdRef\.current/);
});
