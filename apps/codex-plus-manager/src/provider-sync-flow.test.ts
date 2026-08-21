import assert from "node:assert/strict";
import test from "node:test";

import {
  historicalCleanupRecoveryBackupDir,
  resolveProviderSyncCompletion,
} from "./provider-sync-flow.ts";

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

test("cleanup failure with a backup keeps the UI recovery entry available", () => {
  const cleanupFailure = {
    status: "failed",
    message: "the second database failed",
    backupDir: "C:/backup/history-cleanup/partial",
  };

  assert.equal(
    historicalCleanupRecoveryBackupDir(cleanupFailure),
    cleanupFailure.backupDir,
  );
});
