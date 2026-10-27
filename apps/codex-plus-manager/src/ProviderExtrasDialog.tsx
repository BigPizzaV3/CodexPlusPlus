/** 供应商的价格与独立代理：在供应商池的详情里编辑（添加 / 编辑供应商的弹窗只管连接、Key 与模型）。 */
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { Check, RefreshCw, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import { AppSelect } from "./AppSelect";
import { priceFromDraft, type GatewayPoolEntry, type ProviderDraft } from "./gateway-model";
import { draftWithPrice, formatPrice, type PriceMatch } from "./provider-form-model";
import { Avatar } from "./gateway-ui";
import "./ProviderFormDialog.css";

type Result = { status: string; message: string };

/** 按公开价格表（models.dev）查某个模型的价格；查不到或出错返回 null。 */
export async function lookupPublicPrice(baseUrl: string, model: string): Promise<PriceMatch | null> {
  try {
    const result = await invoke<Result & { prices?: Array<{ model: string; price?: PriceMatch | null }> }>("lookup_model_prices", { baseUrl, models: [model] });
    return result.status === "ok" ? result.prices?.[0]?.price ?? null : null;
  } catch {
    return null;
  }
}

type PriceState =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "auto"; match: PriceMatch }
  | { state: "miss" }
  | { state: "error"; message: string };

type Extras = Pick<ProviderDraft, "priceCurrency" | "priceInput" | "priceCached" | "priceOutput" | "proxy">;

function extrasFromEntry(entry: GatewayPoolEntry): Extras {
  return {
    priceCurrency: entry.price?.currency || "USD",
    priceInput: entry.price ? String(entry.price.input) : "",
    priceCached: entry.price ? String(entry.price.cachedInput) : "",
    priceOutput: entry.price ? String(entry.price.output) : "",
    proxy: entry.proxy ?? "",
  };
}

/** 详情里一行的展示文字：没填价格时为空。 */
export function priceSummary(entry: GatewayPoolEntry): string {
  const price = entry.price;
  if (!price) return "";
  const symbol = price.currency === "CNY" ? "¥" : "$";
  return tf("输入 {0} · 缓存 {1} · 输出 {2}（每百万 token）", [
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
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  onSave: (price: ReturnType<typeof priceFromDraft>, proxy: string) => void;
  onCancel: () => void;
}) {
  const [extras, setExtras] = useState<Extras>(() => extrasFromEntry(entry));
  const [price, setPrice] = useState<PriceState>({ state: "idle" });
  const model = entry.models[0] ?? "";
  const update = (patch: Partial<Extras>) => setExtras((previous) => ({ ...previous, ...patch }));
  const proxyError = extras.proxy.trim() && !/^(https?|socks5h?):\/\/\S+$/i.test(extras.proxy.trim())
    ? t("代理地址要写成 http://主机:端口（也支持 https:// 与 socks5://）")
    : "";

  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [onCancel]);

  const fillPublicPrice = async () => {
    if (!model) return;
    setPrice({ state: "loading" });
    const match = await lookupPublicPrice(entry.baseUrl, model);
    if (!match) {
      setPrice({ state: "miss" });
      return;
    }
    setExtras((previous) => {
      const filled = draftWithPrice({ ...previous } as ProviderDraft, match);
      return { ...previous, priceCurrency: filled.priceCurrency, priceInput: filled.priceInput, priceCached: filled.priceCached, priceOutput: filled.priceOutput };
    });
    setPrice({ state: "auto", match });
  };

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="pf-modal pf-extras-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (proxyError) return;
          onSave(priceFromDraft({ ...(extras as ProviderDraft) }), extras.proxy.trim());
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{tf("{0} 的价格与代理", [entry.name])}</strong>
            <small>{t("价格用于「用量」里估算费用；代理只作用于这个供应商")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={onCancel} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            <h4>
              {t("价格")}<small>{t("每百万 token")}</small>
              <button className="pf-action" disabled={!model || price.state === "loading"} onClick={() => void fillPublicPrice()} title={model ? tf("按 {0} 的公开价格填写", [model]) : t("这个供应商还没有模型")} type="button">
                <RefreshCw className={price.state === "loading" ? "animate-spin" : ""} />{t("按公开价格填写")}
              </button>
            </h4>
            {price.state === "auto" ? (
              <p className="pf-note" data-tone="ok">
                <Check />
                {tf("已按 models.dev 的 {0} · {1} 填写（美元）", [price.match.providerName, price.match.modelId])}
              </p>
            ) : price.state === "miss" ? (
              <p className="pf-note">{tf("公开价格表里没找到 {0}，需要的话手动填写。", [model])}</p>
            ) : price.state === "loading" ? (
              <p className="pf-note">{t("正在查公开价格…")}</p>
            ) : (
              <p className="pf-note">{t("不填就不估算费用。")}</p>
            )}
            <div className="pf-price">
              <AppSelect
                label={t("币种")}
                menuWidth={140}
                onChange={(next) => update({ priceCurrency: next })}
                options={[
                  { value: "USD", label: "$ USD" },
                  { value: "CNY", label: "¥ CNY" },
                ]}
                size="sm"
                value={extras.priceCurrency === "CNY" ? "CNY" : "USD"}
              />
              <label><span>{t("输入")}</span><Input inputMode="decimal" onChange={(event) => update({ priceInput: event.currentTarget.value })} placeholder="0" value={extras.priceInput} /></label>
              <label><span>{t("缓存命中")}</span><Input inputMode="decimal" onChange={(event) => update({ priceCached: event.currentTarget.value })} placeholder="0" value={extras.priceCached} /></label>
              <label><span>{t("输出")}</span><Input inputMode="decimal" onChange={(event) => update({ priceOutput: event.currentTarget.value })} placeholder="0" value={extras.priceOutput} /></label>
            </div>
          </section>

          <section className="pf-section">
            <h4>{t("代理")}<small>{t("只这个供应商单独走的 HTTP 代理，留空沿用系统代理")}</small></h4>
            <Input className="pf-mono" onChange={(event) => update({ proxy: event.currentTarget.value })} placeholder="http://127.0.0.1:7890" spellCheck={false} value={extras.proxy} />
          </section>
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{proxyError}</span>
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy || Boolean(proxyError)} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}
