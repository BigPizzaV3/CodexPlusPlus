/**
 * 使用模式：启动时问一次（在首次引导之前），设置页里也能改。
 * 「只管理 Codex」隐藏其他 Agent 的相关内容，网关按 Codex 专用的单 Agent 模式显示；
 * 「只用增强功能」隐藏供应商、网关、用量，供应商交给别的工具；「全部功能」和以前一样。
 */
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { Check, LayoutGrid, LoaderCircle, RefreshCw, Sparkles } from "lucide-react";
import { t, tf } from "./i18n";
import { FOCUS_MODES, type AgentFocus, type FocusMode } from "./agent-focus-model";
import { AgentGlyph } from "./HubOverview";
import type { GatewayResult } from "./gateway-model";
import { useEscapeLayer } from "./use-escape-layer";
import { useModalFocus } from "./use-modal-focus";
import "./OnboardingTour.css";

const MODE_TEXT: Record<FocusMode, { name: string; hint: string }> = {
  codex: { name: "只管理 Codex", hint: "管理 Codex 的供应商、增强、皮肤和拓展。隐藏其他 Agent 的界面，已有网关连接保持不变。" },
  enhance: { name: "只用增强功能", hint: "保留 Codex、Claude Code 的增强、皮肤和拓展，隐藏供应商、网关和用量，并关闭旧版供应商配置切换。已有网关连接保持不变。" },
  all: { name: "全部功能", hint: "同时管理 Codex、Claude Code、Grok 等多个 Agent，经同一个网关共用供应商池。" },
};

type GatewayState = { status: "loading" | "error"; agents: string[] } | { status: "ready"; agents: string[] };

export function AgentFocusDialog({ initial, onDone, onCancel }: {
  initial: AgentFocus;
  onDone: (mode: FocusMode) => Promise<boolean>;
  onCancel?: () => void;
}) {
  // 第一次问默认选「只管理 Codex」：大多数人装 Codex++ 是为了 Codex。
  const [value, setValue] = useState<FocusMode>(initial ?? "codex");
  const [pending, setPending] = useState(false);
  const [saveFailed, setSaveFailed] = useState(false);
  const [gateway, setGateway] = useState<GatewayState>({ status: "loading", agents: [] });
  const [loadAttempt, setLoadAttempt] = useState(0);
  const dialog = useRef<HTMLDivElement>(null);
  const saving = useRef(false);
  const mounted = useRef(true);
  useModalFocus(dialog);
  useEscapeLayer(true, () => { if (!saving.current) onCancel?.(); });
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    let cancelled = false;
    setGateway({ status: "loading", agents: [] });
    // 未读到网关状态时不能承诺没有既有连接；超时后保留重试入口。
    const timeout = window.setTimeout(() => {
      cancelled = true;
      setGateway({ status: "error", agents: [] });
    }, 10_000);
    void invoke<GatewayResult>("load_gateway")
      .then((result) => {
        if (cancelled) return;
        if (result.status !== "ok" || !Array.isArray(result.agents)) {
          setGateway({ status: "error", agents: [] });
          return;
        }
        setGateway({ status: "ready", agents: result.agents.filter((agent) => agent.enabled).map((agent) => agent.name) });
      })
      .catch(() => { if (!cancelled) setGateway({ status: "error", agents: [] }); })
      .finally(() => window.clearTimeout(timeout));
    return () => {
      cancelled = true;
      window.clearTimeout(timeout);
    };
  }, [loadAttempt]);

  const choose = (mode: FocusMode) => {
    if (saving.current) return;
    setValue(mode);
    setSaveFailed(false);
  };
  const selectWithKeys = (event: KeyboardEvent<HTMLButtonElement>, mode: FocusMode) => {
    if (event.defaultPrevented || event.nativeEvent.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
    const index = FOCUS_MODES.indexOf(mode);
    let next: FocusMode | undefined;
    if (event.key === "ArrowDown" || event.key === "ArrowRight") next = FOCUS_MODES[(index + 1) % FOCUS_MODES.length];
    else if (event.key === "ArrowUp" || event.key === "ArrowLeft") next = FOCUS_MODES[(index + FOCUS_MODES.length - 1) % FOCUS_MODES.length];
    else if (event.key === "Home") next = FOCUS_MODES[0];
    else if (event.key === "End") next = FOCUS_MODES[FOCUS_MODES.length - 1];
    if (!next || saving.current) return;
    event.preventDefault();
    choose(next);
    dialog.current?.querySelector<HTMLButtonElement>(`[data-mode="${next}"]`)?.focus();
  };
  const gatewayPending = value === "enhance" && gateway.status !== "ready";
  const submit = async () => {
    if (saving.current || gatewayPending) return;
    saving.current = true;
    setPending(true);
    setSaveFailed(false);
    try {
      const saved = await onDone(value);
      if (mounted.current) setSaveFailed(!saved);
    } catch {
      if (mounted.current) setSaveFailed(true);
    } finally {
      saving.current = false;
      if (mounted.current) setPending(false);
    }
  };
  return createPortal(
    <div className="tour-root" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="agent-focus-title" aria-describedby="agent-focus-description" aria-busy={pending} tabIndex={-1}>
      <div className="tour-dim" />
      <div className="tour-card agent-focus-card" data-centered="true">
        <h3 id="agent-focus-title">{t("你打算怎么用 Codex++？")}</h3>
        <p id="agent-focus-description">{t("选择要使用的功能，之后可以在设置里修改。已有的 Agent 网关连接不会随模式切换而断开。")}</p>
        <div className="agent-focus-list" role="radiogroup" aria-label={t("使用模式")}>
          {FOCUS_MODES.map((mode) => {
            const checked = value === mode;
            return (
              <button aria-checked={checked} className="agent-focus-option" data-autofocus={checked || undefined} data-checked={checked} data-mode={mode} disabled={pending} key={mode} onClick={() => choose(mode)} onKeyDown={(event) => selectWithKeys(event, mode)} role="radio" tabIndex={checked ? 0 : -1} type="button">
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
        {value === "enhance" && gateway.status === "loading" ? (
          <p className="agent-focus-warning" role="status">{t("正在确认已有网关连接…")}</p>
        ) : value === "enhance" && gateway.status === "error" ? (
          <div className="agent-focus-status">
            <p className="agent-focus-warning" role="alert">{t("无法确认已有网关连接，请重试后再完成模式切换。")}</p>
            <div className="tour-actions"><button className="tour-ghost" onClick={() => setLoadAttempt((attempt) => attempt + 1)} type="button"><RefreshCw aria-hidden="true" size={13} />{t("重试")}</button></div>
          </div>
        ) : value === "enhance" && gateway.agents.length ? (
          <p className="agent-focus-warning">{tf("{0} 目前经 Codex++ 网关接入，切换后会继续运行并管理对应配置。要交给其他供应商工具，请先在「全部功能」的「Agent 接入」中断开。", [gateway.agents.join("、")])}</p>
        ) : null}
        {saveFailed ? <p className="agent-focus-warning" role="alert">{t("模式尚未切换。请重试，或取消后检查设置。")}</p> : null}
        <div className="tour-actions agent-focus-actions">
          {onCancel ? <button className="tour-ghost" disabled={pending} onClick={onCancel} type="button">{t(initial ? "取消" : "稍后设置")}</button> : null}
          <button className="tour-primary" disabled={pending || gatewayPending} onClick={() => void submit()} type="button">
            {pending ? <LoaderCircle aria-hidden="true" size={13} /> : null}
            {t(pending ? "正在保存…" : saveFailed ? "重试保存" : "完成")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
