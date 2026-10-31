/**
 * 使用模式：启动时问一次，「只管理 Codex」还是「全部功能」。
 * 只管理 Codex 时，Claude Code、Grok 等其他 Agent 的相关内容在界面里隐藏，
 * 网关按单 Agent 模式呈现（只给 Codex 用的供应商池），不再讲「多个 Agent 共用」。
 *
 * 只影响界面显示，不改任何配置：其他 Agent 已接入网关的话照常工作。
 * 记录存在 localStorage（和首次引导同一处），托盘窗口同源，也能读到。
 */

export const FOCUS_MODES = ["codex", "all"] as const;
export type FocusMode = (typeof FOCUS_MODES)[number];

/** 问卷版本：选项大改时加一，所有人会再被问一次。 */
export const AGENT_FOCUS_VERSION = 1;
export const AGENT_FOCUS_STORAGE_KEY = "codex-plus-plus.agent-focus";
/** 同一窗口里改了选择时广播；别的窗口靠 `storage` 事件。 */
export const AGENT_FOCUS_EVENT = "codexplus:agent-focus";

/** null 表示没问过（或记录无效）：按全部功能显示。 */
export type AgentFocus = FocusMode | null;

type Stored = { version: number; mode: string };

function readStored(stored: string | null): Partial<Stored> | null {
  if (!stored) return null;
  try {
    const value = JSON.parse(stored) as unknown;
    return value && typeof value === "object" ? (value as Partial<Stored>) : null;
  } catch {
    return null;
  }
}

export function parseAgentFocus(stored: string | null): AgentFocus {
  const mode = readStored(stored)?.mode;
  return FOCUS_MODES.find((item) => item === mode) ?? null;
}

export function serializeAgentFocus(mode: FocusMode): string {
  return JSON.stringify({ version: AGENT_FOCUS_VERSION, mode } satisfies Stored);
}

/** 是否要在启动时问：没记录、记录无效，或是旧版本问卷的记录。 */
export function shouldAskAgentFocus(stored: string | null): boolean {
  if (!parseAgentFocus(stored)) return true;
  const version = Number(readStored(stored)?.version);
  return !Number.isFinite(version) || version < AGENT_FOCUS_VERSION;
}

/** 某个 Agent 要不要显示：只管理 Codex 时只显示 Codex。 */
export function agentVisible(focus: AgentFocus, agent: string): boolean {
  return focus !== "codex" || agent === "codex";
}

/** 单 Agent 模式下的那个 Agent（只管理 Codex 时是 codex），否则 null。 */
export function singleAgent(focus: AgentFocus): string | null {
  return focus === "codex" ? "codex" : null;
}

/** 按使用模式过滤带 `agent` 字段的列表；全部功能时原样返回。 */
export function visibleAgents<T extends { agent: string }>(focus: AgentFocus, items: T[]): T[] {
  return focus === "codex" ? items.filter((item) => agentVisible(focus, item.agent)) : items;
}
