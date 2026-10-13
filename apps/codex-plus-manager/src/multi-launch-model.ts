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
