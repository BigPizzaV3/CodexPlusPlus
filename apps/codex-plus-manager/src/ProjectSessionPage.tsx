import { useEffect, useRef, useState, type ReactNode } from "react";
import { Archive, ArchiveRestore, ArrowLeft, ArrowRight, Check, ChevronDown, Copy, FolderInput, FolderSymlink, Loader2, RefreshCw, Trash2 } from "lucide-react";
import { t, tf } from "./i18n";
import { matchesManagementQuery, sessionArchiveFilter, sessionGroups, type LocalSessionFilter, type ManagedSession, type SessionArchiveView, type SessionProjectCount } from "./management-model";
import { relativeTime } from "./gateway-model";
import { ManagementDrawer, PageButton as Button } from "./ManagementPages";
import { SessionRelocateDrawer, type SessionRelocationRequest } from "./SessionRelocateDrawer";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxRow, CxToolbar } from "./cx-layout";
import { TopbarActions } from "./TopbarActions";
import "./ManagementPages.css";

/**
 * 会话的 `model_provider`（Codex 配置里的供应商键）换成看得懂的名字：
 * `openai` 是官方会话身份（经本地服务时也是它），`custom` 是直连的自定义供应商，其余照原样显示。
 * 原始键放在悬停提示里。
 */
export function sessionProviderLabel(provider: string): string {
  switch (provider.trim()) {
    case "":
    case "openai": return t("OpenAI 会话");
    case "custom": return t("自定义供应商");
    case "codexplus":
    case "codex_plus_gateway": return t("Codex++ 本地服务");
    default: return provider;
  }
}

type SessionPageData = {
  sessions: ManagedSession[]; totalCount: number; hasMore: boolean; offset: number; limit: number;
  projects?: SessionProjectCount[]; query?: string; archived?: boolean | null; status?: string; message?: string;
};
/** 停止输入多久后才去后端搜索。 */
const SEARCH_DEBOUNCE_MS = 300;


/** 复制后按钮短暂变成对勾；复制失败时记下是哪一个，由调用方显示提示。 */
export function useCopyFeedback() {
  const [copied, setCopied] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const copy = async (key: string, text: string) => {
    try {
      if (!navigator.clipboard) throw new Error(t("剪贴板不可用"));
      await navigator.clipboard.writeText(text);
      setFailed(null);
      setCopied(key);
      setTimeout(() => setCopied((current) => (current === key ? null : current)), 1500);
      return true;
    } catch {
      setCopied(null);
      setFailed(key);
      setTimeout(() => setFailed((current) => (current === key ? null : current)), 2500);
      return false;
    }
  };
  return { copied, failed, copy };
}

/** 各 Agent 会话列表共用的一行：标题、相对时间（悬停看完整时间）、复制恢复命令、删除。 */
export function SessionRow({ title, updatedAtMs, now, icon, badges, detail, tooltip, selected, onOpen, copied, copyFailed, onCopy, onDelete, deleting, deleteDisabled }: {
  title: string; updatedAtMs: number | null; now: number; icon?: ReactNode; badges?: ReactNode; detail?: ReactNode; tooltip?: string;
  selected?: boolean; onOpen?: () => void; copied: boolean; copyFailed?: boolean; onCopy: () => void; onDelete: () => void; deleting?: boolean; deleteDisabled?: boolean;
}) {
  const when = updatedAtMs ? relativeTime(updatedAtMs, now) : null;
  return <CxRow className="cx-session-row" icon={icon} title={title} badges={badges} detail={detail} tooltip={tooltip} selected={selected} onClick={onOpen}>
    <time className="cx-session-time" title={updatedAtMs ? new Date(updatedAtMs).toLocaleString() : undefined}>{when ? (when.args.length ? tf(when.text, when.args) : t(when.text)) : t("未知时间")}</time>
    <button type="button" className="mg-icon-button" onClick={onCopy} aria-label={tf("复制恢复命令：{0}", [title])} title={copyFailed ? t("复制失败") : t("复制恢复命令")}>{copied ? <Check size={15} aria-hidden="true" /> : <Copy size={15} aria-hidden="true" />}</button>
    <button type="button" className="mg-icon-button cx-session-delete" disabled={deleteDisabled} onClick={onDelete} aria-label={tf("删除会话：{0}", [title])} title={t("删除会话")}>{deleting ? <Loader2 className="animate-spin" size={15} aria-hidden="true" /> : <Trash2 size={15} aria-hidden="true" />}</button>
  </CxRow>;
}

export function ProjectSessionPage({
  sessions,
  onRefresh,
  onDelete,
  onDeleteMany,
  onArchiveMany,
  onListProjectSessions,
  onPickFolder,
  onRelocated,
  repairPanel,
}: {
  sessions: SessionPageData | null;
  // 不传 filter 时沿用上次的搜索词和归档筛选。
  onRefresh: (offset?: number, filter?: LocalSessionFilter) => Promise<unknown>;
  onDelete: (session: ManagedSession) => Promise<boolean | void>;
  // 返回 false 表示用户取消或没有执行，此时保留当前选择。
  onDeleteMany: (sessions: ManagedSession[]) => Promise<boolean | void>;
  onArchiveMany: (sessions: ManagedSession[], archived?: boolean) => Promise<boolean | void>;
  onListProjectSessions: (cwd: string) => Promise<ManagedSession[]>;
  // 让用户选一个文件夹，取消时返回 null。
  onPickFolder: (title: string) => Promise<string | null>;
  // 会话目录改完（或没改成）之后：提示结果并刷新列表。
  onRelocated: (message: string, ok: boolean) => Promise<void>;
  repairPanel?: ReactNode;
}) {
  // 搜索词和归档筛选在后端全部会话里生效；离开页面再回来沿用上次的条件。
  const [query, setQuery] = useState(() => sessions?.query ?? "");
  const [archiveView, setArchiveView] = useState<SessionArchiveView>(() => (sessions?.archived === true ? "archived" : sessions?.archived === null ? "all" : "active"));
  const [mode, setMode] = useState<"project" | "time">("project");
  const [expanded, setExpanded] = useState<Map<string, boolean>>(new Map());
  const [selection, setSelection] = useState(false);
  const [selected, setSelected] = useState<Map<string, ManagedSession>>(new Map());
  const [detail, setDetail] = useState<ManagedSession | null>(null);
  const [projectLoading, setProjectLoading] = useState<string | null>(null);
  const searchTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(searchTimer.current), []);
  const all = sessions?.sessions || [];
  // 后端结果到来之前，先在当前页里按新输入的词筛一遍，输入时列表立即跟着变。
  const visible = all.filter((session) => matchesManagementQuery([session.title, session.cwd, session.id, session.modelProvider], query));
  const groups = sessionGroups(visible, mode, Date.now(), sessions?.projects);
  const now = Date.now();
  const selectedRows = [...selected.values()];
  const filterFor = (nextQuery: string, view: SessionArchiveView): LocalSessionFilter => ({ query: nextQuery.trim(), archived: sessionArchiveFilter(view) });
  const search = (value: string) => {
    setQuery(value);
    setExpanded(new Map());
    clearTimeout(searchTimer.current);
    searchTimer.current = setTimeout(() => void onRefresh(0, filterFor(value, archiveView)), SEARCH_DEBOUNCE_MS);
  };
  const showArchive = (view: SessionArchiveView) => {
    if (view === archiveView) return;
    setArchiveView(view);
    clearTimeout(searchTimer.current);
    void onRefresh(0, filterFor(query, view));
  };
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
      // 整项目选择也遵守当前的归档筛选，免得把看不见的会话一起删掉。
      const archived = sessionArchiveFilter(archiveView);
      setSelected((old) => {
        const next = new Map(old);
        projectSessions.filter((session) => archived === null || session.archived === archived).forEach((session) => next.set(session.id, session));
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
  const archiveSelected = async (archived: boolean) => {
    if (!selectedRows.length) return;
    if ((await onArchiveMany(selectedRows, archived)) !== false) setSelected(new Map());
  };
  // 会话目录重定位（#1085）：先选文件夹，再在抽屉里预览、确认。
  const [relocation, setRelocation] = useState<SessionRelocationRequest | null>(null);
  const moveSelected = async () => {
    if (!selectedRows.length) return;
    const to = await onPickFolder(t("选择要把会话移到的文件夹"));
    if (to) setRelocation({ kind: "move", threadIds: selectedRows.map((session) => session.id), to });
  };
  const relocateProject = async (from: string) => {
    const to = await onPickFolder(t("选择这个项目现在所在的文件夹"));
    if (to) setRelocation({ kind: "remap", from, to });
  };
  const relocated = async (message: string, ok: boolean) => {
    setRelocation(null);
    if (ok) setSelected(new Map());
    await onRelocated(message, ok);
  };
  // 选择可以跨分页（整项目选择），不能按当前页修剪；单删成功的会话要移出选择，
  // 否则随后的批量操作会把已删除的会话再提交一次并报失败。删掉的正是详情里的会话时关掉详情。
  const deleteOne = async (session: ManagedSession) => {
    if ((await onDelete(session)) === true) {
      setDetail((current) => (current?.id === session.id ? null : current));
      setSelected((old) => {
        if (!old.has(session.id)) return old;
        const next = new Map(old);
        next.delete(session.id);
        return next;
      });
    }
  };
  const { copied, failed: copyFailed, copy } = useCopyFeedback();
  // 复制恢复命令（和其他 Agent 的会话列表一致），按钮短暂变成对勾作为反馈。
  const copyResume = (session: ManagedSession) => {
    const command = `codex resume ${session.id}`;
    void copy(session.id, session.cwd ? `cd ${JSON.stringify(session.cwd)} && ${command}` : command);
  };
  const pageSize = sessions?.limit || 50;
  const page = sessions ? Math.floor(sessions.offset / pageSize) + 1 : 0;
  const searching = Boolean(query.trim());
  const empty = !sessions
    ? <CxEmpty title={t("正在读取会话…")}><Button className="cx-secondary" onClick={() => void onRefresh()}>{t("刷新")}</Button></CxEmpty>
    : searching
      ? <CxEmpty title={t("没有匹配的会话")} detail={t("已在全部会话里搜索标题、目录和会话 ID。")}><Button className="cx-secondary" onClick={() => search("")}>{t("清除搜索")}</Button></CxEmpty>
      : archiveView === "archived"
        ? <CxEmpty title={t("没有已归档的会话")}><Button className="cx-secondary" onClick={() => showArchive("active")}>{t("查看未归档")}</Button></CxEmpty>
        : <CxEmpty title={t("还没有会话")} detail={t("在 Codex 里开始对话后，会话会出现在这里。")}><Button className="cx-secondary" onClick={() => void onRefresh()}>{t("刷新")}</Button></CxEmpty>;
  // 参照 Cursor 的列表：工具条（搜索 + 视图 + 选择）→ 按项目分节、每个会话一行 → 分页；修复与维护收成底部一条折叠区。
  return <div className="mg-page cx-page mg-session-page">
    <TopbarActions>
      <Button className="cx-secondary" onClick={() => void onRefresh()} title={t("刷新列表")} aria-label={t("刷新列表")}><RefreshCw size={14} aria-hidden="true" />{t("刷新")}</Button>
    </TopbarActions>
    <CxToolbar query={query} onQuery={search} placeholder={t("搜索标题、目录或会话 ID")}>
      <div className="cx-session-mode" role="group" aria-label={t("归档状态")}>
        {([["active", t("未归档")], ["archived", t("已归档")], ["all", t("全部")]] as Array<[SessionArchiveView, string]>).map(([view, label]) =>
          <button type="button" key={view} data-active={archiveView === view} aria-pressed={archiveView === view} onClick={() => showArchive(view)}>{label}</button>)}
      </div>
      <div className="cx-session-mode" role="group" aria-label={t("会话视图")}>
        <button type="button" data-active={mode === "project"} aria-pressed={mode === "project"} onClick={() => setMode("project")}>{t("按项目")}</button>
        <button type="button" data-active={mode === "time"} aria-pressed={mode === "time"} onClick={() => setMode("time")}>{t("按时间")}</button>
      </div>
      <Button className={selection ? "cx-primary" : "cx-secondary"} aria-pressed={selection} onClick={() => { setSelection((old) => !old); setSelected(new Map()); }}>{selection ? t("完成") : t("选择多个…")}</Button>
    </CxToolbar>
    {selection ? <div className="cx-session-selection">
      <span className="mg-session-selected-count" role="status" aria-live="polite">{tf("已选择 {0}", [selectedRows.length])}</span>
      <Button className="cx-secondary" onClick={selectCurrentPage}>{t("全选当前列表")}</Button>
      <Button className="cx-secondary" disabled={!selectedRows.length} onClick={() => setSelected(new Map())}>{t("取消全部选择")}</Button>
      <span className="cx-session-selection-spacer" />
      <Button className="cx-secondary" disabled={!selectedRows.length} onClick={() => void moveSelected()} title={t("把选中会话的工作目录改到另一个文件夹；会先列出受影响的会话，确认后才执行。")}><FolderInput size={14} aria-hidden="true" />{t("移动已选…")}</Button>
      {archiveView !== "archived" ? <Button className="cx-secondary" disabled={!selectedRows.length} onClick={() => void archiveSelected(true)}><Archive size={14} aria-hidden="true" />{t("归档已选")}</Button> : null}
      {archiveView !== "active" ? <Button className="cx-secondary" disabled={!selectedRows.length} onClick={() => void archiveSelected(false)}><ArchiveRestore size={14} aria-hidden="true" />{t("取消归档已选")}</Button> : null}
      <Button className="cx-secondary" danger disabled={!selectedRows.length} onClick={() => void deleteSelected()} title={t("删除会创建本地备份；如果 Codex App 正在使用该会话，建议先关闭对应会话窗口再操作。")}><Trash2 size={14} aria-hidden="true" />{tf("删除已选 ({0})", [selectedRows.length])}</Button>
    </div> : null}
    {sessions?.status && sessions.status !== "ok" && sessions.message ? <CxBanner tone="warn" title={t("部分会话读取失败")} detail={t(sessions.message)} /> : null}
    {!visible.length ? <CxCard>{empty}</CxCard> : groups.map((group, index) => {
      // 默认展开；手动折叠过的保持折叠。
      const open = expanded.get(group.key) ?? true;
      const groupId = `session-group-${mode}-${index}`;
      const label = group.translatable ? t(group.label) : group.label;
      // 分页只截当前页；整个项目在筛选结果里有更多会话时写成「本页 / 共」。
      const partial = group.total !== undefined && group.total > group.sessions.length;
      return <section className="cx-section cx-session-group" key={group.key}>
        <header className="cx-session-group-head" data-mode={mode}>
          <button type="button" className="cx-session-group-toggle" aria-expanded={open} aria-controls={groupId} onClick={() => setExpanded((old) => new Map(old).set(group.key, !open))}>
            <ChevronDown size={14} aria-hidden="true" className={open ? "" : "mg-chevron-collapsed"} />
            <strong title={label}>{label}</strong>
            <span className="cx-section-count" title={partial ? tf("本页 {0} 个，这个项目共 {1} 个", [group.sessions.length, group.total!]) : undefined}>{partial ? `${group.sessions.length} / ${group.total}` : group.sessions.length}</span>
            {group.path ? <small title={group.path}>{group.path}</small> : null}
          </button>
          {mode === "project" && group.path ? <button type="button" className="mg-icon-button cx-session-relocate" onClick={() => void relocateProject(group.path)} aria-label={tf("重新定位目录：{0}", [group.path])} title={t("重新定位目录：项目文件夹挪了位置时，把这个目录下的全部会话改到新的文件夹")}><FolderSymlink size={15} aria-hidden="true" /></button> : null}
          {selection && mode === "project" && group.path ? <Button className="cx-secondary" disabled={projectLoading !== null} onClick={() => void selectProject(group.path)}>{projectLoading === group.path ? t("读取中…") : t("全选项目")}</Button> : null}
        </header>
        <div id={groupId} hidden={!open}>
          {open ? <CxCard>{group.sessions.map((session) => <SessionRow key={session.id}
            icon={selection ? <input type="checkbox" aria-label={tf("选择会话：{0}", [session.title || session.id])} checked={selected.has(session.id)} onChange={() => toggle(session)} /> : undefined}
            title={session.title || t("未命名会话")}
            badges={session.archived ? <CxBadge>{t("已归档")}</CxBadge> : undefined}
            detail={<span title={mode === "time" ? session.cwd : `model_provider = ${session.modelProvider || "openai"}`}>{mode === "time" ? session.cwd || t("未记录项目路径") : sessionProviderLabel(session.modelProvider)}</span>}
            updatedAtMs={session.updatedAtMs}
            now={now}
            selected={selection && selected.has(session.id)}
            onOpen={selection ? undefined : () => setDetail(session)}
            copied={copied === session.id}
            copyFailed={copyFailed === session.id}
            onCopy={() => copyResume(session)}
            onDelete={() => void deleteOne(session)} />)}</CxCard> : null}
        </div>
      </section>;
    })}
    <div className="cx-session-pagination">
      <span>{sessions ? tf("第 {0} 页 · 共 {1} 个会话", [page, sessions.totalCount]) : t("正在读取…")}</span>
      <Button className="cx-secondary" onClick={() => void onRefresh(Math.max(0, (sessions?.offset || 0) - pageSize))} disabled={!sessions || !sessions.offset}><ArrowLeft size={14} aria-hidden="true" />{t("上一页")}</Button>
      <Button className="cx-secondary" onClick={() => void onRefresh((sessions?.offset || 0) + pageSize)} disabled={!sessions?.hasMore}>{t("下一页")}<ArrowRight size={14} aria-hidden="true" /></Button>
    </div>
    {repairPanel}
    {relocation ? <SessionRelocateDrawer request={relocation} onClose={() => setRelocation(null)} onDone={(message, ok) => void relocated(message, ok)} /> : null}
    {detail ? <ManagementDrawer title={detail.title || t("未命名会话")} onClose={() => setDetail(null)} footer={<>
      <Button className="mg-button" onClick={() => void copy(`id:${detail.id}`, detail.id)}>{copied === `id:${detail.id}` ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}{copied === `id:${detail.id}` ? t("已复制") : copyFailed === `id:${detail.id}` ? t("复制失败") : t("复制会话 ID")}</Button>
      <Button className="mg-button" danger onClick={() => void deleteOne(detail)}><Trash2 size={14} aria-hidden="true" />{t("删除会话")}</Button>
    </>}><dl className="mg-detail"><dt>{t("会话 ID")}</dt><dd>{detail.id}</dd><dt>{t("项目目录")}</dt><dd>{detail.cwd || "—"}</dd><dt>{t("模型供应商")}</dt><dd title={`model_provider = ${detail.modelProvider || "openai"}`}>{sessionProviderLabel(detail.modelProvider)}</dd><dt>{t("状态")}</dt><dd>{detail.archived ? t("已归档") : t("未归档")}</dd><dt>{t("数据库")}</dt><dd>{detail.dbPath || "—"}</dd></dl></ManagementDrawer> : null}
  </div>;
}
