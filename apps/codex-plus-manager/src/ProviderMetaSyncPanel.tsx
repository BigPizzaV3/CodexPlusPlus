/** 上下文窗口 / 思考等级弹窗里的「从上游同步」（#2807，原来单独的同步弹窗并进来了）：
 * 用存下的凭据拉上游 `/models`，读每个模型的元数据，列出和现有设置的差异（新值、旧值、来源字段）。
 * 现值是用户手动设的默认不勾（可勾选覆盖）；「填入」只改弹窗里的表格并高亮，保存时才写入。
 * 上游字段不合理、档位认不出的在下方列出，不会写入。 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw } from "lucide-react";
import { t, tf } from "./i18n";
import { errorText } from "@/lib/utils";
import {
  changeRows,
  currentSourceLabel,
  defaultPicked,
  ignoredFor,
  ignoredReasonLabel,
  normalizePreview,
  type MetaSyncKind,
  type MetaSyncPreview,
} from "./provider-meta-sync-model";

type PreviewResult = { status: string; message: string } & Partial<MetaSyncPreview>;
type Loaded = { state: "loading" } | { state: "error"; message: string } | { state: "ready"; preview: MetaSyncPreview };

export function ProviderMetaSyncPanel({
  entryId,
  kind,
  onFill,
}: {
  entryId: string;
  kind: MetaSyncKind;
  /** 把勾选的上游值填进弹窗的表格（不写入）。 */
  onFill: (preview: MetaSyncPreview, picked: ReadonlySet<string>) => void;
}) {
  const [loaded, setLoaded] = useState<Loaded>({ state: "loading" });
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [filled, setFilled] = useState<number | null>(null);

  const load = async () => {
    setLoaded({ state: "loading" });
    setFilled(null);
    try {
      const result = await invoke<PreviewResult>("preview_gateway_provider_meta_sync", { id: entryId });
      if (result.status !== "ok") {
        setLoaded({ state: "error", message: t(result.message) });
        return;
      }
      const preview = normalizePreview(result);
      setPicked(defaultPicked(preview, kind));
      setLoaded({ state: "ready", preview });
    } catch (error) {
      setLoaded({ state: "error", message: t(errorText(error)) });
    }
  };

  useEffect(() => {
    void load();
    // 打开时拉一次；「重试」「重新读取」再拉。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const preview = loaded.state === "ready" ? loaded.preview : null;
  const rows = preview ? changeRows(preview, kind, t("默认")) : [];
  const ignored = preview ? ignoredFor(preview, kind) : [];
  const found = preview ? (kind === "windows" ? preview.withWindow : preview.withReasoning) : 0;
  const toggle = (model: string) => {
    const next = new Set(picked);
    if (next.has(model)) next.delete(model);
    else next.add(model);
    setPicked(next);
  };

  return (
    <section className="pf-section pf-meta-sync-panel">
      <h4>
        {t("从上游同步")}
        <small>{t("读取上游 /models 返回的模型资料，先列出差异，填入后保存才会写入")}</small>
        {preview ? (
          <button className="pf-action" onClick={() => void load()} type="button"><RefreshCw />{t("重新读取")}</button>
        ) : null}
      </h4>
      {loaded.state === "loading" ? (
        <p className="pf-note"><RefreshCw className="animate-spin" />{t("正在用已保存的 Key 读取上游模型资料…")}</p>
      ) : null}
      {loaded.state === "error" ? (
        <>
          <p className="pf-note" data-tone="error">{loaded.message}</p>
          <button className="pf-link" onClick={() => void load()} type="button"><RefreshCw />{t("重试")}</button>
        </>
      ) : null}
      {preview ? (
        <p className="pf-note" data-tone={rows.length && filled === null ? undefined : "ok"}>
          {filled !== null
            ? tf("已填入 {0} 项，下方高亮的就是；点「保存」才会写入。", [filled])
            : found === 0
              ? tf("上游返回了 {0} 个模型，列表里 {1} 个能对上，但都没有带这项资料。", [preview.upstreamModels, preview.matched])
              : rows.length
                ? tf("列表里 {0} 个模型上游带了这项资料，{1} 个和现在不同。", [found, rows.length])
                : tf("列表里 {0} 个模型上游带了这项资料，都和现在一致。", [found])}
        </p>
      ) : null}

      {rows.length && filled === null ? (
        <div className="pf-picker">
          <div className="pf-picker-head">
            <span className="pf-meta-sync-count">{tf("已选 {0} / {1}", [picked.size, rows.length])}</span>
            <button className="pf-link" disabled={picked.size === rows.length} onClick={() => setPicked(new Set(rows.map((row) => row.model)))} type="button">{t("全选")}</button>
            <button className="pf-link" disabled={!picked.size} onClick={() => setPicked(new Set())} type="button">{t("全不选")}</button>
            <button
              className="pf-link"
              disabled={!picked.size || !preview}
              onClick={() => {
                if (!preview) return;
                onFill(preview, picked);
                setFilled(picked.size);
              }}
              type="button"
            >
              {tf("填入 {0} 项", [picked.size])}
            </button>
          </div>
          <div className="pf-picker-list pf-meta-sync-list">
            {rows.map((row) => (
              <label key={row.model} title={tf("来源字段：{0}", [row.source])}>
                <input checked={picked.has(row.model)} onChange={() => toggle(row.model)} type="checkbox" />
                <span className="pf-meta-sync-model">{row.model}</span>
                <span className="pf-meta-sync-values">
                  <s>{row.current || t("未设置")}</s>
                  <span aria-hidden="true">→</span>
                  <b>{row.next}</b>
                </span>
                <span className="pf-picker-tag" data-tone={row.manual ? "gone" : undefined}>{t(currentSourceLabel(row.currentSource))}</span>
                <code className="pf-meta-sync-source">{row.source}</code>
                {row.options ? <small className="pf-meta-sync-options">{tf("可选档位 {0}", [row.options])}</small> : null}
              </label>
            ))}
          </div>
          <p className="pf-hint">{t("手动设置过的默认不覆盖，勾选后才覆盖。")}</p>
        </div>
      ) : null}

      {ignored.length ? (
        <details className="pf-meta-sync-ignored-fold">
          <summary>{tf("已忽略的上游字段 {0} 个（数值不合理或认不出的，不会写入）", [ignored.length])}</summary>
          <ul className="pf-meta-sync-ignored">
            {ignored.slice(0, 50).map((note, index) => (
              <li key={`${note.model}:${note.field}:${index}`}>
                <code>{note.model}</code> <code>{note.field}</code> = <code>{note.value}</code> · {t(ignoredReasonLabel(note.reason))}
              </li>
            ))}
          </ul>
        </details>
      ) : null}
    </section>
  );
}
