/** 管理页只对当前搜索范围操作；会话分组提供项目/时间两种视图。 */
export type ManagedSession = {
  id: string; title: string; cwd: string; modelProvider: string; archived: boolean;
  updatedAtMs: number | null; rolloutPath: string; dbPath: string;
};
export function matchesManagementQuery(values: string[], query: string): boolean {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const text = values.join(" ").toLocaleLowerCase();
  return terms.every((term) => text.includes(term));
}
/**
 * 项目路径的分组 / 展示形式：统一分隔符、去掉末尾斜杠，保留大小写与空格。
 * Windows 上 Codex 记录的 cwd 可能带长路径前缀（`\\?\C:\...`、`\\?\UNC\srv\share`），
 * 直接换斜杠会显示成 `//?/C:/...`，还会和不带前缀的同一目录分成两组（#2791）。
 * 这里去掉前缀；后端 normalized_project_path 用同一规则按项目查询会话。
 */
export function normalizeProjectPath(cwd: string): string {
  const path = cwd.replace(/\\/g, "/").replace(/\/+$/, "");
  if (/^\/\/[?.]\/UNC\//i.test(path)) return `//${path.slice(8)}`;
  if (/^\/\/[?.]\/[A-Za-z]:(?:\/|$)/.test(path)) return path.slice(4);
  return path;
}
export function sessionGroups(sessions: ManagedSession[], mode: "project" | "time", now = Date.now()) {
  const sorted = [...sessions].sort((a, b) => (b.updatedAtMs ?? 0) - (a.updatedAtMs ?? 0));
  const today = new Date(now); today.setHours(0, 0, 0, 0);
  const yesterday = new Date(today); yesterday.setDate(yesterday.getDate() - 1);
  const week = new Date(today); week.setDate(week.getDate() - ((week.getDay() + 6) % 7));
  const groups = new Map<string, { key: string; label: string; path: string; sessions: ManagedSession[] }>();
  for (const session of sorted) {
    const path = normalizeProjectPath(session.cwd);
    const timestamp = session.updatedAtMs ?? 0;
    const key = mode === "project" ? path : timestamp >= +today ? "today" : timestamp >= +yesterday ? "yesterday" : timestamp >= +week ? "week" : "earlier";
    if (!groups.has(key)) groups.set(key, { key, label: mode === "project" ? path.split("/").at(-1) || "未知目录" : ({ today: "今天", yesterday: "昨天", week: "本周", earlier: "更早" }[key] || "更早"), path: mode === "project" ? path : "", sessions: [] });
    groups.get(key)!.sessions.push(session);
  }
  return [...groups.values()];
}
export function skillRepoKey(repo: { owner: string; name: string; branch: string; subdir: string }): string {
  return `${repo.owner}/${repo.name}@${repo.branch}${repo.subdir ? `:${repo.subdir}` : ""}`;
}
