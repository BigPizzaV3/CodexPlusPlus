/** 供应商的价格与独立代理：在供应商池的详情里编辑（添加 / 编辑供应商的弹窗只管连接、Key 与模型）。
 * 价格按模型填：同一家的不同模型价格往往差好几倍。 */
import { useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { Check, RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import { AppSelect } from "./AppSelect";
import { maskAccountName, type GatewayPoolEntry, type ProviderPrice } from "./gateway-model";
import {
  EMPTY_PRICE_ROW,
  formatPrice,
  modelPricesFromRows,
  priceFromRow,
  priceRowFrom,
  priceRowFromMatch,
  type PriceMatch,
  type PriceRow,
} from "./provider-form-model";
import { Avatar } from "./gateway-ui";
import { useEscapeLayer } from "./use-escape-layer";
import "./ProviderFormDialog.css";

type Result = { status: string; message: string };

/** 按公开价格表（models.dev）查一组模型的价格；查不到的不在结果里，出错返回空。 */
export async function lookupPublicPrices(baseUrl: string, models: string[]): Promise<Record<string, PriceMatch>> {
  if (!models.length) return {};
  try {
    const result = await invoke<Result & { prices?: Array<{ model: string; price?: PriceMatch | null }> }>("lookup_model_prices", { baseUrl, models });
    const found: Record<string, PriceMatch> = {};
    if (result.status === "ok") for (const entry of result.prices ?? []) if (entry.price) found[entry.model] = entry.price;
    return found;
  } catch {
    return {};
  }
}

type PriceState =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "done"; found: number }
  | { state: "error"; message: string };

/** 「其它模型」那一行的键：没单独填价的模型用它。 */
const FALLBACK = "";

/** 详情里一行的展示文字：没填价格时为空。 */
export function priceSummary(entry: GatewayPoolEntry): string {
  const perModel = Object.keys(entry.modelPrices ?? {}).length;
  const price = entry.price;
  if (perModel) {
    const priced = entry.models.filter((model) => entry.modelPrices?.[model] || price).length;
    return tf("{0} / {1} 个模型", [priced, entry.models.length]);
  }
  if (!price) return "";
  const symbol = price.currency === "CNY" ? "¥" : "$";
  return tf("所有模型 · 输入 {0} · 缓存 {1} · 输出 {2}", [
    `${symbol}${formatPrice(price.input)}`,
    `${symbol}${formatPrice(price.cachedInput)}`,
    `${symbol}${formatPrice(price.output)}`,
  ]);
}

export function ProviderExtrasDialog({
  entry,
  busy,
  onSave,
  onCancel,
  section,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  /** 只编辑价格或只编辑代理；另一项原样保留。 */
  section: "price" | "proxy";
  /** `modelPrices` 为 null 表示不改按模型的价格。 */
  onSave: (price: ProviderPrice | null, modelPrices: Record<string, ProviderPrice> | null, proxy: string) => void;
  onCancel: () => void;
}) {
  // 每个模型一行，最后一行「其它模型」是没单独填价时用的默认价。
  const [rows, setRows] = useState<Record<string, PriceRow>>(() => {
    const initial: Record<string, PriceRow> = { [FALLBACK]: priceRowFrom(entry.price) };
    for (const model of entry.models) initial[model] = priceRowFrom(entry.modelPrices?.[model]);
    return initial;
  });
  const [currency, setCurrency] = useState(() => Object.values(entry.modelPrices ?? {})[0]?.currency || entry.price?.currency || "USD");
  const [proxy, setProxy] = useState(entry.proxy ?? "");
  const [price, setPrice] = useState<PriceState>({ state: "idle" });
  const setRow = (model: string, patch: Partial<PriceRow>) => setRows((previous) => ({ ...previous, [model]: { ...(previous[model] ?? EMPTY_PRICE_ROW), ...patch } }));
  const proxyError = proxy.trim() && !/^(https?|socks5h?):\/\/\S+$/i.test(proxy.trim())
    ? t("代理地址要写成 http://主机:端口（也支持 https:// 与 socks5://）")
    : "";

  // Esc 栈：币种下拉开着时 Esc 只关下拉，不会把整个价格弹窗（和填了一半的价格）关掉。
  useEscapeLayer(true, onCancel);

  const fillPublicPrices = async () => {
    if (!entry.models.length) return;
    setPrice({ state: "loading" });
    const found = await lookupPublicPrices(entry.baseUrl, entry.models);
    const hits = Object.entries(found);
    if (hits.length) {
      setCurrency("USD");
      setRows((previous) => {
        const next = { ...previous };
        for (const [model, match] of hits) next[model] = priceRowFromMatch(match);
        return next;
      });
    }
    setPrice({ state: "done", found: hits.length });
  };

  const row = (model: string, label: ReactNode) => {
    const value = rows[model] ?? EMPTY_PRICE_ROW;
    return (
      <div className="pf-price-row" key={model || "*"}>
        {label}
        <Input aria-label={t("输入")} inputMode="decimal" onChange={(event) => setRow(model, { input: event.currentTarget.value })} placeholder="—" value={value.input} />
        <Input aria-label={t("缓存命中")} inputMode="decimal" onChange={(event) => setRow(model, { cached: event.currentTarget.value })} placeholder="—" value={value.cached} />
        <Input aria-label={t("输出")} inputMode="decimal" onChange={(event) => setRow(model, { output: event.currentTarget.value })} placeholder="—" value={value.output} />
      </div>
    );
  };

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="pf-modal pf-extras-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (section === "proxy") {
            if (!proxyError) onSave(entry.price ?? null, null, proxy.trim());
            return;
          }
          const perModel: Record<string, PriceRow> = { ...rows };
          delete perModel[FALLBACK];
          onSave(priceFromRow(rows[FALLBACK] ?? EMPTY_PRICE_ROW, currency), modelPricesFromRows(perModel, currency), entry.proxy ?? "");
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{section === "price" ? tf("{0} 的价格", [maskAccountName(entry.name)]) : tf("{0} 的代理", [maskAccountName(entry.name)])}</strong>
            <small>{section === "price" ? t("用于「用量」里估算费用") : t("只作用于这个供应商")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={onCancel} type="button"><X /></button>
        </header>

        <div className="pf-body">
          {section === "price" ? (
          <section className="pf-section">
            <h4>
              {t("价格")}<small>{t("每百万 token，按模型计")}</small>
              <AppSelect
                className="pf-currency"
                label={t("币种")}
                menuWidth={140}
                onChange={setCurrency}
                options={[
                  { value: "USD", label: "$ USD" },
                  { value: "CNY", label: "¥ CNY" },
                ]}
                size="sm"
                value={currency === "CNY" ? "CNY" : "USD"}
              />
              <button className="pf-action" disabled={!entry.models.length || price.state === "loading"} onClick={() => void fillPublicPrices()} type="button">
                <RefreshCw className={price.state === "loading" ? "animate-spin" : ""} />{t("按公开价格填写")}
              </button>
            </h4>
            {price.state === "done" ? (
              <p className="pf-note" data-tone={price.found ? "ok" : undefined}>
                {price.found ? <Check /> : null}
                {tf("models.dev 里找到 {0} / {1} 个模型的价格（美元），其余需要的话手动填写。", [price.found, entry.models.length])}
              </p>
            ) : price.state === "loading" ? (
              <p className="pf-note">{t("正在查公开价格…")}</p>
            ) : (
              <p className="pf-note">{t("留空的模型用「其它模型」那一行；都不填就不估算费用。")}</p>
            )}
            <div className="pf-price-table">
              <div className="pf-price-row pf-price-head">
                <span>{t("模型")}</span>
                <span>{t("输入")}</span>
                <span>{t("缓存命中")}</span>
                <span>{t("输出")}</span>
              </div>
              <div className="pf-price-rows">
                {entry.models.map((model) => row(model, <code title={model}>{model}</code>))}
                {row(FALLBACK, <span className="pf-price-fallback">{t("其它模型")}</span>)}
              </div>
            </div>
          </section>
          ) : (
          <section className="pf-section">
            <h4>{t("代理")}<small>{t("只这个供应商单独走的 HTTP 代理，留空沿用系统代理")}</small></h4>
            <Input className="pf-mono" onChange={(event) => setProxy(event.currentTarget.value)} placeholder="http://127.0.0.1:7890" spellCheck={false} value={proxy} />
          </section>
          )}
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{section === "proxy" ? proxyError : ""}</span>
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy || (section === "proxy" && Boolean(proxyError))} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}
