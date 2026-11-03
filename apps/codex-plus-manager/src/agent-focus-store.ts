/** 使用模式的读写与订阅。规则在 agent-focus-model.ts，这里只管 localStorage 和广播。 */
import { useEffect, useState } from "react";
import {
  AGENT_FOCUS_EVENT,
  AGENT_FOCUS_STORAGE_KEY,
  parseAgentFocus,
  serializeAgentFocus,
  shouldAskAgentFocus,
  type AgentFocus,
  type FocusMode,
} from "./agent-focus-model";

let sessionFallback: string | undefined;

function readStored(): string | null {
  if (sessionFallback !== undefined) return sessionFallback;
  try {
    return window.localStorage.getItem(AGENT_FOCUS_STORAGE_KEY);
  } catch {
    return null;
  }
}

export function loadAgentFocus(): AgentFocus {
  return typeof window === "undefined" ? null : parseAgentFocus(readStored());
}

/** 启动时要不要弹问卷；预览模式（?previewRoute=）不弹。 */
export function needAgentFocusPrompt(): boolean {
  if (typeof window === "undefined") return false;
  if (new URLSearchParams(window.location.search).has("previewRoute")) return false;
  return shouldAskAgentFocus(readStored());
}

export function saveAgentFocus(mode: FocusMode) {
  const value = serializeAgentFocus(mode);
  try {
    window.localStorage.setItem(AGENT_FOCUS_STORAGE_KEY, value);
    sessionFallback = undefined;
  } catch {
    // 存不了就只在本次会话里生效。
    sessionFallback = value;
  }
  window.dispatchEvent(new CustomEvent(AGENT_FOCUS_EVENT));
}

/** 当前模式；问卷、设置页改了之后本窗口立即更新，别的窗口（托盘）经 storage 事件更新。 */
export function useAgentFocus(): AgentFocus {
  const [focus, setFocus] = useState<AgentFocus>(loadAgentFocus);
  useEffect(() => {
    const reload = () => setFocus(loadAgentFocus());
    const onStorage = (event: StorageEvent) => {
      if (event.key === AGENT_FOCUS_STORAGE_KEY || event.key === null) {
        sessionFallback = undefined;
        reload();
      }
    };
    window.addEventListener(AGENT_FOCUS_EVENT, reload);
    window.addEventListener("storage", onStorage);
    return () => {
      window.removeEventListener(AGENT_FOCUS_EVENT, reload);
      window.removeEventListener("storage", onStorage);
    };
  }, []);
  return focus;
}
