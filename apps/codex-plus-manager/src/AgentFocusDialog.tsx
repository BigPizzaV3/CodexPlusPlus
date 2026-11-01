/**
 * 使用模式：启动时问一次（在首次引导之前），设置页里也能改。
 * 「只管理 Codex」隐藏其他 Agent 的相关内容，网关按 Codex 专用的单 Agent 模式显示；
 * 「只用增强功能」隐藏供应商、网关、用量，供应商交给别的工具；「全部功能」和以前一样。
 */
import { useState } from "react";
import { createPortal } from "react-dom";
import { Check, LayoutGrid, Sparkles } from "lucide-react";
import { t, tf } from "./i18n";
import { FOCUS_MODES, type AgentFocus, type FocusMode } from "./agent-focus-model";
import { AgentGlyph, useGatewayPayload } from "./HubOverview";
import { useEscapeLayer } from "./use-escape-layer";
import "./OnboardingTour.css";

const MODE_TEXT: Record<FocusMode, { name: string; hint: string }> = {
  codex: { name: "只管理 Codex", hint: "换供应商、增强、皮肤、拓展。隐藏 Claude Code、Grok 等其他 Agent 的内容，网关只给 Codex 用。" },
  enhance: { name: "只用增强功能", hint: "供应商用别的工具管理。保留 Codex、Claude Code 的增强、皮肤、拓展，隐藏供应商、网关和用量，Codex++ 也不再改 Codex 的供应商配置。" },
  all: { name: "全部功能", hint: "同时管理 Codex、Claude Code、Grok 等多个 Agent，经同一个网关共用供应商池。" },
};

export function AgentFocusDialog({ initial, onDone }: { initial: AgentFocus; onDone: (mode: FocusMode) => void }) {
  // 第一次问默认选「只管理 Codex」：大多数人装 Codex++ 是为了 Codex。
  const [value, setValue] = useState<FocusMode>(initial ?? "codex");
  // Esc = 全部功能，和以前一样。
  useEscapeLayer(true, () => onDone(initial ?? "all"));
  // 只用增强功能时网关界面会隐藏，但已经接入网关的 Agent 仍然走网关：提醒先断开，免得和别的供应商工具冲突。
  const gateway = useGatewayPayload(30_000);
  const viaGateway = (gateway?.agents ?? []).filter((agent) => agent.enabled).map((agent) => agent.name);
  return createPortal(
    <div className="tour-root" role="dialog" aria-modal="true" aria-labelledby="agent-focus-title">
      <div className="tour-dim" />
      <div className="tour-card agent-focus-card" data-centered="true">
        <h3 id="agent-focus-title">{t("你打算怎么用 Codex++？")}</h3>
        <p>{t("Codex++ 能统一管理多个 Agent 和供应商，也能只给 Codex 加增强功能。选最接近你的用法，用不到的内容会被隐藏，界面更清爽。以后可以在设置里改。")}</p>
        <div className="agent-focus-list" role="radiogroup" aria-label={t("使用模式")}>
          {FOCUS_MODES.map((mode) => {
            const checked = value === mode;
            return (
              <button aria-checked={checked} className="agent-focus-option" data-checked={checked} key={mode} onClick={() => setValue(mode)} role="radio" type="button">
                <span className="agent-focus-icon">
                  {mode === "codex" ? <AgentGlyph agent="codex" /> : mode === "enhance" ? <Sparkles aria-hidden="true" size={16} /> : <LayoutGrid aria-hidden="true" size={16} />}
                </span>
                <span className="agent-focus-text">
                  <strong>{t(MODE_TEXT[mode].name)}</strong>
                  <small>{t(MODE_TEXT[mode].hint)}</small>
                </span>
                <span className="agent-focus-check" aria-hidden="true">{checked ? <Check size={12} /> : null}</span>
              </button>
            );
          })}
        </div>
        {value === "enhance" && viaGateway.length ? (
          <p className="agent-focus-warning">{tf("{0} 目前经 Codex++ 网关接入。切到只用增强功能后网关照常运行；要交给别的工具管理供应商，先在「全部功能」里的「Agent 接入」断开。", [viaGateway.join("、")])}</p>
        ) : null}
        <div className="tour-actions agent-focus-actions">
          <button className="tour-primary" onClick={() => onDone(value)} type="button">{t("完成")}</button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
