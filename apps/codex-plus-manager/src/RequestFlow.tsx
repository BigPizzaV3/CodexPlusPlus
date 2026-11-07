import { flowProviderSlot } from "./flow-provider-colors";
import "./flow-provider-colors.css";
/**
 * 实时请求流：托盘「流量」页签和总览共用的轮询 hook 与请求行。
 * 光点动画画在 HubTopology 上（HubOverview.tsx），这里只读取请求列表。
 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t, tf } from "./i18n";
import { agentName, formatElapsed, formatTokens, groupDisplayName, maskAccountName, relativeTime, type GatewayPayload } from "./gateway-model";
import { AgentIcon } from "./gateway-ui";
import { clockTime, flowAgent, flowElapsed, flowStatus, returningLinks, type FlowEntry } from "./request-flow-model";
import "./RequestFlow.css";

/**
 * 按间隔轮询 `load_request_flow`，窗口不可见时不读。
 * 返回最近的请求（开始时间倒序），动画直接使用当前进行中的状态。
 */
export function useRequestFlow(intervalMs: number, limit = 30): { entries: FlowEntry[]; loaded: boolean } {
  const [entries, setEntries] = useState<FlowEntry[]>([]);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    const tick = async () => {
      if (document.visibilityState === "visible") {
        try {
          const result = await invoke<{ entries?: FlowEntry[] }>("load_request_flow", { limit });
          if (cancelled) return;
          const next = result.entries ?? [];
          setEntries(next);
          setLoaded(true);
        } catch {
          // 读不到时保持上一轮的列表。
        }
      }
      if (!cancelled) timer = window.setTimeout(() => void tick(), intervalMs);
    };
    void tick();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [intervalMs, limit]);

  return { entries, loaded };
}

/**
 * 一条请求（全用 span，总览里它放在可展开的按钮里）。
 * 左边一条状态竖条（进行中闪、成功绿、失败红）；中间上行是实际用的模型，下行 Agent · 供应商 · 输入/输出 token；
 * 右边上行是耗时（进行中的实时走秒），下行是多久以前。
 */
export function FlowRequestRow({ entry, payload, now = Date.now() }: { entry: FlowEntry; payload: GatewayPayload | null; now?: number }) {
  const status = flowStatus(entry);
  const agent = flowAgent(entry);
  const relay = maskAccountName(entry.relayName || entry.relayId || "");
  const requested = groupDisplayName(payload, entry.model) ?? entry.model;
  const upstream = groupDisplayName(payload, entry.upstreamModel ?? "") ?? entry.upstreamModel;
  const model = upstream || requested || "—";
  const remapped = Boolean(entry.upstreamModel && entry.model && entry.upstreamModel !== entry.model);
  const when = relativeTime(entry.startedAtMs || entry.timestampMs, now);
  const tokens = entry.usage
    ? `↑${formatTokens(entry.usage.inputTokens)} ↓${formatTokens(entry.usage.outputTokens)}`
    : status === "running" && entry.estimatedOutputTokens
      ? `↓${formatTokens(entry.estimatedOutputTokens)}`
      : "";
  const failure = status === "error" ? entry.error || (entry.statusCode ? `HTTP ${entry.statusCode}` : t("失败")) : "";
  return (
    <span className="rf-row" data-status={status} title={clockTime(entry.startedAtMs || entry.timestampMs)}>
      <span className="rf-rail" />
      <span className="rf-main">
        <span className="rf-model" title={remapped ? `${requested} → ${upstream}` : model}>
          {model}
          {remapped ? <small>{tf("由 {0} 转来", [requested])}</small> : null}
        </span>
        <span className="rf-sub">
          <AgentIcon agent={agent} className="rf-agent-icon" />
          <span>{agentName(agent)}</span>
          {relay ? <><i>/</i><i className="flow-provider-mark" data-flow-provider={flowProviderSlot(entry.relayId ?? "")} aria-hidden="true" /><span>{relay}</span></> : null}
          {tokens ? <><i>/</i><span className="rf-tokens">{tokens}</span></> : null}
        </span>
        {failure ? <span className="rf-error" title={failure}>{failure}</span> : null}
      </span>
      <span className="rf-side">
        <b>
          {status === "running" ? <i className="rf-live-dot" /> : null}
          {formatElapsed(flowElapsed(entry, now))}
        </b>
        <small>
          {status === "running"
            ? entry.phase === "retrying" ? tf("第 {0} 次尝试", [entry.attempts ?? 1])
              : returningLinks([entry]).size ? `← ${t("响应回流")}` : `→ ${t("等待响应")}`
            : `${status === "error" ? "✕" : "✓"} ${t(status === "error" ? "失败" : "成功")} · ${when.args.length ? tf(when.text, when.args) : t(when.text)}`}
        </small>
      </span>
    </span>
  );
}
