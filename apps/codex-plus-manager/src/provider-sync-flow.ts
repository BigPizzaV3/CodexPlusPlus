export type ProviderSyncNoticeResult = {
  status: string;
  message: string;
};

export type ProviderSyncStreamProgress = {
  phase:
    | "scanning"
    | "planning"
    | "backing_up"
    | "rewriting"
    | "updating_indexes"
    | "rolling_back"
    | "complete";
  totalRolloutFiles: number;
  scannedRolloutFiles: number;
  plannedRewriteFiles: number;
  appliedRewriteFiles: number;
  skippedLockedRolloutFiles: number;
};

function boundedPercent(value: number, total: number, start: number, end: number): number {
  if (!Number.isFinite(total) || total <= 0) return end;
  const completed = Number.isFinite(value) ? Math.min(Math.max(value, 0), total) : 0;
  return Math.min(100, Math.max(0, start + ((end - start) * completed) / total));
}

export function providerSyncStreamPercent(progress: ProviderSyncStreamProgress): number {
  switch (progress.phase) {
    case "scanning":
      return boundedPercent(progress.scannedRolloutFiles, progress.totalRolloutFiles, 0, 45);
    case "planning":
      return 50;
    case "backing_up":
      return 55;
    case "rewriting":
      return boundedPercent(progress.appliedRewriteFiles, progress.plannedRewriteFiles, 60, 88);
    case "updating_indexes":
      return 92;
    case "rolling_back":
      return 96;
    case "complete":
      return 100;
  }
}

export function resolveProviderSyncCompletion<T extends ProviderSyncNoticeResult>(
  syncResult: T,
  cleanupFailure: ProviderSyncNoticeResult | null,
) {
  if (!cleanupFailure) {
    return {
      result: syncResult,
      progressMessage: null,
      noticeKind: "sync" as const,
    };
  }
  return {
    result: {
      ...syncResult,
      status: cleanupFailure.status,
      message: cleanupFailure.message,
    },
    progressMessage: cleanupFailure.message,
    noticeKind: "cleanup" as const,
  };
}
