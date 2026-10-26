export type FrontendRecoveryReport = {
  status: "pending" | "completed" | "partial" | "unavailable" | "skipped";
  reason?: string | null;
  loadedPages: number;
  metadataPages: number;
};

export function historyRecoveryStatusLabel(report: FrontendRecoveryReport): string {
  switch (report.status) {
    case "pending": return "后台补载中";
    case "completed": return "已完成";
    case "partial": return "部分完成";
    case "unavailable": return "不可用";
    case "skipped": return "已跳过";
  }
}
