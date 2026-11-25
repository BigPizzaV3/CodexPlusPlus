import { useRef, useState } from "react";
import { ArrowUpRight, ChevronLeft, ChevronRight, Grid2X2, List, MessageSquare, Pin, Radio, Search } from "lucide-react";
import { CxEmpty, CxSection } from "./cx-layout";
import { agentName, formatTokens, maskAccountName, type GatewayPayload } from "./gateway-model";
import { t, tf } from "./i18n";
import { clockTime, flowAgent, flowStatus, type FlowEntry } from "./request-flow-model";
import { FlowRequestRow, useRequestFlow } from "./RequestFlow";
import { RequestRouteDetails } from "./RequestRouteDetails";
import { contextPercent, RoutingContext } from "./RoutingContext";
import { RequestRowMetrics, RoutingExchange, RoutingPath, RoutingTotals } from "./RoutingSnapshot";
import { groupRoutingRequests, isContextCompaction, requestContext, requestTime, routingSessionKey, selectRoutingRequest } from "./routing-session-model";
import { openRequestLog, requestLogAgent } from "./UsageTabs";
import "./RoutingObservatory.css";

const limit = 120;
const batch = 20;
const showId = (id: string) => id.length > 18 ? `${id.slice(0, 12)}…${id.slice(-4)}` : id;
const statusLabel = { running: "进行中", ok: "成功", error: "失败" } as const;
const periodLabel = { recent: "最近", today: "今天", yesterday: "昨天" } as const;
const purposeLabel = { all: "全部用途", normal: "模型调用", compact: "上下文压缩" } as const;
const sessionName = (group: { title?: string; thread?: string }) => group.title || (group.thread ? showId(group.thread) : t("未关联会话"));

export function RecentRouteFeed({ payload }: { payload: GatewayPayload | null }) {
  const { entries, loaded, failed } = useRequestFlow(3000, limit, true);
  const [view, setView] = useState<"sessions" | "requests">("requests");
  const [status, setStatus] = useState<"all" | keyof typeof statusLabel>("all");
  const [period, setPeriod] = useState<keyof typeof periodLabel>("recent");
  const [purpose, setPurpose] = useState<keyof typeof purposeLabel>("all");
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<{ request: string; session: string }>();
  const [visible, setVisible] = useState(batch);
  const historyRef = useRef<HTMLDivElement>(null);
  const detailRef = useRef<HTMLDivElement>(null);
  const contextRef = useRef<HTMLDivElement>(null);
  const groups = groupRoutingRequests(entries.filter(entry => (entry.route || entry.agent && /(?:\/(?:responses|messages|completions|compact)|:(?:stream)?generateContent)(?:[/?]|$)/i.test(entry.path ?? ""))));
  const needle = query.trim().toLocaleLowerCase();
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const matching = groups.flatMap(group => {
    const shows = group.requests.filter(entry => {
      const when = requestTime(entry);
      if (period === "today" && when < +today || period === "yesterday" && (when < +yesterday || when >= +today)) return false;
      if (purpose !== "all" && isContextCompaction(entry) !== (purpose === "compact")) return false;
      if (status !== "all" && flowStatus(entry) !== status) return false;
      if (!needle) return true;
      const provider = (id?: string, name?: string) => maskAccountName(name || id || "");
      return [group.title, group.thread, entry.id, flowAgent(entry), agentName(flowAgent(entry)), entry.model,
        entry.upstreamModel, provider(entry.relayId, entry.relayName),
        ...(entry.route?.steps.flatMap(step => [provider(step.relayId, step.relayName), step.model]) ?? []),
      ].some(value => value?.toLocaleLowerCase().includes(needle));
    });
    return shows.length ? [{ group, shows }] : [];
  });
  const rows = matching.flatMap(({ shows }) => shows).sort((a, b) => requestTime(b) - requestTime(a));
  const { active, entry, pinned } = selectRoutingRequest(matching, rows[0], chosen, view === "sessions");
  const pick = (request: FlowEntry, fromHistory = false) => {
    if (view === "requests" && !rows.some(candidate => candidate.id === request.id)) setView("sessions");
    setChosen({ request: request.id, session: routingSessionKey(request) });
    if (fromHistory) detailRef.current?.focus({ preventScroll: true });
    if (fromHistory) detailRef.current?.scrollIntoView({ block: "start" });
  };
  const timeline = active?.group.requests ?? [];
  const current = timeline.findIndex(request => request.id === entry?.id);
  const resetList = () => { setVisible(batch); setChosen(undefined); };
  const count = view === "sessions" ? matching.length : rows.length;
  const stats = payload?.activity?.agents ?? [];
  const live = period === "recent" && status === "all" && purpose === "all" && !needle ? matching.flatMap(({ group, shows }) => {
    const newest = shows.find(request => !isContextCompaction(request)
      && (request.contextSummary || request.usage || flowStatus(request) === "running"));
    if (!group.thread || !newest || flowStatus(newest) !== "running" && Date.now() - requestTime(newest) > 15 * 60_000) return [];
    const reading = group.requests.find(request => flowStatus(request) === "ok" && !isContextCompaction(request)
      && requestContext(request).ratio !== null);
    return reading ? [{ group, newest, reading: requestContext(reading) }] : [];
  }).slice(0, 6).sort((a, b) => a.group.key.localeCompare(b.group.key)) : [];
  const filtered = needle || status !== "all" || period !== "recent" || purpose !== "all";

  return <CxSection className="route-observatory" title={t("实时路由")} aside={<div className="route-observatory-jumps">
    {entry && <button type="button" className="hub-routing-more" onClick={() => contextRef.current?.scrollIntoView({ block: "start" })}>
      <Grid2X2 aria-hidden="true" />{t("上下文")}</button>}
    <button type="button" className="hub-routing-more" onClick={() => historyRef.current?.scrollIntoView({ block: "start" })}>
      <List aria-hidden="true" />{t("最近请求")}</button>
  </div>}>
    {entry && active ? <div className="route-observatory-detail" ref={detailRef} tabIndex={-1}>
      <div className="route-observatory-session">
        <div>
          <strong>{sessionName(active.group)}</strong>
          <small title={tf("最近记录中有 {0} 条请求", [timeline.length])}>{agentName(flowAgent(entry))} · {tf("本会话 {0} 条请求", [timeline.length])}</small>
        </div>
        <div className="route-observatory-controls">
          <button type="button" aria-pressed={!pinned} title={t(pinned ? "跟随最新请求" : "锁定此请求")} onClick={() => pinned ? setChosen(undefined) : pick(entry)}>
            {pinned ? <Pin /> : <Radio />}{t(pinned ? "已锁定" : "跟随最新")}
          </button>
          <div className="mg-segmented" role="group" aria-label={t("会话内请求")}>
            <button type="button" title={t("上一条请求")} aria-label={t("上一条请求")} disabled={!timeline[current + 1]}
              onClick={() => pick(timeline[current + 1])}><ChevronLeft /></button>
            <span>{timeline.length - current} / {timeline.length}</span>
            <button type="button" title={t("下一条请求")} aria-label={t("下一条请求")} disabled={!timeline[current - 1]}
              onClick={() => pick(timeline[current - 1])}><ChevronRight /></button>
          </div>
        </div>
      </div>
      <RoutingPath entry={entry} payload={payload} />
      <RoutingTotals activity={stats} onFailure={groups.some(group => group.requests.some(request => flowStatus(request) === "error")) ? () => {
        setPeriod("recent"); setPurpose("all"); setQuery(""); setStatus("error"); resetList();
      } : undefined} />
      <RequestRouteDetails entry={entry} agent={agentName(flowAgent(entry))} hideUsage story
        groupLabel={payload?.groups.find(group => group.id === entry.route?.groupId)?.name}
        exchange={<RoutingExchange entry={entry} onUsage={() => openRequestLog({ agent: requestLogAgent(flowAgent(entry)), relayId: entry.relayId || undefined, model: entry.model || undefined, range: "all" })} />} />
      <div className="route-context-target" ref={contextRef}><RoutingContext entry={entry} requests={timeline} onPick={pick} /></div>
      {live.length > 1 && <div className="route-live-sessions" role="group" aria-label={t("各会话的上下文窗口")}>
        <h4>{t("各会话的上下文窗口")}</h4>
        <div>{live.map(({ group, newest, reading }) => <button type="button" key={group.key} aria-current={active.group.key === group.key} onClick={() => pick(newest, true)}>
          <b title={group.title || group.thread}>{sessionName(group)}</b>
          <small>{agentName(flowAgent(newest))} · {formatTokens(reading.input ?? 0)} / {formatTokens(reading.window ?? 0)} · {contextPercent(reading.ratio ?? 0)}</small>
          <span aria-hidden="true"><i style={{ width: `${Math.min(1, reading.ratio ?? 0) * 100}%` }} /></span>
        </button>)}</div>
      </div>}
    </div> : null}
    <div className="route-observatory-history" ref={historyRef}>
    <header className="route-history-heading"><h3>{t("最近请求")}</h3><small>{tf("网关最近 {0} 条", [groups.reduce((n, group) => n + group.requests.length, 0)])}</small></header>
    <div className="hub-routing-filters">
      <div className="mg-segmented" role="group" aria-label={t("路由分组")}>
        {(["sessions", "requests"] as const).map(mode => <button type="button" key={mode}
          aria-pressed={view === mode} data-active={view === mode} onClick={() => { setView(mode); setVisible(batch); }}>
          {t(mode === "sessions" ? "按会话" : "逐条请求")}
        </button>)}
      </div>
      <label className="cx-search">
        <Search aria-hidden="true" />
        <input type="search" aria-label={t("搜索会话、模型或供应商")} placeholder={t("搜索会话、模型或供应商")}
          value={query} onChange={event => { setQuery(event.target.value); resetList(); }} />
      </label>
      <div className="mg-segmented" role="group" aria-label={t("请求状态")}>
        {(["all", "ok", "error", "running"] as const).map(value => <button type="button" key={value}
          aria-pressed={status === value} data-active={status === value} onClick={() => { setStatus(value); resetList(); }}>
          {t(value === "all" ? "全部" : statusLabel[value])}
        </button>)}
      </div>
      <button type="button" className="hub-routing-more" onClick={() => openRequestLog({ range: "all" })}>
        {t("查看请求日志")} <ArrowUpRight aria-hidden="true" />
      </button>
    </div>
    <div className="route-history-filters">
      <div className="mg-segmented" role="group" aria-label={t("请求时间范围")}>
        {(Object.keys(periodLabel) as (keyof typeof periodLabel)[]).map(value => <button type="button" key={value} aria-pressed={period === value}
          data-active={period === value} onClick={() => { setPeriod(value); resetList(); }}>{t(periodLabel[value])}</button>)}
      </div>
      <div className="mg-segmented" role="group" aria-label={t("请求用途")}>
        {(Object.keys(purposeLabel) as (keyof typeof purposeLabel)[]).map(value => <button type="button" key={value} aria-pressed={purpose === value}
          data-active={purpose === value} onClick={() => { setPurpose(value); resetList(); }}>{t(purposeLabel[value])}</button>)}
      </div>
    </div>
    {!loaded || !entry || !active ? <CxEmpty title={t(failed ? "读取失败" : !loaded ? "读取中…" : filtered ? "没有匹配的路由请求" : "暂无网关路由记录")} /> : (
        <div className="route-observatory-sidebar">
          <div className="route-observatory-list" role="group" aria-label={t(view === "sessions" ? "会话" : "请求")}>
            {view === "sessions" ? matching.slice(0, visible).map(({ group, shows }) => {
              const latest = shows[0];
              const lastReading = group.requests.find(request => flowStatus(request) === "ok"
                && !isContextCompaction(request) && requestContext(request).ratio !== null);
              const reading = lastReading && requestContext(lastReading).ratio;
              return <button type="button" key={group.key} aria-current={active.group.key === group.key} onClick={() => pick(latest, true)}>
                <MessageSquare aria-hidden="true" />
                <span className="route-observatory-list-text">
                  <strong title={group.title || group.thread}>{sessionName(group)}</strong>
                  <small>{agentName(flowAgent(latest))} · {tf("{0} 条请求", [group.requests.length])}{reading != null && ` · ${contextPercent(reading)}`}</small>
                </span>
                <time>{clockTime(requestTime(latest))}</time>
              </button>;
            }) : rows.slice(0, visible).map(request => <button type="button" key={request.id} aria-current={entry.id === request.id}
              onClick={() => pick(request, true)}><FlowRequestRow entry={request} payload={payload} /><RequestRowMetrics entry={request} /></button>)}
          </div>
          {count > visible ? <button type="button" className="hub-routing-more" onClick={() => setVisible(value => value + batch)}>
            {t("显示更多记录")}
          </button> : null}
        </div>
    )}
    </div>
  </CxSection>;
}
