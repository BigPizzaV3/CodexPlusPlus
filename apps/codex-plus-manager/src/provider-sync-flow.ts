export type ProviderSyncNoticeResult = {
  status: string;
  message: string;
};

export function historicalCleanupRecoveryBackupDir(
  result: { backupDir?: string | null } | null,
): string | null {
  const backupDir = result?.backupDir?.trim();
  return backupDir ? backupDir : null;
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
