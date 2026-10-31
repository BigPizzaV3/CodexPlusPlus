// 供应商卡片上的中转站（sub2api / new-api）倍率 / 余额。直连供应商和网关供应商池共用：
// 两边只是识别请求不同（直连传整个 profile，网关按 id 由后端取 Key）。
import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw } from "lucide-react";
import { t, tf } from "./i18n";
import type { ProviderBalance } from "./gateway-model";
import {
  cachedProviderBalances,
  loadProviderBalances,
  providerBalanceFor,
  subscribeProviderBalances,
} from "./provider-balances";
import {
  formatRelayAmount,
  formatRelayMultiplier,
  mergeRelayBillingResult,
  newApiAccessHint,
  normalizeRelayBillingProbe,
  rememberRelayBillingNegative,
  relayAppliedPeak,
  relayPlatformLabel,
  relayBalanceBadge,
  relayBillingCacheUsable,
  relayBillingNegativeCached,
  type RelayBillingCacheEntry,
  type RelayBillingProbe,
} from "./relay-billing";

type ProbeResponse = Partial<RelayBillingProbe> & { status?: string };

// 识别结果在内存里缓存（余额会变；请求失败只记几分钟），明确不是的才记进 localStorage。
const probeCache = new Map<string, RelayBillingCacheEntry>();
const probeInflight = new Map<string, Promise<RelayBillingCacheEntry | null | undefined>>();

function storage(): Storage | null {
  try { return window.localStorage; } catch { return null; }
}

/** 返回并入后的缓存项；null 表示明确不是中转站；undefined 表示命令本身失败、没有结论。 */
function requestProbe(key: string, request: () => Promise<ProbeResponse>): Promise<RelayBillingCacheEntry | null | undefined> {
  const pending = probeInflight.get(key);
  if (pending) return pending;
  const next = request()
    .then((result) => {
      // 命令本身失败（如网关里已删掉这个供应商）不算识别结论，下次再试。
      if (result.status && result.status !== "ok") return undefined;
      const { entry, negative } = mergeRelayBillingResult(probeCache.get(key), normalizeRelayBillingProbe(result), Date.now());
      if (entry) probeCache.set(key, entry);
      else probeCache.delete(key);
      if (negative !== "keep") rememberRelayBillingNegative(storage(), key, negative === "remember");
      return entry;
    })
    // 识别失败不打扰用户，卡片照常显示，只是没有倍率 / 余额。
    .catch(() => undefined)
    .finally(() => probeInflight.delete(key));
  probeInflight.set(key, next);
  return next;
}

type ProbeState = {
  key: string;
  entry: RelayBillingCacheEntry | null;
  /** 已有结论（识别过、或命中「明确不是」的缓存）：没识别出时据此显示刷新入口。 */
  settled: boolean;
  loading: boolean;
};

/** `key` 为空表示这个供应商不识别；key 变化（换地址 / Key）会重新识别。 */
export function useRelayBillingProbe(key: string, request: () => Promise<ProbeResponse>) {
  const [state, setState] = useState<ProbeState>(() => {
    const entry = key ? probeCache.get(key) ?? null : null;
    return { key, entry, settled: entry !== null, loading: false };
  });
  const requestRef = useRef(request);
  requestRef.current = request;

  const load = useCallback((force: boolean) => {
    if (!key) return;
    const cached = probeCache.get(key);
    if (!force) {
      if (relayBillingCacheUsable(cached, Date.now())) {
        setState({ key, entry: cached ?? null, settled: true, loading: false });
        return;
      }
      if (!cached && relayBillingNegativeCached(storage(), key)) {
        setState({ key, entry: null, settled: true, loading: false });
        return;
      }
    }
    setState((current) => ({
      key,
      entry: cached ?? null,
      settled: current.key === key && current.settled,
      loading: true,
    }));
    void requestProbe(key, () => requestRef.current()).then((entry) => {
      setState((current) => current.key === key
        ? { key, entry: entry === undefined ? current.entry : entry, settled: true, loading: false }
        : current);
    });
  }, [key]);

  useEffect(() => { load(false); }, [load]);

  const current = state.key === key ? state : null;
  return {
    /** 最近一次识别成功的结果（请求失败时仍保留旧值）。 */
    probe: current?.entry?.probe ?? null,
    /** 最近一次请求失败（时间与原因）；成功或明确不是时为 null。 */
    failure: current?.entry?.failure ?? null,
    settled: Boolean(key && current?.settled),
    loading: Boolean(current?.loading),
    refresh: () => load(true),
  };
}

function relayBillingParts(probe: RelayBillingProbe): { text: string; tone?: string }[] {
  const { rate, usage } = probe;
  const peak = relayAppliedPeak(rate);
  const parts: { text: string; tone?: string }[] = [];
  if (rate) {
    parts.push({ text: tf("倍率 {0}", [formatRelayMultiplier(rate.multiplier)]) });
    if (peak !== null) parts.push({ text: tf("含高峰 ×{0}", [String(Number(peak.toFixed(4)))]), tone: "warning" });
  }
  if (usage) {
    if (!usage.isValid) parts.push({ text: t("Key 已失效"), tone: "danger" });
    if (usage.remaining !== null) {
      const amount = formatRelayAmount(usage.remaining, usage.unit);
      const tone = usage.remaining <= 0 ? "danger" : undefined;
      if (usage.kind === "keyQuota") {
        parts.push({
          text: usage.quotaLimit !== null
            ? tf("Key 额度剩余 {0} / {1}", [amount, formatRelayAmount(usage.quotaLimit, usage.unit)])
            : tf("Key 额度剩余 {0}", [amount]),
          tone,
        });
      } else if (usage.kind === "subscription") {
        parts.push({ text: usage.planName ? tf("{0} 剩余 {1}", [usage.planName, amount]) : tf("订阅剩余 {0}", [amount]), tone });
      } else {
        parts.push({ text: tf("余额 {0}", [amount]), tone });
      }
    } else if (usage.kind === "unlimited") {
      parts.push({ text: t("Key 不限额度") });
    } else if (usage.kind === "subscription" && usage.planName) {
      parts.push({ text: usage.planName });
    }
    for (const rateLimit of usage.rateLimits) {
      parts.push({
        text: tf("{0} 剩 {1}", [rateLimit.window, formatRelayAmount(rateLimit.remaining, usage.unit)]),
        tone: rateLimit.remaining <= 0 ? "danger" : undefined,
      });
    }
  }
  return parts;
}

function relayRateSource(probe: RelayBillingProbe): string {
  const { rate } = probe;
  if (probe.platform === "newapi") {
    if (!rate) return t("这个 Key 还没有消费记录，拿不到分组倍率");
    return rate.group
      ? tf("分组倍率取自最近一次消费记录（分组 {0}）", [rate.group])
      : t("分组倍率取自最近一次消费记录");
  }
  return rate ? tf("倍率接口：{0}", [rate.endpoint]) : t("这个 sub2api 版本没有倍率接口");
}

type RelayBillingFailure = { at: number; detail: string } | null;

function relayBillingFailureText(failure: NonNullable<RelayBillingFailure>): string {
  return failure.detail
    ? tf("暂时获取失败，显示的是上次的结果：{0}", [failure.detail])
    : t("暂时获取失败，显示的是上次的结果");
}

function relayBillingTitle(probe: RelayBillingProbe, failure: RelayBillingFailure = null): string {
  return [
    relayRateSource(probe),
    probe.usage ? tf("余额接口：{0}", [probe.usage.endpoint]) : t("余额接口不可用"),
    failure ? relayBillingFailureText(failure) : "",
  ].filter(Boolean).join("\n");
}

function RelayBillingRefreshButton({
  loading,
  onRefresh,
  className = "",
  title = t("刷新倍率和余额"),
}: {
  loading: boolean;
  onRefresh: () => void;
  className?: string;
  title?: string;
}) {
  return (
    <button
      aria-label={title}
      className={`relay-billing-refresh ${className}`.trim()}
      disabled={loading}
      onClick={(event) => {
        event.stopPropagation();
        onRefresh();
      }}
      onKeyDown={(event) => event.stopPropagation()}
      title={title}
      type="button"
    >
      <RefreshCw className={`h-3 w-3 ${loading ? "animate-spin" : ""}`} />
    </button>
  );
}

/** 没识别出中转站（明确不是或暂时获取失败）时的刷新入口：平时隐藏，悬停卡片才出现。 */
export function RelayBillingRetry({ failure, loading, onRefresh }: { failure: RelayBillingFailure; loading: boolean; onRefresh: () => void }) {
  const title = failure
    ? failure.detail
      ? tf("暂时获取不到倍率和余额，点击重试：{0}", [failure.detail])
      : t("暂时获取不到倍率和余额，点击重试")
    : t("没有识别出 sub2api / new-api 中转站，点击重新识别");
  return <RelayBillingRefreshButton className="relay-billing-retry" loading={loading} onRefresh={onRefresh} title={title} />;
}

export function RelayBillingLine({
  probe,
  failure = null,
  loading,
  onRefresh,
}: {
  probe: RelayBillingProbe;
  failure?: RelayBillingFailure;
  loading: boolean;
  onRefresh: () => void;
}) {
  if (!probe.platform) return null;
  const parts = relayBillingParts(probe);
  return (
    // 卡片上不显示中转站类型（sub2api / new-api），只放在悬停提示里。
    <span className="relay-billing-line" title={`${relayPlatformLabel(probe.platform)}\n${relayBillingTitle(probe, failure)}`}>
      {parts.map((part, index) => (
        <span className="relay-billing-part" data-tone={part.tone} key={index}>{part.text}</span>
      ))}
      {failure ? <span className="relay-billing-part" data-tone="warning">{t("暂时获取失败")}</span> : null}
      <RelayBillingRefreshButton loading={loading} onRefresh={onRefresh} />
    </span>
  );
}

/** 供应商池 new-api 卡片的余额提示（#2793）：没配访问令牌时说明配了能显示余额，配了但失败时给原因。 */
function newApiAccessText(probe: RelayBillingProbe): { text: string; failed: boolean } | null {
  const hint = newApiAccessHint(probe);
  if (!hint) return null;
  return hint.failed
    ? { text: tf("访问令牌查询余额失败：{0}", [hint.text]), failed: true }
    : { text: t("配置访问令牌后可显示余额（展开供应商详情，在「访问令牌」里填写）"), failed: false };
}

/** 网关供应商池右侧：和订阅账号的「套餐 + 额度条」同一位置、同一样式。只显示倍率，
 * 中转站类型（sub2api / new-api）放在悬停提示里；没有倍率时不显示，由调用方显示 Key 数。 */
export function RelayBillingPoolLabel({ probe }: { probe: RelayBillingProbe }) {
  if (!probe.platform || !probe.rate) return null;
  const peak = relayAppliedPeak(probe.rate);
  const platform = relayPlatformLabel(probe.platform);
  const access = newApiAccessText(probe);
  return (
    <span className="gw-billing-label" title={[platform, relayBillingParts(probe).map((part) => part.text).join(" · "), relayRateSource(probe), access?.text ?? ""].filter(Boolean).join("\n")}>
      {tf("倍率 {0}", [formatRelayMultiplier(probe.rate.multiplier)])}
      {peak !== null ? <em>{tf("含高峰 ×{0}", [String(Number(peak.toFixed(4)))])}</em> : null}
    </span>
  );
}

export function RelayBillingPoolBadge({
  probe,
  failure = null,
  loading,
  onRefresh,
}: {
  probe: RelayBillingProbe;
  failure?: RelayBillingFailure;
  loading: boolean;
  onRefresh: () => void;
}) {
  const badge = probe.usage ? relayBalanceBadge(probe.usage) : null;
  const details = relayBillingParts(probe).map((part) => part.text).join("\n");
  const access = newApiAccessText(probe);
  return (
    <span className="gw-billing-status">
      {failure ? <em className="gw-billing-stale" title={relayBillingFailureText(failure)}>{t("暂时获取失败")}</em> : null}
      {access?.failed ? <em className="gw-billing-stale" title={access.text}>{t("余额查询失败")}</em> : null}
      {badge ? (
        <span className="gw-quota-badge" data-tone={badge.tone} title={`${details}\n${relayBillingTitle(probe, failure)}`}>
          {badge.ratio !== null ? <i style={{ "--gw-quota": `${Math.round(badge.ratio * 100)}%` } as CSSProperties} /> : null}
          {probe.usage?.kind === "wallet" ? tf("余额 {0}", [badge.amount]) : tf("剩 {0}", [badge.amount])}
        </span>
      ) : probe.usage?.kind === "unlimited" ? (
        <span className="gw-quota-badge" data-tone={probe.usage.isValid ? "ok" : "low"} title={[details, relayBillingTitle(probe), access?.text ?? ""].filter(Boolean).join("\n")}>
          {t("Key 不限额度")}
        </span>
      ) : null}
      <RelayBillingRefreshButton loading={loading} onRefresh={onRefresh} />
    </span>
  );
}

/**
 * 有余额接口的厂商（DeepSeek、Kimi 等）在供应商池卡片上的余额（#2793）：和「用量」页用同一份查询结果，
 * 几张卡片一起要也只查一次；卡片 5 分钟内不重查，点刷新才强制重查。
 */
export function useProviderBalance(id: string, enabled: boolean) {
  const [balances, setBalances] = useState<ProviderBalance[] | null>(() => cachedProviderBalances().balances);
  const [loading, setLoading] = useState(false);
  const load = useCallback((force: boolean) => {
    setLoading(true);
    void loadProviderBalances(() => invoke<{ status: string; balances?: ProviderBalance[] }>("load_gateway_balances"), { force })
      .then(setBalances)
      .finally(() => setLoading(false));
  }, []);
  useEffect(() => subscribeProviderBalances(setBalances), []);
  useEffect(() => {
    if (!enabled) return;
    // 缓存里没有这家（刚添加、刚打开）时重查一次，其余按缓存时间。
    const cached = cachedProviderBalances().balances;
    load(cached !== null && !providerBalanceFor(cached, id));
  }, [enabled, id, load]);
  return {
    balance: enabled ? providerBalanceFor(balances, id) : null,
    loading,
    refresh: () => load(true),
  };
}

/** 供应商池右侧的厂商余额：样式与中转站余额一致（同一个徽章和刷新按钮）。 */
export function ProviderBalancePoolBadge({
  balance,
  loading,
  onRefresh,
}: {
  balance: ProviderBalance;
  loading: boolean;
  onRefresh: () => void;
}) {
  const amount = typeof balance.amount === "number" && Number.isFinite(balance.amount) ? balance.amount : null;
  return (
    <span className="gw-billing-status">
      {amount !== null ? (
        <span className="gw-quota-badge" data-tone={amount <= 0 ? "low" : "ok"} title={t("余额接口查询，和「用量」页、托盘面板同一来源")}>
          {tf("余额 {0}", [formatRelayAmount(amount, balance.currency)])}
        </span>
      ) : (
        <em className="gw-billing-stale" title={t(balance.error || "查不到")}>{t("暂时获取失败")}</em>
      )}
      <RelayBillingRefreshButton loading={loading} onRefresh={onRefresh} title={t("刷新余额")} />
    </span>
  );
}
