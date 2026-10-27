/** 本地网关 · 用量：余额、token / 缓存 / 推理 / 调用次数、按天的柱状图，以及按 Agent / 供应商 / 模型拆分。 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { appConfirm } from "./app-dialog";
import { RefreshCw } from "lucide-react";
import { getLanguage, t, tf } from "./i18n";
import {
  agentName,
  axisLabelIndexes,
  cacheHitRate,
  formatMoney,
  formatTokens,
  shortDate,
  type GatewayPayload,
  type ProviderBalance,
  type UsageGroup,
  type UsageRange,
  type UsageSummary,
  maskAccountName,
} from "./gateway-model";
import { AGENT_ICONS, Avatar, type Notify, type Run } from "./gateway-ui";

const memo: { range: UsageRange; summary: UsageSummary | null; balances: ProviderBalance[] | null } = { range: "30d", summary: null, balances: null };

export function UsageView({ payload, busy, run, notify }: { payload: GatewayPayload | null; busy: string | null; run: Run; notify: Notify }) {
  const [range, setRangeState] = useState<UsageRange>(memo.range);
  const [summary, setSummary] = useState<UsageSummary | null>(memo.summary);
  const [balances, setBalances] = useState<ProviderBalance[] | null>(memo.balances);
  const [loadingBalances, setLoadingBalances] = useState(false);
  const [split, setSplit] = useState<"provider" | "model" | "agent">("provider");

  const load = async (next: UsageRange) => {
    try {
      const result = await invoke<{ status: string; message: string } & Partial<UsageSummary>>("load_gateway_usage", { range: next });
      if (result.status !== "ok" || !result.totals) {
        await notify(t("读取用量失败"), result.message, "failed");
        return;
      }
      const value = result as UsageSummary;
      memo.summary = value;
      setSummary(value);
    } catch (error) {
      await notify(t("调用失败"), String(error), "failed");
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
  const groups: UsageGroup[] = split === "provider" ? summary?.byProvider ?? [] : split === "model" ? summary?.byModel ?? [] : summary?.byAgent ?? [];
  const groupMax = Math.max(1, ...groups.map((group) => group.inputTokens + group.outputTokens));

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
        <div className="gw-usage-cost">
          {summary?.costs.length ? (
            <>
              <strong>≈{summary.costs.map((cost) => formatMoney(cost.amount, cost.currency)).join(" + ")}</strong>
              <em>{t("按填写的价格估算")}</em>
            </>
          ) : (
            <em>{t("在供应商里填上价格，这里会估算费用")}</em>
          )}
          <button aria-label={t("刷新")} className="gw-icon-button" onClick={() => { void load(range); void loadBalances(); }} title={t("刷新")} type="button">
            <RefreshCw className={`h-4 w-4 ${loadingBalances ? "animate-spin" : ""}`} />
          </button>
        </div>
      </div>

      {balances && balances.length ? (
        <div className="gw-balance-grid">
          {balances.map((balance) => (
            <div className="gw-card gw-balance-card" key={balance.providerId}>
              <div className="gw-balance-name">
                <Avatar baseUrl={balance.baseUrl} name={balance.provider} size="sm" />
                <strong>{balance.provider}</strong>
              </div>
              <div className="gw-balance-row">
                <span>{t("余额")}</span>
                {balance.amount !== null && balance.amount !== undefined ? <strong>{formatMoney(balance.amount, balance.currency)}</strong> : <small title={balance.error ?? ""}>{t(balance.error ?? "查不到")}</small>}
              </div>
            </div>
          ))}
        </div>
      ) : null}

      <div className="gw-card gw-stat-grid">
        <div>
          <strong>{formatTokens((totals?.inputTokens ?? 0) + (totals?.outputTokens ?? 0))}</strong>
          <span>{t("TOKENS")}</span>
          <small>{tf("{0} 输入 · {1} 输出", [formatTokens(totals?.inputTokens ?? 0), formatTokens(totals?.outputTokens ?? 0)])}</small>
        </div>
        <div>
          <strong>{formatTokens(totals?.cachedTokens ?? 0)}</strong>
          <span>{t("缓存读取")}</span>
          <small>{hit === null ? t("没有输入") : tf("命中率 {0}%", [Math.round(hit * 100)])}</small>
        </div>
        <div>
          <strong>{formatTokens(totals?.reasoningTokens ?? 0)}</strong>
          <span>{t("推理")}</span>
          <small>{t("包含在输出里")}</small>
        </div>
        <div>
          <strong>{totals?.calls ?? 0}</strong>
          <span>{t("调用")}</span>
          <small>{totals?.errors ? tf("{0} 次出错", [totals.errors]) : t("没有出错")}</small>
        </div>
      </div>

      <div className="gw-card gw-chart-card">
        <UsageChart summary={summary} />
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
              if (await appConfirm(t("清空全部用量统计？请求日志不受影响。"), { title: t("清空用量"), confirmText: t("清空"), danger: true })) {
                if (await run("usage:clear", "clear_gateway_usage", undefined, t("清空用量"))) void load(range);
              }
            })()}
            type="button"
          >
            {t("清空")}
          </button>
        </div>
      </div>
      <div className="gw-card">
        {groups.map((group) => {
          const total = group.inputTokens + group.outputTokens;
          const providerEntry = payload?.pool.find((entry) => entry.id === group.key);
          return (
            <div className="gw-usage-row" key={group.key || "-"}>
              {split === "agent" ? (
                AGENT_ICONS[group.key] ? <img alt="" className="gw-usage-agent" src={AGENT_ICONS[group.key]} /> : <Avatar name={group.label} size="sm" />
              ) : (
                <Avatar baseUrl={providerEntry?.baseUrl} name={group.label || "?"} size="sm" />
              )}
              <strong>{split === "agent" ? agentName(group.key) : maskAccountName(group.label) || t("未知")}</strong>
              <span className="gw-usage-bar"><i style={{ width: `${Math.max(2, (total / groupMax) * 100)}%` }} /></span>
              <span className="gw-usage-num">{formatTokens(total)}</span>
              <small>{tf("{0} 次", [group.calls])}</small>
            </div>
          );
        })}
        {summary && groups.length === 0 ? <div className="gw-empty">{t("这段时间还没有请求。")}</div> : null}
      </div>
    </>
  );
}

/**
 * 按天的柱状图：浅色是输入，深色是输出，叠在一起。
 * 鼠标移到某一天（整列都是感应区，矮柱子也好指）：这一列高亮、其余变暗，上方浮出当天的明细卡。
 */
function UsageChart({ summary }: { summary: UsageSummary | null }) {
  const days = summary?.days ?? [];
  const english = getLanguage() === "en";
  const [hover, setHover] = useState<number | null>(null);
  const max = Math.max(1, ...days.map((day) => day.inputTokens + day.outputTokens));
  const labels = new Set(axisLabelIndexes(days.length, 7));
  const width = 1000;
  const height = 180;
  const gap = days.length > 60 ? 1 : 6;
  const barWidth = days.length ? (width - gap * (days.length - 1)) / days.length : 0;
  const active = hover !== null ? days[hover] : undefined;
  const activeLeft = hover !== null && days.length ? ((hover + 0.5) / days.length) * 100 : 0;
  return (
    <div className="gw-chart" data-hovering={active ? "true" : undefined} onMouseLeave={() => setHover(null)}>
      <span className="gw-chart-max">{formatTokens(max)}</span>
      <svg aria-label={t("每天用量")} preserveAspectRatio="none" role="img" viewBox={`0 0 ${width} ${height}`}>
        {days.map((day, index) => {
          const x = index * (barWidth + gap);
          const inputHeight = (day.inputTokens / max) * height;
          const outputHeight = (day.outputTokens / max) * height;
          return (
            <g className="gw-bar" data-active={hover === index ? "true" : undefined} key={day.date}>
              <rect className="gw-bar-hit" height={height} width={barWidth + gap} x={x - gap / 2} y={0} onMouseEnter={() => setHover(index)} />
              <rect className="gw-bar-input" height={inputHeight} rx={2} width={barWidth} x={x} y={height - inputHeight} />
              <rect className="gw-bar-output" height={outputHeight} rx={2} width={barWidth} x={x} y={height - inputHeight - outputHeight} />
            </g>
          );
        })}
      </svg>
      {active ? (
        <div className="gw-chart-tip" data-edge={activeLeft < 18 ? "start" : activeLeft > 82 ? "end" : undefined} role="status" style={{ left: `${activeLeft}%` }}>
          <strong>{shortDate(active.date, english)}</strong>
          <dl>
            <div><dt><i data-kind="total" />{t("合计")}</dt><dd>{formatTokens(active.inputTokens + active.outputTokens)}</dd></div>
            <div><dt><i data-kind="input" />{t("输入")}</dt><dd>{formatTokens(active.inputTokens)}</dd></div>
            <div><dt><i data-kind="output" />{t("输出")}</dt><dd>{formatTokens(active.outputTokens)}</dd></div>
            {active.cachedTokens ? <div><dt><i data-kind="cache" />{t("缓存读取")}</dt><dd>{formatTokens(active.cachedTokens)}</dd></div> : null}
            <div><dt><i data-kind="calls" />{t("调用")}</dt><dd>{active.errors ? tf("{0} 次 · {1} 次出错", [active.calls, active.errors]) : tf("{0} 次", [active.calls])}</dd></div>
          </dl>
        </div>
      ) : null}
      <div className="gw-chart-axis">
        {days.map((day, index) => (labels.has(index) || hover === index ? <span data-active={hover === index ? "true" : undefined} key={day.date} style={{ left: `${((index + 0.5) / days.length) * 100}%` }}>{shortDate(day.date, english)}</span> : null))}
      </div>
    </div>
  );
}
