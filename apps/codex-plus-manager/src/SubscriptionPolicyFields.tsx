import { t, tf } from "./i18n";
import { QuotaWindows } from "./gateway-ui";
import type { ProviderQuota } from "./gateway-model";
import { normalizeSubscriptionPolicy, subscriptionPercentLimit, withSubscriptionWindowLimit, type SubscriptionPolicy, type SubscriptionPolicyInput } from "./subscription-policy";
import "./NativeAuthControls.css";
import { AppSelect } from "./AppSelect";

export function SubscriptionPolicyFields({ value, onChange, quota, disabled = false }: {
  value?: SubscriptionPolicyInput | null;
  onChange: (policy: SubscriptionPolicy) => void;
  quota?: ProviderQuota | null;
  disabled?: boolean;
}) {
  const policy = normalizeSubscriptionPolicy(value);
  const update = (patch: Partial<SubscriptionPolicy>) => onChange({ ...policy, ...patch });
  const actual = quota?.windows.flatMap((window) => window.windowMinutes === null ? [] : [String(window.windowMinutes)]) ?? [];
  const windows = [...new Set(quota?.windowsComplete ? actual : [...actual, ...Object.keys(policy.windowUsedPercentLimits)])];
  return (
    <section className="pf-section">
      <h4>{t("ChatGPT 订阅额度")}</h4>
      {quota ? <div className="gw-chip-row"><QuotaWindows maxAgeSeconds={policy.maxAgeSeconds} quota={quota} /></div> : <p className="pf-hint">{t("尚未取得官方用量")}</p>}
      <div className="subscription-policy">
        <label>
          <span>{t("所有窗口已用上限（%）")}</span>
          <input aria-label={t("所有窗口已用上限（%）")} disabled={disabled} max={100} min={0} onChange={(event) => { const limit = subscriptionPercentLimit(event.currentTarget.value); if (limit !== undefined) update({ defaultUsedPercentLimit: limit }); }} placeholder="100" step={1} type="number" value={policy.defaultUsedPercentLimit ?? ""} />
        </label>
        {windows.map((minutes) => (
          <label key={minutes}>
            <span>{minutes === "300" ? t("5 小时已用上限（%）") : minutes === "10080" ? t("每周已用上限（%）") : tf("{0} 分钟已用上限（%）", [minutes])}</span>
            <input aria-label={tf("{0} 分钟已用上限（%）", [minutes])} disabled={disabled} max={100} min={0} onChange={(event) => onChange(withSubscriptionWindowLimit(policy, minutes, event.currentTarget.value))} placeholder="100" step={1} type="number" value={policy.windowUsedPercentLimits[minutes] ?? ""} />
          </label>
        ))}
        <label>
          <span>{t("达到上限后")}</span>
          <AppSelect ariaLabel={t("达到上限后")} disabled={disabled} onChange={(onLimit) => update({ onLimit: onLimit as SubscriptionPolicy["onLimit"] })} options={[
            { value: "stop", label: t("停止请求") },
            { value: "nextPermitted", label: t("下一个订阅账号") },
            { value: "apiFallback", label: t("回退到 API 供应商") },
          ]} value={policy.onLimit} />
        </label>
        <label>
          <span>{t("用量未知或过期时")}</span>
          <AppSelect ariaLabel={t("用量未知或过期时")} disabled={disabled} onChange={(unknownUsage) => update({ unknownUsage: unknownUsage as SubscriptionPolicy["unknownUsage"] })} options={[
            { value: "block", label: t("阻止请求") },
            { value: "allow", label: t("允许请求") },
          ]} value={policy.unknownUsage} />
        </label>
        <label>
          <span>{t("用量有效期（秒）")}</span>
          <input disabled={disabled} max={86400} min={1} onChange={(event) => { const number = Number(event.currentTarget.value); if (Number.isInteger(number) && number >= 1 && number <= 86400) update({ maxAgeSeconds: number }); }} type="number" value={policy.maxAgeSeconds} />
        </label>
      </div>
      <p className="pf-hint">{t("上限按官方累计已用比例计算，100 或留空表示不设上限。同一真实账号的供应商共享官方额度，原生客户端等其他来源的使用也计入。")}</p>
      <p className="pf-hint">{t("策略约束新的网关请求；用量更新存在延迟，进行中的请求可能超过阈值。")}</p>
    </section>
  );
}
