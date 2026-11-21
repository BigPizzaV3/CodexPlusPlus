import { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "./components/ui/button";
import { getLanguage, t, tf } from "./i18n";
import { formatRemaining, maskAccountName, providerEnabled, quotaWindowLabel, type GatewayPoolEntry } from "./gateway-model";
import { quotaPercentLeft, quotaRange, quotaSegments } from "./quota-timeline";
import "./ProviderQuotaPanel.css";

export function quotaDate(value: number): string {
  return new Date(value).toLocaleString(getLanguage() === "en" ? "en-US" : "zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false });
}

function useQuotaClock() {
  const [, setTick] = useState(0);
  useEffect(() => { const timer = setInterval(() => setTick(tick => tick + 1), 30_000); return () => clearInterval(timer); }, []);
  return Date.now();
}

/** 在原有供应商详情里补充剩余比例、进度和准确的重置时间。 */
export function AccountQuotaDetails({ entry }: { entry: GatewayPoolEntry }) {
  const now = useQuotaClock();
  const quota = entry.quota;
  return <div className="gw-account-quota-details">
    <div className="gw-account-windows">
      {quota?.windows.length ? quota.windows.map((window, index) => {
        const label = quotaWindowLabel(window);
        const name = label.args.length ? tf(label.text, label.args) : t(label.text);
        const left = quotaPercentLeft(quota, window, now, entry.subscriptionPolicy?.maxAgeSeconds);
        return <div className="gw-account-window" key={`${window.windowMinutes}:${index}`} data-tone={left === null ? "unknown" : left <= 10 ? "low" : left <= 30 ? "mid" : "ok"}>
          <div><b>{name}</b><strong>{left === null ? t("待刷新") : tf("剩余 {0}%", [left])}</strong></div>
          <div className="gw-account-reset-time">{window.resetsAtMs ? <><time dateTime={new Date(window.resetsAtMs).toISOString()}>{quotaDate(window.resetsAtMs)}</time><span>{window.resetsAtMs > now ? tf("{0} 后重置", [formatRemaining(window.resetsAtMs - now)]) : t("已到重置时间，请刷新")}</span></> : t("未提供重置时间")}</div>
          <div className="gw-account-progress" role="progressbar" aria-label={tf("{0}剩余额度", [name])} aria-valuemin={0} aria-valuemax={100} aria-valuenow={left ?? undefined} aria-valuetext={left === null ? t("待刷新") : undefined}><i style={{ width: `${left ?? 0}%` }} /></div>
        </div>;
      }) : <p className="gw-muted">{t("尚未取得官方用量")}</p>}
    </div>
    <span className="gw-muted">{t("Credit 状态")}{t("：")}{quota?.creditsAvailable == null ? t("未知") : quota.creditsAvailable ? t("可用") : t("不可用")}</span>
  </div>;
}

export function ProviderQuotaTimeline({ entries }: { entries: GatewayPoolEntry[] }) {
  const now = useQuotaClock();
  const [selectedMode, setMode] = useState<"week" | "fiveHour" | null>(null);
  const [offset, setOffset] = useState(0);
  const accounts = entries.filter(entry => entry.account === "chatgpt");
  if (!accounts.length) return null;
  const hasWindow = (minutes: number) => accounts.some(entry => entry.quota?.windows.some(window => window.windowMinutes === minutes));
  const mode = selectedMode ?? (!hasWindow(10080) && hasWindow(300) ? "fiveHour" : "week");
  const windowMinutes = mode === "week" ? 10080 : 300;
  const range = quotaRange(now, mode, offset);
  const expiryMarkers = accounts.map(entry => (entry.resetCredits?.credits ?? []).flatMap(card =>
    card.status === "available" && card.usable && card.expiresAtMs != null &&
    card.expiresAtMs > now && card.expiresAtMs >= range.start && card.expiresAtMs < range.end
      ? [{ id: card.id, expiresAtMs: card.expiresAtMs }]
      : []
  ));
  const position = (at: number) => `${100 * (at - range.start) / (range.end - range.start)}%`;
  const width = (duration: number) => `${100 * duration / (range.end - range.start)}%`;
  const date = (at: number) => new Date(at).toLocaleDateString(getLanguage() === "en" ? "en-US" : "zh-CN", { month: "2-digit", day: "2-digit" });
  return <section className="gw-quota-timeline" aria-label={t("配额窗口")}>
    <header><div><h2>{t("配额窗口")}</h2><p>{date(range.start)} – {date(range.end - 1)} · {mode === "week" ? t("两周") : t("24 小时")} · {Intl.DateTimeFormat().resolvedOptions().timeZone}</p></div>
      <div className="gw-quota-controls"><div><Button variant="ghost" size="icon" aria-label={t("上一时段")} onClick={() => setOffset(offset - 1)}><ChevronLeft /></Button><Button variant="ghost" onClick={() => setOffset(0)}>{t("今天")}</Button><Button variant="ghost" size="icon" aria-label={t("下一时段")} onClick={() => setOffset(offset + 1)}><ChevronRight /></Button></div>
        <div>{(["week", "fiveHour"] as const).map(value => <Button key={value} variant={mode === value ? "secondary" : "ghost"} aria-pressed={mode === value} onClick={() => { setMode(value); setOffset(0); }}>{value === "week" ? t("按周") : t("5 小时")}</Button>)}</div>
      </div>
    </header>
    <div className="gw-quota-table-scroll" tabIndex={0} role="region" aria-label={t("配额时间轴")}><table className="gw-quota-table"><thead><tr><th scope="col">{t("账号")}</th><th scope="col"><div className="gw-quota-ticks">{range.ticks.slice(0, -1).map((at, index) => <span key={at} style={{ flexBasis: width(range.ticks[index + 1] - at) }}>{mode === "week" ? <><small>{new Date(at).toLocaleDateString(getLanguage() === "en" ? "en-US" : "zh-CN", { weekday: "short" })}</small>{date(at)}</> : new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}</span>)}</div></th></tr></thead>
      <tbody>{accounts.map((entry, index) => {
        const quota = entry.quota;
        const window = quota?.windows.find(item => item.windowMinutes === windowMinutes);
        const left = quota && window ? quotaPercentLeft(quota, window, now, entry.subscriptionPolicy?.maxAgeSeconds) : null;
        const segments = window ? quotaSegments(window, range.start, range.end) : [];
        return <tr key={entry.id} data-off={!providerEnabled(entry)}><th scope="row"><strong title={maskAccountName(entry.name)}>{maskAccountName(entry.name)}</strong><small>{left === null ? t("额度待刷新") : tf("剩余 {0}%", [left])} · {entry.plan || t("未知")}</small></th>
          <td><div className="gw-quota-track">
            {range.ticks.slice(1, -1).map(at => <i aria-hidden="true" className="gw-quota-grid-line" key={at} style={{ left: position(at) }} />)}
            {segments.length ? segments.map(segment => <span className="gw-quota-segment" key={segment.resetAt} data-projected={segment.projected} data-stale={!segment.projected && left === null} style={{ left: position(segment.start), width: width(segment.end - segment.start) }} title={`${segment.projected ? t("后续窗口（推算）") : t("已知窗口")} · ${tf("自然重置 {0}", [quotaDate(segment.resetAt)])}`}>
              {segment.projected ? t("预计") : left === null ? t("待刷新") : tf("剩余 {0}%", [left])} · {quotaDate(segment.resetAt)}
            </span>) : <span className="gw-quota-no-data">{window?.resetsAtMs ? t("此时段无已知窗口") : t("未提供重置时间")}</span>}
            {expiryMarkers[index].map(card => <i className="gw-quota-expiry-marker" key={card.id} style={{ left: position(card.expiresAtMs) }} title={tf("重置卡到期 {0}", [quotaDate(card.expiresAtMs)])} />)}
            {now >= range.start && now < range.end ? <i className="gw-quota-now" style={{ left: position(now) }} title={t("当前时间")} /> : null}
          </div></td></tr>;
      })}</tbody></table></div>
    <p className="gw-quota-legend"><span><i />{t("已知窗口")}</span><span><i data-projected="true" />{t("后续窗口（推算）")}</span>{expiryMarkers.some(cards => cards.length > 0) ? <span><i data-marker="true" />{t("重置卡到期")}</span> : null}</p>
  </section>;
}
