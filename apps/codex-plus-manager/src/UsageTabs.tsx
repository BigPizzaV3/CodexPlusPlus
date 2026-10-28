/** 「用量」页顶部的两个页签：统计（按 Agent、供应商、模型汇总）与请求（每条请求的日志）。
 * 两个页签各是一条路由（gatewayUsage / requestLog），侧栏里只有一个「用量」入口。 */
import { t } from "./i18n";
import type { UsageRange } from "./gateway-model";

export type UsageRoute = "gatewayUsage" | "requestLog";

/** 统计表里点一行，切到「请求」页签并按它筛选：Agent、供应商（id 与显示名）或模型，时间段跟着统计页。 */
export type RequestLogFocus = {
  agent?: "codex" | "claude" | "grok";
  relayId?: string;
  relayLabel?: string;
  model?: string;
  range?: UsageRange;
};

const OPEN_REQUESTS_EVENT = "codexplus:open-requests";

export function openRequestLog(focus: RequestLogFocus) {
  window.dispatchEvent(new CustomEvent<RequestLogFocus>(OPEN_REQUESTS_EVENT, { detail: focus }));
}

export function onOpenRequestLog(handler: (focus: RequestLogFocus) => void): () => void {
  const listener = (event: Event) => handler((event as CustomEvent<RequestLogFocus>).detail ?? {});
  window.addEventListener(OPEN_REQUESTS_EVENT, listener);
  return () => window.removeEventListener(OPEN_REQUESTS_EVENT, listener);
}

export function UsageTabs({ value, onChange }: { value: UsageRoute; onChange: (next: UsageRoute) => void }) {
  const tabs: Array<{ id: UsageRoute; label: string }> = [
    { id: "gatewayUsage", label: t("统计") },
    { id: "requestLog", label: t("请求") },
  ];
  return (
    <nav aria-label={t("用量")} className="tp-tabs usage-tabs" role="tablist">
      {tabs.map((tab) => (
        <button aria-selected={value === tab.id} data-active={value === tab.id} key={tab.id} onClick={() => onChange(tab.id)} role="tab" type="button">
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
