export type ProviderSyncNoticeResult = {
  status: string;
  message: string;
};

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
