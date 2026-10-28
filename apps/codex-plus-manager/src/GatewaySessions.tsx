/**
 * 会话：Codex、Claude Code、Grok 的本地会话放在一起，可以复制恢复命令或删除。
 * 删除前确认；Codex 会话走它自己的删除（带备份），Claude Code / Grok 的会话移进废纸篓，可以找回。
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Copy, Loader2, RefreshCw, Search, Trash2 } from "lucide-react";
import { appConfirm } from "./app-dialog";
import { t, tf } from "./i18n";
import { agentName, filterSessions, relativeTime, shortPath, type AgentSessionView, type SessionAgent } from "./gateway-model";
import { AGENT_ICONS, AgentIcon, type Notify } from "./gateway-ui";

const memo: { sessions: AgentSessionView[] | null; agent: SessionAgent | "all" } = { sessions: null, agent: "all" };

type LocalSession = { id: string; title: string; cwd: string; updatedAtMs: number };

/** Codex 的会话按页取（后端每页最多 100 条），最多取 CODEX_SESSION_LIMIT 条。 */
const CODEX_SESSION_PAGE = 100;
const CODEX_SESSION_LIMIT = 1000;

async function loadCodexSessions(): Promise<LocalSession[]> {
  const all: LocalSession[] = [];
  for (let offset = 0; offset < CODEX_SESSION_LIMIT; offset += CODEX_SESSION_PAGE) {
    const page = await invoke<{ sessions?: LocalSession[]; hasMore?: boolean }>("list_local_sessions", { request: { offset, limit: CODEX_SESSION_PAGE } });
    all.push(...(page.sessions ?? []));
    if (!page.hasMore || !(page.sessions ?? []).length) break;
  }
  return all;
}

async function loadAll(): Promise<{ sessions: AgentSessionView[]; errors: string[] }> {
  const errors: string[] = [];
  const codex = loadCodexSessions()
    .then((sessions) => sessions.map((session): AgentSessionView => ({ agent: "codex", id: session.id, title: session.title, cwd: session.cwd, updatedAtMs: session.updatedAtMs })))
    .catch((error) => {
      errors.push(String(error));
      return [] as AgentSessionView[];
    });
  const other = (agent: "claude" | "grok") =>
    invoke<{ sessions?: AgentSessionView[] }>("list_agent_sessions", { agent })
      .then((result) => result.sessions ?? [])
      .catch((error) => {
        errors.push(String(error));
        return [] as AgentSessionView[];
      });
  const [codexSessions, claudeSessions, grokSessions] = await Promise.all([codex, other("claude"), other("grok")]);
  return { sessions: [...codexSessions, ...claudeSessions, ...grokSessions], errors };
}

/** 传入 `agent` 时由外层页签决定显示哪个 Agent 的会话，不再显示本组件自己的筛选条。 */
export function SessionsView({ notify, agent: fixedAgent }: { notify: Notify; agent?: SessionAgent | "all" }) {
  const [sessions, setSessions] = useState<AgentSessionView[] | null>(memo.sessions);
  const [ownAgent, setAgentState] = useState<SessionAgent | "all">(memo.agent);
  const agent = fixedAgent ?? ownAgent;
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const setAgent = (next: SessionAgent | "all") => {
    memo.agent = next;
    setAgentState(next);
  };
  const refresh = async () => {
    setLoading(true);
    const result = await loadAll();
    memo.sessions = result.sessions;
    setSessions(result.sessions);
    setLoading(false);
    if (result.errors.length) await notify(t("部分会话读取失败"), result.errors.join("\n"), "warning");
  };
  useEffect(() => {
    void refresh();
    // 进入时读一次，之后手动刷新。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const visible = useMemo(() => filterSessions(sessions ?? [], agent, query), [sessions, agent, query]);
  const counts = useMemo(() => {
    const all = sessions ?? [];
    return { all: all.length, codex: all.filter((s) => s.agent === "codex").length, claude: all.filter((s) => s.agent === "claude").length, grok: all.filter((s) => s.agent === "grok").length };
  }, [sessions]);
  const now = Date.now();

  const [deleting, setDeleting] = useState<string | null>(null);
  const remove = async (session: AgentSessionView) => {
    const title = session.title || t("（没有标题）");
    const confirmed = await appConfirm(
      session.agent === "codex"
        ? tf("删除 Codex 会话「{0}」？删除前会自动备份。", [title])
        : tf("删除 {0} 会话「{1}」？会话文件会移到废纸篓，需要时可以找回。", [agentName(session.agent), title]),
      { title: t("删除会话"), confirmText: t("删除"), danger: true },
    );
    if (!confirmed) return;
    const key = `${session.agent}:${session.id}`;
    setDeleting(key);
    try {
      const result = session.agent === "codex"
        ? await invoke<{ status: string; message: string }>("delete_local_session", { request: { sessionId: session.id, title: session.title } })
        : await invoke<{ status: string; message: string }>("delete_agent_session", { agent: session.agent, id: session.id });
      // Codex 的删除结果里还有一层自己的 status（server_deleted / local_deleted），摊平后会盖掉外层的 ok。
      const done = sessionDeleteSucceeded(result.status);
      await notify(done ? t("会话已删除") : t("删除失败"), result.message, done ? "ok" : "failed");
      if (done) {
        const next = (memo.sessions ?? sessions ?? []).filter((item) => `${item.agent}:${item.id}` !== key);
        memo.sessions = next;
        setSessions(next);
      }
    } catch (error) {
      await notify(t("删除失败"), String(error), "failed");
    } finally {
      setDeleting(null);
    }
  };
  const copyCommand = async (session: AgentSessionView) => {
    const command = session.agent === "codex" ? `codex resume ${session.id}` : `${session.agent} --resume ${session.id}`;
    const text = session.cwd ? `cd ${JSON.stringify(session.cwd)} && ${command}` : command;
    try {
      await navigator.clipboard.writeText(text);
      await notify(t("已复制"), text, "ok");
    } catch (error) {
      await notify(t("复制失败"), String(error), "failed");
    }
  };

  const filters: Array<{ id: SessionAgent | "all"; label: string; count: number }> = [
    { id: "all", label: t("全部"), count: counts.all },
    { id: "codex", label: "Codex", count: counts.codex },
    { id: "claude", label: "Claude Code", count: counts.claude },
    { id: "grok", label: "Grok", count: counts.grok },
  ];

  return (
    <>
      <div className="gw-usage-head">
        {fixedAgent ? <span /> : (
          <nav className="gw-tabs gw-range-tabs" role="tablist">
            {filters.map((item) => (
              <button aria-selected={agent === item.id} data-active={agent === item.id} key={item.id} onClick={() => setAgent(item.id)} role="tab" type="button">
                {item.id !== "all" ? <img alt="" className="gw-tab-icon" src={AGENT_ICONS[item.id === "codex" ? "codexApp" : item.id]} /> : null}
                {item.label}
                <em className="gw-tab-count">{item.count}</em>
              </button>
            ))}
          </nav>
        )}
        <label className="gw-sheet-search gw-session-search">
          <Search className="h-4 w-4" />
          <input onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("搜索标题或目录")} value={query} />
        </label>
        <button aria-label={t("刷新")} className="gw-icon-button" disabled={loading} onClick={() => void refresh()} title={t("刷新")} type="button">
          <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
        </button>
      </div>
      <div className="gw-card">
        {visible.slice(0, 300).map((session) => {
          const when = relativeTime(session.updatedAtMs, now);
          return (
            <div className="gw-session-row" key={`${session.agent}:${session.id}`}>
              <span className="gw-session-agent-wrap" title={agentName(session.agent)}><AgentIcon agent={session.agent} className="gw-session-agent" /></span>
              <div className="gw-session-main">
                <strong title={session.title}>{session.title || t("（没有标题）")}</strong>
                <small title={session.cwd}>{session.cwd ? shortPath(session.cwd) : t("（没有目录）")}</small>
              </div>
              <span className="gw-session-time">{when.args.length ? tf(when.text, when.args) : t(when.text)}</span>
              <button className="gw-icon-button" onClick={() => void copyCommand(session)} title={t("复制恢复命令")} type="button">
                <Copy className="h-4 w-4" />
              </button>
              <button
                aria-label={t("删除会话")}
                className="gw-icon-button gw-session-delete"
                disabled={deleting !== null}
                onClick={() => void remove(session)}
                title={t("删除会话")}
                type="button"
              >
                {deleting === `${session.agent}:${session.id}` ? <Loader2 className="h-4 w-4 animate-spin" /> : <Trash2 className="h-4 w-4" />}
              </button>
            </div>
          );
        })}
        {sessions && visible.length === 0 ? <div className="gw-empty">{query ? t("没有匹配的会话") : t("还没有会话")}</div> : null}
        {!sessions ? <div className="gw-empty">{t("正在读取…")}</div> : null}
      </div>
    </>
  );
}

/** 删除会话是否成功：外层 ok，或 Codex 删除结果摊平后留下的 server_deleted / local_deleted。 */
export function sessionDeleteSucceeded(status: string | undefined): boolean {
  return status === "ok" || status === "server_deleted" || status === "local_deleted";
}
