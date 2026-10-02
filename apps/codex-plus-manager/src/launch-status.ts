export type LaunchStatusSnapshot = {
  status: string;
  message: string;
  started_at_ms: number;
};

export type LaunchStatusResolution = "pending" | "success" | "failed" | "stale";

const SUCCESS_STATUSES = new Set(["running", "running_degraded"]);
const FAILURE_STATUSES = new Set(["failed", "crashed", "stopped"]);

export function launchCompletionNotice(status: LaunchStatusSnapshot | null, requestStartedAtMs: number) {
  const resolution = resolveLaunchStatus(status, requestStartedAtMs);
  if (resolution === "pending" || resolution === "stale") {
    return { status: "accepted", message: "启动仍在后台进行，可在概览的“最近启动”中查看状态。" };
  }
  if (resolution === "failed") {
    return { status: "failed", message: status?.message || "Codex 启动失败。" };
  }
  return { status: "ok", message: status?.status === "running_degraded"
    ? "Codex 已启动，增强功能仍在等待页面连接。" : "Codex 已成功启动。" };
}

export function resolveLaunchStatus(
  status: LaunchStatusSnapshot | null,
  requestStartedAtMs: number,
): LaunchStatusResolution {
  if (!status || status.started_at_ms < requestStartedAtMs) return "stale";
  if (SUCCESS_STATUSES.has(status.status)) return "success";
  if (FAILURE_STATUSES.has(status.status)) return "failed";
  return "pending";
}
