// 网关总览之外也常用的两样：今日用量 hook 和 Agent 图标（侧栏、首页、各页面用），
// 从 HubOverview.tsx 拆出来，常驻的页面引它们时不用把网关总览和网关页一起加载。
import { useEffect, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { agentName, type UsageSummary } from "./gateway-model";
import { AGENT_ICONS, agentIconIsColor } from "./gateway-ui";
import { useGatewayPayload } from "./gateway-store";

const USAGE_REFRESH_MS = 30_000;

/** 侧栏状态卡、总览、网关页共用的网关数据在 gateway-store 里；这里转出 hook。 */
export { useGatewayPayload };

/** 今日用量（总览统计、首页网关一行共用），定时刷新。 */
export function useTodayUsage(): UsageSummary | null {
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  useEffect(() => {
    let cancelled = false;
    const load = () =>
      invoke<{ status: string } & Partial<UsageSummary>>("load_gateway_usage", { range: "today" })
        .then((result) => {
          if (!cancelled && result.status === "ok" && result.totals) setSummary(result as UsageSummary);
        })
        .catch(() => undefined);
    void load();
    const timer = window.setInterval(load, USAGE_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);
  return summary;
}

/** 单色品牌图标用 mask 着色，深浅主题下都跟随文字颜色。 */
export function AgentGlyph({ agent, className = "" }: { agent: string; className?: string }) {
  const url = AGENT_ICONS[agent];
  if (url && agentIconIsColor(agent)) return <img alt="" className={`hub-agent-glyph hub-agent-glyph-plain ${className}`} data-agent-icon={agent} src={url} />;
  // 没有打包图标的通用 Agent：名字首字母，不画空的遮罩方块。
  if (!url) return <i aria-hidden="true" className={`hub-agent-glyph hub-agent-glyph-plain gw-agent-letter ${className}`}>{(agentName(agent).trim()[0] ?? "?").toUpperCase()}</i>;
  return <i aria-hidden="true" className={`hub-agent-glyph ${className}`} style={url ? ({ "--hub-icon": `url("${url}")` } as CSSProperties) : undefined} />;
}
