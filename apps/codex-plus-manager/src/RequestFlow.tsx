/**
 * 实时请求流：托盘「流量」页签和总览共用的轮询 hook 与请求行。
 * 光点动画画在 HubTopology 上（HubOverview.tsx），这里只产出光点和请求列表。
 */
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t, tf } from "./i18n";
import { agentName, formatElapsed, formatTokens, maskAccountName, relativeTime } from "./gateway-model";
import { AgentIcon } from "./gateway-ui";
import { clockTime, diffFlow, flowAgent, flowElapsed, flowStatus, type FlowEntry, type FlowPulse, type FlowSeen } from "./request-flow-model";
import "./RequestFlow.css";

/** 保留最近这么多颗光点给画动画的一方；已经放过的按 seq 跳过。 */
const KEEP_PULSES = 24;

/**
 * 按间隔轮询 `load_request_flow`，窗口不可见时不读。
 * 返回最近的请求（开始时间倒序）和新产生的光点。
 */
export function useRequestFlow(intervalMs: number, limit = 30): { entries: FlowEntry[]; pulses: FlowPulse[]; loaded: boolean } {
  const [entries, setEntries] = useState<FlowEntry[]>([]);
  const [pulses, setPulses] = useState<FlowPulse[]>([]);
  const [loaded, setLoaded] = useState(false);
  const seenRef = useRef<FlowSeen | null>(null);
  const seqRef = useRef(0);

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    const tick = async () => {
      if (document.visibilityState === "visible") {
        try {
          const result = await invoke<{ entries?: FlowEntry[] }>("load_request_flow", { limit });
          if (cancelled) return;
          const next = result.entries ?? [];
          const diff = diffFlow(seenRef.current, next, seqRef.current);
          seenRef.current = diff.seen;
          seqRef.current += diff.pulses.length;
          setEntries(next);
          setLoaded(true);
          if (diff.pulses.length) setPulses((current) => [...current, ...diff.pulses].slice(-KEEP_PULSES));
        } catch {
          // 读不到时保持上一轮的列表。
        }
      } else {
        // 看不见期间的变化不攒着回放：回来后第一轮重新只记状态。
        seenRef.current = null;
      }
      if (!cancelled) timer = window.setTimeout(() => void tick(), intervalMs);
    };
    void tick();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [intervalMs, limit]);

  return { entries, pulses, loaded };
}

/**
 * 一条请求（全用 span，总览里它放在可展开的按钮里）。
 * 左边一条状态竖条（进行中闪、成功绿、失败红）；中间上行是实际用的模型，下行 Agent · 供应商 · 输入/输出 token；
 * 右边上行是耗时（进行中的实时走秒），下行是多久以前。
 */
export function FlowRequestRow({ entry, now = Date.now() }: { entry: FlowEntry; now?: number }) {
  const status = flowStatus(entry);
  const agent = flowAgent(entry);
  const relay = maskAccountName(entry.relayName || entry.relayId || "");
  const model = entry.upstreamModel || entry.model || "—";
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
        <span className="rf-model" title={remapped ? `${entry.model} → ${entry.upstreamModel}` : model}>
          {model}
          {remapped ? <small>{tf("由 {0} 转来", [entry.model])}</small> : null}
        </span>
        <span className="rf-sub">
          <AgentIcon agent={agent} className="rf-agent-icon" />
          <span>{agentName(agent)}</span>
          {relay ? <><i>/</i><span>{relay}</span></> : null}
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
            ? (entry.attempts ?? 1) > 1 ? tf("第 {0} 次尝试", [entry.attempts ?? 1]) : t("进行中")
            : when.args.length ? tf(when.text, when.args) : t(when.text)}
        </small>
      </span>
    </span>
  );
}
