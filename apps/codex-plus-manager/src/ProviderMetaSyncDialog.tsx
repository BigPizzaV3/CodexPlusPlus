/** 供应商详情的「从上游同步上下文窗口 / 思考等级」（#2807）：用存下的凭据拉上游 `/models`，
 * 读每个模型的元数据，列出和现有设置的差异（新值、旧值、来源字段），不改任何数据；用户勾选后才写入。
 * 现值是用户手动设的默认不勾（可勾选覆盖）；上游字段不合理、档位认不出的在下方列出，不会写入。 */
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import { maskAccountName, type GatewayPoolEntry } from "./gateway-model";
import { Avatar } from "./gateway-ui";
import { useEscapeLayer } from "./use-escape-layer";
import {
  changeRows,
  currentSourceLabel,
  defaultPicked,
  ignoredFor,
  ignoredReasonLabel,
  normalizePreview,
  reasoningPicks,
  windowPicks,
  type MetaSyncKind,
  type MetaSyncPreview,
} from "./provider-meta-sync-model";
import "./ProviderFormDialog.css";

type PreviewResult = { status: string; message: string } & Partial<MetaSyncPreview>;
type Loaded = { state: "loading" } | { state: "error"; message: string } | { state: "ready"; preview: MetaSyncPreview };

export function ProviderMetaSyncDialog({
  entry,
  kind,
  busy,
  onApplyWindows,
  onApplyReasoning,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  kind: MetaSyncKind;
  busy: boolean;
  /** 只写勾选的；写入成功后由调用方关掉弹窗。 */
  onApplyWindows: (picks: Array<{ model: string; window: number }>) => void;
  onApplyReasoning: (picks: Array<{ model: string; levels: string[]; defaultLevel: string | null }>) => void;
  onCancel: () => void;
}) {
  const [loaded, setLoaded] = useState<Loaded>({ state: "loading" });
  const [picked, setPicked] = useState<Set<string>>(new Set());
  useEscapeLayer(true, onCancel);

  const load = async () => {
    setLoaded({ state: "loading" });
    try {
      const result = await invoke<PreviewResult>("preview_gateway_provider_meta_sync", { id: entry.id });
      if (result.status !== "ok") {
        setLoaded({ state: "error", message: result.message });
        return;
      }
      const preview = normalizePreview(result);
      setPicked(defaultPicked(preview, kind));
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
  const rows = preview ? changeRows(preview, kind, t("默认")) : [];
  const ignored = preview ? ignoredFor(preview, kind) : [];
  const found = preview ? (kind === "windows" ? preview.withWindow : preview.withReasoning) : 0;
  const toggle = (model: string) => {
    const next = new Set(picked);
    if (next.has(model)) next.delete(model);
    else next.add(model);
    setPicked(next);
  };
  const name = maskAccountName(entry.name);
  const title = kind === "windows" ? tf("从上游同步 {0} 的上下文窗口", [name]) : tf("从上游同步 {0} 的思考等级", [name]);

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="pf-modal pf-sync-modal pf-meta-sync-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (!preview || !picked.size) return;
          if (kind === "windows") onApplyWindows(windowPicks(preview, picked));
          else onApplyReasoning(reasoningPicks(preview, picked));
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{title}</strong>
            <small>{t("读取上游 /models 返回的模型资料，先列出差异，勾选后才会写入")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={onCancel} type="button"><X /></button>
        </header>

        <div className="pf-body">
          {loaded.state === "loading" ? (
            <section className="pf-section">
              <p className="pf-note"><RefreshCw className="animate-spin" />{t("正在用已保存的 Key 读取上游模型资料…")}</p>
            </section>
          ) : null}
          {loaded.state === "error" ? (
            <section className="pf-section">
              <p className="pf-note" data-tone="error">{t(loaded.message)}</p>
              <button className="pf-link" onClick={() => void load()} type="button"><RefreshCw />{t("重试")}</button>
            </section>
          ) : null}
          {preview ? (
            <section className="pf-section">
              <p className="pf-note" data-tone={rows.length ? undefined : "ok"}>
                {found === 0
                  ? tf("上游返回了 {0} 个模型，列表里 {1} 个能对上，但都没有带这项资料。", [preview.upstreamModels, preview.matched])
                  : rows.length
                    ? tf("列表里 {0} 个模型上游带了这项资料，{1} 个和现在不同。", [found, rows.length])
                    : tf("列表里 {0} 个模型上游带了这项资料，都和现在一致。", [found])}
              </p>
            </section>
          ) : null}

          {rows.length ? (
            <section className="pf-section">
              <h4>
                {t("和上游不同")}<em>{rows.length}</em>
                <small>{t("手动设置过的默认不覆盖，勾选后才覆盖")}</small>
              </h4>
              <div className="pf-picker">
                <div className="pf-picker-head">
                  <span className="pf-meta-sync-count">{tf("已选 {0} / {1}", [picked.size, rows.length])}</span>
                  <button className="pf-link" disabled={picked.size === rows.length} onClick={() => setPicked(new Set(rows.map((row) => row.model)))} type="button">{t("全选")}</button>
                  <button className="pf-link" disabled={!picked.size} onClick={() => setPicked(new Set())} type="button">{t("全不选")}</button>
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
              </div>
              {kind === "windows" ? (
                <p className="pf-hint">{t("写进这个模型单独的上下文窗口；Agent 的模型列表和路由都按它算。")}</p>
              ) : (
                <p className="pf-hint">{t("整组替换这个模型的思考档位；上游没给默认档时保留原来的默认档。")}</p>
              )}
            </section>
          ) : null}

          {ignored.length ? (
            <section className="pf-section">
              <h4>
                {t("已忽略的上游字段")}<em>{ignored.length}</em>
                <small>{t("数值不合理或认不出的，不会写入")}</small>
              </h4>
              <ul className="pf-meta-sync-ignored">
                {ignored.slice(0, 50).map((note, index) => (
                  <li key={`${note.model}:${note.field}:${index}`}>
                    <code>{note.model}</code> <code>{note.field}</code> = <code>{note.value}</code> · {t(ignoredReasonLabel(note.reason))}
                  </li>
                ))}
              </ul>
            </section>
          ) : null}
        </div>

        <footer className="pf-foot">
          <span className="pf-error" />
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{preview && !rows.length ? t("关闭") : t("取消")}</Button>
          {preview && rows.length ? (
            <Button disabled={busy || !picked.size} size="sm" type="submit">{busy ? t("保存中") : tf("应用 {0} 项", [picked.size])}</Button>
          ) : null}
        </footer>
      </form>
    </div>,
    document.body,
  );
}
