/** 供应商详情的「从上游同步」（#2785）：先用存下的凭据拉上游、列出差异，不改任何数据；
 * 用户勾选后才写入。上游已没有的默认勾选移除（可取消以保留），上游新增的默认不勾、可搜索多选；
 * 默认模型在移除之列时提示会换成哪个。 */
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import { maskAccountName, type GatewayPoolEntry } from "./gateway-model";
import { Avatar } from "./gateway-ui";
import { useEscapeLayer } from "./use-escape-layer";
import { filterModels, syncOutcome, type ModelSyncPreview } from "./provider-sync-model";
import "./ProviderFormDialog.css";

type PreviewResult = { status: string; message: string } & Partial<ModelSyncPreview>;
type Loaded = { state: "loading" } | { state: "error"; message: string } | { state: "ready"; preview: ModelSyncPreview };

export function ProviderSyncDialog({
  entry,
  busy,
  onApply,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  /** 只删 `remove`、只加 `add`；写入成功后由调用方关掉弹窗。 */
  onApply: (remove: string[], add: string[]) => void;
  onCancel: () => void;
}) {
  const [loaded, setLoaded] = useState<Loaded>({ state: "loading" });
  // 勾选移除的（默认全部）和勾选新增的（默认不勾）。
  const [remove, setRemove] = useState<Set<string>>(new Set());
  const [add, setAdd] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  useEscapeLayer(true, onCancel);

  const load = async () => {
    setLoaded({ state: "loading" });
    try {
      const result = await invoke<PreviewResult>("preview_gateway_provider_models_sync", { id: entry.id });
      if (result.status !== "ok" || !result.upstream) {
        setLoaded({ state: "error", message: result.message });
        return;
      }
      const preview: ModelSyncPreview = {
        upstream: result.upstream,
        current: result.current ?? [],
        defaultModel: result.defaultModel ?? "",
        added: result.added ?? [],
        removed: result.removed ?? [],
      };
      setRemove(new Set(preview.removed));
      setAdd(new Set());
      setLoaded({ state: "ready", preview });
    } catch (error) {
      setLoaded({ state: "error", message: String(error) });
    }
  };

  useEffect(() => {
    void load();
    // 打开时拉一次；「重试」再拉。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const preview = loaded.state === "ready" ? loaded.preview : null;
  const visibleAdded = useMemo(() => filterModels(preview?.added ?? [], query, (model) => model), [preview, query]);
  const outcome = preview ? syncOutcome(preview, remove, add) : null;
  const changes = remove.size + add.size;
  const toggle = (set: Set<string>, model: string, update: (next: Set<string>) => void) => {
    const next = new Set(set);
    if (next.has(model)) next.delete(model);
    else next.add(model);
    update(next);
  };

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="pf-modal pf-sync-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (!preview || !changes || !outcome?.models.length) return;
          onApply(preview.removed.filter((model) => remove.has(model)), preview.added.filter((model) => add.has(model)));
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{tf("从上游同步 {0} 的模型", [maskAccountName(entry.name)])}</strong>
            <small>{t("先列出和上游的差异，勾选后才会改模型列表")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={onCancel} type="button"><X /></button>
        </header>

        <div className="pf-body">
          {loaded.state === "loading" ? (
            <section className="pf-section">
              <p className="pf-note"><RefreshCw className="animate-spin" />{t("正在用已保存的 Key 或登录获取上游模型…")}</p>
            </section>
          ) : null}
          {loaded.state === "error" ? (
            <section className="pf-section">
              <p className="pf-note" data-tone="error">{loaded.message}</p>
              <button className="pf-link" onClick={() => void load()} type="button"><RefreshCw />{t("重试")}</button>
            </section>
          ) : null}
          {preview && !preview.added.length && !preview.removed.length ? (
            <section className="pf-section">
              <p className="pf-note" data-tone="ok">{tf("上游有 {0} 个模型，列表已是最新。", [preview.upstream.length])}</p>
            </section>
          ) : null}

          {preview?.removed.length ? (
            <section className="pf-section">
              <h4>
                {t("上游已没有")}<em>{preview.removed.length}</em>
                <small>{t("默认移除，取消勾选可保留")}</small>
              </h4>
              <div className="pf-picker">
                <div className="pf-picker-list">
                  {preview.removed.map((model) => (
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

          {preview?.added.length ? (
            <section className="pf-section">
              <h4>
                {t("上游新增")}<em>{preview.added.length}</em>
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
                  {visibleAdded.slice(0, 500).map((model) => (
                    <label key={model}>
                      <input checked={add.has(model)} onChange={() => toggle(add, model, setAdd)} type="checkbox" />
                      <span>{model}</span>
                    </label>
                  ))}
                  {!visibleAdded.length ? <span className="pf-picker-empty">{t("没有匹配的模型")}</span> : null}
                </div>
              </div>
            </section>
          ) : null}

          {preview && outcome?.defaultReplaced && outcome.defaultModel ? (
            <section className="pf-section">
              <p className="pf-note">{tf("默认模型 {0} 将被移除，改用 {1}。", [preview.defaultModel, outcome.defaultModel])}</p>
            </section>
          ) : null}
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{preview && outcome && changes && !outcome.models.length ? t("至少保留一个模型") : ""}</span>
          {preview && changes ? <span className="pf-sync-summary">{tf("新增 {0} · 移除 {1}", [add.size, remove.size])}</span> : null}
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{preview && !changes ? t("关闭") : t("取消")}</Button>
          {preview && (preview.added.length || preview.removed.length) ? (
            <Button disabled={busy || !changes || !outcome?.models.length} size="sm" type="submit">{busy ? t("保存中") : t("应用")}</Button>
          ) : null}
        </footer>
      </form>
    </div>,
    document.body,
  );
}
