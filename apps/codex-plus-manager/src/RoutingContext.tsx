import { useEffect, useRef, useState } from "react";
import { ChevronDown, ChevronRight, Grid2X2, LayoutPanelTop } from "lucide-react";
import { contextCurve, contextGrid, contextMap, type ContextMapKind } from "./context-map-model";
import { agentName, formatTokens } from "./gateway-model";
import { t, tf } from "./i18n";
import { clockTime, flowAgent, flowStatus, type ContextPart, type ContextPartKind, type FlowEntry } from "./request-flow-model";
import { contextSeries, isContextCompaction, requestContext, requestTime } from "./routing-session-model";

export const contextPercent = (ratio: number) => ratio.toLocaleString(undefined, { style: "percent", maximumFractionDigits: 2 });
const tokens = (value: number | null) => value === null ? t("未报告") : formatTokens(value);
const captions: Record<ContextPartKind, string> = {
  instructions: "系统指令", tools: "工具定义", user: "用户消息", assistant: "助手消息", toolCalls: "工具调用", toolResults: "工具结果", other: "其他内容",
};
const count = (parts: ContextPart[]) => parts.reduce((sum, part) => sum + part.estimatedTokens, 0);
const statuses = { ok: "成功", error: "失败", running: "进行中" } as const;
const cellCount = 100;
const firstTurns = 4;

function ContextTiles({ parts, kind, onPick }: { parts: ContextPart[]; kind?: ContextPartKind; onPick: (value: ContextPartKind) => void }) {
  const [largest, ...rest] = [...parts].sort((a, b) => b.estimatedTokens - a.estimatedTokens);
  const rows: [ContextPart[], ContextPart[]] = [[], []];
  for (const part of rest) rows[count(rows[0]) <= count(rows[1]) ? 0 : 1].push(part);
  const strips = rows.filter(row => row.length > 0);
  const tile = (part: ContextPart) => {
    const label = t(captions[part.kind]);
    const detail = `≈${tokens(part.estimatedTokens)} token`;
    return <button type="button" className="route-context-tile" key={part.kind} data-kind={part.kind} data-muted={Boolean(kind && part.kind !== kind)}
      aria-pressed={kind === part.kind} style={{ flexGrow: part.estimatedTokens }} title={`${label} ×${part.items} · ${detail}`} aria-label={`${label} ×${part.items} · ${detail}`}
      onClick={() => onPick(part.kind)}><span>{label}</span><small>{detail}</small></button>;
  };
  return <div className="route-context-tiles">{tile(largest)}
    {rest.length > 0 && <div className="route-context-tiles-rest" style={{ flexGrow: count(rest) }}>
      {strips.map((row, index) => <div key={index} style={{ flexGrow: count(row) }}>{row.map(tile)}</div>)}
    </div>}
  </div>;
}

export function RoutingContext({ entry, requests, onPick }: { entry: FlowEntry; requests: FlowEntry[]; onPick: (entry: FlowEntry) => void }) {
  const [open, setOpen] = useState(true);
  const [look, setLook] = useState<"blocks" | "grid">("blocks");
  const [filter, setFilter] = useState<ContextPartKind>();
  const [allTurns, setAllTurns] = useState(false);
  const summary = entry.contextSummary;
  const { input, window, cached } = requestContext(entry);
  const estimate = summary ? count(summary.parts) : null;
  const used = input ?? (estimate === 0 ? null : estimate);
  const ratio = used !== null && window !== null && (input !== null || !summary?.partial) ? used / window : null;
  const room = ratio !== null && used !== null && window !== null ? Math.max(0, window - used) : null;
  const hit = cached !== null && input ? cached / input : null;
  const health = ratio === null ? null : ratio > 1 ? "overflow" : ratio >= .9 ? "near" : ratio >= .75 ? "filling" : "healthy";
  const kind = summary?.parts.some(part => part.kind === filter) ? filter : undefined;
  const session = entry.threadTitle || entry.threadId;
  const parts = summary?.parts.filter(part => part.estimatedTokens > 0) ?? [];
  const map = contextMap(parts, used, room);
  const tones = (value: ContextMapKind) => Boolean(kind && value !== kind && value !== "free");
  const inspectedTurns = kind ? (summary?.turns ?? []).slice().reverse().flatMap(turn => {
    const part = turn.parts.find(item => item.kind === kind);
    return part ? [{ number: turn.number, part }] : [];
  }) : [];
  const turns = [...(summary?.turns ?? [])].reverse().filter(turn => turn.parts.some(part => !kind || part.kind === kind));
  const pickPart = (value: ContextPartKind) => setFilter(old => old === value ? undefined : value);
  const freeLabel = room === null ? null : tf("剩余空间 {0}", [tokens(room)]);
  const freeShare = map.usedShare === null ? null : contextPercent(1 - map.usedShare);
  const freeDetail = freeShare === null ? undefined : `${freeShare} · ${tokens(window)} token`;
  return <section className="route-context" aria-label={t("上下文窗口")}>
    <header className="route-context-head">
      <h3><button type="button" aria-expanded={open} onClick={() => setOpen(!open)}>
        {open ? <ChevronDown /> : <ChevronRight />}{t("上下文窗口")}
      </button></h3>
      <span className="route-context-head-reading">{tokens(used)}{window !== null && ` / ${tokens(window)}`}</span>
      <span className="route-context-model" title={entry.model}>{agentName(flowAgent(entry))} <b>›</b> <code>{entry.model}</code></span>
      <span className="route-context-state" data-counted={input !== null}>{t(isContextCompaction(entry) ? "压缩请求" : input !== null ? "已计数" : used !== null ? "粗估" : "未报告")}</span>
    </header>
    {open && <div className="route-context-body">
      <div className="route-context-metrics">
        <div>
          <div>{t("已用上下文")} {health && <span className="route-context-health" data-health={health}>{t(health === "healthy" ? "健康" : health === "filling" ? "逐渐填满" : health === "near" ? "接近上限" : "超过上限")}</span>}</div>
          <div className="route-context-value"><strong>{tokens(used)}</strong>{window !== null && <span> / {tokens(window)} token</span>}</div>
          <small>{ratio === null ? t("未记录上下文占比") : tf("已用 {0} · 剩余 {1}", [contextPercent(ratio), tokens(room)])}</small>
        </div>
        <div>
          <div>{t("提示词缓存")}</div>
          <div className="route-context-value"><strong>{hit === null ? t("未报告") : contextPercent(hit)}</strong>{hit !== null && <span> {t("命中")}</span>}</div>
          {hit !== null && cached !== null && input !== null && <>
            <small>{tf("{0} 来自缓存 · {1} 新增", [tokens(cached), tokens(input - cached)])}</small>
            <div className="route-context-meter" title={contextPercent(hit)}><i data-cache style={{ width: `${hit * 100}%` }} /></div>
          </>}
        </div>
      </div>
      {map.slices.length > 0 && <div className="route-context-composition">
        <div className="route-context-title-row"><b>{t("窗口分布")}</b>
          <small>{t(summary?.partial ? "可见文本粗估 · 部分内容未计入" : parts.length ? "分类大小为估算" : "未记录内容分类")}</small>
          <div className="mg-segmented" role="group" aria-label={t("构成视图")}>
            <button type="button" title={t("上下文地图")} aria-label={t("上下文地图")} aria-pressed={look === "blocks"} data-active={look === "blocks"} onClick={() => setLook("blocks")}><LayoutPanelTop /></button>
            <button type="button" title={t("方格")} aria-label={t("方格")} aria-pressed={look === "grid"} data-active={look === "grid"} onClick={() => setLook("grid")}><Grid2X2 /></button>
          </div>
        </div>
        {look === "blocks" ? <div className="route-context-blocks">
          {map.mapShare > 0 && <div className="route-context-occupied" style={{ flexGrow: map.mapShare }}>
            {parts.length > 0 ? <ContextTiles parts={parts} kind={kind} onPick={pickPart} />
              : <div className="route-context-tile route-context-space" data-kind="unclassified" title={t("已用 · 未分解")}><span>{t("已用 · 未分解")}</span></div>}
          </div>}
          {map.mapShare < 1 && <div className="route-context-tile route-context-space" data-kind="free" style={{ flexGrow: 1 - map.mapShare }} title={`${freeLabel} · ${freeDetail}`}>
            <span>{t("剩余空间")}</span><strong>{tokens(room)}</strong><small>{freeDetail}</small>
          </div>}
        </div> : <div className="route-context-grid" role="img" aria-label={t(map.usedShare === null ? "内容构成" : "上下文窗口")}
          aria-describedby="routing-context-capacity">
          {contextGrid(map.slices, cellCount).map((cell, index) => <i key={index} aria-hidden="true" data-kind={cell} data-muted={tones(cell)} />)}
        </div>}
        <div id="routing-context-capacity" className="route-context-map-key">
          {ratio === null ? <small>{t("未记录窗口占比 · 仅显示已用内容")}</small> : <>
            <small>{tf("已用 {0}", [contextPercent(ratio)])}</small>
            {look === "blocks" && map.mapShare !== map.usedShare && <small>{t("已用区域已放大")}</small>}
            {look === "grid" && <small>{t("每格约 1%")}</small>}
            <small data-free-key>{freeLabel} · {freeShare}</small>
          </>}
        </div>
      </div>}
      {summary ? <div className="route-context-composition">
        <div className="route-context-title-row"><b>{t("内容")}</b><small>{t("结构摘要")}</small></div>
        <div className="route-content-filters" role="group" aria-label={t("内容类型")}>
          <button type="button" aria-pressed={!kind} onClick={() => setFilter(undefined)}>{t("全部")}</button>
          {summary.parts.map(part => <button type="button" key={part.kind} data-kind={part.kind} aria-pressed={kind === part.kind}
            onClick={() => pickPart(part.kind)}><i />{t(captions[part.kind])}</button>)}
        </div>
        {kind && inspectedTurns.length > 0 && <div className="route-context-inspect" data-kind={kind}>
          {inspectedTurns.slice(0, 3).map(({ number, part }) => <small key={number}>
            {tf("第 {0} 轮", [number])} · ×{part.items}{part.estimatedTokens > 0 && ` · ≈${tokens(part.estimatedTokens)} token`}
          </small>)}
        </div>}
        <div className="route-content-rows">
          {summary.parts.filter(part => !kind || part.kind === kind).map(part => <div key={part.kind} className="route-content-row" data-kind={part.kind}>
            <span>{t(captions[part.kind])} <small>×{part.items}</small></span>
            <i style={{ width: `${estimate ? Math.max(2, part.estimatedTokens / estimate * 100) : 0}%` }} />
            <strong>{part.estimatedTokens ? `≈${tokens(part.estimatedTokens)}` : "—"}</strong>
          </div>)}
        </div>
        {turns.length > 0 && <div className="route-context-turns">
          <b>{t("会话内容")}</b>
          {summary.omittedTurns > 0 && <small>{tf("已省略较早的 {0} 轮", [summary.omittedTurns])}</small>}
          {(allTurns ? turns : turns.slice(0, firstTurns)).map(turn => {
            const shown = turn.parts.filter(part => !kind || part.kind === kind);
            return shown.length ? <details key={`${entry.id}:${turn.number}`}><summary>
              <span>{tf("第 {0} 轮", [turn.number])}</span><b>≈{tokens(count(shown))}</b>
            </summary><div>{shown.map(part => <div className="route-turn-row" key={part.kind}>
              <span>{t(captions[part.kind])} <small>×{part.items}</small></span><small>{part.estimatedTokens ? `≈${tokens(part.estimatedTokens)} token` : t("未报告")}</small>
            </div>)}</div></details> : null;
          })}
          {turns.length > firstTurns && <button type="button" className="hub-routing-more" onClick={() => setAllTurns(!allTurns)}>
            {allTurns ? t("收起") : tf("显示全部 {0} 轮", [turns.length])}
          </button>}
        </div>}
      </div> : <small className="route-context-unavailable">{t("这条请求未记录内容构成")}</small>}
      <div className="route-context-crumb"><span>{agentName(flowAgent(entry))}</span>{session && <span title={session}>› {session}</span>}<code title={entry.id}>› #{entry.id}</code></div>
      {requests.length > 1 && <ContextHistory entry={entry} requests={requests} onPick={onPick} />}
    </div>}
  </section>;
}

function ContextHistory({ entry, requests, onPick }: { entry: FlowEntry; requests: FlowEntry[]; onPick: (entry: FlowEntry) => void }) {
  const series = contextSeries(requests);
  const scroller = useRef<HTMLDivElement>(null);
  const selected = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const container = scroller.current;
    const point = selected.current;
    if (!container || !point) return;
    const left = point.offsetLeft;
    const right = left + point.offsetWidth;
    if (left < container.scrollLeft) container.scrollLeft = left;
    else if (right > container.scrollLeft + container.clientWidth) container.scrollLeft = right - container.clientWidth;
  }, [entry.id, series.length]);
  const peak = Math.max(...series.map(point => point.ratio ?? 0));
  const scale = [.01, .02, .05, .1, .25, .5, 1].find(value => value >= peak) ?? Math.max(1, peak);
  const y = (value: number) => 98 - value / scale * 90;
  const curve = contextCurve(series.map(point => point.ratio), scale, 1000, 98, 8);
  return <div className="route-context-history">
    <div className="route-context-title-row"><b>{t("本会话请求")}</b><small>{tf("{0} 条 · 占比刻度 0–{1}", [series.length, contextPercent(scale)])}{curve.gaps && ` · ${t("虚线：中间请求无读数")}`}</small></div>
    <div ref={scroller} className="route-context-series-scroller"><div className="route-context-series" style={{ minWidth: `${series.length * 30}px` }}>
      <svg viewBox="0 0 1000 100" preserveAspectRatio="none" aria-hidden="true">
        <path className="route-history-guide" d="M0 8H1000M0 53H1000M0 98H1000" />
        <path className="route-history-area" d={curve.area} />
        <path className="route-history-line" d={curve.observed} vectorEffect="non-scaling-stroke" />
        <path className="route-history-line route-history-gap" d={curve.gaps} vectorEffect="non-scaling-stroke" />
      </svg>
      {series.map(({ entry: request, ratio }, index) => {
        const label = [tf("请求 {0}", [index + 1]), clockTime(requestTime(request)), t(statuses[flowStatus(request)]),
          isContextCompaction(request) ? t("压缩请求") : ratio === null ? t("未报告") : `${tokens(requestContext(request).input)} · ${contextPercent(ratio)}`].join(" · ");
        return <button type="button" key={request.id} ref={entry.id === request.id ? selected : undefined} title={label} aria-label={label} aria-current={entry.id === request.id}
          onClick={() => onPick(request)}>
          <span aria-hidden="true" data-missing={ratio === null || undefined} data-compact={isContextCompaction(request) || undefined}>
            {ratio !== null && <i style={{ top: `${y(ratio)}%` }} />}
          </span><small>{index + 1}</small>
        </button>;
      })}
    </div></div>
  </div>;
}
