import assert from "node:assert/strict";
import test from "node:test";

import {
  providerSyncStreamPercent,
  resolveProviderSyncCompletion,
  type ProviderSyncStreamProgress,
} from "./provider-sync-flow.ts";

function progress(
  phase: ProviderSyncStreamProgress["phase"],
  overrides: Partial<ProviderSyncStreamProgress> = {},
): ProviderSyncStreamProgress {
  return {
    phase,
    totalRolloutFiles: 10,
    scannedRolloutFiles: 0,
    plannedRewriteFiles: 4,
    appliedRewriteFiles: 0,
    skippedLockedRolloutFiles: 0,
    ...overrides,
  };
}

test("provider sync stream progress maps scanning counts into the scan range", () => {
  assert.equal(providerSyncStreamPercent(progress("scanning")), 0);
  assert.equal(providerSyncStreamPercent(progress("scanning", { scannedRolloutFiles: 5 })), 22.5);
  assert.equal(providerSyncStreamPercent(progress("scanning", { scannedRolloutFiles: 10 })), 45);
});

test("provider sync stream progress maps rewrite counts into the write range", () => {
  assert.equal(providerSyncStreamPercent(progress("rewriting")), 60);
  assert.equal(providerSyncStreamPercent(progress("rewriting", { appliedRewriteFiles: 2 })), 74);
  assert.equal(providerSyncStreamPercent(progress("rewriting", { appliedRewriteFiles: 4 })), 88);
});

test("provider sync stream progress handles zero totals without NaN", () => {
  const scanning = providerSyncStreamPercent(progress("scanning", { totalRolloutFiles: 0 }));
  const rewriting = providerSyncStreamPercent(progress("rewriting", { plannedRewriteFiles: 0 }));

  assert.equal(scanning, 45);
  assert.equal(rewriting, 88);
  assert.ok(Number.isFinite(scanning));
  assert.ok(Number.isFinite(rewriting));
});

test("provider sync stream progress maps fixed phases and completion", () => {
  assert.equal(providerSyncStreamPercent(progress("planning")), 50);
  assert.equal(providerSyncStreamPercent(progress("backing_up")), 55);
  assert.equal(providerSyncStreamPercent(progress("updating_indexes")), 92);
  assert.equal(providerSyncStreamPercent(progress("rolling_back")), 96);
  assert.equal(providerSyncStreamPercent(progress("complete")), 100);
});

test("provider sync success remains the final visible result when cleanup succeeds", () => {
  const syncResult = { status: "ok", message: "sync complete", changedSessionFiles: 2 };

  const completion = resolveProviderSyncCompletion(syncResult, null);

  assert.equal(completion.noticeKind, "sync");
  assert.equal(completion.progressMessage, null);
  assert.equal(completion.result, syncResult);
});

test("cleanup failure remains final and preserves its recovery path", () => {
  const syncResult = { status: "ok", message: "sync complete", changedSessionFiles: 2 };
  const cleanupFailure = {
    status: "failed",
    message: "cleanup failed; restore from C:/backup/provider-sync/20260715",
  };

  const completion = resolveProviderSyncCompletion(syncResult, cleanupFailure);

  assert.equal(completion.noticeKind, "cleanup");
  assert.equal(completion.progressMessage, cleanupFailure.message);
  assert.equal(completion.result.status, "failed");
  assert.equal(completion.result.message, cleanupFailure.message);
  assert.equal(completion.result.changedSessionFiles, 2);
  assert.notEqual(completion.result.message, syncResult.message);
});
