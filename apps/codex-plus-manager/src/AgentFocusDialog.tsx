/**
 * 使用模式：启动时问一次（在首次引导之前），设置页里也能改。
 * 「只管理 Codex」隐藏其他 Agent 的相关内容，网关按 Codex 专用的单 Agent 模式显示；「全部功能」和以前一样。
 */
import { useState } from "react";
import { createPortal } from "react-dom";
import { Check, LayoutGrid } from "lucide-react";
import { t } from "./i18n";
import { FOCUS_MODES, type AgentFocus, type FocusMode } from "./agent-focus-model";
import { AgentGlyph } from "./HubOverview";
import { useEscapeLayer } from "./use-escape-layer";
import "./OnboardingTour.css";

const MODE_TEXT: Record<FocusMode, { name: string; hint: string }> = {
  codex: { name: "只管理 Codex", hint: "换供应商、增强、皮肤、拓展。隐藏 Claude Code、Grok 等其他 Agent 的内容，网关只给 Codex 用。" },
  all: { name: "全部功能", hint: "同时管理 Codex、Claude Code、Grok 等多个 Agent，经同一个网关共用供应商池。" },
};

export function AgentFocusDialog({ initial, onDone }: { initial: AgentFocus; onDone: (mode: FocusMode) => void }) {
  // 第一次问默认选「只管理 Codex」：大多数人装 Codex++ 是为了 Codex。
  const [value, setValue] = useState<FocusMode>(initial ?? "codex");
  // Esc = 全部功能，和以前一样。
  useEscapeLayer(true, () => onDone(initial ?? "all"));
  return createPortal(
    <div className="tour-root" role="dialog" aria-modal="true" aria-labelledby="agent-focus-title">
      <div className="tour-dim" />
      <div className="tour-card agent-focus-card" data-centered="true">
        <h3 id="agent-focus-title">{t("你打算怎么用 Codex++？")}</h3>
        <p>{t("Codex++ 除了 Codex 还能管理其他 Agent。只用 Codex 的话，可以隐藏用不到的内容，界面更清爽。以后可以在设置里改。")}</p>
        <div className="agent-focus-list" role="radiogroup" aria-label={t("使用模式")}>
          {FOCUS_MODES.map((mode) => {
            const checked = value === mode;
            return (
              <button aria-checked={checked} className="agent-focus-option" data-checked={checked} key={mode} onClick={() => setValue(mode)} role="radio" type="button">
                <span className="agent-focus-icon">{mode === "codex" ? <AgentGlyph agent="codex" /> : <LayoutGrid aria-hidden="true" size={16} />}</span>
                <span className="agent-focus-text">
                  <strong>{t(MODE_TEXT[mode].name)}</strong>
                  <small>{t(MODE_TEXT[mode].hint)}</small>
                </span>
                <span className="agent-focus-check" aria-hidden="true">{checked ? <Check size={12} /> : null}</span>
              </button>
            );
          })}
        </div>
        <div className="tour-actions agent-focus-actions">
          <button className="tour-primary" onClick={() => onDone(value)} type="button">{t("完成")}</button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
