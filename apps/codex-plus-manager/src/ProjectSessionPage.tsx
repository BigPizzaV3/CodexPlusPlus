import { useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Archive, ArrowLeft, ArrowRight, ChevronDown, Copy, Info, RefreshCw, Search, Trash2 } from "lucide-react";
import { t, tf } from "./i18n";
import { matchesManagementQuery, sessionGroups, type ManagedSession } from "./management-model";
import { ManagementDrawer } from "./ManagementPages";
import "./ManagementPages.css";

type SessionPageData = { sessions: ManagedSession[]; totalCount: number; hasMore: boolean; offset: number; limit: number };
function Button({ children, className = "", danger, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { danger?: boolean }) {
  return <button type="button" className={`mg-button ${danger ? "mg-session-danger" : ""} ${className}`} {...props}>{children}</button>;
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
  onDelete: (session: ManagedSession) => Promise<boolean | void>;
  // 返回 false 表示用户取消或没有执行，此时保留当前选择。
  onDeleteMany: (sessions: ManagedSession[]) => Promise<boolean | void>;
  onArchiveMany: (sessions: ManagedSession[], archived?: boolean) => Promise<boolean | void>;
  onListProjectSessions: (cwd: string) => Promise<ManagedSession[]>;
  repairPanel?: ReactNode;
}) {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<"project" | "time">("project");
  const [expanded, setExpanded] = useState<Map<string, boolean>>(new Map());
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
    if ((await onDeleteMany(selectedRows)) !== false) setSelected(new Map());
  };
  const archiveSelected = async () => {
    if (!selectedRows.length) return;
    if ((await onArchiveMany(selectedRows, true)) !== false) setSelected(new Map());
  };
  // 选择可以跨分页（整项目选择），不能按当前页修剪；单删成功的会话要移出选择，
  // 否则随后的批量操作会把已删除的会话再提交一次并报失败。
  const deleteOne = async (session: ManagedSession) => {
    if ((await onDelete(session)) === true) {
      setSelected((old) => {
        if (!old.has(session.id)) return old;
        const next = new Map(old);
        next.delete(session.id);
        return next;
      });
    }
  };
  const pageSize = sessions?.limit || 50;
  const page = sessions ? Math.floor(sessions.offset / pageSize) + 1 : 0;
  return <div className="mg-page mg-session-page">
    <div className="mg-session-toolbar">
      <SearchBox value={query} onChange={(value) => { setQuery(value); setExpanded(new Map()); }} placeholder={t("搜索标题、目录或会话 ID")} />
      <div className="mg-segmented" role="group" aria-label={t("会话视图")}>
        <button type="button" data-active={mode === "project"} aria-pressed={mode === "project"} onClick={() => setMode("project")}>{t("按项目")}</button>
        <button type="button" data-active={mode === "time"} aria-pressed={mode === "time"} onClick={() => setMode("time")}>{t("按时间")}</button>
      </div>
      <div className="mg-session-toolbar-actions">
        <Button className="mg-session-refresh" onClick={() => void onRefresh()} title={t("刷新列表")} aria-label={t("刷新列表")}><RefreshCw size={15} aria-hidden="true" /></Button>
        <Button aria-pressed={selection} onClick={() => { setSelection((old) => !old); setSelected(new Map()); }}>{selection ? t("完成") : t("选择多个…")}</Button>
      </div>
    </div>
    {repairPanel}
    {selection ? <div className="mg-session-selection">
      <div className="mg-session-selection-actions">
        <span className="mg-session-selected-count" role="status" aria-live="polite">{tf("已选择 {0}", [selectedRows.length])}</span>
        <Button onClick={selectCurrentPage}>{t("全选当前列表")}</Button>
        <Button disabled={!selectedRows.length} onClick={() => setSelected(new Map())}>{t("取消全部选择")}</Button>
        <div className="mg-session-batch-actions">
          <Button disabled={!selectedRows.length} onClick={() => void archiveSelected()}><Archive size={14} aria-hidden="true" />{t("归档已选")}</Button>
          <Button danger disabled={!selectedRows.length} onClick={() => void deleteSelected()}><Trash2 size={14} aria-hidden="true" />{tf("删除已选 ({0})", [selectedRows.length])}</Button>
        </div>
      </div>
      <p className="mg-session-selection-hint"><Info size={14} aria-hidden="true" /><span>{t("删除会创建本地备份；如果 Codex App 正在使用该会话，建议先关闭对应会话窗口再操作。")}</span></p>
    </div> : null}
    <div className="mg-content">
      <div className="mg-session-list">
        {!visible.length ? <div className="mg-empty"><h2>{t("还没有匹配的会话")}</h2><Button onClick={() => void onRefresh()}>{t("刷新")}</Button></div> : groups.map((group, index) => {
          const open = expanded.get(group.key) ?? Boolean(query.trim() || groups.length === 1);
          const groupId = `session-group-${mode}-${index}`;
          return <section className="mg-session-group" key={group.key}>
            <div className="mg-session-group-header" data-mode={mode}>
              <button type="button" className="mg-session-group-toggle" aria-expanded={open} aria-controls={groupId} onClick={() => setExpanded((old) => new Map(old).set(group.key, !open))}>
                <ChevronDown size={15} aria-hidden="true" className={open ? "" : "mg-chevron-collapsed"} />
                <span className="mg-session-group-text"><strong title={group.label}>{group.label}</strong>{group.path ? <small title={group.path}>{group.path}</small> : null}</span>
                <span className="mg-session-group-count">{group.sessions.length} {t("个会话")}</span>
              </button>
              {selection && mode === "project" && group.path ? <Button className="mg-session-select-project" disabled={projectLoading !== null} onClick={() => void selectProject(group.path)}>{projectLoading === group.path ? t("读取中…") : t("全选项目")}</Button> : null}
            </div>
            <div id={groupId} hidden={!open}>
              {open && group.sessions.map((session) => <article className="mg-session-row" data-selection={selection} key={session.id}>
                {selection ? <div className="mg-session-check"><input type="checkbox" aria-label={`${t("已选择")}：${session.title || session.id}`} checked={selected.has(session.id)} onChange={() => toggle(session)} /></div> : null}
                <button type="button" className="mg-session-main" onClick={() => setDetail(session)}>
                  <strong title={session.title}>{session.title || t("未命名会话")}</strong>
                  <small title={mode === "time" ? session.cwd : session.modelProvider}>{mode === "time" ? session.cwd || t("未记录项目路径") : session.modelProvider || "Codex"}</small>
                </button>
                <div className="mg-session-meta"><time>{session.updatedAtMs ? new Date(session.updatedAtMs).toLocaleString() : t("未知时间")}</time></div>
                <div className="mg-row-actions"><button type="button" className="mg-icon-button mg-session-delete" onClick={() => void deleteOne(session)} aria-label={`${t("删除会话")}：${session.title || session.id}`} title={t("删除会话")}><Trash2 size={15} aria-hidden="true" /></button></div>
              </article>)}
            </div>
          </section>;
        })}
      </div>
    </div>
    <div className="mg-pagination">
      <span>{sessions ? tf("第 {0} 页 · 共 {1} 个会话", [page, sessions.totalCount]) : "—"}</span>
      <div className="mg-session-page-actions">
        <Button onClick={() => void onRefresh(Math.max(0, (sessions?.offset || 0) - pageSize))} disabled={!sessions || !sessions.offset}><ArrowLeft size={14} aria-hidden="true" />{t("上一页")}</Button>
        <Button onClick={() => void onRefresh((sessions?.offset || 0) + pageSize)} disabled={!sessions?.hasMore}>{t("下一页")}<ArrowRight size={14} aria-hidden="true" /></Button>
      </div>
    </div>
    {detail ? <ManagementDrawer title={detail.title || t("未命名会话")} onClose={() => setDetail(null)} footer={<><Button onClick={() => navigator.clipboard.writeText(detail.id)}><Copy size={14} />{t("复制会话 ID")}</Button><Button danger onClick={() => void deleteOne(detail)}><Trash2 size={14} />{t("删除会话")}</Button></>}><dl className="mg-detail"><dt>{t("会话 ID")}</dt><dd>{detail.id}</dd><dt>{t("项目目录")}</dt><dd>{detail.cwd || "—"}</dd><dt>{t("模型供应商")}</dt><dd>{detail.modelProvider || "Codex"}</dd><dt>{t("数据库")}</dt><dd>{detail.dbPath || "—"}</dd></dl></ManagementDrawer> : null}
  </div>;
}
