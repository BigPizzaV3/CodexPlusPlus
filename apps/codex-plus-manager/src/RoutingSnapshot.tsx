import { useState } from "react";
import { ArrowRight, CircleCheck, CircleMinus, CircleX, GitBranch, Server, Terminal } from "lucide-react";
import { agentName, formatTokens, maskAccountName, type GatewayPayload } from "./gateway-model";
import { t, tf } from "./i18n";
import { flowAgent, type FlowEntry } from "./request-flow-model";
import { requestContext } from "./routing-session-model";
import { requestRouteTiming } from "./request-route-timing";

export function RoutingPath({ entry, payload }: { entry: FlowEntry; payload: GatewayPayload | null }) {
  const [expanded, setExpanded] = useState(false);
  const route = entry.route;
  const used = route?.steps.filter(step => step.outcome === "answered").at(-1);
  const plan = route?.planned ?? [];
  const targets = plan.length ? plan : entry.relayId
    ? [{ relayId: entry.relayId, relayName: entry.relayName ?? "", model: entry.upstreamModel || entry.model }]
    : [];
  const group = payload?.groups.find(item => item.id === route?.groupId)?.name;
  const compact = targets.filter((target, index) => index < 3
    || used?.relayId === target.relayId && used?.model === target.model);
  const show = expanded ? targets : compact;
  return <div className="route-path-map" role="group" aria-label={t("路由路径")}>
    <div className="route-path-node"><Terminal aria-hidden="true" /><div><b>{agentName(flowAgent(entry))}</b><code>{entry.model || t("未知模型")}</code></div></div>
    <ArrowRight aria-hidden="true" />
    <div className="route-path-node"><GitBranch aria-hidden="true" /><div><b>Codex++</b><small>{group || t("聚合网关")}</small></div></div>
    <ArrowRight aria-hidden="true" />
    <div className="route-path-targets">
      {targets.length === 0 && <div className="route-path-node"><Server aria-hidden="true" /><small>{t("未记录供应商")}</small></div>}
      {show.map(target => {
        const observed = route?.steps.filter(step => step.relayId === target.relayId && step.model === target.model).at(-1);
        const accepted = used?.relayId === target.relayId && used?.model === target.model;
        const state = accepted ? "answered" : observed?.outcome;
        const Icon = accepted ? CircleCheck : state === "skipped" ? CircleMinus : state ? CircleX : Server;
        return <div className="route-path-node" key={JSON.stringify([target.relayId, target.model])} data-outcome={state}>
          <Icon aria-hidden="true" /><div><b>{maskAccountName(target.relayName || target.relayId)}</b><code>{target.model}</code></div>
          <small>{t(accepted ? "上游已接受" : state === "skipped" ? "已跳过" : state ? "尝试失败" : "候选")}</small>
        </div>;
      })}
      {targets.length > compact.length && <button type="button" className="hub-routing-more" aria-expanded={expanded}
        onClick={() => setExpanded(!expanded)}>{expanded ? t("收起") : tf("展开全部 {0} 个候选", [targets.length])}</button>}
    </div>
  </div>;
}

export function RoutingExchange({ entry, onUsage }: { entry: FlowEntry; onUsage: () => void }) {
  const { input, cached } = requestContext(entry);
  const { total, first } = requestRouteTiming(entry);
  return <div className="route-exchange">
    <details key={entry.id}>
      <summary>{t("请求与响应")}</summary>
      <div className="route-exchange-columns">
        <dl><dt>{t("请求路径")}</dt><dd><code>{entry.path?.split("?")[0] || "—"}</code></dd>
          <dt>{t("请求模型")}</dt><dd>{entry.model || "—"}</dd>
          <dt>{t("上游模型")}</dt><dd>{entry.upstreamModel || "—"}</dd>
          <dt>{t("传输方式")}</dt><dd>{entry.stream === undefined || entry.state === "running" && !entry.stream ? t("未报告") : t(entry.stream ? "流式" : "非流式")}</dd></dl>
        <dl><dt>{t("响应状态")}</dt><dd>{entry.statusCode ? `HTTP ${entry.statusCode}` : t("未报告")}</dd>
          <dt>{t("输入 / 输出 token")}</dt><dd>{input !== null ? formatTokens(input) : "—"} / {entry.usage ? formatTokens(entry.usage.outputTokens) : "—"}</dd>
          <dt>{t("缓存读取")}</dt><dd>{cached !== null ? formatTokens(cached) : "—"}</dd>
          <dt>{t("首段 / 总耗时")}</dt><dd>{first ?? "—"} / {total ?? "—"} ms</dd></dl>
      </div>
      <small>{t("不记录请求和响应正文")}</small>
    </details>
    <button type="button" className="hub-routing-more" onClick={onUsage}>{t("查看用量")} <ArrowRight aria-hidden="true" /></button>
  </div>;
}

export function RoutingTotals({ activity, onFailure }: {
  activity: NonNullable<GatewayPayload["activity"]>["agents"]; onFailure?: () => void;
}) {
  if (!activity.length) return null;
  return <div className="route-snapshot-totals" aria-label={t("网关启动以来的统计")}>
    <span><b>{activity.reduce((n, agent) => n + agent.requests, 0)}</b> {t("请求")}</span>
    <span><b>{activity.reduce((n, agent) => n + agent.rerouted, 0)}</b> {t("改道")}</span>
    {onFailure ? <button type="button" onClick={onFailure}><b>{activity.reduce((n, agent) => n + agent.errors, 0)}</b> {t("Agent 收到的错误")}</button>
      : <span><b>{activity.reduce((n, agent) => n + agent.errors, 0)}</b> {t("Agent 收到的错误")}</span>}
  </div>;
}

export function RequestRowMetrics({ entry }: { entry: FlowEntry }) {
  const { first } = requestRouteTiming(entry);
  const { input, cached } = requestContext(entry);
  return <span className="route-list-metrics">
    {first !== null && <small>{tf("首段 {0} ms", [first])}</small>}
    {input != null && cached != null && input > 0 && <small>{tf("缓存 {0}%", [(cached / input * 100).toLocaleString(undefined, { maximumFractionDigits: 2 })])}</small>}
  </span>;
}
