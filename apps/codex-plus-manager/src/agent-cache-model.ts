export type CacheGroup = {
  id: string;
  app: string;
  kind: string;
  path: string;
  totalBytes: number;
  eligibleBytes: number;
  eligibleFiles: number;
  skippedFiles: number;
  cleanable: boolean;
};

export function selectableCacheGroups(groups: CacheGroup[]) {
  return groups.filter((group) => group.cleanable && group.eligibleFiles > 0);
}

// 推荐选择排除日志；全部选择也只包含后端允许清理的目录。
export function cacheSelection(groups: CacheGroup[], mode: "recommended" | "all") {
  return selectableCacheGroups(groups)
    .filter((group) => mode === "all" || group.kind !== "logs")
    .map((group) => group.id);
}

// 确认框只显示应用汇总，避免目录数量增加时把操作按钮挤出屏幕。
export function cacheCleanupSummary(groups: CacheGroup[]) {
  const eligible = selectableCacheGroups(groups);
  return {
    apps: [...new Set(eligible.map((group) => group.app))],
    files: eligible.reduce((sum, group) => sum + group.eligibleFiles, 0),
    bytes: eligible.reduce((sum, group) => sum + group.eligibleBytes, 0),
  };
}

export function cacheApps(groups: CacheGroup[]) {
  const names = [...new Set(["Codex", "Claude", "Codex++", ...groups.map((group) => group.app)])];
  return names.map((name) => {
    const appGroups = groups.filter((group) => group.app === name);
    return {
      name,
      groups: appGroups,
      totalBytes: appGroups.reduce((sum, group) => sum + group.totalBytes, 0),
      eligibleBytes: appGroups.reduce((sum, group) => sum + group.eligibleBytes, 0),
    };
  });
}
