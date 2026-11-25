import type { ReactNode } from "react";
import { ArrowRight, CircleCheck, CircleMinus, CircleX } from "lucide-react";
import { failureLabel, formatTokens, maskAccountName, type RouteRecordView, type RouteStepView } from "./gateway-model";
import { t, tf } from "./i18n";
import { requestRouteTiming, requestTimingStages } from "./request-route-timing";
import "./RequestRouteDetails.css";

type RouteEntry = {
  model: string;
  upstreamModel?: string;
  relayId?: string;
  relayName?: string;
  timestampMs?: number;
  startedAtMs?: number;
  threadId?: string | null;
  threadTitle?: string | null;
  statusCode?: number;
  durationMs: number;
  firstTokenMs?: number | null;
  stream?: boolean;
  state?: string | null;
  success?: boolean;
  usage?: { inputTokens: number; outputTokens?: number; cachedTokens?: number; cachedTokensKnown?: boolean } | null;
  route?: RouteRecordView | null;
};

const duration = (ms: number) => ms < 1000 ? tf("{0} 毫秒", [ms]) : tf("{0} 秒", [(ms / 1000).toFixed(2)]);
const percentage = (part: number, total: number) => `${(part / total * 100).toLocaleString(undefined, { maximumFractionDigits: 2 })}%`;
const provider = (id: string | undefined, name: string) => maskAccountName(name || id || "");

function stepLabel(step: RouteStepView): string {
  if (step.outcome === "answered") return t("上游已接受");
  if (step.outcome === "skipped") return t("订阅额度策略跳过");
  if (step.outcome === "transport") return t("连接失败");
  return t("请求失败");
}

const stageLabel: Record<string, string> = {
  prepare: "请求进入到路由开始", route: "路由开始到上游接受", body: "上游接受到首段响应", transfer: "首段响应至完成", finish: "上游接受到完成", first: "请求开始到首段响应",
};

export function RequestRouteDetails({ entry, agent, groupLabel, hideUsage = false, story = false, exchange }: {
  entry: RouteEntry; agent: string; groupLabel?: string; hideUsage?: boolean; story?: boolean; exchange?: ReactNode;
}) {
  const route = entry.route;
  const input = entry.usage?.inputTokens;
  const window = route?.contextWindow;
  const { total, first, afterFirst } = requestRouteTiming(entry);
  const finished = entry.state !== "running";
  const accepted = route?.steps.filter(step => step.outcome === "answered").at(-1)?.elapsedMs;
  const timestamp = entry.startedAtMs || entry.timestampMs;
  const clock = timestamp && new Date(timestamp).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23" });
  const cacheKnown = entry.usage?.cachedTokensKnown ?? (entry.usage?.cachedTokens ?? 0) > 0;
  const hit = cacheKnown && input && entry.usage?.cachedTokens != null && entry.usage.cachedTokens <= input
    ? entry.usage.cachedTokens / input
    : null;
  const requestedModel = route?.requestedModel || entry.model;
  const actualModel = entry.upstreamModel;
  const contextRatio = input != null && window && window > 0 ? input / window * 100 : null;
  const upstream = entry.relayName ? provider(entry.relayId, entry.relayName) : t("未知供应商");
  const stages = story ? requestTimingStages(entry) : [];
  const outcome = !finished ? t("进行中") : entry.success === undefined ? "" : t(entry.success ? "成功" : "失败");
  const status = [outcome, entry.statusCode ? `HTTP ${entry.statusCode}` : ""].filter(Boolean).join(" · ");
  const attempts = route?.steps.filter(step => step.outcome !== "skipped") ?? [];
  const switchedUpstream = attempts.slice(1).some(step => step.relayId !== attempts[0].relayId);
  const candidates = route?.planned ?? [];
  const resultEvidence = [
    total !== null ? duration(total) : "",
    input != null ? tf("{0} 输入 token", [formatTokens(input)]) : "",
    entry.usage?.outputTokens != null ? tf("{0} 输出 token", [formatTokens(entry.usage.outputTokens)]) : "",
    first != null ? tf("{0} 后收到首段响应", [duration(first)]) : "",
    hit != null ? tf("请求缓存命中率 {0}", [percentage(hit, 1)]) : "",
  ].filter(Boolean).join(" · ");
  return <section className="request-route" aria-label={t("本次请求路由")}>
    <div className="request-route-heading">
      <h3>{clock ? tf("{0} 的请求是怎么路由的", [clock]) : t("本次请求路由")}</h3>
      {status ? <span data-success={finished ? entry.success : undefined}>{status}</span> : null}
    </div>
    {exchange}
    {story ? <ol className="request-route-story">
      <li><div>{tf("{0} 请求", [agent])} <code>{requestedModel || t("未知模型")}</code>
        {groupLabel && <> · {t("模型组")} <b>{groupLabel}</b></>}
        {actualModel ? <> · {t("发往上游的模型")} <code>{actualModel}</code></> : null}</div></li>
      {route && <li><div>
        {candidates.length === 1 && !route.truncated ? tf("路由计划只有一个供应商与模型组合：{0} · {1}", [provider(candidates[0].relayId, candidates[0].relayName), candidates[0].model])
          : candidates.length ? tf("路由计划记录了 {0} 个候选组合 · 已记录 {1} 次尝试", [candidates.length, attempts.length]) : t("本次未记录候选")}
        {switchedUpstream && <small>{t("已尝试切换供应商")}</small>}
      </div></li>}
      {finished ? <li data-success={entry.success}><div>
        <b>{entry.relayName ? upstream : outcome || t("请求结束")}</b>{actualModel && <> · <code>{actualModel}</code></>}
        {entry.relayName && outcome && ` · ${outcome}`}
        {resultEvidence && <small>{resultEvidence}</small>}
      </div></li> : <li><div>{t("进行中")}</div></li>}
    </ol> : <div className="request-route-path">
      <b>{agent}</b><ArrowRight aria-hidden="true" />
      <code>{requestedModel || t("未知模型")}</code>
      {actualModel ? <><ArrowRight aria-hidden="true" /><span>{upstream}</span><code>{actualModel}</code></> : null}
    </div>}
    {route ? <>
      {!story && <div className="request-route-summary">
        {groupLabel ? <span>{t("模型组")} · <b>{groupLabel}</b></span> : null}
        {route.planned.length === 1 && !route.truncated ? <span>{t("路由计划中只有一个供应商与模型组合。")}</span> : null}
        {switchedUpstream ? <span>{t("已尝试切换供应商")}</span> : null}
        {attempts.length ? <span>{tf(route.truncated ? "显示 {0} 次尝试" : "{0} 次上游尝试", [attempts.length])}</span> : null}
      </div>}
      {!story && route.planned.length ? <div className="request-route-candidates">
        <span>{t("候选顺序")}</span>
        {route.planned.map((member, index) => <span key={index}>{index + 1}. {provider(member.relayId, member.relayName)} <code>{member.model}</code></span>)}
      </div> : null}
      {route.steps.length ? <ol className="request-route-steps">
        {route.steps.map((step, index) => {
          const Icon = step.outcome === "answered" ? CircleCheck : step.outcome === "skipped" ? CircleMinus : CircleX;
          const reason = step.outcome === "skipped" ? "" : failureLabel(step.failure);
          return <li key={index} data-outcome={step.outcome}>
            <Icon aria-hidden="true" /><div><b>{provider(step.relayId, step.relayName)}</b> <code>{step.model}</code></div>
            <small>{stepLabel(step)}{step.status != null ? ` · HTTP ${step.status}` : ""}{reason ? ` · ${t(reason)}` : ""}</small>
            <time title={tf("距路由开始 {0}", [duration(step.elapsedMs)])}>{story && step.attemptMs != null
              ? tf("本次尝试 {0}", [duration(step.attemptMs)]) : tf("距路由开始 {0}", [duration(step.elapsedMs)])}</time>
          </li>;
        })}
      </ol> : null}
      {route.coolingMs != null ? <small>{tf("上游冷却中，建议 {0} 后重试", [duration(route.coolingMs)])}</small> : null}
      {route.truncated ? <small>{t("路由记录已截断")}</small> : null}
    </> : <small>{t("未记录候选与切换过程")}</small>}
    <div className="request-route-timing">
      {story ? <details className="request-route-waterfall" open>
        <summary><h4>{t("时间花在哪里")}</h4>{total !== null && <strong>{duration(total)}</strong>}</summary>
        {stages.length > 0 && total != null && total > 0 && <div className="request-route-duration-bar" aria-hidden="true">
          {stages.map(stage => <i key={stage.key} data-stage={stage.key} style={{ flexGrow: stage.ms }} />)}
        </div>}
        <div className="request-route-waterfall-rows">
          {stages.map(stage => <div key={stage.key} data-stage={stage.key}><span><i />{t(stageLabel[stage.key])}</span><strong>{duration(stage.ms)}</strong></div>)}
          {!stages.some(stage => stage.key === "route") && accepted != null && <div><span>{t("路由至上游接受（单独计时）")}</span><strong>{duration(accepted)}</strong></div>}
          <small>{t("未单独采集网络发送耗时和上游首字节")}</small>
        </div>
      </details> : <>
      <h4>{t("时间花在哪里")}</h4>
      {total != null && total > 0 && first != null && afterFirst != null ? <div className="request-route-duration-bar" aria-hidden="true">
        <i style={{ flexGrow: first }} /><i style={{ flexGrow: afterFirst }} />
      </div> : null}
      <div className="request-route-metrics">
        {total != null ? <div><span>{t("总耗时")}</span><strong>{duration(total)}</strong></div> : null}
        {first != null ? <div><span><i className="request-route-dot" />{t("首段响应（代理观测）")}</span><strong>{duration(first)}</strong></div> : null}
        {afterFirst != null ? <div><span><i className="request-route-dot" data-stream />{t("首段响应至完成")}</span><strong>{duration(afterFirst)}</strong></div> : null}
        {accepted != null ? <div><span>{t("路由至上游接受")}</span><strong>{duration(accepted)}</strong></div> : null}
        {!finished && first == null ? <small>{t("进行中")}</small> : null}
      </div>
      </>}
    </div>
    {!hideUsage && (entry.usage || window || entry.threadId || entry.threadTitle) ? <div className="request-route-receipt">
      <h4>{t("请求用量")}</h4>
      {entry.threadId || entry.threadTitle ? <small>{t("会话")} · {entry.threadTitle || <code>{entry.threadId}</code>}</small> : null}
      <div className="request-route-metrics">
      <div><span>{t("输入 / 上下文上限")}</span><strong>{input == null ? t("未知") : formatTokens(input)} / {window ? formatTokens(window) : t("未知")}</strong>
        {contextRatio != null && input != null && window ? <><small>{tf("占上限 {0} · 余量 {1}", [percentage(input, window), formatTokens(Math.max(0, window - input))])}</small><span className="request-route-context-bar" aria-hidden="true"><i style={{ width: `${Math.min(100, contextRatio)}%` }} /></span></> : null}
      </div>
      <div><span>{t("提示词缓存")}</span><strong>{hit == null ? t("未报告") : percentage(hit, 1)}</strong>{hit != null && input != null && entry.usage?.cachedTokens != null ? <>
        <small>{tf("{0} 缓存 · {1} 新增", [formatTokens(entry.usage.cachedTokens), formatTokens(input - entry.usage.cachedTokens)])}</small>
        <span className="request-route-context-bar" data-cache aria-hidden="true"><i style={{ width: `${hit * 100}%` }} /></span>
      </> : null}</div>
      {entry.usage?.outputTokens != null ? <div><span>{t("输出")}</span><strong>{formatTokens(entry.usage.outputTokens)}</strong></div> : null}
      </div>
    </div> : null}
  </section>;
}
