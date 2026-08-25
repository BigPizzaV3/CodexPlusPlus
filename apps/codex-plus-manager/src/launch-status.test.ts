import assert from "node:assert/strict";
import test from "node:test";

import { resolveLaunchStatus } from "./launch-status.ts";

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
