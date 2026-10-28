/** 本地网关 · 用量：余额、费用 / token / 缓存 / 调用 / 耗时、按天的柱状图与 Agent 占比，以及按供应商 / 模型 / Agent 的明细表。 */
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { appConfirm } from "./app-dialog";
import { RefreshCw } from "lucide-react";
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
  niceCeiling,
  prettyModelName,
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
import { openRequestLog } from "./UsageTabs";
import { TopbarActions } from "./TopbarActions";

const memo: { range: UsageRange; summary: UsageSummary | null; balances: ProviderBalance[] | null } = { range: "30d", summary: null, balances: null };

export function UsageView({ payload, busy, run, notify }: { payload: GatewayPayload | null; busy: string | null; run: Run; notify: Notify }) {
  const [range, setRangeState] = useState<UsageRange>(memo.range);
  const [summary, setSummary] = useState<UsageSummary | null>(memo.summary);
  const [balances, setBalances] = useState<ProviderBalance[] | null>(memo.balances);
  const [loadingBalances, setLoadingBalances] = useState(false);
  const [split, setSplit] = useState<"provider" | "model" | "agent">("provider");

  // 每次读取的序号：快速切换范围时，先发出的请求可能后回来，只认最后一次的结果。
  const latest = useRef(0);
  const load = async (next: UsageRange) => {
    const id = ++latest.current;
    try {
      const result = await invoke<{ status: string; message: string } & Partial<UsageSummary>>("load_gateway_usage", { range: next });
      if (id !== latest.current) return;
      if (result.status !== "ok" || !result.totals) {
        await notify(t("读取用量失败"), result.message, "failed");
        return;
      }
      const value = result as UsageSummary;
      memo.summary = value;
      setSummary(value);
    } catch (error) {
      if (id === latest.current) await notify(t("调用失败"), String(error), "failed");
    }
  };
  const loadBalances = async () => {
    setLoadingBalances(true);
    try {
      const result = await invoke<{ status: string; balances?: ProviderBalance[] }>("load_gateway_balances");
      memo.balances = result.balances ?? [];
      setBalances(memo.balances);
    } catch {
      setBalances([]);
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
    if (!memo.balances) void loadBalances();
    // 进入时读一次；切换范围时由 setRange 读。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
  const anyCost = groups.some((group) => costText(group.costs));

  return (
    <>
      <div className="gw-usage-head">
        <nav className="gw-tabs gw-range-tabs" role="tablist">
          {ranges.map((item) => (
            <button aria-selected={range === item.id} data-active={range === item.id} key={item.id} onClick={() => setRange(item.id)} role="tab" type="button">
              {item.label}
            </button>
          ))}
        </nav>
      </div>
      {/* 刷新放进外壳顶栏（网关页在用量页签不再放它自己的刷新），页面里不再重复一个。 */}
      <TopbarActions>
        <button aria-label={t("刷新")} className="gw-icon-button gw-usage-refresh" onClick={() => { void load(range); void loadBalances(); }} title={t("刷新")} type="button">
          <RefreshCw className={`h-4 w-4 ${loadingBalances ? "animate-spin" : ""}`} />
        </button>
      </TopbarActions>

      {balances && balances.length ? (
        <div className="gw-balance-grid">
          {balances.map((balance) => (
            <div className="gw-card gw-balance-card" key={balance.providerId}>
              <div className="gw-balance-name">
                <Avatar baseUrl={balance.baseUrl} name={balance.provider} size="sm" />
                <strong>{maskAccountName(balance.provider)}</strong>
              </div>
              <div className="gw-balance-row">
                <span>{t("余额")}</span>
                {balance.amount !== null && balance.amount !== undefined ? <strong>{formatMoney(balance.amount, balance.currency)}</strong> : <small title={balance.error ?? ""}>{t(balance.error ?? "查不到")}</small>}
              </div>
            </div>
          ))}
        </div>
      ) : null}

      <div className="gw-card gw-usage-stats">
        <div className="gw-usage-stat" data-kind="cost">
          <span>{t("估算费用")}</span>
          <strong>{cost ? `≈${cost}` : "—"}</strong>
          <small>{cost ? t("按供应商里填写的价格") : t("在供应商里填上价格后估算")}</small>
        </div>
        <div className="gw-usage-stat">
          <span>{t("Tokens")}</span>
          <strong>{formatTokens((totals?.inputTokens ?? 0) + (totals?.outputTokens ?? 0))}</strong>
          <small>
            {tf("{0} 输入 · {1} 输出", [formatTokens(totals?.inputTokens ?? 0), formatTokens(totals?.outputTokens ?? 0)])}
            {totals?.reasoningTokens ? ` · ${tf("推理 {0}", [formatTokens(totals.reasoningTokens)])}` : ""}
          </small>
        </div>
        <div className="gw-usage-stat">
          <span>{t("缓存命中")}</span>
          <strong>{hit === null ? "—" : `${Math.round(hit * 100)}%`}</strong>
          <small>{hit === null ? t("没有输入") : tf("读取 {0}", [formatTokens(totals?.cachedTokens ?? 0)])}</small>
          <i className="gw-usage-meter" aria-hidden><b style={{ width: `${Math.round((hit ?? 0) * 100)}%` }} /></i>
        </div>
        <div className="gw-usage-stat">
          <span>{t("调用")}</span>
          <strong>{totals?.calls ?? 0}</strong>
          <small data-tone={totals?.errors ? "err" : undefined}>
            {rate === null ? t("还没有请求") : totals?.errors ? tf("成功率 {0}% · {1} 次出错", [(rate * 100).toFixed(1), totals.errors]) : t("全部成功")}
            {totals?.images ? ` · ${tf("生图 {0} 次", [totals.images])}` : ""}
          </small>
        </div>
        <div className="gw-usage-stat">
          <span>{t("平均耗时")}</span>
          <strong>{average === null ? "—" : formatElapsed(average)}</strong>
          <small>{average === null ? t("新请求开始记录") : tf("{0} 次请求计时", [totals?.timedCalls ?? 0])}</small>
        </div>
      </div>

      <div className="gw-card gw-chart-card">
        <UsageChart summary={summary} />
        {summary && summary.byAgent.length > 0 ? <AgentShare groups={summary.byAgent} /> : null}
      </div>

      <div className="gw-section-label">
        <span>{t("明细")}</span>
        <div className="gw-section-aside">
          <nav className="gw-tabs gw-split-tabs" role="tablist">
            {([["provider", t("按供应商")], ["model", t("按模型")], ["agent", t("按 Agent")]] as const).map(([id, label]) => (
              <button aria-selected={split === id} data-active={split === id} key={id} onClick={() => setSplit(id)} role="tab" type="button">{label}</button>
            ))}
          </nav>
          <button
            className="gw-text-button"
            disabled={busy !== null}
            onClick={() => void (async () => {
              if (await appConfirm(t("清空全部用量统计？请求日志不受影响。"), { title: t("清空统计"), confirmText: t("清空"), danger: true })) {
                if (await run("usage:clear", "clear_gateway_usage", undefined, t("清空统计"))) void load(range);
              }
            })()}
            type="button"
          >
            {t("清空统计")}
          </button>
        </div>
      </div>
      <div className="gw-card gw-usage-table" data-cost={anyCost}>
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
          const name = split === "agent" ? agentName(group.key) : split === "model" ? prettyModelName(group.label) : maskAccountName(group.label);
          return (
            <div
              className="gw-usage-row gw-usage-row-link"
              key={group.key || "-"}
              // 点一行看这一项的每条请求：按 Agent、供应商 id 或模型名筛，时间段跟着这里选的。
              onClick={() => openRequestLog(split === "agent"
                ? { agent: group.key === "claude" || group.key === "grok" ? group.key : "codex", range }
                : split === "provider"
                  ? { relayId: group.key, relayLabel: maskAccountName(group.label), range }
                  : { model: group.key, range })}
              onKeyDown={(event) => { if (event.key === "Enter") (event.currentTarget as HTMLElement).click(); }}
              role="button"
              tabIndex={0}
              title={t("查看这些请求")}
            >
              <span className="gw-usage-name">
                {split === "agent" ? (
                  AGENT_ICONS[group.key] ? <img alt="" className="gw-usage-agent" src={AGENT_ICONS[group.key]} /> : <Avatar name={group.label} size="sm" />
                ) : (
                  <Avatar baseUrl={providerEntry?.baseUrl} name={group.label || "?"} size="sm" />
                )}
                <strong title={split === "provider" ? maskAccountName(group.label) : group.label}>{name || t("未知")}</strong>
              </span>
              <span className="gw-usage-share">
                <span className="gw-usage-bar"><i style={{ width: `${Math.max(2, share * 100)}%` }} /></span>
                <em>{share >= 0.995 ? "100%" : `${(share * 100).toFixed(share < 0.1 ? 1 : 0)}%`}</em>
              </span>
              <span className="gw-usage-num">{group.calls}</span>
              <span className="gw-usage-num" title={tf("{0} 输入 · {1} 输出", [formatTokens(group.inputTokens), formatTokens(group.outputTokens)])}>{formatTokens(total)}</span>
              <span className="gw-usage-num">{groupHit === null ? "—" : `${Math.round(groupHit * 100)}%`}</span>
              <span className="gw-usage-num" data-tone={group.errors ? "err" : undefined}>{group.errors || "—"}</span>
              <span className="gw-usage-num">{costText(group.costs) || "—"}</span>
            </div>
          );
        })}
        {summary && groups.length === 0 ? <div className="gw-empty">{t("这段时间还没有请求。")}</div> : null}
      </div>
    </>
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

/**
 * 按天的柱状图：自下而上是未命中缓存的输入、缓存读取、输出，叠在一起；纵轴按 1 / 2 / 5 取整画两条刻度线。
 * 鼠标移到某一天（整列都是感应区，矮柱子也好指）：这一列高亮、其余变暗，上方浮出当天的明细卡。
 * 没有请求的日子只画一条底线，提示里也只说「没有请求」。
 */
function UsageChart({ summary }: { summary: UsageSummary | null }) {
  const days = summary?.days ?? [];
  const english = getLanguage() === "en";
  const [hover, setHover] = useState<number | null>(null);
  const top = niceCeiling(Math.max(0, ...days.map((day) => day.inputTokens + day.outputTokens)));
  const stats = usageDayStats(days);
  const labels = new Set(axisLabelIndexes(days.length, 7));
  const width = 1000;
  const height = 180;
  const slot = days.length ? width / days.length : 0;
  const barWidth = Math.min(slot * (days.length > 60 ? 0.9 : 0.64), 34);
  const active = hover !== null ? days[hover] : undefined;
  const activeLeft = hover !== null && days.length ? ((hover + 0.5) / days.length) * 100 : 0;
  const scale = (value: number) => (value / top) * height;
  return (
    <div className="gw-chart" data-hovering={active ? "true" : undefined} onMouseLeave={() => setHover(null)}>
      <div className="gw-chart-head">
        <strong>{t("每天 token")}</strong>
        <span className="gw-chart-legend">
          <span><i data-kind="input" />{t("输入（未命中缓存）")}</span>
          <span><i data-kind="cache" />{t("缓存读取")}</span>
          <span><i data-kind="output" />{t("输出")}</span>
        </span>
        {stats.peak ? (
          <span className="gw-chart-facts">
            <span>{tf("峰值 {0} · {1}", [shortDate(stats.peak.date, english), formatTokens(stats.peak.inputTokens + stats.peak.outputTokens)])}</span>
            {days.length > 1 ? <span>{tf("{0} 天有请求 · 日均 {1}", [stats.activeDays, formatTokens(stats.dailyAverage)])}</span> : null}
          </span>
        ) : null}
      </div>
      <div className="gw-chart-plot">
        {[1, 0.5].map((ratio) => (
          <span className="gw-chart-grid" key={ratio} style={{ top: `${(1 - ratio) * 100}%` }}><em>{formatTokens(top * ratio)}</em></span>
        ))}
        <svg aria-label={t("每天用量")} preserveAspectRatio="none" role="img" viewBox={`0 0 ${width} ${height}`}>
          {days.map((day, index) => {
            const x = index * slot + (slot - barWidth) / 2;
            const cached = Math.min(day.cachedTokens, day.inputTokens);
            const segments: Array<[string, number]> = [["gw-bar-input", day.inputTokens - cached], ["gw-bar-cache", cached], ["gw-bar-output", day.outputTokens]];
            let y = height;
            return (
              <g className="gw-bar" data-active={hover === index ? "true" : undefined} data-empty={day.calls === 0 ? "true" : undefined} key={day.date}>
                <rect className="gw-bar-hit" height={height} width={slot} x={index * slot} y={0} onMouseEnter={() => setHover(index)} />
                {day.calls === 0 ? (
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
            <strong>{shortDate(active.date, english)}</strong>
            {active.calls === 0 ? (
              <p>{t("没有请求")}</p>
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
        {days.map((day, index) => (labels.has(index) || hover === index ? <span data-active={hover === index ? "true" : undefined} key={day.date} style={{ left: `${((index + 0.5) / days.length) * 100}%` }}>{shortDate(day.date, english)}</span> : null))}
      </div>
    </div>
  );
}
