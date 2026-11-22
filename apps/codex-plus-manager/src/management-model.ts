/** 管理页只对当前搜索范围操作；会话分组提供项目/时间两种视图。 */
export type ManagedSession = {
  id: string; title: string; cwd: string; modelProvider: string; archived: boolean;
  updatedAtMs: number | null; rolloutPath: string; dbPath: string;
};
/** 会话页的筛选：搜索词在后端全部会话里匹配；archived 为 null 表示归档和未归档都看。 */
export type LocalSessionFilter = { query: string; archived: boolean | null };
export type SessionProjectCount = { path: string; count: number };
export type SessionArchiveView = "active" | "archived" | "all";
export function sessionArchiveFilter(view: SessionArchiveView): boolean | null {
  return view === "active" ? false : view === "archived" ? true : null;
}
export function sessionArchiveView(archived: boolean | null | undefined): SessionArchiveView {
  return archived === false ? "active" : archived === true ? "archived" : "all";
}
/** 所有会话列表共用的搜索：每个词都要出现在任一字段里，不区分大小写。 */
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
  const plain = /^\/\/[?.]\/[A-Za-z]:(?:\/|$)/.test(path) ? path.slice(4) : path;
  // 盘符统一大写：VS Code 里起的 Claude Code 等会把 cwd 记成 `c:\…`，和 Codex 的 `C:\…` 是同一个目录（#2937）。
  return /^[a-z]:(?:\/|$)/.test(plain) ? plain[0].toUpperCase() + plain.slice(1) : plain;
}
/**
 * 按项目或按时间分组。`label` 在按时间分组和「未知目录」时是待翻译的中文文案（`translatable` 为 true），
 * 项目名原样显示；`total` 是整个项目在筛选结果里的会话数（传了 `projects` 才有），分页只截当前页。
 */
export function sessionGroups(sessions: ManagedSession[], mode: "project" | "time", now = Date.now(), projects: SessionProjectCount[] = []) {
  const sorted = [...sessions].sort((a, b) => (b.updatedAtMs ?? 0) - (a.updatedAtMs ?? 0));
  const today = new Date(now); today.setHours(0, 0, 0, 0);
  const yesterday = new Date(today); yesterday.setDate(yesterday.getDate() - 1);
  const week = new Date(today); week.setDate(week.getDate() - ((week.getDay() + 6) % 7));
  const totals = new Map(projects.map((project) => [project.path, project.count]));
  const groups = new Map<string, { key: string; label: string; translatable: boolean; path: string; total?: number; sessions: ManagedSession[] }>();
  for (const session of sorted) {
    const path = normalizeProjectPath(session.cwd);
    const timestamp = session.updatedAtMs ?? 0;
    const key = mode === "project" ? path : timestamp >= +today ? "today" : timestamp >= +yesterday ? "yesterday" : timestamp >= +week ? "week" : "earlier";
    if (!groups.has(key)) {
      const name = mode === "project" ? path.split("/").at(-1) || "" : "";
      groups.set(key, {
        key,
        label: mode === "project" ? name || "未知目录" : ({ today: "今天", yesterday: "昨天", week: "本周", earlier: "更早" }[key] || "更早"),
        translatable: mode !== "project" || !name,
        path: mode === "project" ? path : "",
        total: mode === "project" ? totals.get(path) : undefined,
        sessions: [],
      });
    }
    groups.get(key)!.sessions.push(session);
  }
  return [...groups.values()];
}
export function skillRepoKey(repo: { owner: string; name: string; branch: string; subdir: string }): string {
  return `${repo.owner}/${repo.name}@${repo.branch}${repo.subdir ? `:${repo.subdir}` : ""}`;
}
