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
import { DIRECT_PROXY, isDirectProxy, maskAccountName, parseKeyLimit, proxyAddressError, type GatewayPoolEntry, type ProviderPrice } from "./gateway-model";
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
  // 代理三种：沿用系统代理（留空）、单独代理（填地址）、不使用代理（直连，#2760）。
  const [proxyMode, setProxyMode] = useState<"system" | "custom" | "direct">(
    isDirectProxy(entry.proxy) ? "direct" : (entry.proxy ?? "").trim() ? "custom" : "system",
  );
  const [proxy, setProxy] = useState(isDirectProxy(entry.proxy) ? "" : entry.proxy ?? "");
  const [price, setPrice] = useState<PriceState>({ state: "idle" });
  const setRow = (model: string, patch: Partial<PriceRow>) => setRows((previous) => ({ ...previous, [model]: { ...(previous[model] ?? EMPTY_PRICE_ROW), ...patch } }));
  const proxyValue = proxyMode === "direct" ? DIRECT_PROXY : proxyMode === "custom" ? proxy.trim() : "";
  const proxyAddressProblem = proxyMode === "custom" ? proxyAddressError(proxy) : null;
  const proxyError = proxyAddressProblem
    ? t(proxyAddressProblem)
    : proxyMode === "custom" && !proxy.trim() ? t("请填写代理地址") : "";

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
            if (!proxyError) onSave(entry.price ?? null, null, proxyValue);
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
            <h4>{t("代理")}<small>{t("这个供应商的请求怎么走代理")}</small></h4>
            <div className="pf-seg" role="radiogroup" aria-label={t("代理")}>
              {([
                ["system", t("沿用系统代理")],
                ["custom", t("单独代理")],
                ["direct", t("不使用代理（直连）")],
              ] as const).map(([mode, label]) => (
                <button aria-checked={proxyMode === mode} data-active={proxyMode === mode} key={mode} onClick={() => setProxyMode(mode)} role="radio" type="button">
                  {label}
                </button>
              ))}
            </div>
            {proxyMode === "custom" ? (
              <Input autoFocus className="pf-mono" onChange={(event) => setProxy(event.currentTarget.value)} placeholder="http://127.0.0.1:7890" spellCheck={false} value={proxy} />
            ) : (
              <p className="pf-hint">
                {proxyMode === "direct"
                  ? t("不走系统代理和代理环境变量，直接连这个供应商；适合国内可直连的中转。")
                  : t("跟随系统代理（以及代理环境变量）；没有设置系统代理时就是直连。")}
              </p>
            )}
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

/** 详情里「限速」一行的展示文字：没设上限时为空。 */
export function keyLimitSummary(entry: GatewayPoolEntry): string {
  const parts: string[] = [];
  if (entry.keyRpm) parts.push(tf("每分钟 {0} 次", [entry.keyRpm]));
  if (entry.keyConcurrency) parts.push(tf("同时 {0} 个", [entry.keyConcurrency]));
  return parts.length ? tf("每个 Key · {0}", [parts.join(" · ")]) : "";
}

/** 供应商的主动限速（#1130 / #2726）：每个 Key 的 RPM / 并发上限，留空表示不限。 */
export function ProviderLimitsDialog({
  entry,
  busy,
  onSave,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  onSave: (rpm: number | null, concurrency: number | null) => void;
  onCancel: () => void;
}) {
  const [rpm, setRpm] = useState(entry.keyRpm ? String(entry.keyRpm) : "");
  const [concurrency, setConcurrency] = useState(entry.keyConcurrency ? String(entry.keyConcurrency) : "");
  const rpmValue = parseKeyLimit(rpm);
  const concurrencyValue = parseKeyLimit(concurrency);
  const invalid = Number.isNaN(rpmValue) || Number.isNaN(concurrencyValue);
  useEscapeLayer(true, onCancel);

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="pf-modal pf-extras-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (!invalid) onSave(rpmValue, concurrencyValue);
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{tf("{0} 的限速", [maskAccountName(entry.name)])}</strong>
            <small>{t("按每个 Key 计，留空表示不限")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={onCancel} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            <h4>{t("限速")}<small>{t("上游限制了 RPM 或并发时，在这里主动限住，不用等 429 再冷却")}</small></h4>
            <label className="pf-field">
              <span>{t("每分钟最多请求数")} <small>RPM</small></span>
              <Input inputMode="numeric" onChange={(event) => setRpm(event.currentTarget.value)} placeholder={t("不限")} value={rpm} />
            </label>
            <label className="pf-field">
              <span>{t("同时进行的最多请求数")} <small>{t("并发")}</small></span>
              <Input inputMode="numeric" onChange={(event) => setConcurrency(event.currentTarget.value)} placeholder={t("不限")} value={concurrency} />
            </label>
            <p className="pf-note">{t("按每个 Key 计；达到上限时先换别的 Key 或下一家，都满了就排队等待（最多 2 分钟）。")}</p>
          </section>
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{invalid ? t("上限要填正整数，留空表示不限") : ""}</span>
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy || invalid} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}

/**
 * new-api 的访问令牌（#2793）：new-api「个人设置 → 安全设置」里生成的访问令牌，区别于 API Key，
 * 只用来查账户余额。已保存的令牌只显示打码后的样子；令牌框留空表示沿用。
 */
export function ProviderNewApiAccessDialog({
  entry,
  busy,
  onSave,
  onClear,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  onSave: (token: string | null, userId: string) => void;
  onClear: () => void;
  onCancel: () => void;
}) {
  const saved = entry.newapiAccess ?? null;
  const [token, setToken] = useState("");
  const [userId, setUserId] = useState(saved?.userId ?? "");
  const tokenValue = token.trim();
  const userIdValue = userId.trim();
  // 还没保存过令牌时必须填；空着只是不能保存，不提示错误。
  const missing = !saved && !tokenValue;
  const error = /\s/.test(tokenValue)
    ? t("访问令牌里不能有空格或换行")
    : !/^\d*$/.test(userIdValue)
      ? t("用户 ID 只能是数字")
      : "";
  useEscapeLayer(true, onCancel);

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="pf-modal pf-extras-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (!error && !missing) onSave(tokenValue || null, userIdValue);
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{tf("{0} 的访问令牌", [maskAccountName(entry.name)])}</strong>
            <small>{t("可选：只用来查 new-api 账户余额，留空不查")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={onCancel} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            <h4>{t("访问令牌")}<small>{t("在 new-api 的「个人设置 → 安全设置」里生成，不是 API Key")}</small></h4>
            <label className="pf-field">
              <span>{t("访问令牌")}</span>
              <Input
                autoComplete="off"
                onChange={(event) => setToken(event.currentTarget.value)}
                placeholder={saved ? tf("已保存 {0}，留空不改", [saved.masked]) : t("粘贴访问令牌")}
                spellCheck={false}
                type="password"
                value={token}
              />
            </label>
            <label className="pf-field">
              <span>{t("用户 ID")} <small>New-Api-User</small></span>
              <Input inputMode="numeric" onChange={(event) => setUserId(event.currentTarget.value)} placeholder={t("个人设置里显示的 ID")} value={userId} />
            </label>
            <p className="pf-note">{t("按 new-api 文档，查余额要同时带访问令牌和用户 ID（旧版必须填用户 ID）；新版的分权限令牌需要勾选「个人资料 · 查看」。令牌只存在本机，和 Key 一样不显示原文。")}</p>
          </section>
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{error}</span>
          {saved ? (
            <Button disabled={busy} onClick={onClear} size="sm" type="button" variant="ghost">{t("清除令牌")}</Button>
          ) : null}
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy || missing || Boolean(error)} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}
