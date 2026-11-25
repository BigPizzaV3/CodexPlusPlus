/** 本地网关 · 用量：额度、余额、费用 / token / 缓存 / 调用 / 耗时、按天的柱状图与 Agent 占比，以及按供应商 / 模型 / Agent 的明细表。 */
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { appConfirm } from "./app-dialog";
import { ChartColumn, ChartLine, ChevronLeft, ChevronRight, RefreshCw } from "lucide-react";
import { getLanguage, t, tf } from "./i18n";
import {
  agentName,
  averageDuration,
  axisLabelIndexes,
  cacheHitRate,
  costText,
  formatElapsed,
  formatMoney,
  formatTokens,
  groupDisplayName,
  niceCeiling,
  prettyModelName,
  quotaEntries,
  shortDate,
  usageDayStats,
  type GatewayPayload,
  type ProviderBalance,
  type UsageGroup,
  type UsageRange,
  type UsageSummary,
  maskAccountName,
} from "./gateway-model";
import { AGENT_ICONS, Avatar, type Notify, type Run } from "./gateway-ui";
import { successRate } from "./hub-overview-model";
import { openRequestLog, requestLogAgent } from "./UsageTabs";
import { TopbarActions } from "./TopbarActions";
import { cachedProviderBalances, loadProviderBalances, subscribeProviderBalances } from "./provider-balances";
import { CxCard, CxEmpty, CxRow, CxSection, CxStats } from "./cx-layout";
import { Menu, MenuItem } from "./ManagementPages";
import { UsageVisuals } from "./UsageVisuals";
import { QuotaCards } from "./ProviderQuotaPanel";
import { onTabListKeyDown } from "./tablist-keys";

type LoadedUsage = { range: UsageRange; summary: UsageSummary };
const memo: { range: UsageRange; loaded: LoadedUsage | null } = { range: "30d", loaded: null };

export function UsageView({ payload, busy, run, notify }: { payload: GatewayPayload | null; busy: string | null; run: Run; notify: Notify }) {
  const [range, setRangeState] = useState<UsageRange>(memo.range);
  const [loaded, setLoaded] = useState(memo.loaded);
  const [failedRange, setFailedRange] = useState<UsageRange>();
  const summary = loaded?.range === range ? loaded.summary : null;
  // 余额和供应商池卡片共用一份（#2793），谁先查到另一边直接用。
  const [balances, setBalances] = useState<ProviderBalance[] | null>(cachedProviderBalances().balances);
  const [loadingBalances, setLoadingBalances] = useState(false);
  // 读用量中：第一次进来（没有缓存）不先显示「0 次调用 / 还没有请求」，切换时间段时旧数字变淡。
  const [loading, setLoading] = useState(false);
  const [split, setSplit] = useState<"provider" | "model" | "agent">("provider");
  const [chartStyle, setChartStyle] = useState<"bar" | "line">("bar");

  // 每次读取的序号：快速切换范围时，先发出的请求可能后回来，只认最后一次的结果。
  const latest = useRef(0);
  const load = async (next: UsageRange) => {
    const id = ++latest.current;
    setFailedRange(undefined);
    setLoading(true);
    try {
      const result = await invoke<{ status: string; message: string } & Partial<UsageSummary>>("load_gateway_usage", { range: next });
      if (id !== latest.current) return;
      if (result.status !== "ok" || !result.totals) {
        setFailedRange(next);
        await notify(t("读取用量失败"), result.message, "failed");
        return;
      }
      const value = { range: next, summary: result as UsageSummary };
      memo.loaded = value;
      setLoaded(value);
    } catch (error) {
      if (id === latest.current) {
        setFailedRange(next);
        await notify(t("调用失败"), String(error), "failed");
      }
    } finally {
      if (id === latest.current) setLoading(false);
    }
  };
  const loadBalances = async (force: boolean) => {
    setLoadingBalances(true);
    try {
      setBalances(await loadProviderBalances(
        () => invoke<{ status: string; balances?: ProviderBalance[] }>("load_gateway_balances"),
        { force, maxAge: null },
      ));
    } finally {
      setLoadingBalances(false);
    }
  };
  const setRange = (next: UsageRange) => {
    memo.range = next;
    setRangeState(next);
    void load(next);
  };

  useEffect(() => {
    void load(range);
    void loadBalances(false);
    // 进入时读一次；切换范围时由 setRange 读。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // 共享供应商卡片那边刷新了余额，这里跟着更新。
  useEffect(() => subscribeProviderBalances(setBalances), []);

  const ranges: Array<{ id: UsageRange; label: string }> = [
    { id: "today", label: t("今天") },
    { id: "7d", label: t("7 天") },
    { id: "30d", label: t("30 天") },
    { id: "all", label: t("全部") },
  ];
  const totals = summary?.totals;
  const hit = totals ? cacheHitRate(totals) : null;
  const rate = totals ? successRate(totals) : null;
  const average = totals ? averageDuration(totals) : null;
  const cost = costText(summary?.costs);
  const groups: UsageGroup[] = split === "provider" ? summary?.byProvider ?? [] : split === "model" ? summary?.byModel ?? [] : summary?.byAgent ?? [];
  const tokenTotal = Math.max(1, groups.reduce((sum, group) => sum + group.inputTokens + group.outputTokens, 0));
  const usageCost = (group: UsageGroup) => costText(group.costs);
  const anyCost = groups.some(group => usageCost(group));
  const quotas = quotaEntries(payload?.pool);

  // 参照 Cursor 的设置页：时间段在标题行、数字一排、图表一张卡、明细一张表；余额放最后。
  return (
    <div aria-busy={loading || undefined} className="gw-usage cx-page" data-loading={loading || undefined}>
      {/* 时间段和刷新都放进标题行右侧：页内只剩「统计 / 请求」一排胶囊，不再上下两排。 */}
      <TopbarActions>
        <div aria-label={t("时间段")} className="mg-segmented gw-usage-range" role="radiogroup">
          {ranges.map((item) => (
            <button aria-checked={item.id === range} data-active={item.id === range} key={item.id} onClick={() => setRange(item.id)} role="radio" type="button">
              {item.label}
            </button>
          ))}
        </div>
        <button aria-label={t("刷新")} className="gw-icon-button gw-usage-refresh" onClick={() => { void load(range); void loadBalances(true); }} title={t("刷新")} type="button">
          <RefreshCw className={`h-4 w-4 ${loading || loadingBalances ? "animate-spin" : ""}`} />
        </button>
      </TopbarActions>

      <CxStats
        items={[
          {
            label: t("估算费用"),
            value: cost ? `≈${cost}` : "—",
            hint: <span title={cost ? t("按供应商里填写的价格") : undefined}>{cost ? t("按供应商价格估算") : t("在供应商里填上价格后估算")}</span>,
          },
          {
            label: t("Tokens"),
            value: formatTokens((totals?.inputTokens ?? 0) + (totals?.outputTokens ?? 0)),
            hint: (
              <span title={totals?.reasoningTokens ? tf("推理 {0}", [formatTokens(totals.reasoningTokens)]) : undefined}>
                {tf("{0} 输入 · {1} 输出", [formatTokens(totals?.inputTokens ?? 0), formatTokens(totals?.outputTokens ?? 0)])}
              </span>
            ),
          },
          {
            label: t("缓存命中"),
            value: hit === null ? "—" : `${Math.round(hit * 100)}%`,
            hint: hit === null ? t("没有输入") : tf("读取 {0}", [formatTokens(totals?.cachedTokens ?? 0)]),
          },
          {
            label: t("调用"),
            value: totals?.calls ?? 0,
            hint: (
              <span data-tone={totals?.errors ? "err" : undefined}>
                {rate === null ? (loading && !summary ? t("正在读取…") : t("还没有请求")) : totals?.errors ? tf("成功率 {0}% · {1} 次出错", [(rate * 100).toFixed(1), totals.errors]) : t("全部成功")}
                {totals?.images ? ` · ${tf("生图 {0} 次", [totals.images])}` : ""}
              </span>
            ),
          },
          {
            label: t("平均耗时"),
            value: average === null ? "—" : formatElapsed(average),
            hint: average === null ? t("新请求开始记录") : tf("{0} 次请求计时", [totals?.timedCalls ?? 0]),
          },
        ]}
      />

      <p className="gw-muted">{t("统计经过 Codex++ 代理的请求；Token 用量以已收到的计数为准。订阅剩余额度单独显示。")}</p>
      <CxCard className="gw-chart-card">
        <UsageChartPages label={t(range === "today" ? "每小时 token" : "每天 token")}
          controls={<div className="mg-segmented gw-chart-format" role="group" aria-label={t("图表样式")}>
            {([["bar", ChartColumn, t("柱状图")], ["line", ChartLine, t("折线图")]] as const).map(([style, Icon, name]) =>
              <button key={style} type="button" aria-label={name} aria-pressed={chartStyle === style} data-active={chartStyle === style}
                title={name} onClick={() => setChartStyle(style)}><Icon aria-hidden="true" /></button>)}
          </div>}
          distribution={<UsageVisuals summary={summary} range={range} failed={failedRange === range} payload={payload} />}>
          <UsageChart key={range} summary={summary} range={range} style={chartStyle} />
          {summary && summary.byAgent.length > 0 ? <AgentShare groups={summary.byAgent} /> : null}
        </UsageChartPages>
      </CxCard>

      <CxSection
        aside={
          <>
            <nav aria-label={t("明细")} className="tp-tabs cx-pills gw-split-pills" onKeyDown={onTabListKeyDown} role="tablist">
              {([["provider", t("按供应商")], ["model", t("按模型")], ["agent", t("按 Agent")]] as const).map(([id, label]) => (
                <button aria-controls="gw-usage-detail" aria-selected={split === id} data-active={split === id} key={id} onClick={() => setSplit(id)} role="tab" tabIndex={split === id ? 0 : -1} type="button">{label}</button>
              ))}
            </nav>
            {/* 清空是少用的破坏性操作：收进三点菜单，确认框保留。 */}
            <Menu>
              <MenuItem
                danger
                disabled={busy !== null}
                onClick={() => void (async () => {
                  if (await appConfirm(t("清空全部用量统计？请求日志不受影响。"), { title: t("清空统计"), confirmText: t("清空"), danger: true })) {
                    if (await run("usage:clear", "clear_gateway_usage", undefined, t("清空统计"))) void load(range);
                  }
                })()}
              >
                {t("清空统计")}
              </MenuItem>
            </Menu>
          </>
        }
        title={t("明细")}
      >
        <div aria-label={t("明细")} className="cx-card gw-usage-table" data-cost={anyCost} id="gw-usage-detail" role="tabpanel">
          {groups.length ? (
            <div className="gw-usage-row gw-usage-row-head">
              <span>{split === "provider" ? t("供应商") : split === "model" ? t("模型") : t("Agent")}</span>
              <span>{t("占比")}</span>
              <span>{t("调用")}</span>
              <span>{t("Tokens")}</span>
              <span>{t("缓存命中")}</span>
              <span>{t("出错")}</span>
              <span>{t("费用")}</span>
            </div>
          ) : null}
          {groups.map((group) => {
            const total = group.inputTokens + group.outputTokens;
            const share = total / tokenTotal;
            const groupHit = cacheHitRate(group);
            const providerEntry = payload?.pool.find((entry) => entry.id === group.key);
            const name = split === "agent" ? agentName(group.key) : split === "model" ? groupDisplayName(payload, group.key) ?? prettyModelName(group.label) : maskAccountName(group.label);
            return (
              <div
                className="gw-usage-row gw-usage-row-link"
                key={group.key || "-"}
                // 点一行看这一项的每条请求：按 Agent、供应商 id 或模型名筛，时间段跟着这里选的。
                onClick={() => openRequestLog(split === "agent"
                  ? { agent: requestLogAgent(group.key || "codex"), range }
                  : split === "provider"
                    ? group.key
                      ? { relayId: group.key, relayLabel: maskAccountName(group.label), range }
                      : { relayUnknown: true, relayLabel: t("未知"), range }
                    : { model: group.key, range })}
                onKeyDown={(event) => {
                  // role="button"：Enter 和空格都能打开（空格不让页面滚动）。
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    (event.currentTarget as HTMLElement).click();
                  }
                }}
                role="button"
                tabIndex={0}
                title={t("查看这些请求")}
              >
                <span className="gw-usage-name">
                  {split === "agent" ? (
                    AGENT_ICONS[group.key] ? <img alt="" className="gw-usage-agent" data-agent-icon={group.key} src={AGENT_ICONS[group.key]} /> : <Avatar name={group.label} size="sm" />
                  ) : (
                    <Avatar baseUrl={providerEntry?.baseUrl} name={name || "?"} size="sm" />
                  )}
                  <strong title={split === "model" || split === "provider" ? name : group.label}>{name || t("未知")}</strong>
                </span>
                <span className="gw-usage-share">
                  <span className="gw-usage-bar"><i style={{ width: `${Math.max(2, share * 100)}%` }} /></span>
                  <em>{share >= 0.995 ? "100%" : `${(share * 100).toFixed(share < 0.1 ? 1 : 0)}%`}</em>
                </span>
                <span className="gw-usage-num">{group.calls}</span>
                <span className="gw-usage-num" title={tf("{0} 输入 · {1} 输出", [formatTokens(group.inputTokens), formatTokens(group.outputTokens)])}>{formatTokens(total)}</span>
                <span className="gw-usage-num">{groupHit === null ? "—" : `${Math.round(groupHit * 100)}%`}</span>
                <span className="gw-usage-num" data-tone={group.errors ? "err" : undefined}>{group.errors || "—"}</span>
                <span className="gw-usage-num">{usageCost(group) || "—"}</span>
              </div>
            );
          })}
          {summary && groups.length === 0 ? <CxEmpty title={t("这段时间还没有请求。")} /> : null}
        </div>
      </CxSection>

      {/* 额度（#3084）：订阅账号和 OpenCode Go、Command Code 等套餐的时间窗口，和共享供应商卡片同一份缓存。 */}
      {quotas.length ? (
        <CxSection count={quotas.length} title={t("配额窗口")}>
          <QuotaCards entries={quotas} busy={busy} refresh={entry =>
            void run(`quota:${entry.id}`, "refresh_gateway_quota", { id: entry.id }, t("刷新额度"), true)
          } />
        </CxSection>
      ) : null}

      {balances && balances.length ? (
        <CxSection count={balances.length} title={t("余额")}>
          <CxCard>
            {balances.map((balance) => (
              <CxRow
                icon={<Avatar baseUrl={balance.baseUrl} name={balance.provider} size="sm" />}
                key={balance.providerId}
                title={maskAccountName(balance.provider)}
              >
                {balance.amount !== null && balance.amount !== undefined
                  ? <strong className="gw-usage-balance">{formatMoney(balance.amount, balance.currency)}</strong>
                  : <small className="gw-usage-balance-missing" title={balance.error ?? (balance.note ? t(balance.note) : "")}>{balance.error ? t(balance.error) : balance.note ? "—" : t("查不到")}</small>}
              </CxRow>
            ))}
          </CxCard>
        </CxSection>
      ) : null}
    </div>
  );
}

/** 按 Agent 拆分的一条横向占比条（按 token），图表卡片的底部。 */
function AgentShare({ groups }: { groups: UsageGroup[] }) {
  const tokens = (group: UsageGroup) => group.inputTokens + group.outputTokens;
  const total = groups.reduce((sum, group) => sum + tokens(group), 0);
  if (total <= 0) return null;
  return (
    <div className="gw-usage-agents">
      <div className="gw-usage-agents-bar" aria-label={t("按 Agent 拆分")}>
        {groups.map((group) => <i data-agent={group.key} key={group.key} style={{ flex: tokens(group) }} title={`${agentName(group.key)} · ${formatTokens(tokens(group))}`} />)}
      </div>
      <div className="gw-usage-agents-legend">
        {groups.map((group) => (
          <span key={group.key}><i data-agent={group.key} />{agentName(group.key)}<b>{Math.round((tokens(group) / total) * 100)}%</b></span>
        ))}
      </div>
    </div>
  );
}

function UsageChartPages({ label, controls, distribution, children }: {
  label: string; controls: React.ReactNode; distribution: React.ReactNode; children: React.ReactNode;
}) {
  const [page, setPage] = useState(0);
  const id = useId();
  const viewport = useRef<HTMLDivElement>(null);
  const panels = useRef<HTMLDivElement[]>([]);
  const tabs = useRef<HTMLButtonElement[]>([]);
  const labels = [label, t("用量分布")];
  const go = (index: number) => {
    const node = viewport.current;
    if (!node) return;
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    node.scrollTo({ left: node.clientWidth * index, behavior: reduceMotion ? "auto" : "smooth" });
  };
  useLayoutEffect(() => {
    const node = viewport.current;
    const panel = panels.current[page];
    if (!node || !panel) return;
    const fit = () => { node.style.height = `${Math.ceil(panel.getBoundingClientRect().height)}px`; };
    fit();
    if (typeof ResizeObserver !== "function") return;
    const observer = new ResizeObserver(fit);
    observer.observe(panel);
    return () => observer.disconnect();
  }, [page, children, distribution]);
  return <>
    <div className="gw-chart-switch">
      <div className="mg-segmented gw-chart-tabs" role="tablist" aria-label={t("用量图表")}>
        {labels.map((name, index) => <button key={index} id={`${id}-tab-${index}`}
          ref={node => { if (node) tabs.current[index] = node; }}
          type="button" role="tab" aria-selected={page === index} data-active={page === index}
          aria-controls={`${id}-panel-${index}`} tabIndex={page === index ? 0 : -1}
          onClick={() => go(index)} onKeyDown={event => {
            if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
            event.preventDefault();
            const next = 1 - index;
            tabs.current[next]?.focus();
            go(next);
          }}>{name}</button>)}
      </div>
      <div className="gw-chart-page-actions">
        <div className="gw-chart-format-slot" style={{ visibility: page === 0 ? "visible" : "hidden" }}>{controls}</div>
        <button className="gw-icon-button" type="button" disabled={page === 0} title={t("上一张图")}
          aria-label={t("上一张图")} onClick={() => go(0)}><ChevronLeft aria-hidden="true" /></button>
        <button className="gw-icon-button" type="button" disabled={page === 1} title={t("下一张图")}
          aria-label={t("下一张图")} onClick={() => go(1)}><ChevronRight aria-hidden="true" /></button>
      </div>
    </div>
    <div className="gw-chart-pages" ref={viewport} onScroll={event => {
      const node = event.currentTarget;
      if (node.clientWidth) setPage(Number(node.scrollLeft >= node.clientWidth / 2));
    }}>
      {[children, distribution].map((content, index) => <div key={index} ref={node => { if (node) panels.current[index] = node; }}
        className="gw-chart-page" role="tabpanel" id={`${id}-panel-${index}`} aria-labelledby={`${id}-tab-${index}`}
        aria-hidden={page !== index} inert={page !== index}>{content}</div>)}
    </div>
  </>;
}

function UsageChart({ summary, range, style }: { summary: UsageSummary | null; range: UsageRange; style: "bar" | "line" }) {
  const hourly = range === "today";
  const english = getLanguage() === "en";
  const [hover, setHover] = useState<number | null>(null);
  const collected = new Map(summary?.hours?.map(bucket => [bucket.hour, bucket]));
  const trackedCalls = [...collected.values()].reduce((total, bucket) => total + bucket.calls, 0);
  const partial = hourly && trackedCalls < (summary?.totals.calls ?? 0);
  const hourText = (hour: number) => `${String(hour).padStart(2, "0")}:00`;
  const days = hourly ? Array.from({ length: 24 }, (_, hour) => ({
    calls: 0, errors: 0, inputTokens: 0, outputTokens: 0, cachedTokens: 0, reasoningTokens: 0,
    ...collected.get(hour), date: hourText(hour),
  })) : summary?.days ?? [];
  const label = (date: string) => hourly ? date : shortDate(date, english);
  const currentHour = new Date().getHours();
  const top = niceCeiling(Math.max(0, ...days.map((day) => day.inputTokens + day.outputTokens)));
  const stats = usageDayStats(days);
  const labels = new Set(hourly ? [0, 4, 8, 12, 16, 20, 23] : axisLabelIndexes(days.length, 7));
  const width = 1000;
  const height = 180;
  const slot = days.length ? width / days.length : 0;
  const barWidth = Math.min(slot * (days.length > 60 ? 0.9 : 0.64), 34);
  const active = hover !== null ? days[hover] : undefined;
  const activeLeft = hover !== null && days.length ? ((hover + 0.5) / days.length) * 100 : 0;
  const scale = (value: number) => (value / top) * height;
  const line: string[][] = [];
  let run: string[] | undefined;
  if (style === "line" && summary) days.forEach((day, index) => {
    if (hourly && !day.calls && (index > currentHour || partial)) { run = undefined; return; }
    if (!run) { run = []; line.push(run); }
    run.push(`${(index + .5) * slot},${Math.max(1, Math.min(height - 1, height - scale(day.inputTokens + day.outputTokens)))}`);
  });
  return (
    <div className="gw-chart" data-hovering={active ? "true" : undefined} onMouseLeave={() => setHover(null)}>
      <div className="gw-chart-head">
        {style === "line" ? <span className="gw-chart-legend"><span><i data-kind="total" />{t("Token 合计")}</span></span> :
        <span className="gw-chart-legend">
          <span><i data-kind="input" />{t("输入（未命中缓存）")}</span>
          <span><i data-kind="cache" />{t("缓存读取")}</span>
          <span><i data-kind="output" />{t("输出")}</span>
        </span>}
        {stats.peak ? (
          <span className="gw-chart-facts">
            <span>{tf("峰值 {0} · {1}", [label(stats.peak.date), formatTokens(stats.peak.inputTokens + stats.peak.outputTokens)])}</span>
            {hourly
              ? <span>{tf("{0} 个小时有已记录请求", [stats.activeDays])}</span>
              : days.length > 1 ? <span>{tf("{0} 天有请求 · 日均 {1}", [stats.activeDays, formatTokens(stats.dailyAverage)])}</span> : null}
          </span>
        ) : null}
      </div>
      <div className="gw-chart-plot">
        {[1, 0.5].map((ratio) => (
          <span className="gw-chart-grid" key={ratio} style={{ top: `${(1 - ratio) * 100}%` }}><em>{formatTokens(top * ratio)}</em></span>
        ))}
        <svg aria-label={t(hourly ? "每小时用量" : "每天用量")} preserveAspectRatio="none" role="img" viewBox={`0 0 ${width} ${height}`}>
          {line.map((points, index) => <path className="gw-token-line" key={index}
            d={`M${points.join(" L")}${points.length === 1 ? " h0.01" : ""}`} />)}
          {style === "line" && hover !== null ? <line className="gw-token-cursor"
            x1={(hover + .5) * slot} x2={(hover + .5) * slot} y1={0} y2={height} /> : null}
          {days.map((day, index) => {
            const x = index * slot + (slot - barWidth) / 2;
            const cached = Math.min(day.cachedTokens, day.inputTokens);
            const segments: Array<[string, number]> = [["gw-bar-input", day.inputTokens - cached], ["gw-bar-cache", cached], ["gw-bar-output", day.outputTokens]];
            let y = height;
            return (
              <g className="gw-bar" data-active={hover === index ? "true" : undefined} data-empty={day.calls === 0 ? "true" : undefined}
                data-future={hourly && day.calls === 0 && index > currentHour ? "true" : undefined} key={day.date}>
                <rect className="gw-bar-hit" height={height} width={slot} x={index * slot} y={0} onMouseEnter={() => setHover(index)} />
                {style === "line" ? null : day.calls === 0 ? (
                  <rect className="gw-bar-empty" height={2} width={barWidth} x={x} y={height - 2} />
                ) : (
                  segments.map(([className, value]) => {
                    const h = scale(value);
                    y -= h;
                    return h > 0 ? <rect className={className} height={h} key={className} width={barWidth} x={x} y={y} /> : null;
                  })
                )}
              </g>
            );
          })}
        </svg>
        {active ? (
          <div className="gw-chart-tip" data-edge={activeLeft < 18 ? "start" : activeLeft > 82 ? "end" : undefined} role="status" style={{ left: `${activeLeft}%` }}>
            <strong>{hourly && hover !== null ? `${active.date} – ${hourText(hover + 1)}` : label(active.date)}</strong>
            {active.calls === 0 ? (
              <p>{t(hourly && hover !== null && hover > currentHour ? "时段尚未开始" : partial ? "无小时记录" : "没有请求")}</p>
            ) : (
              <dl>
                <div><dt><i data-kind="total" />{t("合计")}</dt><dd>{formatTokens(active.inputTokens + active.outputTokens)}</dd></div>
                <div><dt><i data-kind="input" />{t("输入")}</dt><dd>{formatTokens(active.inputTokens)}</dd></div>
                {active.cachedTokens ? <div><dt><i data-kind="cache" />{t("缓存读取")}</dt><dd>{formatTokens(active.cachedTokens)}</dd></div> : null}
                <div><dt><i data-kind="output" />{t("输出")}</dt><dd>{formatTokens(active.outputTokens)}</dd></div>
                <div><dt><i data-kind="calls" />{t("调用")}</dt><dd>{active.errors ? tf("{0} 次 · {1} 次出错", [active.calls, active.errors]) : tf("{0} 次", [active.calls])}</dd></div>
              </dl>
            )}
          </div>
        ) : null}
      </div>
      <div className="gw-chart-axis">
        {days.map((day, index) => (labels.has(index) || hover === index ? <span data-active={hover === index ? "true" : undefined} key={day.date} style={{ left: `${((index + 0.5) / days.length) * 100}%` }}>{label(day.date)}</span> : null))}
      </div>
      {partial ? <p className="gw-chart-coverage">{trackedCalls
        ? tf("小时图覆盖 {0}/{1} 次调用，其余只计入当天合计。", [trackedCalls, summary?.totals.calls ?? 0])
        : t("今天的用量只有按天汇总，尚无可展示的小时记录。")}</p> : null}
    </div>
  );
}
