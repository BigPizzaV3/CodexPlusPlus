/** 使用模式的读写与订阅。规则在 agent-focus-model.ts，这里只管 localStorage、设置文件里的备份和广播。 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AGENT_FOCUS_EVENT,
  AGENT_FOCUS_STORAGE_KEY,
  FOCUS_MODES,
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

function writeLocal(mode: FocusMode) {
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

/**
 * 设置文件里再存一份（#2902）：页面本地存储会丢（例如管理员身份启动时 WebView 数据目录不同），
 * 启动时由 restoreAgentFocusFromSettings 补回。网页预览里没有 Tauri，跳过。
 */
export function backupAgentFocus(mode: FocusMode) {
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    void invoke("set_manager_agent_focus", { mode }).catch(() => undefined);
  }
}

export function saveAgentFocus(mode: FocusMode) {
  writeLocal(mode);
  backupAgentFocus(mode);
}

/**
 * 本地存储里没有（或版本过旧）而设置文件里记着时，补回本地存储并返回 true，调用方据此关掉问卷。
 * 只补不覆盖：本地已经有有效选择时以本地为准。
 */
export function restoreAgentFocusFromSettings(saved: string | null | undefined): boolean {
  if (typeof window === "undefined" || !shouldAskAgentFocus(readStored())) return false;
  const mode = FOCUS_MODES.find((item) => item === saved?.trim());
  if (!mode) return false;
  writeLocal(mode);
  return true;
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
