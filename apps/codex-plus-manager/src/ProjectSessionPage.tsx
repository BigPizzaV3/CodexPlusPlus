import { useMemo, useState, type ReactNode } from "react";
import { ArrowLeft, ArrowRight, ChevronDown, Copy, History, RefreshCw, Search, Trash2 } from "lucide-react";
import { t, tf } from "./i18n";
import { matchesManagementQuery, sessionGroups, type ManagedSession } from "./management-model";
import { ManagementDrawer } from "./ManagementPages";
import "./ManagementPages.css";

type SessionPageData = { sessions: ManagedSession[]; totalCount: number; hasMore: boolean; offset: number; limit: number };
type Notify = (title: string, message: string, status?: "ok" | "failed") => Promise<void>;

function Button({ children, onClick, disabled, primary }: { children: ReactNode; onClick: () => void; disabled?: boolean; primary?: boolean }) {
  return <button type="button" className={`mg-button ${primary ? "mg-primary" : ""}`} onClick={onClick} disabled={disabled}>{children}</button>;
}

function SearchBox({ value, onChange, placeholder }: { value: string; onChange: (value: string) => void; placeholder: string }) {
  return <div className="mg-search"><Search size={14} /><input aria-label={placeholder} value={value} placeholder={placeholder} onChange={(event) => onChange(event.target.value)} /></div>;
}

export function ProjectSessionPage({
  sessions,
  onRefresh,
  onDelete,
  onDeleteMany,
  onArchiveMany,
  onListProjectSessions,
  repairPanel,
}: {
  sessions: SessionPageData | null;
  onRefresh: (offset?: number) => Promise<void>;
  onDelete: (session: ManagedSession) => Promise<void>;
  onDeleteMany: (sessions: ManagedSession[]) => Promise<void>;
  onArchiveMany: (sessions: ManagedSession[], archived?: boolean) => Promise<void>;
  onListProjectSessions: (cwd: string) => Promise<ManagedSession[]>;
  repairPanel?: ReactNode;
}) {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<"project" | "time">("project");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selection, setSelection] = useState(false);
  const [selected, setSelected] = useState<Map<string, ManagedSession>>(new Map());
  const [detail, setDetail] = useState<ManagedSession | null>(null);
  const [projectLoading, setProjectLoading] = useState<string | null>(null);
  const all = sessions?.sessions || [];
  const visible = all.filter((session) => matchesManagementQuery([session.title, session.cwd, session.id, session.modelProvider], query));
  const groups = sessionGroups(visible, mode);
  const selectedRows = [...selected.values()];
  const toggle = (session: ManagedSession) => setSelected((old) => {
    const next = new Map(old);
    if (next.has(session.id)) next.delete(session.id); else next.set(session.id, session);
    return next;
  });
  const selectCurrentPage = () => setSelected((old) => {
    const next = new Map(old);
    visible.forEach((session) => next.set(session.id, session));
    return next;
  });
  const selectProject = async (cwd: string) => {
    if (!cwd || projectLoading) return;
    setProjectLoading(cwd);
    try {
      const projectSessions = await onListProjectSessions(cwd);
      setSelected((old) => {
        const next = new Map(old);
        projectSessions.forEach((session) => next.set(session.id, session));
        return next;
      });
    } finally {
      setProjectLoading(null);
    }
  };
  const deleteSelected = async () => {
    if (!selectedRows.length) return;
    await onDeleteMany(selectedRows);
    setSelected(new Map());
  };
  const archiveSelected = async () => {
    if (!selectedRows.length) return;
    await onArchiveMany(selectedRows, true);
    setSelected(new Map());
  };
  const pageSize = sessions?.limit || 50;
  const page = sessions ? Math.floor(sessions.offset / pageSize) + 1 : 0;
  return <div className="mg-page mg-session-page">
    <header className="mg-header" data-tauri-drag-region>
      <div className="mg-header-title"><History size={20} /><h1>{t("会话")}</h1></div>
      <div className="mg-header-actions">
        <Button onClick={() => void onRefresh()}><RefreshCw size={14} />{t("刷新列表")}</Button>
        <Button onClick={() => { setSelection((old) => !old); setSelected(new Map()); }}>{t("选择多个…")}</Button>
      </div>
    </header>
    {repairPanel}
    <div className="mg-session-controls">
      <SearchBox value={query} onChange={setQuery} placeholder={t("搜索标题、目录或会话 ID")} />
      <div className="mg-segmented">
        <button type="button" data-active={mode === "project"} onClick={() => setMode("project")}>{t("按项目")}</button>
        <button type="button" data-active={mode === "time"} onClick={() => setMode("time")}>{t("按时间")}</button>
      </div>
      {selection ? <>
        <Button onClick={selectCurrentPage}>{t("全选当前列表")}</Button>
        <Button disabled={!selectedRows.length} onClick={() => setSelected(new Map())}>{t("取消全部选择")}</Button>
        <Button disabled={!selectedRows.length} onClick={() => void archiveSelected()}>{t("归档已选")}</Button>
        <Button primary disabled={!selectedRows.length} onClick={() => void deleteSelected()}><Trash2 size={14} />{tf("删除已选 ({0})", [selectedRows.length])}</Button>
      </> : null}
    </div>
    <div className="mg-content"><div className="mg-session-list">
      {!visible.length ? <div className="mg-empty"><h2>{t("还没有匹配的会话")}</h2><Button onClick={() => void onRefresh()}>{t("刷新")}</Button></div> : groups.map((group) => {
        const open = query.trim() || expanded.has(group.key) || groups.length === 1;
        return <section key={group.key}>
          <div className="mg-group-head">
            <button type="button" className="mg-group-head" onClick={() => setExpanded((old) => { const next = new Set(old); next.has(group.key) ? next.delete(group.key) : next.add(group.key); return next; })}>
              <ChevronDown size={15} className={open ? "" : "mg-chevron-collapsed"} /><strong>{group.label}</strong><span>{group.path}</span><small>{group.sessions.length} {t("个会话")}</small>
            </button>
            {selection && mode === "project" && group.path ? <Button disabled={projectLoading !== null} onClick={() => void selectProject(group.path)}>{projectLoading === group.path ? t("读取中…") : t("全选项目")}</Button> : null}
          </div>
          {open && group.sessions.map((session) => <article className="mg-session-row" key={session.id}>
            <div className="mg-session-check">{selection ? <input type="checkbox" checked={selected.has(session.id)} onChange={() => toggle(session)} /> : null}</div>
            <button type="button" className="mg-session-main" onClick={() => setDetail(session)}><strong>{session.title || t("未命名会话")}</strong><small>{session.cwd || t("未记录项目路径")}</small><code>{session.id}</code></button>
            <div className="mg-session-meta"><span>{session.modelProvider || "Codex"}</span><time>{session.updatedAtMs ? new Date(session.updatedAtMs).toLocaleString() : t("未知时间")}</time></div>
            <div className="mg-row-actions"><button type="button" className="mg-icon-button" onClick={() => void onDelete(session)} aria-label={t("删除会话")}><Trash2 size={15} /></button></div>
          </article>)}
        </section>;
      })}
    </div></div>
    <div className="mg-pagination">
      <Button onClick={() => void onRefresh(Math.max(0, (sessions?.offset || 0) - pageSize))} disabled={!sessions || !sessions.offset}><ArrowLeft size={14} />{t("上一页")}</Button>
      <span>{sessions ? tf("第 {0} 页 · 共 {1} 个会话", [page, sessions.totalCount]) : "—"}</span>
      <Button onClick={() => void onRefresh((sessions?.offset || 0) + pageSize)} disabled={!sessions?.hasMore}><ArrowRight size={14} />{t("下一页")}</Button>
    </div>
    {detail ? <ManagementDrawer title={detail.title || t("未命名会话")} onClose={() => setDetail(null)} footer={<><Button onClick={() => navigator.clipboard.writeText(detail.id)}><Copy size={14} />{t("复制会话 ID")}</Button><Button primary onClick={() => void onDelete(detail)}><Trash2 size={14} />{t("删除会话")}</Button></>}><dl className="mg-detail"><dt>{t("项目目录")}</dt><dd>{detail.cwd || "—"}</dd><dt>{t("模型供应商")}</dt><dd>{detail.modelProvider || "Codex"}</dd><dt>{t("数据库")}</dt><dd>{detail.dbPath || "—"}</dd></dl></ManagementDrawer> : null}
  </div>;
}
