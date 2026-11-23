/**
 * 会话：Codex、Claude Code（含 Claude Desktop 里的）、Grok 的本地会话放在一起，可以复制恢复命令或删除。
 * 只读当前页签要看的 Agent，切到别的页签时才去读它的会话；读过的留在内存里，回来不用重读。
 * 删除前确认；Codex 会话走它自己的删除（带备份，可以撤销），Claude Code / Grok 的会话移进废纸篓，可以找回。
 * Claude Desktop 的会话由后端按 Desktop 的会话记录合并（标题、归档状态以 Desktop 为准），删除要求 Desktop 已退出。
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, TriangleAlert, Undo2 } from "lucide-react";
import { appConfirm } from "./app-dialog";
import { t, tf } from "./i18n";
import { agentName, shortPath, type AgentSessionView, type SessionAgent } from "./gateway-model";
import { AGENT_ICONS, AgentIcon, type Notify } from "./gateway-ui";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxFilters, CxLoading, CxSection, CxToolbar } from "./cx-layout";
import { matchesManagementQuery, normalizeProjectPath } from "./management-model";
import { SessionRow, useCopyFeedback } from "./ProjectSessionPage";
import { TopbarActions } from "./TopbarActions";

type LocalSession = { id: string; title: string; cwd: string; updatedAtMs: number; archived?: boolean };
type AgentList = { sessions: AgentSessionView[]; total: number };

const SESSION_AGENTS: SessionAgent[] = ["codex", "claude", "grok"];
/** 每个页签最多显示的会话数；Codex 的会话按页取（后端每页最多 100 条）。 */
const SHOWN_LIMIT = 300;
const CODEX_SESSION_PAGE = 100;
/** 每个项目先显示的会话数；搜索时全部显示。 */
const SESSION_GROUP_PREVIEW = 5;
const SEARCH_DEBOUNCE_MS = 300;

/** 读过的列表留到下次进页面；Codex 的列表和读它时用的搜索词一起记。 */
const memo: { lists: Partial<Record<SessionAgent, AgentList>>; codexQuery: string; agent: SessionAgent | "all" } = { lists: {}, codexQuery: "", agent: "all" };

/** 所有会话列表共用 matchesManagementQuery：每个词都要出现在标题、目录或 ID 里。 */
const matchesSession = (session: AgentSessionView, query: string) => matchesManagementQuery([session.title, session.cwd, session.id], query);

/** Codex 的会话：带上搜索词在后端全部会话里找，只取前 SHOWN_LIMIT 条，总数用后端给的。 */
async function loadCodexSessions(query: string): Promise<AgentList> {
  const sessions: AgentSessionView[] = [];
  let total = 0;
  for (let offset = 0; offset < SHOWN_LIMIT; offset += CODEX_SESSION_PAGE) {
    const page = await invoke<{ status: string; message: string; sessions?: LocalSession[]; hasMore?: boolean; totalCount?: number }>("list_local_sessions", { request: { offset, limit: CODEX_SESSION_PAGE, query } });
    if (page.status !== "ok" && !page.sessions?.length) throw new Error(page.message);
    total = page.totalCount ?? sessions.length;
    sessions.push(...(page.sessions ?? []).map((session): AgentSessionView => ({ agent: "codex", id: session.id, title: session.title, cwd: session.cwd, updatedAtMs: session.updatedAtMs, archived: session.archived })));
    if (!page.hasMore || !page.sessions?.length) break;
  }
  return { sessions, total: Math.max(total, sessions.length) };
}

async function loadAgentSessions(agent: SessionAgent, codexQuery: string): Promise<AgentList> {
  if (agent === "codex") return loadCodexSessions(codexQuery);
  const result = await invoke<{ sessions?: AgentSessionView[] }>("list_agent_sessions", { agent });
  const sessions = result.sessions ?? [];
  return { sessions, total: sessions.length };
}

/** 传入 `agent` 时由外层页签决定显示哪个 Agent 的会话，不再显示本组件自己的筛选条。 */
export function SessionsView({ notify, agent: fixedAgent }: { notify: Notify; agent?: SessionAgent | "all" }) {
  const [lists, setLists] = useState<Partial<Record<SessionAgent, AgentList>>>(memo.lists);
  const [ownAgent, setAgentState] = useState<SessionAgent | "all">(memo.agent);
  const agent = fixedAgent ?? ownAgent;
  const [query, setQuery] = useState(memo.codexQuery);
  const [codexQuery, setCodexQuery] = useState(memo.codexQuery);
  const [loading, setLoading] = useState<Set<SessionAgent>>(new Set());
  const [errors, setErrors] = useState<Partial<Record<SessionAgent, string>>>({});
  const setAgent = (next: SessionAgent | "all") => {
    memo.agent = next;
    setAgentState(next);
  };
  // 「全部」读所有 Agent 的会话。
  const wanted = useMemo(() => (agent === "all" ? [...SESSION_AGENTS] : [agent]), [agent]);
  const requests = useRef(new Map<SessionAgent, number>());

  const load = async (agents: SessionAgent[], searchQuery = codexQuery) => {
    if (!agents.length) return;
    setLoading((old) => new Set([...old, ...agents]));
    await Promise.all(agents.map(async (item) => {
      const request = (requests.current.get(item) ?? 0) + 1;
      requests.current.set(item, request);
      try {
        const list = await loadAgentSessions(item, searchQuery);
        if (requests.current.get(item) !== request) return;
        memo.lists = { ...memo.lists, [item]: list };
        if (item === "codex") memo.codexQuery = searchQuery;
        setLists(memo.lists);
        setErrors((old) => ({ ...old, [item]: undefined }));
      } catch (error) {
        if (requests.current.get(item) === request) setErrors((old) => ({ ...old, [item]: String(error) }));
      } finally {
        if (requests.current.get(item) === request) setLoading((old) => { const next = new Set(old); next.delete(item); return next; });
      }
    }));
  };
  // 进入页签时只读还没读过的 Agent。
  useEffect(() => {
    void load(wanted.filter((item) => !memo.lists[item] || (item === "codex" && memo.codexQuery !== codexQuery)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wanted]);
  // Codex 的会话可能有上千条，搜索词停下来后交给后端在全部会话里搜。
  useEffect(() => {
    if (codexQuery === memo.codexQuery || !wanted.includes("codex")) return;
    void load(["codex"], codexQuery);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [codexQuery]);
  const searchTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(searchTimer.current), []);
  const search = (value: string) => {
    setQuery(value);
    clearTimeout(searchTimer.current);
    searchTimer.current = setTimeout(() => setCodexQuery(value.trim()), SEARCH_DEBOUNCE_MS);
  };

  const matched = useMemo(() => wanted
    .flatMap((item) => lists[item]?.sessions ?? [])
    .filter((session) => matchesSession(session, query))
    .sort((left, right) => right.updatedAtMs - left.updatedAtMs), [wanted, lists, query]);
  const shown = useMemo(() => matched.slice(0, SHOWN_LIMIT), [matched]);
  // Codex 只读了前 SHOWN_LIMIT 条，它的总数用后端按同一搜索词数出来的。
  const total = wanted.reduce((sum, item) => {
    const list = lists[item];
    if (!list) return sum;
    const local = list.sessions.filter((session) => matchesSession(session, query)).length;
    return sum + (item === "codex" ? Math.max(list.total, local) : local);
  }, 0);
  const now = Date.now();
  const loaded = wanted.every((item) => lists[item] || errors[item]);
  const failed = wanted.filter((item) => errors[item]);

  const [deleting, setDeleting] = useState<string | null>(null);
  // 刚删掉的 Codex 会话：页面上给一个「撤销」，按删除结果里的 token 从备份恢复。
  const [undo, setUndo] = useState<{ title: string; token: string } | null>(null);
  const forget = (key: string) => {
    for (const item of SESSION_AGENTS) {
      const list = memo.lists[item];
      if (!list) continue;
      const sessions = list.sessions.filter((session) => `${session.agent}:${session.id}` !== key);
      if (sessions.length !== list.sessions.length) memo.lists = { ...memo.lists, [item]: { sessions, total: Math.max(0, list.total - 1) } };
    }
    setLists(memo.lists);
  };
  const remove = async (session: AgentSessionView) => {
    const title = session.title || t("（没有标题）");
    const confirmed = await appConfirm(
      session.agent === "codex"
        ? tf("删除 Codex 会话「{0}」？删除前会自动备份。", [title])
        : session.surface === "desktop"
          ? tf("从 Claude Desktop 删除会话「{0}」？会话记录和对话文件会移到废纸篓。需要先退出 Claude Desktop。", [title])
          : tf("删除 {0} 会话「{1}」？会话文件会移到废纸篓，需要时可以找回。", [agentName(session.agent), title]),
      { title: t("删除会话"), confirmText: t("删除"), danger: true },
    );
    if (!confirmed) return;
    const key = `${session.agent}:${session.id}`;
    setDeleting(key);
    try {
      const result = session.agent === "codex"
        ? await invoke<{ status: string; message: string; undo_token?: string | null }>("delete_local_session", { request: { sessionId: session.id, title: session.title } })
        : await invoke<{ status: string; message: string; undo_token?: string | null }>("delete_agent_session", { agent: session.agent, id: session.id });
      // Codex 的删除结果里还有一层自己的 status（server_deleted / local_deleted），摊平后会盖掉外层的 ok。
      const done = sessionDeleteSucceeded(result.status);
      await notify(done ? t("会话已删除") : t("删除失败"), result.message, done ? "ok" : "failed");
      if (done) {
        forget(key);
        setUndo(result.undo_token ? { title, token: result.undo_token } : null);
      }
    } catch (error) {
      await notify(t("删除失败"), String(error), "failed");
    } finally {
      setDeleting(null);
    }
  };
  const undoDelete = async () => {
    if (!undo) return;
    const current = undo;
    setUndo(null);
    try {
      const result = await invoke<{ status: string; message: string }>("undo_delete_local_sessions", { token: current.token });
      await notify(t("撤销删除"), result.message, result.status === "undone" ? "ok" : "failed");
      await load(["codex"]);
    } catch (error) {
      await notify(t("撤销删除"), String(error), "failed");
    }
  };
  const { copied, failed: copyFailed, copy } = useCopyFeedback();
  const copyCommand = async (session: AgentSessionView) => {
    const id = session.resumeId ?? session.id;
    const command = session.agent === "codex" ? `codex resume ${id}` : `${session.agent} --resume ${id}`;
    const text = session.cwd ? `cd ${JSON.stringify(session.cwd)} && ${command}` : command;
    if (!(await copy(`${session.agent}:${session.id}`, text))) await notify(t("复制失败"), t("剪贴板不可用"), "failed");
  };

  const filters: Array<{ id: SessionAgent | "all"; label: string; count?: number }> = [
    { id: "all", label: t("全部"), count: SESSION_AGENTS.every((item) => lists[item]) ? SESSION_AGENTS.reduce((sum, item) => sum + (lists[item]?.total ?? 0), 0) : undefined },
    { id: "codex", label: "Codex", count: lists.codex?.total },
    { id: "claude", label: "Claude Code", count: lists.claude?.total },
    { id: "grok", label: "Grok", count: lists.grok?.total },
  ];
  // 按项目分节（最近用过的项目在前）；每节先露几条，其余点「再显示」。
  const [openGroups, setOpenGroups] = useState<Set<string>>(new Set());
  const groups = useMemo(() => {
    const map = new Map<string, { key: string; label: string; path: string; sessions: AgentSessionView[] }>();
    for (const session of shown) {
      const path = normalizeProjectPath(session.cwd);
      if (!map.has(path)) map.set(path, { key: path, label: path.split("/").at(-1) || t("未知目录"), path, sessions: [] });
      map.get(path)!.sessions.push(session);
    }
    return [...map.values()];
  }, [shown]);
  const preview = query.trim() ? Infinity : SESSION_GROUP_PREVIEW;
  const busy = wanted.some((item) => loading.has(item));
  const refresh = () => void load(wanted);

  return (
    <div className="cx-page cx-sessions">
      <TopbarActions>
        <button className="cx-secondary" disabled={busy} onClick={refresh} type="button">
          <RefreshCw className={busy ? "animate-spin" : ""} />
          {t("刷新")}
        </button>
      </TopbarActions>
      <CxToolbar onQuery={search} placeholder={t("搜索标题或目录")} query={query} />
      {fixedAgent ? null : (
        <CxFilters
          items={filters.map((item) => ({ id: item.id, label: item.label, count: item.count, icon: item.id !== "all" ? <img alt="" className="gw-tab-icon" src={AGENT_ICONS[item.id === "codex" ? "codexApp" : item.id]} /> : undefined }))}
          label={t("按 Agent 查看会话")}
          onChange={setAgent}
          value={agent}
        />
      )}
      {failed.length ? (
        <CxBanner tone="warn" icon={<TriangleAlert />} title={t("部分会话读取失败")} detail={failed.map((item) => `${agentName(item)}: ${errors[item]}`).join("\n")}>
          <button className="cx-secondary" disabled={busy} onClick={() => void load(failed)} type="button">{t("重试")}</button>
        </CxBanner>
      ) : null}
      {undo ? (
        <CxBanner tone="ok" icon={<Undo2 />} title={tf("已删除「{0}」", [undo.title])} detail={t("删除前已备份，可以撤销。")}>
          <button className="cx-secondary" onClick={() => void undoDelete()} type="button">{t("撤销")}</button>
        </CxBanner>
      ) : null}
      {total > shown.length ? <p className="cx-session-limit" role="status">{tf("显示前 {0} 个，共 {1} 个；搜索可以找到其余会话。", [shown.length, total])}</p> : null}
      {groups.map((group) => {
        const expanded = openGroups.has(group.key);
        const visible = expanded ? group.sessions : group.sessions.slice(0, preview);
        const hidden = group.sessions.length - visible.length;
        return (
          <CxSection aside={group.path ? <span className="cx-session-path" title={group.path}>{shortPath(group.path)}</span> : null} count={group.sessions.length} key={group.key} title={group.label}>
            <CxCard>
              {visible.map((session) => {
                const key = `${session.agent}:${session.id}`;
                const title = session.title || t("（没有标题）");
                return (
                  <SessionRow
                    key={key}
                    icon={<AgentIcon agent={session.agent} className="gw-session-agent" />}
                    title={title}
                    badges={session.surface === "desktop" || session.archived ? (
                      <>
                        {session.surface === "desktop" ? <CxBadge title={t("Claude Desktop 里的会话")}>Desktop</CxBadge> : null}
                        {session.archived ? <CxBadge>{t("已归档")}</CxBadge> : null}
                      </>
                    ) : undefined}
                    tooltip={`${agentName(session.agent)} · ${session.cwd || t("（没有目录）")}`}
                    updatedAtMs={session.updatedAtMs}
                    now={now}
                    copied={copied === key}
                    copyFailed={copyFailed === key}
                    onCopy={() => void copyCommand(session)}
                    deleting={deleting === key}
                    deleteDisabled={deleting !== null}
                    onDelete={() => void remove(session)}
                  />
                );
              })}
              {hidden > 0 ? (
                <button className="cx-session-more" onClick={() => setOpenGroups((old) => new Set(old).add(group.key))} type="button">
                  {tf("再显示 {0} 个", [hidden])}
                </button>
              ) : null}
            </CxCard>
          </CxSection>
        );
      })}
      {!shown.length ? (
        <CxCard>
          {!loaded ? (
            <CxLoading />
          ) : query.trim() ? (
            <CxEmpty title={t("没有匹配的会话")}><button className="cx-secondary" onClick={() => search("")} type="button">{t("清除搜索")}</button></CxEmpty>
          ) : failed.length === wanted.length ? (
            <CxEmpty title={t("会话读取失败")}><button className="cx-secondary" disabled={busy} onClick={refresh} type="button">{t("重试")}</button></CxEmpty>
          ) : (
            <CxEmpty title={t("还没有会话")} detail={t("在 Agent 里开始对话后，会话会出现在这里。")} />
          )}
        </CxCard>
      ) : null}
    </div>
  );
}

/** 删除会话是否成功：外层 ok，或 Codex 删除结果摊平后留下的 server_deleted / local_deleted。 */
export function sessionDeleteSucceeded(status: string | undefined): boolean {
  return status === "ok" || status === "server_deleted" || status === "local_deleted";
}
