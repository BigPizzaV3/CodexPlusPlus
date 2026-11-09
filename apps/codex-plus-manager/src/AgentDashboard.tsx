/**
 * Agent 概览仪表板（Codex、Claude Code、Grok 共用）：一条状态行（版本 + 接入状态 + 切换）、CxStats 一排数字、
 * 「最近会话」「最近请求」两节列表、底部配置摘要，Codex 另在下方放健康检查（children）。样式在 cx-pages-agents.css。
 * Claude Code / Grok 的状态、操作和今日数据由这里按网关数据算；Codex 的由概览页传入（供应商切换、24 小时请求统计）。
 * 数据都来自现成接口，这里只读不写；跳到会话、请求日志时带上 Agent，落在它自己的页签和过滤下。
 */
import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, CheckCircle2, Loader2, Play, XCircle } from "lucide-react";
import { t, tf } from "./i18n";
import { agentAccessState, agentUsageStats, recentAgentRequests, type DashboardRequest } from "./agent-dashboard-model";
import { AgentGlyph, useGatewayPayload } from "./HubOverview";
import { formatElapsed, formatTokens, groupDisplayName, modelDisplay, relativeTime, shortPath, type AgentSessionView, type UsageSummary } from "./gateway-model";
import { providersHidden } from "./agent-focus-model";
import { useAgentFocus } from "./agent-focus-store";
import { CxBadge, CxCard, CxEmpty, CxRow, CxSection, CxStats, CxStatusIcon } from "./cx-layout";

export type DashboardAgent = "codex" | "claude" | "grok";
type Route = "gateway" | "context" | "skills";
type Notify = (title: string, message: string, status?: string) => void;
type Result = { status: string; message: string };

/** 状态栏第二行：圆点颜色 + 一句话。 */
/** relay = Codex 直连旧版供应商（紫色，和侧栏一致）；direct = 未接入网关、用它自己的设置（灰色）。 */
export type DashboardStatus = { tone: "gateway" | "drift" | "direct" | "relay" | "missing"; text: string };
export type DashboardStat = { label: string; value: ReactNode; tone?: "ok" | "warn"; mono?: boolean };

export function AgentDashboard({
  agent,
  name,
  version,
  status,
  actions,
  stats,
  onNavigate,
  onOpenList,
  notify,
  children,
}: {
  agent: DashboardAgent;
  name: string;
  version?: string | null;
  /** 不传就按网关接入情况算（Claude Code / Grok）。 */
  status?: DashboardStatus;
  /** 不传就是「接入网关 / 管理接入」+「在终端启动」。 */
  actions?: ReactNode;
  /** 不传就是网关今日用量里这个 Agent 的数据。 */
  stats?: DashboardStat[];
  onNavigate: (route: Route) => void;
  /** 打开会话页 / 请求日志，并选中这个 Agent。 */
  onOpenList: (kind: "sessions" | "requestLog", agent: DashboardAgent) => void;
  notify: Notify;
  children?: ReactNode;
}) {
  const payload = useGatewayPayload(10_000);
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [requests, setRequests] = useState<DashboardRequest[] | null>(null);
  const [sessions, setSessions] = useState<AgentSessionView[] | null>(null);
  const [mcpCount, setMcpCount] = useState<number | null>(null);
  const [skillCount, setSkillCount] = useState<number | null>(null);
  const [launching, setLaunching] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  // 只用增强功能：供应商交给别的工具，不显示接入状态、调用统计和最近请求，只留会话、MCP、Skills。
  const enhanceOnly = providersHidden(useAgentFocus());

  useEffect(() => {
    let cancelled = false;
    const done = <T,>(apply: (value: T) => void) => (value: T) => { if (!cancelled) apply(value); };
    setRequests(null);
    setSessions(null);
    if (!stats) {
      invoke<Result & Partial<UsageSummary>>("load_gateway_usage", { range: "today" })
        .then(done((result) => setUsage(result.totals ? (result as UsageSummary) : null)))
        .catch(() => undefined);
    }
    invoke<{ entries?: Parameters<typeof recentAgentRequests>[0] }>("load_request_logs", { limit: 300 })
      .then(done((result) => setRequests(recentAgentRequests(result.entries ?? [], agent))))
      .catch(done(() => setRequests([])));
    const sessionSource = agent === "codex"
      ? invoke<{ sessions?: Array<{ id: string; title: string; cwd: string; updatedAtMs: number }> }>("list_local_sessions", { request: { offset: 0, limit: 20 } })
        .then((result) => (result.sessions ?? []).map((session): AgentSessionView => ({ agent: "codex", id: session.id, title: session.title, cwd: session.cwd, updatedAtMs: session.updatedAtMs })))
      : invoke<{ sessions?: AgentSessionView[] }>("list_agent_sessions", { agent }).then((result) => result.sessions ?? []);
    sessionSource
      .then(done((list) => setSessions([...list].sort((a, b) => b.updatedAtMs - a.updatedAtMs).slice(0, 5))))
      .catch(done(() => setSessions([])));
    const mcpSource = agent === "codex"
      ? invoke<Result & { entries?: { mcpServers?: unknown[] } }>("read_live_context_entries").then((result) => result.entries?.mcpServers?.length ?? 0)
      : invoke<Result & { config?: { entries?: unknown[] } }>("load_agent_mcp", { agent }).then((result) => result.config?.entries?.length ?? 0);
    mcpSource.then(done(setMcpCount)).catch(() => undefined);
    invoke<{ skills?: Array<{ installed?: boolean; agents?: string[] }> }>("list_installed_skills")
      .then(done((result) => setSkillCount((result.skills ?? []).filter((skill) => skill.installed && skill.agents?.includes(agent)).length)))
      .catch(() => undefined);
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => { cancelled = true; window.clearInterval(timer); };
    // stats 只用来判断要不要读今日用量，内容变化不需要重新读。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agent]);

  const view = payload?.agents.find((item) => item.agent === agent);
  const access = agentAccessState(view);
  const binding = payload?.gateway.bindings[agent];
  const model = view?.enabled && binding ? modelDisplay(payload, binding.model, binding.poolExclude ?? []).title : "";
  const usageStats = agentUsageStats(usage, agent);

  const launch = async () => {
    setLaunching(true);
    try {
      const result = await invoke<Result>("open_agent_terminal", { agent });
      notify(tf("启动 {0}", [name]), result.message, result.status);
    } catch (error) {
      notify(tf("启动 {0}", [name]), String(error), "failed");
    } finally {
      setLaunching(false);
    }
  };

  const heroStatus: DashboardStatus = enhanceOnly ? { tone: "direct", text: t("供应商由其他工具管理") } : status ?? {
    tone: access,
    text: access === "gateway" ? (model ? tf("经 Codex++ 网关 · {0}", [model]) : t("经 Codex++ 网关"))
      : access === "drift" ? (view?.drift?.by ? tf("配置被 {0} 改掉了，没有经过网关", [view.drift.by]) : t("配置被别的工具改掉了，没有经过网关"))
        : access === "direct" ? t("未接入网关 · 用它自己的设置")
          : t("读取中…"),
  };
  const launchButton = (
    <button className="cx-secondary" disabled={launching} onClick={() => void launch()} type="button">
      {launching ? <Loader2 aria-hidden="true" className="cx-dash-spin" /> : <Play aria-hidden="true" />}
      {t("在终端启动")}
    </button>
  );
  // 每个 Agent 只有一个主操作：没接入 / 被改掉时是「接入网关」，接入后退成次要的「管理接入」。
  const heroActions = enhanceOnly ? (agent === "codex" ? null : launchButton) : actions ?? (
    <>
      {launchButton}
      <button className={access === "direct" || access === "drift" ? "cx-primary cx-dash-primary" : "cx-secondary"} onClick={() => onNavigate("gateway")} type="button">
        {access === "gateway" ? t("管理接入") : access === "drift" ? t("重新接入") : t("接入网关")}
      </button>
    </>
  );
  const statItems: DashboardStat[] = stats ?? [
    { label: t("今日调用"), value: usageStats ? usageStats.calls : "—" },
    { label: t("成功率"), value: usageStats?.successRate == null ? "—" : `${Math.round(usageStats.successRate * 1000) / 10}%`, tone: usageStats?.successRate != null && usageStats.successRate < 0.9 ? "warn" : "ok" },
    { label: "Tokens", value: usageStats ? formatTokens(usageStats.tokens) : "—" },
    { label: t("可用模型"), value: view?.enabled ? view.models.length : "—" },
  ];
  const configPath = view?.configPath ?? (agent === "codex" ? "~/.codex/config.toml" : "");
  const statusDot = ({ gateway: "ok", relay: "ok", drift: "warn", missing: "warn", direct: "off" } as const)[heroStatus.tone];

  return (
    <div className="cx-page cx-dash">
      {/* 状态行：Agent、版本、经哪家供应商 / 网关，右侧切换与管理。 */}
      <CxCard className="cx-dash-status">
        <CxRow
          badges={version || view?.version ? <CxBadge>{version || view?.version}</CxBadge> : null}
          detail={heroStatus.text}
          icon={<CxStatusIcon tone={statusDot}><AgentGlyph agent={agent} /></CxStatusIcon>}
          title={name}
        >
          {heroActions}
        </CxRow>
      </CxCard>

      {enhanceOnly ? null : (
        <CxStats items={statItems.map((item) => ({
          label: item.label,
          value: <span className="cx-dash-stat-value" data-mono={item.mono === false ? "false" : undefined} title={typeof item.value === "string" ? item.value : undefined}>{item.value}</span>,
          tone: item.tone === "warn" ? "warn" : undefined,
        }))} />
      )}

      <div className="cx-dash-lists" data-single={enhanceOnly ? "true" : undefined}>
        <CxSection
          aside={<button className="cx-dash-link" onClick={() => onOpenList("sessions", agent)} type="button">{t("全部")}<ArrowUpRight aria-hidden="true" size={13} /></button>}
          title={t("最近会话")}
        >
          <CxCard className="cx-dash-list">
            {sessions === null ? <CxEmpty title={t("读取中…")} /> : sessions.length === 0 ? (
              <CxEmpty title={tf("还没有 {0} 的本地会话。", [name])} />
            ) : sessions.map((session) => {
              const time = relativeTime(session.updatedAtMs, now);
              return (
                <CxRow detail={<span title={session.cwd}>{shortPath(session.cwd) || "~"}</span>} key={session.id} title={<span title={session.title}>{session.title || t("未命名会话")}</span>}>
                  <time className="cx-dash-time">{time.args.length ? tf(time.text, time.args) : t(time.text)}</time>
                </CxRow>
              );
            })}
          </CxCard>
        </CxSection>

        {enhanceOnly ? null : (
          <CxSection
            aside={<button className="cx-dash-link" onClick={() => onOpenList("requestLog", agent)} type="button">{t("请求日志")}<ArrowUpRight aria-hidden="true" size={13} /></button>}
            title={t("最近请求")}
          >
            <CxCard className="cx-dash-list">
              {requests === null ? <CxEmpty title={t("读取中…")} /> : requests.length === 0 ? (
                <CxEmpty
                  detail={agent === "codex"
                    ? t("直连且未开启协议转换时，请求不经过本地代理，这里不会有记录。")
                    : view?.enabled ? t("在它里面发一条消息试试。") : t("接入网关后，这里显示它的每次请求走了哪家供应商。")}
                  title={agent === "codex" ? t("最近没有经过本地代理的请求") : t("还没有经过网关的请求")}
                />
              ) : requests.map((request) => {
                const name = groupDisplayName(payload, request.model) ?? request.model;
                return (
                  <CxRow detail={request.provider ? `→ ${request.provider}` : "—"} key={request.id} title={<span title={name}>{name}</span>}>
                    <time className="cx-dash-time">{request.running ? t("进行中") : formatElapsed(request.durationMs)}</time>
                    {request.running ? <Loader2 aria-hidden="true" className="cx-dash-spin cx-dash-state" size={14} />
                      : request.success ? <CheckCircle2 aria-label={t("成功")} className="cx-dash-state" data-tone="ok" size={14} />
                        : <XCircle aria-label={t("失败")} className="cx-dash-state" data-tone="err" size={14} />}
                  </CxRow>
                );
              })}
            </CxCard>
          </CxSection>
        )}
      </div>

      <footer className="cx-dash-foot">
        <button onClick={() => onNavigate("context")} type="button">MCP <b>{mcpCount ?? "—"}</b><ArrowUpRight aria-hidden="true" size={12} /></button>
        <button onClick={() => onNavigate("skills")} type="button">Skills <b>{skillCount ?? "—"}</b><ArrowUpRight aria-hidden="true" size={12} /></button>
        {configPath ? <span title={configPath}>{t("配置文件")} <code>{shortPath(configPath)}</code></span> : null}
      </footer>

      {children}
    </div>
  );
}
