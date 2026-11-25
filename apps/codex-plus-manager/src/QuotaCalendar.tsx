import { useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "./components/ui/button";
import { type ResetCredit, type GatewayPoolEntry } from "./gateway-model";
import { getLanguage, t, tf } from "./i18n";
import { quotaCalendarRange, quotaCalendarSlots, quotaPercentLeft, type quotaCycle } from "./quota-timeline";
import "./QuotaCalendar.css";

type Range = ReturnType<typeof quotaCalendarRange>;
type Detail = { dates: string; status: string };
const slotLabel = { current: "当前窗口", history: "已记录窗口", projected: "后续窗口（推算）" } as const;

function percentAt(at: number, range: Range) {
  return `${100 * (at - range.start) / (range.end - range.start)}%`;
}

function resetCardUsable(card: ResetCredit, now: number) {
  return card.usable && card.status === "available" && (card.expiresAtMs == null || card.expiresAtMs > now);
}

export function QuotaCalendar({ entry, minutes, cycle, now, formatDate: quotaDate }: {
  entry: GatewayPoolEntry; minutes: number | null; cycle: ReturnType<typeof quotaCycle>; now: number; formatDate: (at: number) => string;
}) {
  const [offset, setOffset] = useState(0);
  const [inspected, setInspected] = useState<Detail>();
  const range = quotaCalendarRange(now, minutes, offset);
  const locale = getLanguage() === "en" ? "en-US" : "zh-CN";
  const date = (at: number) => new Date(at).toLocaleDateString(locale, { month: "2-digit", day: "2-digit" });
  const hour = (at: number) => new Date(at).toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  const inRange = now >= range.start && now < range.end;
  const move = (page: number) => { setOffset(page); setInspected(undefined); };
  const slots = quotaCalendarSlots(cycle, minutes, now, range);
  const quota = entry.quota;
  const window = quota?.windows.find(window => window.windowMinutes === minutes);
  const left = quota && window ? quotaPercentLeft(quota, window, now, entry.subscriptionPolicy?.maxAgeSeconds) : null;
  const hasReset = cycle.cycles.some(group => group.points.at(-1)?.resetsAtMs != null);
  const resetToCurrent = range.mode === "day" ? "回到本日" : "回到本期";
  const currentPeriod = range.mode === "day" ? "本日" : "本期";

  return <div className="gw-quota-calendar" data-range={range.mode}>
    <div className="gw-quota-calendar-toolbar">
      <div className="gw-quota-calendar-range" aria-live="polite">
        <strong>{date(range.start)}{range.mode !== "day" && ` – ${date(range.end - 1)}`}</strong>
        <span>{Intl.DateTimeFormat().resolvedOptions().timeZone}</span>
      </div>
      <div className="gw-quota-calendar-nav">
        <Button variant="ghost" size="icon" aria-label={t("上一时段")} title={t("上一时段")} onClick={() => move(offset - 1)}><ChevronLeft /></Button>
        {offset === 0 ? <span>{t(currentPeriod)}</span> : <Button variant="ghost" onClick={() => move(0)}>{t(resetToCurrent)}</Button>}
        <Button variant="ghost" size="icon" aria-label={t("下一时段")} title={t("下一时段")} onClick={() => move(offset + 1)}><ChevronRight /></Button>
      </div>
    </div>
    <div className="gw-quota-calendar-scroll">
      <div className="gw-quota-calendar-grid" role="group" aria-label={t("窗口日历")}>
        <div className="gw-quota-calendar-axis">
          {range.ticks.map(at => <span key={at} data-edge={at > range.end - (range.end - range.start) * .06 || undefined}
            style={{ left: percentAt(at, range) }}>
            {range.mode === "day" ? hour(at) : date(at)}
          </span>)}
        </div>
        <div className="gw-quota-calendar-track">
          {range.ticks.slice(1).map(at => <i key={at} aria-hidden="true" className="gw-quota-calendar-tick" style={{ left: percentAt(at, range) }} />)}
          {inRange && <i className="gw-quota-calendar-now" aria-hidden="true" style={{ left: percentAt(now, range) }} />}
          {slots.map((slot, index) => {
            const marker = slot.start === undefined;
            const remaining = slot.kind === "current" ? left : slot.kind === "history" && slot.sample
              ? Math.round(100 - slot.sample.usedPercent) : null;
            const from = Math.max(slot.start ?? slot.resetAt, range.start);
            const to = Math.min(slot.resetAt, range.end);
            const dates = marker ? quotaDate(slot.resetAt) : `${quotaDate(slot.start!)} – ${quotaDate(slot.resetAt)}`;
            const label = t(slotLabel[slot.kind]);
            const reading = remaining !== null ? tf("剩余 {0}%", [remaining]) : slot.kind === "current" ? t("待刷新") : "";
            const status = [label, slot.kind === "history" && slot.sample && remaining !== null
              ? tf("{0} 实测剩余 {1}%", [quotaDate(slot.sample.observedAtMs), remaining]) : reading].filter(Boolean).join(" · ");
            const inspect = () => setInspected({ dates, status });
            return <button type="button" className="gw-quota-calendar-slot" key={`${slot.kind}:${slot.resetAt}:${index}`}
              data-kind={slot.kind} data-marker={marker || undefined} data-stale={slot.kind === "current" && left === null || undefined}
              style={{ left: percentAt(from, range), width: marker ? undefined : `${100 * (to - from) / (range.end - range.start)}%` }}
              title={`${status} · ${dates}`} aria-label={`${status} · ${dates}`}
              onPointerEnter={inspect} onFocus={inspect} onClick={inspect}>
              {!marker && <span>{reading || (slot.kind === "projected" ? t("预计") : label)}</span>}
            </button>;
          })}
          {!slots.length && <span className="gw-quota-calendar-empty">{t(hasReset ? "此时段无已知窗口" : "未提供重置时间")}</span>}
          {(entry.resetCredits?.credits ?? []).filter(card => resetCardUsable(card, now) &&
            card.expiresAtMs != null && card.expiresAtMs >= range.start && card.expiresAtMs < range.end).map(card => {
            const at = card.expiresAtMs!;
            const dates = quotaDate(at);
            const status = t("主动重置到期");
            const inspect = () => setInspected({ dates, status });
            return <button key={card.id} type="button" className="gw-quota-calendar-expiry" style={{ left: percentAt(at, range) }}
              aria-label={`${status} · ${dates}`} title={`${status} · ${dates}`}
              onPointerEnter={inspect} onFocus={inspect} onClick={inspect} />;
          })}
        </div>
      </div>
    </div>
    <div className="gw-quota-calendar-footer">
      <div className="gw-quota-calendar-key">
        {Object.entries(slotLabel).map(([kind, label]) => <span key={kind}><i data-kind={kind} />{t(label)}</span>)}
        <span><i data-kind="expiry" />{t("主动重置到期")}</span>
        {inRange && <span><i data-kind="now" />{t("当前时间")}</span>}
      </div>
      {inspected && <div className="gw-quota-calendar-detail"><b>{inspected.dates}</b><span>{inspected.status}</span></div>}
    </div>
  </div>;
}
