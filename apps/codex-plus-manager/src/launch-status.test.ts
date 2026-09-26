import assert from "node:assert/strict";
import test from "node:test";

import { resolveLaunchStatus, launchCompletionNotice } from "./launch-status.ts";

test("completion notice never reports pending or stale launch as successful", () => {
  for (const snapshot of [null, { status: "starting", message: "starting", started_at_ms: 100 },
    { status: "running", message: "previous", started_at_ms: 99 }]) {
    assert.equal(launchCompletionNotice(snapshot, 100).status, "accepted");
  }
  assert.equal(launchCompletionNotice({ status: "failed", message: "failure", started_at_ms: 101 }, 100).message, "failure");
  assert.equal(launchCompletionNotice({ status: "running", message: "ready", started_at_ms: 101 }, 100).status, "ok");
});

test("launch status ignores a terminal result from an older request", () => {
  assert.equal(
    resolveLaunchStatus(
      { status: "failed", message: "old failure", started_at_ms: 99 },
      100,
    ),
    "stale",
  );
});

test("launch status waits while the current request is starting", () => {
  assert.equal(
    resolveLaunchStatus(
      { status: "starting", message: "starting", started_at_ms: 100 },
      100,
    ),
    "pending",
  );
});

test("launch status accepts ready and degraded launches", () => {
  assert.equal(
    resolveLaunchStatus(
      { status: "running", message: "ready", started_at_ms: 101 },
      100,
    ),
    "success",
  );
  assert.equal(
    resolveLaunchStatus(
      { status: "running_degraded", message: "waiting for bridge", started_at_ms: 102 },
      100,
    ),
    "success",
  );
});

test("launch status surfaces current background failures", () => {
  assert.equal(
    resolveLaunchStatus(
      { status: "failed", message: "port is occupied", started_at_ms: 101 },
      100,
    ),
    "failed",
  );
});
