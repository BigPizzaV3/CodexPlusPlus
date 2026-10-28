/** 供应商按模型的思考等级：在供应商池的详情里编辑，存进这家的模型资料（格式同直连模式的「模型配置」）。
 * 写进 Agent 的模型目录（Codex 选择器里的档位），转发时按「思考参数」决定怎么发给上游。 */
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import { AppSelect } from "./AppSelect";
import { maskAccountName, type GatewayPoolEntry } from "./gateway-model";
import { modelMetadataKey, parseModelMetadataMap } from "./model-metadata";
import {
  REASONING_LEVELS,
  normalizeReasoningLevels,
  reasoningParameterMode,
  resolveModelReasoningLevels,
  setModelReasoningLevelsForSlug,
  setModelReasoningParameterMode,
  type ReasoningParameterMode,
} from "./model-reasoning";
import { Avatar } from "./gateway-ui";
import { useEscapeLayer } from "./use-escape-layer";
import "./ProviderFormDialog.css";

type BuiltinReasoning = { model: string; builtinLevels: string[]; builtinDefault?: string | null };
type ReasoningResult = { status: string; message: string; models?: BuiltinReasoning[] };

/** 详情里一行的展示文字：改过几个模型；没改过为空。 */
export function reasoningSummary(entry: GatewayPoolEntry): string {
  const map = parseModelMetadataMap(entry.modelMetadata ?? "");
  const changed = entry.models.filter((model) => {
    const metadata = map[modelMetadataKey(model)];
    return metadata && ("supported_reasoning_levels" in metadata || "codex_plus_reasoning_parameters" in metadata);
  }).length;
  return changed ? tf("{0} / {1} 个模型自定义", [changed, entry.models.length]) : "";
}

export function ProviderReasoningDialog({
  entry,
  busy,
  onSave,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  onSave: (modelMetadata: string) => void;
  onCancel: () => void;
}) {
  const [metadata, setMetadata] = useState(entry.modelMetadata ?? "");
  const [builtin, setBuiltin] = useState<Record<string, BuiltinReasoning>>({});
  useEscapeLayer(true, onCancel);

  // 内置档位（没改过时显示的值）由后端按模型资料算。
  useEffect(() => {
    let alive = true;
    invoke<ReasoningResult>("gateway_provider_reasoning", { id: entry.id })
      .then((result) => {
        if (!alive || result.status !== "ok") return;
        const next: Record<string, BuiltinReasoning> = {};
        for (const item of result.models ?? []) next[item.model.toLowerCase()] = item;
        setBuiltin(next);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [entry.id]);

  const map = parseModelMetadataMap(metadata);
  const row = (model: string) => {
    const override = map[modelMetadataKey(model)];
    const base = builtin[model.toLowerCase()];
    const state = resolveModelReasoningLevels(override, base?.builtinLevels ?? []);
    const mode = reasoningParameterMode(override);
    const setLevels = (levels: string[], defaultLevel: string | null) =>
      setMetadata((current) => setModelReasoningLevelsForSlug(current, model, levels, defaultLevel));
    const toggle = (level: string) => {
      const next = normalizeReasoningLevels(state.levels.includes(level) ? state.levels.filter((item) => item !== level) : [...state.levels, level]);
      setLevels(next, state.defaultLevel && next.includes(state.defaultLevel) ? state.defaultLevel : null);
    };
    const shownDefault = state.overridden ? state.defaultLevel : base?.builtinDefault ?? null;
    return (
      <div className="pf-reasoning-row" data-overridden={state.overridden || mode !== "auto"} key={model}>
        <div className="pf-reasoning-head">
          <code title={model}>{model}</code>
          {state.overridden || mode !== "auto" ? (
            <button
              className="pf-reasoning-reset"
              onClick={() => setMetadata((current) => setModelReasoningParameterMode(setModelReasoningLevelsForSlug(current, model, [], null), model, "auto"))}
              type="button"
            >
              {t("恢复内置")}
            </button>
          ) : (
            <span className="pf-reasoning-tag">{t("内置")}</span>
          )}
          <AppSelect
            disabled={mode === "omit" || !state.levels.length}
            label={t("默认等级")}
            menuWidth={150}
            onChange={(next) => setLevels(state.levels, next || null)}
            options={[{ value: "", label: t("默认：自动") }, ...state.levels.map((level) => ({ value: level, label: tf("默认：{0}", [level]) }))]}
            size="sm"
            value={shownDefault ?? ""}
          />
          <AppSelect
            label={t("思考参数")}
            menuWidth={190}
            onChange={(next) => setMetadata((current) => setModelReasoningParameterMode(current, model, next as ReasoningParameterMode))}
            options={[
              { value: "auto", label: t("自动兼容"), hint: t("按上游的写法换成它认的参数") },
              { value: "passthrough", label: t("原样传等级"), hint: t("Chat 上游原样带 reasoning_effort") },
              { value: "omit", label: t("不发送"), hint: t("不给上游发思考参数") },
            ]}
            size="sm"
            value={mode}
          />
        </div>
        <div className="pf-reasoning-levels" data-disabled={mode === "omit"}>
          {REASONING_LEVELS.map((level) => (
            <button
              aria-pressed={state.levels.includes(level)}
              disabled={mode === "omit"}
              key={level}
              onClick={() => toggle(level)}
              type="button"
            >
              {level}
            </button>
          ))}
        </div>
      </div>
    );
  };

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="pf-modal pf-extras-modal pf-reasoning-modal"
        onSubmit={(event) => {
          event.preventDefault();
          onSave(metadata);
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{tf("{0} 的思考等级", [maskAccountName(entry.name)])}</strong>
            <small>{t("Agent 里能选的档位；没改过的按内置资料")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={onCancel} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            <p className="pf-note">{t("点档位勾选或取消；「默认」是 Agent 新开对话时用的档位。认不出的模型借用通用模板的档位，不准的话在这里改。")}</p>
            <div className="pf-reasoning-list">{entry.models.map(row)}</div>
          </section>
        </div>

        <footer className="pf-foot">
          <span className="pf-error" />
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}
