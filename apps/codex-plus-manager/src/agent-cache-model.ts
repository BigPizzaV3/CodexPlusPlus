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

/** 缓存分组的应用名对应哪个 Agent；Codex++ 自己不属于任何 Agent，总是显示。 */
export function cacheAppAgent(app: string): string | null {
  if (app === "Codex") return "codex";
  if (app === "Claude") return "claude";
  if (app === "Grok") return "grok";
  return null;
}

/** 只留当前使用模式下显示的 Agent 的缓存（例如只管理 Codex 时不统计、不清理 Claude 的缓存）。 */
export function visibleCacheGroups(groups: CacheGroup[], visible: (agent: string) => boolean) {
  return groups.filter((group) => {
    const agent = cacheAppAgent(group.app);
    return !agent || visible(agent);
  });
}

export function cacheApps(groups: CacheGroup[], visible: (agent: string) => boolean = () => true) {
  const defaults = ["Codex", "Claude", "Codex++"].filter((name) => {
    const agent = cacheAppAgent(name);
    return !agent || visible(agent);
  });
  const names = [...new Set([...defaults, ...groups.map((group) => group.app)])];
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
