import { useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { ArrowRight, X } from "lucide-react";
import { AppSelect } from "./AppSelect";
import { CxEmpty } from "./cx-layout";
import { agentName, groupDisplayName, maskAccountName, prettyModelName, type GatewayPayload, type UsageRange, type UsageRouteDay, type UsageSummary } from "./gateway-model";
import { getLanguage, t, tf } from "./i18n";
import { openRequestLog, requestLogAgent, type RequestLogFocus } from "./UsageTabs";
import {
  FLOW_HEIGHT, USAGE_DIMENSIONS, calendarValue, heatLevel, matchUsageFilters, usageCalendar, usageFlow, visualDays, weekDay,
  type UsageCalendarDay, type UsageDimension, type UsageNode, type UsageVisualFilters, type UsageVisualMetric,
} from "./usage-visuals-model";
import "./UsageVisuals.css";

const NO_ROUTES: UsageRouteDay[] = [];
const FLOW_WIDTH = 900;
const FLOW_X = [161, 466, 701] as const;
const NODE_WIDTH = 8;
const ALL = "*";

const numberFormats = (locale: string) => [
  new Intl.NumberFormat(locale),
  new Intl.NumberFormat(locale, { notation: "compact", maximumFractionDigits: 1 }),
] as const;
const numbers = { zh: numberFormats("zh-CN"), en: numberFormats("en-US") };
const numberText = (value: number, compact = false) => numbers[getLanguage()][compact ? 1 : 0].format(value);
const metricLabel = (metric: UsageVisualMetric) => metric === "calls" ? t("请求") : "Token";
const dateText = (date: string, options: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat(
  getLanguage() === "zh" ? "zh-CN" : "en-US", { ...options, timeZone: "UTC" },
).format(new Date(`${date}T00:00:00Z`));
const dimensionLabels = () => ({ agent: "Agent", providerId: t("供应商"), model: t("记账模型") });
const allDimensionLabels = () => ({ agent: t("全部 Agent"), providerId: t("全部供应商"), model: t("全部记账模型") });

export function UsageVisuals({ summary, range, failed, payload }: {
  summary: UsageSummary | null;
  range: UsageRange;
  failed: boolean;
  payload: GatewayPayload | null;
}) {
  const joined = summary?.dailyRoutes;
  const language = getLanguage();
  const [metric, setMetric] = useState<UsageVisualMetric>("calls");
  const [userFilters, setFilters] = useState<UsageVisualFilters>({});
  const [selection, setSelection] = useState<{ date: string; range: UsageRange } | null>(null);
  const rows = useMemo(() => joined ?? NO_ROUTES, [joined]);
  const filters = userFilters;
  const days = useMemo(() => visualDays(summary?.days.map(day => day.date) ?? []), [summary]);
  const calendar = useMemo(() => usageCalendar(days, rows, filters), [days, rows, filters]);
  const date = selection?.range === range && days.includes(selection.date) ? selection.date : undefined;
  const selectedRows = useMemo(() => rows.filter(row => (!date || row.date === date) && matchUsageFilters(row, filters)), [date, filters, rows]);
  const flow = useMemo(() => usageFlow(selectedRows, metric), [selectedRows, metric]);
  const metrics: UsageVisualMetric[] = ["calls", "tokens"];
  const names = useMemo(() => {
    const maps: Record<UsageDimension, Map<string, string>> = { agent: new Map(), providerId: new Map(), model: new Map() };
    for (const row of rows) {
      maps.agent.set(row.agent, row.agent ? agentName(row.agent) : t("未知"));
      maps.providerId.set(row.providerId, row.providerId ? maskAccountName(row.provider || row.providerId) : t("未知"));
      maps.model.set(row.model, row.model ? groupDisplayName(payload, row.model) ?? prettyModelName(row.model) : t("未知模型"));
    }
    return maps;
  }, [rows, payload, language]);
  const updateFilter = (dimension: UsageDimension, key: string | undefined) =>
    setFilters(previous => ({ ...previous, [dimension]: key }));
  const clear = () => {
    setFilters({});
    setSelection(null);
  };

  const labels = dimensionLabels();
  const allLabels = allDimensionLabels();
  const canOpenLog = selectedRows.length > 0 && !date && (filters.agent === undefined || !!requestLogAgent(filters.agent)) && filters.model !== "";
  const logFocus: RequestLogFocus = {
    agent: requestLogAgent(filters.agent), range, model: filters.model,
    relayId: filters.providerId || undefined,
    relayLabel: filters.providerId ? names.providerId.get(filters.providerId) : undefined,
    relayUnknown: filters.providerId === "",
  };

  return (
    <div className="usage-visuals">
      {joined === undefined ? <CxEmpty title={failed ? t("读取用量失败") : summary ? t("当前后端未提供用量分布") : t("正在读取用量")} /> : <>
        <div className="usage-visual-controls">
          <div className="mg-segmented usage-visual-metric" role="group" aria-label={t("计量单位")}>
            {metrics.map(option => <button type="button" key={option} data-active={metric === option}
              aria-pressed={metric === option} onClick={() => setMetric(option)}>{metricLabel(option)}</button>)}
          </div>
          {USAGE_DIMENSIONS.map(dimension => {
            const key = filters[dimension];
            const choices = [...names[dimension]].sort((a, b) => a[1].localeCompare(b[1]));
            if (key !== undefined && !names[dimension].has(key)) choices.push([key, (dimension === "providerId" ? maskAccountName(key) : key) || t("未知")]);
            return <AppSelect key={dimension} size="sm" className="usage-visual-filter" ariaLabel={labels[dimension]}
              value={key === undefined ? ALL : `:${key}`}
              options={[{ value: ALL, label: allLabels[dimension] }, ...choices.map(([value, label]) => ({ value: `:${value}`, label }))]}
              onChange={value => updateFilter(dimension, value === ALL ? undefined : value.slice(1))} />;
          })}
          {date || USAGE_DIMENSIONS.some(dimension => userFilters[dimension] !== undefined) ? <button type="button" className="usage-visual-clear"
            title={t("清除用量筛选")} aria-label={t("清除用量筛选")} onClick={clear}><X size={15} /></button> : null}
        </div>
        <div className="usage-visual-grid">
        <div className="usage-visual-band">
          <div className="usage-flow-heading">
            <h3>{t("用量热力图")}</h3>
            {summary && summary.days.length > days.length ? <span className="usage-visual-note">{t("最近 53 周")}</span> : null}
          </div>
          {calendar.length ? <UsageHeatCalendar calendar={calendar} metric={metric} selected={date} onSelect={next =>
            setSelection(next === date ? null : { date: next, range })} /> : <CxEmpty title={t("没有匹配的请求记录")} />}
        </div>
        <div className="usage-visual-band">
          <div className="usage-flow-heading">
            <h3>{t("用量流向")}</h3>
            <div className="usage-flow-actions">
              {date && <button type="button" className="usage-visual-date" onClick={() => setSelection(null)}
                aria-label={t("清除日期筛选")} title={t("清除日期筛选")}>{date}<X size={13} /></button>}
              {canOpenLog ? <button type="button" className="usage-visual-open" onClick={() => openRequestLog(logFocus)}>
                {t("查看筛选内的请求")}<ArrowRight size={14} />
              </button> : null}
            </div>
          </div>
          {flow.total > 0 ? <UsageFlowDiagram flow={flow} metric={metric} names={names} filters={filters} toggle={(dimension, node) => {
            const active = filters[dimension] === node.key && (node.providerId === undefined || filters.providerId === node.providerId);
            if (node.providerId === undefined) updateFilter(dimension, active ? undefined : node.key);
            else setFilters(previous => ({ ...previous, providerId: active ? previous.providerId : node.providerId, model: active ? undefined : node.key }));
          }} /> : <CxEmpty title={!selectedRows.length || metric === "calls" ? t("没有匹配的请求记录") : t("没有可绘制的 Token 用量")} />}
          {metric === "tokens" && selectedRows.some(row => (row.tokenMeteredCalls ?? 0) < row.calls)
            ? <p className="usage-visual-note">{t("部分请求的 Token 用量未观测")}</p> : null}
        </div>
        </div>
      </>}
    </div>
  );
}

function UsageHeatCalendar({ calendar, metric, selected, onSelect }: {
  calendar: UsageCalendarDay[];
  metric: UsageVisualMetric;
  selected?: string;
  onSelect: (date: string) => void;
}) {
  const [focused, setFocused] = useState<string>();
  const [preview, setPreview] = useState<string>();
  const cells = useRef(new Map<string, HTMLButtonElement>());
  const scroller = useRef<HTMLDivElement>(null);
  const atLatest = useRef(true);
  const first = calendar[0].date;
  const last = calendar[calendar.length - 1].date;
  const singleDay = calendar.length === 1;
  const offset = singleDay ? 0 : weekDay(first);
  const rowCount = Math.min(7, offset + calendar.length);
  const columns = Math.ceil((offset + calendar.length) / 7);
  const values = calendar.map(day => calendarValue(day.usage, metric));
  const peak = Math.max(0, ...values.map(value => value ?? 0));
  const active = [focused, selected, last].find(date => calendar.some(day => day.date === date));
  const shown = [preview, selected, active, last].map(date => calendar.findIndex(day => day.date === date)).find(index => index >= 0)!;
  const weekdays = Array.from({ length: rowCount }, (_, day) => dateText(singleDay ? first : `2024-01-0${day + 1}`, { weekday: "short" }));
  const months = new Map<number, string>();
  calendar.forEach(({ date }, index) => {
    const column = Math.floor((index + offset) / 7);
    if (date.endsWith("-01")) months.set(column, dateText(date, { month: "short" }));
  });
  if ((months.keys().next().value ?? Infinity) >= 3) months.set(0, dateText(first, { month: "short" }));
  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const pinLatest = () => {
      if (atLatest.current) element.scrollLeft = element.scrollWidth - element.clientWidth;
    };
    atLatest.current = true;
    pinLatest();
    const observer = new ResizeObserver(pinLatest);
    observer.observe(element);
    return () => observer.disconnect();
  }, [first, last]);

  const details = (index: number) => {
    const day = calendar[index];
    const value = values[index];
    if (value === undefined) return `${day.date} · ${t("未观测")}`;
    return `${day.date} · ${numberText(value)} ${metricLabel(metric)}${metric === "tokens" && day.usage && day.usage.meteredCalls < day.usage.calls
      ? ` · ${t("部分计量")}` : ""}`;
  };
  const handleKey = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const weekday = (index + offset) % 7;
    const changes: Record<string, number> = { ArrowLeft: -7, ArrowRight: 7, ArrowUp: weekday > 0 ? -1 : 0,
      ArrowDown: weekday < 6 ? 1 : 0, Home: -index, End: calendar.length - index - 1 };
    const delta = changes[event.key];
    if (delta === undefined) return;
    event.preventDefault();
    setPreview(undefined);
    const next = calendar[Math.max(0, Math.min(calendar.length - 1, index + delta))].date;
    cells.current.get(next)?.focus({ preventScroll: true });
    cells.current.get(next)?.scrollIntoView({ block: "nearest", inline: "nearest" });
  };

  return <>
    <div className="usage-heat-readout">{details(shown)}</div>
    <div className="usage-heat-scroll" ref={scroller} onScroll={event => {
      const { scrollLeft, scrollWidth, clientWidth } = event.currentTarget;
      atLatest.current = scrollWidth - clientWidth - scrollLeft < 4;
    }}>
      <div className="usage-heat-grid" role="group" aria-label={t("用量热力图")} style={{ gridTemplateColumns: `minmax(2.75rem, auto) repeat(${columns}, var(--usage-heat-cell))` }}>
        {weekdays.map((text, index) => <span key={text} className="usage-heat-weekday" style={{ gridArea: `${index + 2} / 1` }}>{text}</span>)}
        {[...months].map(([column, month]) => <span key={column} className="usage-heat-month" style={{ gridArea: `1 / ${column + 2}` }}>{month}</span>)}
        {calendar.map((day, index) => <button key={day.date} type="button" ref={node => {
          if (node) cells.current.set(day.date, node);
          else cells.current.delete(day.date);
        }} style={{ gridArea: `${(index + offset) % 7 + 2} / ${Math.floor((index + offset) / 7) + 2}` }}
          title={details(index)} aria-label={details(index)} aria-pressed={day.date === selected}
          tabIndex={day.date === active ? 0 : -1} data-level={heatLevel(values[index], peak)}
          onClick={() => onSelect(day.date)} onFocus={() => setFocused(day.date)} onKeyDown={event => handleKey(event, index)}
          onMouseEnter={() => setPreview(day.date)} onMouseLeave={() => setPreview(undefined)} />)}
      </div>
    </div>
    <div className="usage-heat-legend" aria-label={t("用量图例")}>
      <i data-level="-1" aria-hidden="true" />{t("未观测")}
      <i data-level="0" aria-hidden="true" />0
      {[1, 2, 3, 4].map(level => <i data-level={level} aria-hidden="true" key={level} />)}
      <span>{t("较高")}</span>
    </div>
  </>;
}

function UsageFlowDiagram({ flow, metric, names, filters, toggle }: {
  flow: ReturnType<typeof usageFlow>;
  metric: UsageVisualMetric;
  names: Record<UsageDimension, Map<string, string>>;
  filters: UsageVisualFilters;
  toggle: (dimension: UsageDimension, node: UsageNode) => void;
}) {
  const [active, setActive] = useState<string>();
  const labels = dimensionLabels();
  const name = (dimension: UsageDimension, node: UsageNode) => node.key === undefined ? tf("其他（{0} 项）", [node.folded])
    : names[dimension].get(node.key) ?? t("未知");
  return <div className="usage-flow-scroll" role="region" aria-label={t("用量流向")} tabIndex={0}>
    <div className="usage-flow-inner">
      <div className="usage-flow-labels">{USAGE_DIMENSIONS.map(dimension => <span key={dimension}>{labels[dimension]}</span>)}</div>
      <div className="usage-flow-plot" style={{ height: FLOW_HEIGHT }}>
        <svg viewBox={`0 0 ${FLOW_WIDTH} ${FLOW_HEIGHT}`} preserveAspectRatio="none" aria-hidden="true">
          {flow.links.map(link => {
            const x = FLOW_X[link.stage] + NODE_WIDTH;
            const x2 = FLOW_X[link.stage + 1];
            const y = link.sourceTop;
            const y2 = link.targetTop;
            const middle = (x + x2) / 2;
            const lower = link.height;
            const provider = link.stage === 0 ? link.target : link.source;
            return <path key={link.id} className="usage-flow-link"
              data-tone={Math.max(0, flow.columns[1].indexOf(provider)) % 5}
              data-muted={!!active && active !== link.source.id && active !== link.target.id}
              d={`M${x},${y} C${middle},${y} ${middle},${y2} ${x2},${y2} L${x2},${y2 + lower} C${middle},${y2 + lower} ${middle},${y + lower} ${x},${y + lower} Z`}>
              <title>{name(USAGE_DIMENSIONS[link.stage], link.source)} → {name(USAGE_DIMENSIONS[link.stage + 1], link.target)} · {numberText(link.value)} {metricLabel(metric)}</title>
            </path>;
          })}
          {flow.columns.flatMap((column, index) => column.map((node, slot) => <rect key={node.id} className="usage-flow-node-mark"
            data-tone={index === 1 ? slot % 5 : undefined} x={FLOW_X[index]} y={node.y} width={NODE_WIDTH} height={Math.max(1, node.height)} rx="2" />))}
        </svg>
        {flow.columns.flatMap((column, index) => column.map(node => {
          const dimension = USAGE_DIMENSIONS[index];
          const label = name(dimension, node);
          const provider = node.providerId === undefined ? "" : ` · ${names.providerId.get(node.providerId) ?? t("未知")}`;
          const title = `${label}${provider} · ${numberText(node.value)} ${metricLabel(metric)}`;
          const props = { "data-stage": index, title, className: "usage-flow-node",
            style: { top: Math.min(FLOW_HEIGHT - 10, Math.max(10, node.y + node.height / 2)) } };
          const contents = <><span>{label}</span><small>{numberText(node.value, true)}</small></>;
          return node.key === undefined ? <div key={node.id} {...props} aria-label={title}>{contents}</div>
            : <button key={node.id} {...props} type="button" aria-label={title} aria-pressed={filters[dimension] === node.key
              && (node.providerId === undefined || filters.providerId === node.providerId)}
              onClick={() => { setActive(undefined); toggle(dimension, node); }}
              onFocus={() => setActive(node.id)} onBlur={() => setActive(undefined)}
              onMouseEnter={() => setActive(node.id)} onMouseLeave={() => setActive(undefined)}>{contents}</button>;
        }))}
      </div>
    </div>
  </div>;
}
