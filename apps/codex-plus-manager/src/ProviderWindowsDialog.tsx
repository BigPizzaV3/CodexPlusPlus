/** 供应商的上下文窗口：按模型填，在供应商池的详情里编辑。
 * 窗口写进 Agent 的模型资料（Agent 按它压缩上下文），路由时请求比窗口还长的成员排到最后。 */
import { useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Check, RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import { formatTokens, maskAccountName, parseContextWindow, type GatewayPoolEntry } from "./gateway-model";
import { lookupPublicPrices } from "./ProviderExtrasDialog";
import { Avatar } from "./gateway-ui";
import { useEscapeLayer } from "./use-escape-layer";
import "./ProviderFormDialog.css";

/** 「其它模型」那一行的键：没单独写窗口的模型用它。 */
const FALLBACK = "";

type LookupState = { state: "idle" } | { state: "loading" } | { state: "done"; found: number };

/** 某个模型单独写的窗口（忽略大小写）。 */
function ownWindow(entry: GatewayPoolEntry, model: string): number | undefined {
  const windows = entry.modelWindows ?? {};
  const key = Object.keys(windows).find((name) => name.toLowerCase() === model.toLowerCase());
  return key === undefined ? undefined : windows[key];
}

/** 详情里一行的展示文字：没填时为空。 */
export function windowSummary(entry: GatewayPoolEntry): string {
  const own = entry.models.filter((model) => ownWindow(entry, model)).length;
  const fallback = entry.contextWindow ?? null;
  if (own) {
    const values = [...new Set(entry.models.map((model) => ownWindow(entry, model) ?? fallback).filter((value): value is number => Boolean(value)))].sort((a, b) => a - b);
    const range = values.length > 1 ? `${formatTokens(values[0])}–${formatTokens(values[values.length - 1])}` : formatTokens(values[0]);
    return tf("{0} / {1} 个模型 · {2}", [entry.models.filter((model) => ownWindow(entry, model) ?? fallback).length, entry.models.length, range]);
  }
  return fallback ? tf("所有模型 · {0}", [formatTokens(fallback)]) : "";
}

/** 输入框里的写法：1000000 → 1M，200000 → 200K。 */
function windowText(value: number | null | undefined): string {
  return value ? formatTokens(value) : "";
}

export function ProviderWindowsDialog({
  entry,
  busy,
  onSave,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  onSave: (windows: Record<string, number>, fallback: number | null) => void;
  onCancel: () => void;
}) {
  const [rows, setRows] = useState<Record<string, string>>(() => {
    const initial: Record<string, string> = { [FALLBACK]: windowText(entry.contextWindow) };
    for (const model of entry.models) initial[model] = windowText(ownWindow(entry, model));
    return initial;
  });
  const [lookup, setLookup] = useState<LookupState>({ state: "idle" });
  useEscapeLayer(true, onCancel);

  // 写错的（不是 128k / 1M / 纯数字，或小于 1000）在保存前标出来。
  const invalid = Object.entries(rows).filter(([, text]) => {
    const value = parseContextWindow(text);
    return value !== null && (!Number.isFinite(value) || value < 1000);
  }).map(([model]) => model);

  const fillPublic = async () => {
    setLookup({ state: "loading" });
    const found = await lookupPublicPrices(entry.baseUrl, entry.models);
    const hits = Object.entries(found).filter(([, match]) => match.context);
    if (hits.length) {
      setRows((previous) => {
        const next = { ...previous };
        for (const [model, match] of hits) next[model] = windowText(match.context);
        return next;
      });
    }
    setLookup({ state: "done", found: hits.length });
  };

  const row = (model: string, label: ReactNode) => (
    <div className="pf-price-row pf-window-row" data-invalid={invalid.includes(model)} key={model || "*"}>
      {label}
      <Input
        aria-label={t("上下文窗口")}
        onChange={(event) => {
          const text = event.currentTarget.value;
          setRows((previous) => ({ ...previous, [model]: text }));
        }}
        placeholder={model === FALLBACK ? t("用模型自带资料") : "—"}
        spellCheck={false}
        value={rows[model] ?? ""}
      />
    </div>
  );

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="pf-modal pf-extras-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (invalid.length) return;
          const windows: Record<string, number> = {};
          for (const model of entry.models) {
            const value = parseContextWindow(rows[model] ?? "");
            if (value) windows[model] = value;
          }
          onSave(windows, parseContextWindow(rows[FALLBACK] ?? "") || null);
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{tf("{0} 的上下文窗口", [maskAccountName(entry.name)])}</strong>
            <small>{t("Agent 按它压缩上下文；请求比它还长时，路由先试别家")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={onCancel} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            <h4>
              {t("上下文窗口")}<small>{t("按模型填，如 128k、1M")}</small>
              <button className="pf-action" disabled={!entry.models.length || lookup.state === "loading"} onClick={() => void fillPublic()} type="button">
                <RefreshCw className={lookup.state === "loading" ? "animate-spin" : ""} />{t("按公开资料填写")}
              </button>
            </h4>
            {lookup.state === "done" ? (
              <p className="pf-note" data-tone={lookup.found ? "ok" : undefined}>
                {lookup.found ? <Check /> : null}
                {tf("models.dev 里找到 {0} / {1} 个模型的窗口，其余需要的话手动填写。", [lookup.found, entry.models.length])}
              </p>
            ) : lookup.state === "loading" ? (
              <p className="pf-note">{t("正在查公开资料…")}</p>
            ) : (
              <p className="pf-note">{t("留空的模型用「其它模型」那一行；都不填就用模型自带的资料。")}</p>
            )}
            <div className="pf-price-table">
              <div className="pf-price-row pf-window-row pf-price-head">
                <span>{t("模型")}</span>
                <span>{t("上下文窗口")}</span>
              </div>
              <div className="pf-price-rows">
                {entry.models.map((model) => row(model, <code title={model}>{model}</code>))}
                {row(FALLBACK, <span className="pf-price-fallback">{t("其它模型")}</span>)}
              </div>
            </div>
          </section>
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{invalid.length ? t("窗口要写成 128k、1M 或至少 1000 的数字") : ""}</span>
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy || invalid.length > 0} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}
