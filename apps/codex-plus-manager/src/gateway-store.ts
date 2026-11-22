/**
 * 侧栏状态卡、首页、总览、网关各页共用的网关数据：整个窗口只有一个轮询，
 * 间隔取所有在用组件要的最短那个；窗口不可见时不读，重新可见时补一轮。
 * 读取失败时保留上一次的数据，另记一个 `error` 给还没有数据的页面显示重试。
 */
import { createContext, useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { rememberAgentNames, type GatewayPayload, type GatewayResult } from "./gateway-model";

export type GatewayState = { payload: GatewayPayload | null; error: string | null };

let state: GatewayState = { payload: null, error: null };
/** 当前数据的 JSON：读回来和上一份一样就沿用旧引用、不通知，免得每轮都重渲染。 */
let payloadJson = "";
const listeners = new Set<() => void>();
/** 每推一次新数据（保存的结果）加一。读取发出后数据变过，答复就是旧的，丢掉不用，免得盖回刚保存的状态。轮询本身不加。 */
let generation = 0;
/** 在用组件要的轮询间隔（订阅号 → 毫秒）；计时器按最短的走。 */
const intervals = new Map<number, number>();
let nextSubscriber = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
/** 上一轮读取开始的时间：新来的组件要的间隔没过就不用马上再读。 */
let lastStarted = 0;
/** 正在进行的读取：同时要读的共用这一个请求。 */
let inFlight: { generation: number; promise: Promise<GatewayResult> } | null = null;

function hasPayload(result: GatewayResult): result is GatewayResult & GatewayPayload {
  return Boolean(result.gateway && Array.isArray(result.pool));
}

function emit(next: GatewayState) {
  state = next;
  for (const listener of listeners) listener();
}

function acceptPayload(payload: GatewayPayload) {
  rememberAgentNames(payload.agents);
  const json = JSON.stringify(payload);
  if (json === payloadJson && state.payload) {
    if (state.error !== null) emit({ payload: state.payload, error: null });
    return;
  }
  payloadJson = json;
  emit({ payload, error: null });
}

function acceptError(message: string) {
  if (state.error === message) return;
  emit({ ...state, error: message });
}

export function getGatewayState(): GatewayState {
  return state;
}

/** 订阅数据变化（不轮询）；返回取消订阅。 */
export function subscribeGatewayState(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** 命令返回了新的网关数据（如在总览里接入了 Agent、在网关页改了供应商）：立刻推给所有在用的组件，不等下次轮询。 */
export function publishGatewayPayload(result: GatewayResult) {
  if (!hasPayload(result)) return;
  generation += 1;
  acceptPayload(result);
}

/**
 * 立刻读一次网关状态（已有同一代的读取在进行就共用它），返回后端的原始答复。
 * 读的过程中有保存推来了新数据：这份答复是旧的，不收下。
 */
export function refreshGateway(): Promise<GatewayResult> {
  if (inFlight && inFlight.generation === generation) return inFlight.promise;
  const started = generation;
  lastStarted = Date.now();
  const promise: Promise<GatewayResult> = invoke<GatewayResult>("load_gateway")
    .then(
      (result) => {
        if (started === generation) {
          if (hasPayload(result)) acceptPayload(result);
          else acceptError(result.message || "load_gateway failed");
        }
        return result;
      },
      (error: unknown) => {
        if (started === generation) acceptError(String(error));
        throw error;
      },
    )
    .finally(() => {
      if (inFlight?.promise === promise) inFlight = null;
    });
  inFlight = { generation: started, promise };
  return promise;
}

function visible(): boolean {
  return typeof document === "undefined" || document.visibilityState === "visible";
}

/** 按最短间隔排下一轮；没人用或窗口不可见时不排（重新可见时由 visibilitychange 补一轮）。 */
function schedule() {
  clearTimeout(timer);
  timer = undefined;
  if (!intervals.size || !visible()) return;
  const shortest = Math.min(...intervals.values());
  timer = setTimeout(poll, Math.max(0, lastStarted + shortest - Date.now()));
}

function poll() {
  clearTimeout(timer);
  timer = undefined;
  if (!intervals.size || !visible()) return;
  void refreshGateway()
    .catch(() => undefined)
    .finally(schedule);
}

function onVisibilityChange() {
  if (visible()) poll();
  else schedule();
}

/**
 * 登记一个按 `intervalMs` 刷新的使用者，返回注销函数。
 * 还没有数据、或上一轮读取比这个间隔还旧时立刻读一次。
 */
export function watchGateway(intervalMs: number): () => void {
  const id = ++nextSubscriber;
  if (!intervals.size && typeof document !== "undefined") document.addEventListener("visibilitychange", onVisibilityChange);
  intervals.set(id, intervalMs);
  if (state.payload === null || Date.now() - lastStarted >= intervalMs) poll();
  else schedule();
  return () => {
    intervals.delete(id);
    if (!intervals.size && typeof document !== "undefined") document.removeEventListener("visibilitychange", onVisibilityChange);
    schedule();
  };
}

/** 共享的网关数据和读取错误；`intervalMs` 是这个组件希望的刷新间隔。 */
export function useGatewayState(intervalMs: number): GatewayState {
  const snapshot = useSyncExternalStore(subscribeGatewayState, getGatewayState);
  useEffect(() => watchGateway(intervalMs), [intervalMs]);
  return snapshot;
}

export function useGatewayPayload(intervalMs: number): GatewayPayload | null {
  return useGatewayState(intervalMs).payload;
}

/** 网关各页之间跳转时带的落地位置：跳过去的页面读一次就清掉。 */
export const gatewayJump: { provider: string | null; agent: string | null } = { provider: null, agent: null };

/** 取出并清掉跳转带来的落地位置。 */
export function takeGatewayJump(kind: keyof typeof gatewayJump): string | null {
  const target = gatewayJump[kind];
  gatewayJump[kind] = null;
  return target;
}

const viewState = new Map<string, unknown>();

/**
 * 网关各页签切换时会重新挂载：搜索词、展开的行、折叠状态按 `key` 记在模块里，切回来还是原样（只在本次运行内）。
 * `key` 为 null 时就是普通的 state（比如嵌在弹窗里的同一个列表，不和页面共用）；
 * `override` 不是 undefined 时以它为准（从别的页跳过来，指定了要展开的那一行）。
 */
export function useGatewayViewState<T>(key: string | null, initial: T, override?: T): [T, (next: T) => void] {
  const [value, setValue] = useState<T>(() => {
    if (override !== undefined) {
      if (key) viewState.set(key, override);
      return override;
    }
    return key && viewState.has(key) ? (viewState.get(key) as T) : initial;
  });
  const update = useCallback(
    (next: T) => {
      if (key) viewState.set(key, next);
      setValue(next);
    },
    [key],
  );
  return [value, update];
}

/** 网关页里的跳转入口（只在网关页里有；总览、托盘里的模型下拉拿不到就不显示对应链接）。 */
export const GatewayNavContext = createContext<{ openModelGroups?: () => void }>({});
