import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Claude 增强的生效方式和两种方式共用的设置（与 commands.rs 的 claude_enhance_* 对应）。
 * 全局一份：Desktop 增强页、补丁 Desktop 页、侧栏、拓展页都读这里，改了大家一起变。
 */
export type ClaudeDelivery = "full" | "copy";

export type ClaudeEnhanceBashTimeout = {
  enabled: boolean;
  defaultEnabled: boolean;
  defaultSeconds: number;
  maxEnabled: boolean;
  maxSeconds: number;
};

export type ClaudeEnhanceState = {
  delivery: ClaudeDelivery;
  bashTimeout: ClaudeEnhanceBashTimeout;
  claudeExtensions: boolean;
};

type Result = Partial<ClaudeEnhanceState> & { status: string; message: string };

let state: ClaudeEnhanceState | null = null;
let loading: Promise<ClaudeEnhanceState | null> | null = null;
const listeners = new Set<() => void>();

function accept(result: Result): ClaudeEnhanceState | null {
  if (result.status !== "ok" || (result.delivery !== "full" && result.delivery !== "copy") || !result.bashTimeout) {
    throw new Error(result.message || "读取 Claude 增强状态失败");
  }
  state = { delivery: result.delivery, bashTimeout: result.bashTimeout, claudeExtensions: Boolean(result.claudeExtensions) };
  listeners.forEach((listener) => listener());
  return state;
}

/** 读一次；已经读过就直接用，`force` 时重读（别处改了设置后）。 */
export function loadClaudeEnhance(force = false): Promise<ClaudeEnhanceState | null> {
  if (state && !force) return Promise.resolve(state);
  if (!loading) {
    loading = invoke<Result>("claude_enhance_status")
      .then(accept)
      .catch(() => state)
      .finally(() => { loading = null; });
  }
  return loading;
}

export async function setClaudeDelivery(mode: ClaudeDelivery): Promise<ClaudeEnhanceState | null> {
  return accept(await invoke<Result>("claude_enhance_set_delivery", { mode }));
}

export async function saveClaudeBashTimeout(timeout: ClaudeEnhanceBashTimeout): Promise<ClaudeEnhanceState | null> {
  return accept(await invoke<Result>("claude_enhance_set_bash_timeout", { timeout }));
}

/** 别处（补丁页、拓展页）改了共用设置时，把新值同步进来，不用再请求。 */
export function patchClaudeEnhance(patch: Partial<ClaudeEnhanceState>) {
  if (!state) return;
  state = { ...state, ...patch };
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function useClaudeEnhance(): ClaudeEnhanceState | null {
  const value = useSyncExternalStore(subscribe, () => state, () => state);
  if (!value) void loadClaudeEnhance();
  return value;
}
