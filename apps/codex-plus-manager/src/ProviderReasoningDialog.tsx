/** 供应商按模型的思考等级：在共享供应商的详情里编辑，存进这家的模型资料（格式同直连模式的「模型配置」）。
 * 写进 Agent 的模型目录（Codex 选择器里的档位），转发时按「思考参数」决定怎么发给上游。 */
import { useEffect, useId, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, X } from "lucide-react";
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
  fillUpstreamReasoning,
  type ReasoningParameterMode,
  type ReasoningSyncPick,
} from "./model-reasoning";
import { Avatar } from "./gateway-ui";
import { Input } from "@/components/ui/input";
import { ProviderMetaSyncPanel } from "./ProviderMetaSyncPanel";
import { filterModels } from "./provider-sync-model";
import { reasoningPicks } from "./provider-meta-sync-model";
import { useModalShell } from "./use-modal-shell";
import "./ProviderFormDialog.css";

type BuiltinReasoning = { model: string; builtinLevels: string[]; builtinDefault?: string | null };
type ReasoningResult = { status: string; message: string; models?: BuiltinReasoning[] };

/** 从上游同步填进来、保存时档位仍和上游一致的模型：保存后再记成「同步来的」，下次同步才会默认跟着上游更新。 */
export type { ReasoningSyncPick } from "./model-reasoning";

/** 模型多到这个数时，列表上方给一个搜索框。 */
const SEARCH_THRESHOLD = 12;


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
  upstreamSync = false,
  syncOnOpen = false,
  onSave,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  /** 能「从上游同步」（不是订阅账号）。 */
  upstreamSync?: boolean;
  /** 从详情行的「从上游同步」进来：打开就展开同步面板。 */
  syncOnOpen?: boolean;
  onSave: (modelMetadata: string, synced: ReasoningSyncPick[]) => void;
  onCancel: () => void;
}) {
  const titleId = useId();
  const [metadata, setMetadata] = useState(entry.modelMetadata ?? "");
  const [builtin, setBuiltin] = useState<Record<string, BuiltinReasoning>>({});
  // 从上游同步：展开同步面板；填进来的档位记下来做高亮，保存时还没被改掉的记成同步来的。
  const [syncOpen, setSyncOpen] = useState(upstreamSync && syncOnOpen);
  const [filled, setFilled] = useState<ReasoningSyncPick[]>([]);
  const [query, setQuery] = useState("");
  const shownModels = useMemo(() => filterModels(entry.models, query, (model) => model), [entry.models, query]);
  const { backdropProps, dialogProps, requestClose } = useModalShell<HTMLFormElement>({
    onClose: onCancel,
    dirty: metadata !== (entry.modelMetadata ?? ""),
    labelledBy: titleId,
  });

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
  // 填进来的上游档位保存时还在（没被手动改掉）。
  const stillUpstream = (pick: ReasoningSyncPick) => {
    const levels = resolveModelReasoningLevels(map[modelMetadataKey(pick.model)], []).levels;
    return levels.join(",") === normalizeReasoningLevels(pick.levels).join(",");
  };
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
      <div
        className="pf-reasoning-row"
        data-overridden={state.overridden || mode !== "auto"}
        data-upstream={filled.some((pick) => pick.model === model && stillUpstream(pick)) || undefined}
        key={model}
      >
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
    <div className="gw-modal-backdrop gw-portal" {...backdropProps}>
      <form
        {...dialogProps}
        className="pf-modal pf-extras-modal pf-reasoning-modal"
        onSubmit={(event) => {
          event.preventDefault();
          onSave(metadata, filled.filter(stillUpstream));
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong id={titleId}>{tf("{0} 的思考等级", [maskAccountName(entry.name)])}</strong>
            <small>{t("Agent 里能选的档位；没改过的按内置资料")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={() => void requestClose()} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            {upstreamSync && !syncOpen ? (
              <h4>
                {t("思考等级")}
                <button className="pf-action" onClick={() => setSyncOpen(true)} title={t("读取上游 /models 返回的思考等级，先列出差异，勾选后填入")} type="button">
                  <RefreshCw />{t("从上游同步")}
                </button>
              </h4>
            ) : null}
            <p className="pf-note">{t("点档位勾选或取消；「默认」是 Agent 新开对话时用的档位。认不出的模型借用通用模板的档位，不准的话在这里改。")}</p>
            {entry.models.length >= SEARCH_THRESHOLD ? (
              <Input aria-label={t("搜索模型")} className="pf-table-search" onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("搜索模型")} value={query} />
            ) : null}
            <div className="pf-reasoning-list">{shownModels.map(row)}</div>
          </section>
          {syncOpen ? (
            <ProviderMetaSyncPanel
              entryId={entry.id}
              kind="reasoning"
              onFill={(preview, picked) => {
                const picks = reasoningPicks(preview, picked);
                setMetadata((current) => fillUpstreamReasoning(current, picks));
                setFilled((previous) => [...previous.filter((item) => !picks.some((pick) => pick.model === item.model)), ...picks]);
              }}
            />
          ) : null}
        </div>

        <footer className="pf-foot">
          <span className="pf-error" />
          <Button onClick={() => void requestClose()} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}
