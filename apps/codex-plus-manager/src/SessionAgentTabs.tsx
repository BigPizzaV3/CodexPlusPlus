/** 「会话」页顶部的 Agent 页签：Codex / Claude Code / Grok / 全部（默认 Codex）。
 *  Codex 页签显示完整的会话管理（删除、归档、修复），其余显示多 Agent 会话列表（复制恢复命令、删除）。 */
import { t } from "./i18n";
import type { SessionAgent } from "./gateway-model";
import { AgentGlyph } from "./hub-shared";

export type SessionTab = SessionAgent | "all";

export function SessionAgentTabs({ value, onChange }: { value: SessionTab; onChange: (next: SessionTab) => void }) {
  const tabs: Array<{ id: SessionTab; label: string }> = [
    { id: "codex", label: "Codex" },
    { id: "claude", label: "Claude Code" },
    { id: "grok", label: "Grok" },
    { id: "all", label: t("全部") },
  ];
  return (
    // 一组切换按钮（aria-pressed），和「按项目 / 按时间」一样；不是完整的 tab 模式，不声明 tablist / tab。
    <div aria-label={t("按 Agent 查看会话")} className="tp-tabs" role="group">
      {tabs.map((tab) => (
        <button aria-pressed={value === tab.id} data-active={value === tab.id} key={tab.id} onClick={() => onChange(tab.id)} type="button">
          {tab.id !== "all" ? <AgentGlyph agent={tab.id} /> : null}
          {tab.label}
        </button>
      ))}
    </div>
  );
}
