export type LaunchStatusSnapshot = {
  status: string;
  message: string;
  started_at_ms: number;
};

export type LaunchStatusResolution = "pending" | "success" | "failed" | "stale";

const SUCCESS_STATUSES = new Set(["running", "running_degraded"]);
const FAILURE_STATUSES = new Set(["failed", "crashed", "stopped"]);

export function resolveLaunchStatus(
  status: LaunchStatusSnapshot | null,
  requestStartedAtMs: number,
): LaunchStatusResolution {
  if (!status || status.started_at_ms < requestStartedAtMs) return "stale";
  if (SUCCESS_STATUSES.has(status.status)) return "success";
  if (FAILURE_STATUSES.has(status.status)) return "failed";
  return "pending";
}
