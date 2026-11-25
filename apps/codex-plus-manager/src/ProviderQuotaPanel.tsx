import { useEffect, useId, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CalendarDays, ChartLine, RefreshCw } from "lucide-react";
import { Button } from "./components/ui/button";
import { getLanguage, t, tf } from "./i18n";
import { formatRemaining, maskAccountName, quotaWindowLabel, type GatewayPoolEntry, type QuotaObservation, type QuotaWindow } from "./gateway-model";
import { quotaCursorReading, quotaCycle, quotaPace, quotaPercentLeft, quotaTimeScale, quotaTrend } from "./quota-timeline";
import { QuotaCalendar } from "./QuotaCalendar";
import "./ProviderQuotaPanel.css";

function quotaDate(value: number): string {
  return new Date(value).toLocaleString(getLanguage() === "en" ? "en-US" : "zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false });
}

function windowName(window: QuotaWindow): string {
  const label = quotaWindowLabel(window);
  return label.args.length ? tf(label.text, label.args) : t(label.text);
}

function useQuotaClock() {
  const [now, setNow] = useState(Date.now);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 30_000); return () => clearInterval(timer); }, []);
  return now;
}

/** 在原有供应商详情里补充剩余比例、进度和准确的重置时间。 */
export function AccountQuotaDetails({ entry }: { entry: GatewayPoolEntry }) {
  const now = useQuotaClock();
  const quota = entry.quota;
  return <div className="gw-account-quota-details">
    <div className="gw-account-windows">
      {quota?.windows.length ? quota.windows.map((window, index) => {
        const name = windowName(window);
        const left = quotaPercentLeft(quota, window, now, entry.subscriptionPolicy?.maxAgeSeconds);
        return <div className="gw-account-window" key={`${window.windowMinutes}:${index}`} data-tone={left === null ? "unknown" : left <= 10 ? "low" : left <= 30 ? "mid" : "ok"}>
          <div><b>{name}</b><strong>{left === null ? t("待刷新") : tf("剩余 {0}%", [left])}</strong></div>
          <div className="gw-account-reset-time">{window.resetsAtMs ? <><time dateTime={new Date(window.resetsAtMs).toISOString()}>{quotaDate(window.resetsAtMs)}</time><span>{window.resetsAtMs > now ? tf("{0} 后重置", [formatRemaining(window.resetsAtMs - now)]) : t("已到重置时间，请刷新")}</span></> : t("未提供重置时间")}</div>
          <div className="gw-account-progress" role="progressbar" aria-label={tf("{0}剩余额度", [name])} aria-valuemin={0} aria-valuemax={100} aria-valuenow={left ?? undefined} aria-valuetext={left === null ? t("待刷新") : undefined}><i style={{ width: `${left ?? 0}%` }} /></div>
        </div>;
      }) : <p className="gw-muted">{entry.account ? t("尚未取得官方用量") : t("尚未取得套餐用量")}</p>}
    </div>
    {entry.account === "chatgpt" ? <span className="gw-muted">{t("Credit 状态")}{t("：")}{quota?.creditsAvailable == null ? t("未知") : quota.creditsAvailable ? t("可用") : t("不可用")}</span> : null}
  </div>;
}

export function QuotaCards({ entries, busy, refresh }: {
  entries: GatewayPoolEntry[];
  busy?: string | null;
  refresh?: (entry: GatewayPoolEntry) => void;
}) {
  const now = useQuotaClock();
  return <div className="gw-quota-cards">{entries.map(entry =>
    <QuotaCard key={entry.id} entry={entry} now={now} busy={busy} refresh={refresh} />
  )}</div>;
}

function useQuotaHistory(entry: GatewayPoolEntry) {
  const [loaded, setLoaded] = useState<{ id: string; data: QuotaObservation[] | null }>();
  useEffect(() => {
    let active = true;
    const id = entry.id;
    invoke<QuotaObservation[]>("load_gateway_quota_history", { id }).then(
      data => { if (active) setLoaded({ id, data }); },
      () => { if (active) setLoaded({ id, data: null }); },
    );
    return () => { active = false; };
  }, [entry.id, entry.quota?.updatedAtMs]);
  return loaded?.id === entry.id ? loaded : undefined;
}

function QuotaCard({ entry, now, busy, refresh }: {
  entry: GatewayPoolEntry; now: number; busy?: string | null; refresh?: (entry: GatewayPoolEntry) => void;
}) {
  const quota = entry.quota;
  const state = useQuotaHistory(entry);
  const history = state?.data ?? [];
  const windows = new Map<number | null, QuotaWindow>();
  for (const point of [...history].sort((a, b) => a.observedAtMs - b.observedAtMs)) windows.set(point.windowMinutes, point);
  for (const window of quota?.windows ?? []) windows.set(window.windowMinutes, window);
  return <article className="gw-quota-card">
    <div className="gw-quota-card-head">
      <div><h3>{maskAccountName(entry.name)}</h3>{entry.plan && <span>{entry.plan}</span>}</div>
      {refresh && <Button size="icon" variant="ghost" aria-label={t("刷新额度")} title={t("刷新额度")} disabled={busy != null} onClick={() => refresh(entry)}>
        <RefreshCw className={busy === `quota:${entry.id}` ? "animate-spin" : undefined} />
      </Button>}
    </div>
    {state?.data === null && <p className="gw-quota-card-note">{t("无法读取本地额度历史")}</p>}
    {!windows.size && <p className="gw-quota-card-note">{state ? t("等待首次额度采样") : t("读取中…")}</p>}
    {windows.size > 0 && <QuotaChart entry={entry} windows={[...windows.values()]} history={history} now={now} />}
  </article>;
}

function QuotaChart({ entry, history, windows, now }: { entry: GatewayPoolEntry; history: QuotaObservation[]; windows: QuotaWindow[]; now: number }) {
  const [selected, setSelected] = useState<number | "all">("all");
  const [page, setPage] = useState<"trend" | "calendar">("trend");
  const [showRecent, setShowRecent] = useState(false);
  const [hover, setHover] = useState<number | null>(null);
  const tooltipId = useId();
  const quota = entry.quota;
  const age = entry.subscriptionPolicy?.maxAgeSeconds;
  const percent = (value: number) => value.toLocaleString(undefined, { maximumFractionDigits: 2 });
  const series = [...windows].sort((a, b) => (a.windowMinutes ?? Infinity) - (b.windowMinutes ?? Infinity)).map((window, index) => {
    const minutes = window.windowMinutes;
    const cycle = quotaCycle(history, window, quota?.updatedAtMs ?? 0, now);
    const current = quota?.windows.find(item => item.windowMinutes === minutes);
    return {
      key: minutes ?? -1, tone: index % 5, cycle, minutes,
      name: windowName(window),
      last: cycle.points.at(-1),
      fresh: Boolean(quota && current && quotaPercentLeft(quota, current, now, age) !== null),
      trend: quotaTrend(cycle.points, minutes, now, quota, age),
      pace: quotaPace(quota, minutes, now, age),
    };
  });
  const focused = series.find(item => item.key === selected);
  const visible = focused ? [focused] : series;
  const calendarWindow = focused ?? series[0];
  const single = visible.length === 1 ? visible[0] : undefined;
  const canShowRecent = Boolean(single?.cycle.current && single.cycle.cycles.length > 1);
  const plotted = visible.map(item => ({
    ...item,
    cycles: canShowRecent && showRecent ? item.cycle.cycles.slice(-4) : item.cycle.current ? [item.cycle.current] : [],
  }));
  const start = Math.min(...plotted.map(item => {
    const first = item.cycles[0];
    return Math.min(item.cycle.start, first?.fullAt ?? first?.points[0]?.observedAtMs ?? Infinity);
  }));
  const end = Math.max(...visible.map(item => item.cycle.end));
  const short = !focused && visible.length > 1 ? visible.find(item =>
    item.cycle.current?.fullAt !== undefined && item.cycle.current.fullAt <= now && item.cycle.end > now) : undefined;
  const shortStart = short?.cycle.current?.fullAt;
  const band = short && shortStart !== undefined ? { start: shortStart, end: short.cycle.end } : undefined;
  const scale = quotaTimeScale(start, end, band);
  const x = (ms: number) => 2 + 476 * scale.position(ms);
  const y = (left: number) => 4 + (100 - left) * .96;
  const path = (points: { at: number; left: number }[]) => {
    const vertices = points.flatMap((point, i) => {
      const before = points[i - 1];
      const joins = before ? scale.breakpoints.filter(at => before.at < at && at < point.at).map(at => ({
        at, left: before.left + (point.left - before.left) * (at - before.at) / (point.at - before.at),
      })) : [];
      return [...joins, point];
    });
    return `M${vertices.map(p => `${x(p.at)},${y(p.left)}`).join(" L")}${vertices.length === 1 ? " h0.01" : ""}`;
  };
  const stops = [...new Set(plotted.flatMap(item => [
    ...item.cycles.flatMap(cycle => [cycle.fullAt, ...cycle.points.map(point => point.observedAtMs)]),
    item.trend.forecast?.endAt,
  ]).filter((at): at is number => at !== undefined))].sort((a, b) => a - b);
  const inspected = hover === null ? [] : plotted.map(item => ({
    ...item, reading: quotaCursorReading(item.cycles, hover, item.trend.forecast),
  }));
  const hasFullStart = plotted.some(item => item.cycles.some(cycle => cycle.fullAt !== undefined && cycle.fullAt < cycle.points[0].observedAtMs));
  const hasProjection = visible.some(item => item.trend.forecast);
  const chooseWindow = (key: number | "all") => { setSelected(key); setShowRecent(false); setHover(null); };
  const choosePage = (next: typeof page) => {
    if (next === "calendar" && !focused) chooseWindow(calendarWindow.key);
    setHover(null);
    setPage(next);
  };
  return <div className="gw-quota-trend">
    <div className="gw-quota-meters">
      {series.map(({ key, tone, name, last, pace, fresh }) => <div className="gw-quota-meter" data-series={key} data-tone={tone} key={key}>
        <div><span><i />{name}</span><strong>{last && fresh ? tf("剩余 {0}%", [percent(100 - last.usedPercent)]) : t(last ? "待刷新" : "未知")}</strong></div>
        <div className="gw-account-progress" aria-hidden="true"><i style={{ width: `${last && fresh ? 100 - last.usedPercent : 0}%` }} /></div>
        <small>{last?.resetsAtMs ? `${t("重置")} ${quotaDate(last.resetsAtMs)}` : t("未提供重置时间")}
          {pace != null && ` · ${Math.abs(pace) < 1 ? t("接近匀速基线") : tf(pace > 0 ? "比匀速基线多用 {0} 个百分点" : "比匀速基线少用 {0} 个百分点", [percent(Math.abs(pace))])}`}</small>
      </div>)}
    </div>
    <div className="gw-quota-view-controls">
      {series.length > 1 ? <div className="mg-segmented" role="group" aria-label={t("额度窗口")}>
        {(page === "trend" ? [{ key: "all" as const, name: t("总览") }, ...series] : series).map(({ key, name }) => {
          const active = (page === "calendar" ? calendarWindow.key : focused?.key ?? "all") === key;
          return <button key={key} type="button" aria-pressed={active} data-active={active} onClick={() => chooseWindow(key)}>{name}</button>;
        })}
      </div> : <b>{series[0].name}</b>}
      <div className="mg-segmented gw-quota-view-picker" role="tablist" aria-label={t("额度视图")}
        onKeyDown={event => {
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          const next = page === "trend" ? "calendar" : "trend";
          event.currentTarget.querySelector<HTMLButtonElement>(`[data-view="${next}"]`)?.focus();
          choosePage(next);
        }}>
        {([{ id: "trend", label: "走势", Icon: ChartLine }, { id: "calendar", label: "日历", Icon: CalendarDays }] as const).map(({ id, label, Icon }) =>
          <button key={id} type="button" role="tab" id={`${tooltipId}-${id}`} data-view={id} aria-selected={page === id}
            aria-controls={`${tooltipId}-panel`} data-active={page === id} tabIndex={page === id ? 0 : -1} onClick={() => choosePage(id)}>
            <Icon aria-hidden="true" />{t(label)}
          </button>)}
      </div>
    </div>
    <div id={`${tooltipId}-panel`} className="gw-quota-view-panel" role="tabpanel" aria-labelledby={`${tooltipId}-${page}`}>
      {page === "calendar" ? <QuotaCalendar key={calendarWindow.key} entry={entry} minutes={calendarWindow.minutes}
        cycle={calendarWindow.cycle} now={now} formatDate={quotaDate} /> : <>
      <div className="gw-quota-curve-heading">
        <h4>{t("剩余额度走势")}</h4>
        {canShowRecent && <div className="mg-segmented" role="group" aria-label={t("额度图表范围")}>
          {[false, true].map(recent => <button key={String(recent)} type="button" aria-pressed={showRecent === recent}
            data-active={showRecent === recent} onClick={() => { setShowRecent(recent); setHover(null); }}>
            {t(recent ? "最近周期" : "本周期")}
          </button>)}
        </div>}
      </div>
    <div className="gw-quota-curve">
      <div className="gw-quota-curve-values" aria-hidden="true"><span>100%</span><span>50%</span><span>0%</span></div>
      <div className="gw-quota-curve-plot">
      <svg viewBox="0 0 480 104" preserveAspectRatio="none" role="img" aria-label={t("剩余额度走势")}
        tabIndex={stops.length ? 0 : undefined} aria-describedby={hover !== null ? tooltipId : undefined}
        onPointerMove={event => {
          const rect = event.currentTarget.getBoundingClientRect();
          if (!rect.width) return;
          const curveX = (event.clientX - rect.left) / rect.width * 480;
          const nearest = stops.reduce<number | undefined>((a, b) =>
            a === undefined || Math.abs(x(b) - curveX) < Math.abs(x(a) - curveX) ? b : a, undefined);
          setHover(nearest !== undefined && Math.abs(x(nearest) - curveX) * rect.width / 480 <= 5
            ? nearest : scale.timeAt((curveX - 2) / 476));
        }} onPointerLeave={() => setHover(null)} onBlur={() => setHover(null)}
        onKeyDown={event => {
          if (event.key === "Escape") { setHover(null); return; }
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          const next = hover === null ? stops.at(event.key === "ArrowLeft" ? -1 : 0)
            : event.key === "ArrowRight" ? stops.find(at => at > hover)
              : stops.reduce<number | undefined>((previous, at) => at < hover ? at : previous, undefined);
          if (next !== undefined) setHover(next);
        }}>
        {band && short && <g data-tone={short.tone} aria-hidden="true">
          <rect className="gw-quota-focus-band" x={x(band.start)} y="4" width={x(band.end) - x(band.start)} height="96" />
          {[band.start, band.end].map(at => <line key={at} className="gw-quota-focus-edge" x1={x(at)} x2={x(at)} y1="4" y2="100" />)}
        </g>}
        {[0, 50, 100].map(value => <line key={value} x1="2" x2="478" y1={y(value)} y2={y(value)} className="gw-quota-trend-grid" />)}
        {now > start && now < end && <line x1={x(now)} x2={x(now)} y1="4" y2="100" className="gw-quota-curve-now" />}
        {plotted.map(({ key, tone, cycles, last, trend: { forecast } }) => <g data-series={key} data-tone={tone} key={key}>
          {cycles.map(({ points, fullAt }) => {
            const first = points[0];
            return <g key={first.observedAtMs} data-quota-cycle>
              {fullAt !== undefined && fullAt < first.observedAtMs && <>
                <path className="gw-quota-trend-start" d={path([{ at: fullAt, left: 100 }, { at: first.observedAtMs, left: 100 - first.usedPercent }])} />
                <line className="gw-quota-trend-start-dot" x1={x(fullAt)} x2={x(fullAt)} y1={y(100)} y2={y(100)} />
              </>}
              <path className="gw-quota-trend-reading"
                d={path(points.map(point => ({ at: point.observedAtMs, left: 100 - point.usedPercent })))} />
            </g>;
          })}
          {forecast && last && <path className="gw-quota-trend-projection" d={path([
            { at: last.observedAtMs, left: 100 - last.usedPercent }, { at: forecast.endAt, left: forecast.endLeft },
          ])} />}
          {last && <line className="gw-quota-trend-dot" x1={x(last.observedAtMs)} x2={x(last.observedAtMs)}
            y1={y(100 - last.usedPercent)} y2={y(100 - last.usedPercent)} />}
        </g>)}
        {hover != null && <line x1={x(hover)} x2={x(hover)} y1="4" y2="100" className="gw-quota-curve-crosshair" />}
        {inspected.map(({ key, tone, reading }) => reading && <line key={key} data-tone={tone} data-kind={reading.kind}
          x1={x(reading.at)} x2={x(reading.at)} y1={y(reading.left)} y2={y(reading.left)} className="gw-quota-inspect-point" />)}
      </svg>
      {hover !== null && <div id={tooltipId} className="gw-quota-inspect" role="tooltip" style={{
        left: `clamp(0px, calc(${x(hover) / 4.8}% + ${scale.position(hover) > .6 ? -224 : 8}px), max(0px, calc(100% - 216px)))`,
      }}>
        <time>{quotaDate(hover)}</time>
        {inspected.map(({ key, tone, name, reading }) => <div key={key} className="gw-quota-inspect-row" data-tone={tone}>
          <b><i />{name}</b>{reading ? <>
            <strong>{percent(reading.left)}%</strong>
            <small>{t(reading.kind === "sample" ? "额度实测" : reading.kind === "start" ? "推定满额起点" : "速率推算")}
              {reading.kind !== "forecast" && ` · ${quotaDate(reading.at)}`}</small>
          </> : <small>{t("该时刻无读数")}</small>}
        </div>)}
      </div>}
      </div>
      <div className="gw-quota-trend-axis"><span>{quotaDate(start)}</span><span>{quotaDate(end)}{end > now ? ` · ${t("重置")}` : ""}</span></div>
    </div>
    {band && short && <div className="gw-quota-focus-caption" data-tone={short.tone}>
      <b><i />{short.name}</b>
      <span>{t("开始")} <time>{quotaDate(band.start)}</time></span>
      <span>{t("重置")} <time>{quotaDate(band.end)}</time></span>
      {scale.expanded && <span className="gw-quota-focus-scale">{t("分段时间轴 · 短窗口放大")}</span>}
    </div>}
    <div className="gw-quota-trend-key"><span>{t("额度实测")}</span>
      {hasFullStart && <span data-line="start">{t("推定满额起点")}</span>}
      {hasProjection && <span data-line="projection">{t("速率推算")}</span>}
    </div>
    <div className="gw-quota-curve-readings">
      {plotted.map(({ key, tone, name, last, cycle, trend: { forecast, reason } }) => {
        const eta = forecast ? (forecast.endsEmpty ? tf("按此速度，约 {0} 用完", [quotaDate(forecast.endAt)])
          : forecast.reachesReset ? tf("重置时预计剩余 {0}%", [percent(forecast.endLeft)])
            : tf("短期推算：{0} 剩余 {1}%", [quotaDate(forecast.endAt), percent(forecast.endLeft)]))
          : !last ? t("等待首次额度采样") : reason === "stale" ? t("最近读数已过期，暂不预测")
            : reason === "flat" ? t("消耗变化太小，暂不预测") : reason === "exhausted" ? t("最近实测已用完")
              : tf("已记录 {0} 个样本，暂不预测", [cycle.points.length]);
        return <div className="gw-quota-curve-reading" data-series={key} data-tone={tone} key={key}>
          <b><i />{name}</b><span>{last ? `${percent(100 - last.usedPercent)}% · ${quotaDate(last.observedAtMs)}` : t("未报告")}</span>
          <span data-forecast={Boolean(forecast)} title={forecast ? tf("近期每小时消耗约 {0} 个百分点", [percent(forecast.perHour)]) : undefined}>{eta}</span>
        </div>;
      })}
    </div>
      </>}
    </div>
  </div>;
}
