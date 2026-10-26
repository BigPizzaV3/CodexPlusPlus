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

// 卡片里直接列出「缓存 / 日志 / 更新包 / 临时文件」各占多少，不必打开目录明细才知道占用来自哪里。
// 顺序固定，按体积排会让同一类在不同卡片里跳位置。
const KIND_ORDER = ["cache", "update", "logs", "temporary"];

export function cacheKindTotals(groups: CacheGroup[]) {
  const totals = new Map<string, { kind: string; totalBytes: number; eligibleBytes: number }>();
  for (const group of groups) {
    const entry = totals.get(group.kind) ?? { kind: group.kind, totalBytes: 0, eligibleBytes: 0 };
    entry.totalBytes += group.totalBytes;
    entry.eligibleBytes += group.cleanable ? group.eligibleBytes : 0;
    totals.set(group.kind, entry);
  }
  const rank = (kind: string) => {
    const index = KIND_ORDER.indexOf(kind);
    return index === -1 ? KIND_ORDER.length : index;
  };
  return [...totals.values()]
    .filter((entry) => entry.totalBytes > 0)
    .sort((left, right) => rank(left.kind) - rank(right.kind));
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
