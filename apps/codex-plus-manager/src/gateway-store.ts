/** 侧栏状态卡、总览页、网关页共用的网关数据：各自按自己的间隔静默刷新，失败时保留上一次的值。 */
import { createContext, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { GatewayPayload, GatewayResult } from "./gateway-model";

const shared: { payload: GatewayPayload | null } = { payload: null };
const listeners = new Set<(payload: GatewayPayload) => void>();
/** 每推一次新数据（保存的结果）加一。轮询发出后数据变过，答复就是旧的，丢掉不用，免得盖回刚保存的状态。 */
let generation = 0;

export function gatewayGeneration(): number {
  return generation;
}

/** 命令返回了新的网关数据（如在总览里接入了 Agent、在网关页改了供应商）：立刻推给所有在用的组件，不等下次轮询。 */
export function publishGatewayPayload(result: GatewayResult) {
  if (!result.gateway || !Array.isArray(result.pool)) return;
  generation += 1;
  shared.payload = result as GatewayPayload;
  for (const listener of listeners) listener(shared.payload);
}

/** 订阅别处推来的新数据（不轮询）；返回取消订阅。 */
export function subscribeGatewayPayload(listener: (payload: GatewayPayload) => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useGatewayPayload(intervalMs: number): GatewayPayload | null {
  const [payload, setPayload] = useState<GatewayPayload | null>(shared.payload);
  useEffect(() => {
    listeners.add(setPayload);
    return () => {
      listeners.delete(setPayload);
    };
  }, []);
  useEffect(() => {
    let cancelled = false;
    const load = () => {
      const started = generation;
      return invoke<GatewayResult>("load_gateway")
        .then((result) => {
          if (cancelled || started !== generation || !result.gateway || !Array.isArray(result.pool)) return;
          shared.payload = result as GatewayPayload;
          setPayload(shared.payload);
        })
        .catch(() => undefined);
    };
    void load();
    const timer = window.setInterval(load, intervalMs);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [intervalMs]);
  return payload;
}

/** 网关各页之间跳转时带的落地位置：跳过去的页面读一次就清掉。 */
export const gatewayJump: { provider: string | null } = { provider: null };

/** 网关页里的跳转入口（只在网关页里有；总览、托盘里的模型下拉拿不到就不显示对应链接）。 */
export const GatewayNavContext = createContext<{ openModelGroups?: () => void }>({});
