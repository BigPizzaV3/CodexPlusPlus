export type MultiLaunchLaunchResult = {
  status?: string;
  message?: string;
  finishedAtMs?: number;
  debugPort?: number;
  pid?: number;
};

export type MultiLaunchInstance = {
  instanceId: string;
  profileId?: string;
  profileName?: string;
  label?: string;
  isMain?: boolean;
  pid?: number;
  debugPort?: number;
  helperPort?: number;
  status: string;
  running?: boolean;
  startedAtMs?: number;
  startedAt?: string;
  appPath?: string;
  /** worker 落盘的启动结果；status 为 failed 时 message 就是失败原因。 */
  launchResult?: MultiLaunchLaunchResult | null;
};

export function multiLaunchInstanceLabel(
  instance: MultiLaunchInstance,
  labels: { main: string; fallback: string } = { main: "主 Codex", fallback: "Codex 实例" },
): string {
  if (instance.isMain) return labels.main;
  return instance.profileName?.trim() || instance.label?.trim() || instance.instanceId?.trim() || labels.fallback;
}

export function multiLaunchInstanceStatus(instance: MultiLaunchInstance): string {
  return instance.status?.trim().toLowerCase() || "unknown";
}

export function multiLaunchInstanceRunning(instance: MultiLaunchInstance): boolean {
  if (typeof instance.running === "boolean") return instance.running;
  return ["running", "running_degraded", "starting", "ready", "connected", "ok"].includes(multiLaunchInstanceStatus(instance));
}

/**
 * 实例启动失败时给用户看的原因，没失败时返回空串。
 *
 * Windows 上「点了多开没窗口」以前是静默的：列表只显示运行中/未运行。
 * worker 现在会把失败原因写进 launchResult，这里把它取出来展示。
 */
export function multiLaunchInstanceFailure(instance: MultiLaunchInstance): string {
  if (multiLaunchInstanceRunning(instance)) return "";
  const result = instance.launchResult;
  if (!result || result.status !== "failed") return "";
  return result.message?.trim() || "Codex 实例启动失败，原因未记录。";
}
