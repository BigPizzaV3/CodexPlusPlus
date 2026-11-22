/** 供应商的模型列表：手动增删、设默认模型，以及「从上游同步」（#2785，原来单独的同步弹窗并进来了）。
 * 同步用存下的凭据（订阅账号也能刷）拉上游、列出差异：上游已没有的默认勾选移除（可取消以保留），
 * 上游新增的默认不勾、可搜索多选。勾选的改动和手动增删一起在保存时写入；后台检查已发现的差异打开时直接带进来。 */
import { useEffect, useId, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Search, Star, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import { maskAccountName, splitModels, type GatewayPoolEntry } from "./gateway-model";
import { Avatar } from "./gateway-ui";
import { filterModels, sameModel, upstreamDiff, type ModelSyncPreview, type UpstreamDiff } from "./provider-sync-model";
import { useModalShell } from "./use-modal-shell";
import "./ProviderFormDialog.css";

/** 同步勾选的改动：保存时先按它写（被删模型的价格、窗口、思考等级一并清掉），再写最终列表。 */
export type ModelSyncPicks = { remove: string[]; add: string[] };

type PreviewResult = { status: string; message: string } & Partial<ModelSyncPreview>;
type SyncState =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "ready"; diff: UpstreamDiff; upstreamCount: number | null };

/** 上游新增的勾选列表最多画这么多行，多的靠搜索。 */
const PICKER_LIMIT = 500;

export function ProviderModelsDialog({
  entry,
  busy,
  syncOnOpen = false,
  onSave,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  /** 从「从上游同步」「+N 新模型」进来：打开就拉上游。 */
  syncOnOpen?: boolean;
  onSave: (models: string[], defaultModel: string, sync: ModelSyncPicks | null) => void;
  onCancel: () => void;
}) {
  const titleId = useId();
  const [models, setModels] = useState<string[]>(entry.models);
  const [defaultModel, setDefaultModel] = useState(entry.models[0] ?? "");
  const [sync, setSync] = useState<SyncState>(() => {
    if (syncOnOpen) return { state: "loading" };
    // 后台检查已经发现的差异直接带进来，不用再拉。
    const added = entry.upstreamNew ?? [];
    const removed = entry.upstreamGone ?? [];
    return added.length || removed.length ? { state: "ready", diff: { added, removed }, upstreamCount: null } : { state: "idle" };
  });
  // 勾选移除的（拿到差异时默认全部）和勾选新增的（默认不勾）。
  const [remove, setRemove] = useState<Set<string>>(() => new Set(syncOnOpen ? [] : entry.upstreamGone ?? []));
  const [add, setAdd] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [input, setInput] = useState("");

  const load = async () => {
    setSync({ state: "loading" });
    try {
      const result = await invoke<PreviewResult>("preview_gateway_provider_models_sync", { id: entry.id });
      if (result.status !== "ok" || !result.upstream) {
        setSync({ state: "error", message: t(result.message) });
        return;
      }
      // 和对话框里当前的列表比（可能已经手动改过）。
      const diff = upstreamDiff(models, result.upstream);
      setRemove(new Set(diff.removed));
      setAdd(new Set());
      setSync({ state: "ready", diff, upstreamCount: result.upstream.length });
    } catch (error) {
      setSync({ state: "error", message: String(error) });
    }
  };
  useEffect(() => {
    if (syncOnOpen) void load();
    // 只在打开时拉一次；之后点「从上游同步」再拉。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const diff = sync.state === "ready" ? sync.diff : null;
  // 手动增删以后，差异按当前列表重新对一遍：已手动加进来的不算新增，已手动删掉的不算下架。
  const pendingAdded = useMemo(() => (diff?.added ?? []).filter((model) => !models.some((item) => sameModel(item, model))), [diff, models]);
  const pendingRemoved = useMemo(() => models.filter((model) => (diff?.removed ?? []).some((item) => sameModel(item, model))), [diff, models]);
  const picks = useMemo<ModelSyncPicks>(() => ({
    remove: pendingRemoved.filter((model) => remove.has(model)),
    add: pendingAdded.filter((model) => add.has(model)),
  }), [pendingRemoved, pendingAdded, remove, add]);
  const finalModels = useMemo(() => {
    const kept = models.filter((model) => !picks.remove.includes(model));
    for (const model of picks.add) if (!kept.some((item) => sameModel(item, model))) kept.push(model);
    return kept;
  }, [models, picks]);
  const finalDefault = finalModels.find((model) => model === defaultModel) ?? finalModels[0] ?? "";
  const defaultReplaced = Boolean(defaultModel) && picks.remove.includes(defaultModel);
  const visibleAdded = useMemo(() => filterModels(pendingAdded, query, (model) => model), [pendingAdded, query]);
  const dirty = picks.remove.length > 0 || picks.add.length > 0
    || models.length !== entry.models.length || models.some((model, index) => model !== entry.models[index])
    || defaultModel !== (entry.models[0] ?? "");
  const { backdropProps, dialogProps, requestClose } = useModalShell<HTMLFormElement>({ onClose: onCancel, dirty, labelledBy: titleId });

  const addModels = (list: string[]) => setModels((previous) => [...previous, ...list.filter((model) => !previous.some((item) => sameModel(item, model)))]);
  const removeModel = (model: string) => setModels((previous) => previous.filter((item) => item !== model));
  const commitInput = () => {
    if (input.trim()) addModels(splitModels(input));
    setInput("");
  };
  const toggle = (set: Set<string>, model: string, update: (next: Set<string>) => void) => {
    const next = new Set(set);
    if (next.has(model)) next.delete(model);
    else next.add(model);
    update(next);
  };

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" {...backdropProps}>
      <form
        {...dialogProps}
        className="pf-modal pf-sync-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (!finalModels.length) return;
          onSave(finalModels, finalDefault, picks.remove.length || picks.add.length ? picks : null);
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong id={titleId}>{tf("{0} 的模型", [maskAccountName(entry.name)])}</strong>
            <small>{t("Agent 请求这些模型名时会路由到这个供应商")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={() => void requestClose()} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            <h4>
              {t("模型")}<em>{models.length}</em>
              <button
                className="pf-action"
                disabled={sync.state === "loading"}
                onClick={() => void load()}
                title={t("用已保存的 Key 或登录拉取上游模型，先列出差异，勾选后再改列表")}
                type="button"
              >
                <RefreshCw className={sync.state === "loading" ? "animate-spin" : ""} />{sync.state === "loading" ? t("获取中…") : t("从上游同步")}
              </button>
            </h4>
            <div className="pf-chips">
              {models.map((model) => {
                const leaving = picks.remove.includes(model);
                return (
                  <span className="pf-chip" data-default={model === defaultModel} data-gone={leaving} key={model}>
                    <button aria-label={tf("设 {0} 为默认模型", [model])} className="pf-star" onClick={() => setDefaultModel(model)} title={model === defaultModel ? t("默认模型") : t("设为默认模型")} type="button">
                      <Star />
                    </button>
                    <span title={leaving ? t("上游已没有这个模型") : undefined}>{model}</span>
                    <button aria-label={tf("移除 {0}", [model])} className="pf-chip-remove" onClick={() => removeModel(model)} type="button"><X /></button>
                  </span>
                );
              })}
              <input
                aria-label={t("添加模型")}
                className="pf-chip-input"
                onBlur={commitInput}
                onChange={(event) => setInput(event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === ",") {
                    event.preventDefault();
                    commitInput();
                  }
                }}
                placeholder={models.length ? t("添加模型…") : t("输入模型名回车添加，或从上游同步")}
                value={input}
              />
            </div>
            <p className="pf-hint">{t("星标是默认模型。新加的模型保存后会按公开价格表自动填价格。")}</p>
          </section>

          {sync.state === "loading" ? (
            <section className="pf-section">
              <p className="pf-note"><RefreshCw className="animate-spin" />{t("正在用已保存的 Key 或登录获取上游模型…")}</p>
            </section>
          ) : null}
          {sync.state === "error" ? (
            <section className="pf-section">
              <p className="pf-note" data-tone="error">{sync.message}</p>
              <button className="pf-link" onClick={() => void load()} type="button"><RefreshCw />{t("重试")}</button>
            </section>
          ) : null}
          {sync.state === "ready" && !pendingAdded.length && !pendingRemoved.length && sync.upstreamCount !== null ? (
            <section className="pf-section">
              <p className="pf-note" data-tone="ok">{tf("上游有 {0} 个模型，列表已是最新。", [sync.upstreamCount])}</p>
            </section>
          ) : null}

          {pendingRemoved.length ? (
            <section className="pf-section">
              <h4>
                {t("上游已没有")}<em>{pendingRemoved.length}</em>
                <small>{t("默认移除，取消勾选可保留")}</small>
              </h4>
              <div className="pf-picker">
                <div className="pf-picker-list">
                  {pendingRemoved.map((model) => (
                    <label key={model}>
                      <input checked={remove.has(model)} onChange={() => toggle(remove, model, setRemove)} type="checkbox" />
                      <span>{model}</span>
                      <span className="pf-picker-tag" data-tone={remove.has(model) ? "remove" : undefined}>
                        {remove.has(model) ? t("将移除") : t("保留")}
                      </span>
                    </label>
                  ))}
                </div>
              </div>
              <p className="pf-hint">{t("移除的模型单独填的价格、上下文窗口和思考等级会一并清掉。")}</p>
            </section>
          ) : null}

          {pendingAdded.length ? (
            <section className="pf-section">
              <h4>
                {t("上游新增")}<em>{pendingAdded.length}</em>
                <small>{t("默认不加，勾选要加入的")}</small>
              </h4>
              <div className="pf-picker">
                <div className="pf-picker-head">
                  <Search />
                  <input
                    aria-label={t("搜索上游新增的模型")}
                    onChange={(event) => setQuery(event.currentTarget.value)}
                    onKeyDown={(event) => {
                      // 搜索框在表单里：回车只是搜索，不能提交。
                      if (event.key === "Enter") event.preventDefault();
                    }}
                    placeholder={t("搜索模型")}
                    value={query}
                  />
                  <button className="pf-link" disabled={!visibleAdded.length} onClick={() => setAdd(new Set([...add, ...visibleAdded]))} type="button">{t("全选")}</button>
                  <button className="pf-link" disabled={!visibleAdded.some((model) => add.has(model))} onClick={() => setAdd(new Set([...add].filter((model) => !visibleAdded.includes(model))))} type="button">{t("全不选")}</button>
                </div>
                <div className="pf-picker-list">
                  {visibleAdded.slice(0, PICKER_LIMIT).map((model) => (
                    <label key={model}>
                      <input checked={add.has(model)} onChange={() => toggle(add, model, setAdd)} type="checkbox" />
                      <span>{model}</span>
                    </label>
                  ))}
                  {!visibleAdded.length ? <span className="pf-picker-empty">{t("没有匹配的模型")}</span> : null}
                  {visibleAdded.length > PICKER_LIMIT ? (
                    <span className="pf-picker-empty">{tf("只显示前 {0} 个，共 {1} 个；用搜索缩小范围。", [PICKER_LIMIT, visibleAdded.length])}</span>
                  ) : null}
                </div>
              </div>
            </section>
          ) : null}

          {defaultReplaced && finalDefault ? (
            <section className="pf-section">
              <p className="pf-note">{tf("默认模型 {0} 将被移除，改用 {1}。", [defaultModel, finalDefault])}</p>
            </section>
          ) : null}
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{finalModels.length ? "" : t("至少保留一个模型")}</span>
          {picks.add.length || picks.remove.length ? <span className="pf-sync-summary">{tf("新增 {0} · 移除 {1}", [picks.add.length, picks.remove.length])}</span> : null}
          <Button onClick={() => void requestClose()} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy || !finalModels.length} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}
