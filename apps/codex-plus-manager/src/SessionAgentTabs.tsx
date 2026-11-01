/** 「会话」页顶部的 Agent 页签：全部 / Codex / Claude Code / Grok。
 *  Codex 页签显示完整的会话管理（删除、归档、修复），其余显示多 Agent 会话列表（在终端继续）。 */
import { t } from "./i18n";
import type { SessionAgent } from "./gateway-model";
import { AgentGlyph } from "./HubOverview";
import { agentVisible } from "./agent-focus-model";
import { useAgentFocus } from "./agent-focus-store";

export type SessionTab = SessionAgent | "all";

export function SessionAgentTabs({ value, onChange }: { value: SessionTab; onChange: (next: SessionTab) => void }) {
  // 当前使用模式下隐藏的 Agent（如只用增强功能时的 Grok）不出页签。
  const focus = useAgentFocus();
  const tabs = ([
    { id: "all", label: t("全部") },
    { id: "codex", label: "Codex" },
    { id: "claude", label: "Claude Code" },
    { id: "grok", label: "Grok" },
  ] as Array<{ id: SessionTab; label: string }>).filter((tab) => tab.id === "all" || agentVisible(focus, tab.id));
  return (
    <nav aria-label={t("按 Agent 查看会话")} className="tp-tabs" role="tablist">
      {tabs.map((tab) => (
        <button aria-selected={value === tab.id} data-active={value === tab.id} key={tab.id} onClick={() => onChange(tab.id)} role="tab" type="button">
          {tab.id !== "all" ? <AgentGlyph agent={tab.id} /> : null}
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
