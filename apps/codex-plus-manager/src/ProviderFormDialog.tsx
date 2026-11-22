/** 网关「添加 / 编辑供应商」弹窗：连接（名称、协议、地址、多个 Key）、模型（从上游获取后勾选）、价格（按公开价格表自动填）；
 * 编辑时还能从这里打开上下文窗口弹窗。 */
import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { Pencil, Plus, RefreshCw, Search, Star, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import {
  maskAccountName,
  presetHost,
  providerDraftFromEntry,
  providerDraftFromPreset,
  proxyAddressError,
  validateProviderDraft,
  type GatewayPoolEntry,
  type GatewayPreset,
  type GatewayProtocol,
  type ProviderDraft,
} from "./gateway-model";
import { Avatar } from "./gateway-ui";
import { useEscapeLayer } from "./use-escape-layer";
import { useModalShell } from "./use-modal-shell";
import { CompatToggles } from "./CompatToggles";
import { lookupPublicPrices } from "./ProviderExtrasDialog";
import {
  addModel,
  defaultModel,
  draftKeys,
  draftModels,
  hasPrice,
  priceFromMatch,
  removeModel,
  withKeys,
  withModels,
} from "./provider-form-model";
import { windowSummary } from "./ProviderWindowsDialog";
import { filterModels, pickerOutcome, pickerRows, type PickerRow } from "./provider-sync-model";
import "./ProviderFormDialog.css";
import { SubscriptionPolicyFields } from "./SubscriptionPolicyFields";
import { RelayHeaderRows } from "./RelayHeaderRows";
import { providerHeaderInputs } from "./relay-headers";

export type ProviderFormState =
  | { mode: "preset"; preset: GatewayPreset }
  | { mode: "custom" }
  /** focus：从详情里「请求头」的编辑进来时，打开就滚到请求头一节。 */
  | { mode: "edit"; entry: GatewayPoolEntry; focus?: "headers" };

type Result = { status: string; message: string };
const PROTOCOLS: Array<{ id: GatewayProtocol; label: string }> = [
  { id: "chatCompletions", label: "Chat" },
  { id: "responses", label: "Responses" },
  { id: "anthropicMessages", label: "Anthropic" },
];

export function ProviderFormDialog({
  form,
  busy,
  onSubmit,
  onCancel,
  agentPicks,
  windowsEntry,
  onEditWindows,
}: {
  form: ProviderFormState;
  busy: string | null;
  onSubmit: (draft: ProviderDraft) => void;
  onCancel: () => void;
  /** 新添加时显示「给哪些 Agent 用」，编辑时传「给这些 Agent 用」的开关行。 */
  agentPicks?: ReactNode;
  /** 编辑时：这个供应商的最新数据（显示已填的上下文窗口）。新添加时不传，模型还没保存，不显示窗口入口。 */
  windowsEntry?: GatewayPoolEntry;
  /** 编辑时打开上下文窗口弹窗，按弹窗里正在编辑的模型列表列出。 */
  onEditWindows?: (models: string[]) => void;
}) {
  const editing = form.mode === "edit";
  const subscriptionEditing = form.mode === "edit" && form.entry.account === "chatgpt";
  const preset = form.mode === "preset" ? form.preset : null;
  const [draft, setDraft] = useState<ProviderDraft>(() => {
    return form.mode === "edit" ? providerDraftFromEntry(form.entry) : providerDraftFromPreset(preset);
  });
  // 价格在供应商池的详情里按模型编辑；新添加时按公开价格表给每个模型悄悄填上（查不到的不填）。
  const autoPrice = useRef(!editing && !hasPrice(draft));
  const [fetching, setFetching] = useState(false);
  const [fetchNote, setFetchNote] = useState<{ tone: "ok" | "error"; text: string } | null>(null);
  const [picker, setPicker] = useState<PickerState | null>(null);
  const [modelInput, setModelInput] = useState("");
  const update = (patch: Partial<ProviderDraft>) => setDraft((previous) => ({ ...previous, ...patch }));
  const validation = subscriptionEditing
    ? (!draft.name.trim() ? "名称不能为空" : proxyAddressError(draft.proxy))
    : validateProviderDraft(draft, editing);
  const keys = draftKeys(draft);
  const models = draftModels(draft);
  const chosenModel = defaultModel(draft);
  const firstKey = keys.map((key) => key.trim()).find(Boolean) ?? "";
  const headerCount = (draft.customHeaders ?? []).filter((row) => row.key.trim()).length;
  const canFetch = (subscriptionEditing || (Boolean(draft.baseUrl.trim()) && (Boolean(firstKey) || editing))) && !fetching;

  // 弹窗外壳：Esc 走全局栈（「从上游获取」的勾选列表开着时 Esc 先收起列表）；改过再关先确认，
  // 不丢粘贴好的 Key 和挑好的模型。自动查到的价格不算改动。
  const titleId = useId();
  const [initialDraft] = useState(() => JSON.stringify({ ...draft, modelPrices: undefined }));
  const dirty = JSON.stringify({ ...draft, modelPrices: undefined }) !== initialDraft || picker !== null || modelInput.trim() !== "";
  const { backdropProps, dialogProps, requestClose } = useModalShell<HTMLFormElement>({ onClose: onCancel, dirty, labelledBy: titleId });
  // 从详情「请求头」一行进来：直接滚到请求头一节并聚焦第一个输入框。
  const headersRef = useRef<HTMLElement>(null);
  const focusHeaders = form.mode === "edit" && form.focus === "headers";
  useEffect(() => {
    if (!focusHeaders) return;
    const section = headersRef.current;
    section?.scrollIntoView({ block: "start" });
    section?.querySelector<HTMLInputElement>("input")?.focus();
  }, [focusHeaders]);

  const lookupPrices = async (list: string[], baseUrl: string) => {
    const found = await lookupPublicPrices(baseUrl, list);
    if (!autoPrice.current) return;
    const modelPrices = Object.fromEntries(Object.entries(found).map(([model, match]) => [model, priceFromMatch(match)]));
    setDraft((previous) => ({ ...previous, modelPrices }));
  };

  // 地址或模型列表变了就重新查价（停顿半秒再查，打字时不反复请求）。
  const modelKey = models.join("\n");
  useEffect(() => {
    if (!autoPrice.current || !models.length || !draft.baseUrl.trim()) return;
    const timer = window.setTimeout(() => void lookupPrices(models, draft.baseUrl.trim()), 500);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft.baseUrl, modelKey]);

  const fetchModels = async () => {
    setFetching(true);
    setFetchNote(null);
    try {
      const result = await invoke<Result & { models?: string[] }>("fetch_provider_models", {
        baseUrl: draft.baseUrl.trim(),
        protocol: draft.protocol,
        apiKey: firstKey,
        providerId: form.mode === "edit" ? form.entry.id : null,
        // 弹窗里正在编辑的请求头（#3075）：有的中转不带指定头连模型列表都不给。
        customHeaders: subscriptionEditing ? null : providerHeaderInputs(draft.customHeaders ?? []),
      });
      const fetched = result.models ?? [];
      if (result.status !== "ok" || !fetched.length) {
        setFetchNote({ tone: "error", text: result.message });
      } else {
        // 不论多少都让用户挑（#2785）：已在列表里的默认勾上（取消勾选表示移除），上游新增的默认不勾。
        setPicker({ rows: pickerRows(models, fetched), selected: new Set(models), query: "", fetched: fetched.length });
      }
    } catch (error) {
      setFetchNote({ tone: "error", text: String(error) });
    } finally {
      setFetching(false);
    }
  };

  const title = form.mode === "edit" ? tf("编辑 {0}", [maskAccountName(form.entry.name)]) : preset ? tf("添加 {0}", [preset.name]) : t("添加自定义供应商");
  const subtitle = subscriptionEditing ? t("ChatGPT 订阅账号") : preset?.apiKeyUrl ? tf("在 {0} 获取 Key", [presetHost(preset.apiKeyUrl)]) : editing ? tf("现有 {0} 个 Key", [form.entry.keyCount]) : t("任意 OpenAI / Anthropic 兼容接口");
  // 编辑时已保存的 Key 只列打码后的样子，增删、替换在供应商池详情的「Key」一行里单独做（#2782）；
  // 旧版后端没给 Key 列表时仍是「留空表示不改」的输入框。
  const savedKeys = form.mode === "edit" ? form.entry.keys ?? [] : [];
  const keyHint = editing ? (savedKeys.length ? t("在供应商详情的「Key」一行里单独替换、删除或添加") : t("留空表示不改")) : preset?.defaultKey ? t("本机服务不校验 Key，保留默认值即可") : t("多个自动轮换");

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" {...backdropProps}>
      <form
        {...dialogProps}
        className="pf-modal"
        onSubmit={(event) => {
          event.preventDefault();
          if (!validation) onSubmit({ ...draft, model: chosenModel });
        }}
      >
        <header className="pf-head">
          {preset || editing ? <Avatar baseUrl={draft.baseUrl} name={preset?.name ?? draft.name} /> : <span className="gw-avatar gw-avatar-plus"><Plus className="h-4 w-4" /></span>}
          <div>
            <strong id={titleId}>{title}</strong>
            <small>{subtitle}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={() => void requestClose()} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            <h4>{t("连接")}</h4>
            <div className="pf-grid">
              <label className="pf-field">
                <span>{t("名称")}</span>
                <Input onChange={(event) => update({ name: event.currentTarget.value })} value={draft.name} />
              </label>
              {!subscriptionEditing ? <div className="pf-field">
                <span>{t("协议")}</span>
                <div className="pf-seg" role="radiogroup" aria-label={t("协议")}>
                  {PROTOCOLS.map((item) => (
                    <button aria-checked={draft.protocol === item.id} data-active={draft.protocol === item.id} key={item.id} onClick={() => update({ protocol: item.id })} role="radio" type="button">
                      {item.label}
                    </button>
                  ))}
                </div>
              </div> : null}
            </div>
            {!subscriptionEditing ? <>
            <label className="pf-field">
              <span>{t("地址")}</span>
              <Input className="pf-mono" onChange={(event) => update({ baseUrl: event.currentTarget.value })} placeholder="https://api.example.com/v1" value={draft.baseUrl} />
            </label>
            <div className="pf-field">
              <span>{t("API Key")} <small>{keyHint}</small></span>
              {savedKeys.length ? (
                <div className="pf-chips">
                  {savedKeys.map((key) => <span className="pf-chip" key={key.id}><span className="pf-mono">{key.masked}</span></span>)}
                </div>
              ) : null}
              {savedKeys.length ? null : keys.map((key, index) => (
                <div className="pf-key" key={index}>
                  <Input
                    autoComplete="off"
                    autoFocus={index === 0 && Boolean(preset && !preset.defaultKey)}
                    className="pf-mono"
                    onChange={(event) => {
                      // 先取值再进 updater：updater 延迟执行时 currentTarget 已为 null，会抛错白屏
                      const value = event.currentTarget.value;
                      setDraft((previous) => withKeys(previous, draftKeys(previous).map((item, position) => (position === index ? value : item))));
                    }}
                    placeholder={editing ? t("不改就留空") : "sk-..."}
                    spellCheck={false}
                    type="password"
                    value={key}
                  />
                  {keys.length > 1 ? (
                    <button aria-label={t("删除这个 Key")} className="pf-icon" onClick={() => setDraft((previous) => withKeys(previous, draftKeys(previous).filter((_, position) => position !== index)))} type="button"><X /></button>
                  ) : null}
                </div>
              ))}
              {savedKeys.length ? null : (
                <button className="pf-link" onClick={() => setDraft((previous) => withKeys(previous, [...draftKeys(previous), ""]))} type="button">
                  <Plus />{t("再加一个 Key")}
                </button>
              )}
            </div>
            </> : (
              <label className="pf-field">
                <span>{t("HTTP 代理")}</span>
                <Input onChange={(event) => update({ proxy: event.currentTarget.value })} placeholder="http://127.0.0.1:7890" value={draft.proxy} />
              </label>
            )}
          </section>

          <section className="pf-section">
            <h4>
              {t("模型")}<em>{models.length}</em>
              <button className="pf-action" disabled={!canFetch} onClick={() => void fetchModels()} title={canFetch ? t("请求供应商的 /v1/models") : t("先填地址和 API Key")} type="button">
                <RefreshCw className={fetching ? "animate-spin" : ""} />{fetching ? t("获取中…") : t("从上游获取")}
              </button>
            </h4>
            {fetchNote ? <p className="pf-note" data-tone={fetchNote.tone}>{fetchNote.text}</p> : null}
            {picker ? (
              <ModelPicker
                onApply={(chosen, added, removed) => {
                  setDraft((previous) => withModels(previous, chosen));
                  setPicker(null);
                  setFetchNote({ tone: "ok", text: tf("已选 {0} 个模型（新增 {1}，移除 {2}）", [chosen.length, added, removed]) });
                }}
                picker={picker}
                setPicker={setPicker}
              />
            ) : null}
            <div className="pf-chips">
              {models.map((model) => (
                <span className="pf-chip" data-default={model === chosenModel} key={model}>
                  <button aria-label={tf("设 {0} 为默认模型", [model])} className="pf-star" onClick={() => update({ model })} title={model === chosenModel ? t("默认模型") : t("设为默认模型")} type="button">
                    <Star />
                  </button>
                  <span>{model}</span>
                  <button aria-label={tf("移除 {0}", [model])} className="pf-chip-remove" onClick={() => setDraft((previous) => removeModel(previous, model))} type="button"><X /></button>
                </span>
              ))}
              <input
                aria-label={t("添加模型")}
                className="pf-chip-input"
                onBlur={() => { if (modelInput.trim()) { setDraft((previous) => addModel(previous, modelInput)); setModelInput(""); } }}
                onChange={(event) => setModelInput(event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === ",") {
                    event.preventDefault();
                    if (modelInput.trim()) setDraft((previous) => addModel(previous, modelInput));
                    setModelInput("");
                  }
                }}
                placeholder={models.length ? t("添加模型…") : t("输入模型名回车添加，或从上游获取")}
                value={modelInput}
              />
            </div>
            <p className="pf-hint">{t("星标是默认模型；Agent 请求这些模型名时会路由到这个供应商。")}</p>
            {/* 编辑时：加完模型顺手设上下文窗口（#2785），复用详情里的窗口弹窗，按这里正在编辑的模型列表列出。 */}
            {windowsEntry && onEditWindows ? (
              <div className="pf-windows-entry">
                <span>{t("上下文窗口")}</span>
                <span className="pf-windows-summary">{windowSummary({ ...windowsEntry, models }) || t("未填写，用模型自带资料")}</span>
                <button className="pf-link" disabled={!models.length || busy !== null} onClick={() => onEditWindows(models)} type="button">
                  <Pencil />{t("编辑上下文窗口")}
                </button>
              </div>
            ) : null}
          </section>

          {draft.protocol === "responses" ? (
            <section className="pf-section">
              <h4>{t("兼容")}</h4>
              <CompatToggles onChange={(patch) => update(patch)} values={draft} variant="gateway" />
            </section>
          ) : null}
          {/* 自定义请求头（#3075）：和直连模式「高级兼容选项」里的同一套编辑行与规则；订阅账号的请求头由 Codex++ 自己定，不开放。 */}
          {!subscriptionEditing ? (
            <section className="pf-section" ref={headersRef}>
              <h4>{t("自定义请求头")}{headerCount ? <em>{headerCount}</em> : null}</h4>
              <div className="relay-custom-headers">
                <RelayHeaderRows onChange={(customHeaders) => update({ customHeaders })} rows={draft.customHeaders ?? []} />
              </div>
              <p className="pf-hint">{t("测试连接、获取模型和实际请求都会带上。Host、Content-Length 等传输头不能覆盖；配置 Authorization 时以它为准，不再注入 API Key。")}</p>
            </section>
          ) : null}
          {subscriptionEditing ? (
            <SubscriptionPolicyFields disabled={busy !== null} onChange={(subscriptionPolicy) => update({ subscriptionPolicy })} quota={form.mode === "edit" ? form.entry.quota : null} value={draft.subscriptionPolicy} />
          ) : null}
        </div>

        {agentPicks ? <div className="pf-agents">{agentPicks}</div> : null}
        <footer className="pf-foot">
          <span className="pf-error">{validation ? t(validation) : ""}</span>
          <Button onClick={() => void requestClose()} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy !== null || Boolean(validation)} size="sm" type="submit">{busy ? t("保存中") : editing ? t("保存") : t("添加到供应商池")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}

/** 「从上游获取」的勾选列表最多画这么多行，多的靠搜索。 */
const PICKER_LIMIT = 500;

type PickerState = { rows: PickerRow[]; selected: Set<string>; query: string; fetched: number };

/** 「从上游获取」的勾选列表：先列已在列表里的（默认勾上，取消勾选表示移除；上游没有的标出来），
 * 再列上游新增的（默认不勾）。可搜索，「全选 / 全不选」只作用于搜索结果里的新增模型。 */
function ModelPicker({
  picker,
  setPicker,
  onApply,
}: {
  picker: PickerState;
  setPicker: (next: PickerState | null) => void;
  onApply: (models: string[], added: number, removed: number) => void;
}) {
  const visible = useMemo(() => filterModels(picker.rows, picker.query, (row) => row.model), [picker.rows, picker.query]);
  const visibleNew = visible.filter((row) => !row.inList).map((row) => row.model);
  const outcome = pickerOutcome(picker.rows, picker.selected);
  // 勾选列表也是一层：Esc 只收起它，不关掉整个表单。
  useEscapeLayer(true, () => setPicker(null));
  const toggle = (model: string) => {
    const selected = new Set(picker.selected);
    if (selected.has(model)) selected.delete(model);
    else selected.add(model);
    setPicker({ ...picker, selected });
  };
  const tag = (row: PickerRow) => {
    if (row.inList && !picker.selected.has(row.model)) return { text: t("将移除"), tone: "remove" };
    if (row.inList && !row.upstream) return { text: t("上游没有"), tone: "gone" };
    if (row.inList) return { text: t("已在列表"), tone: undefined };
    return picker.selected.has(row.model) ? { text: t("将加入"), tone: "add" } : null;
  };
  return (
    <div className="pf-picker">
      <div className="pf-picker-head">
        <Search />
        <input
          autoFocus
          onChange={(event) => setPicker({ ...picker, query: event.currentTarget.value })}
          onKeyDown={(event) => {
            // 这个搜索框在表单里：回车只是搜索，不能提交整个表单。
            if (event.key === "Enter") event.preventDefault();
          }}
          placeholder={tf("上游有 {0} 个模型，搜索后勾选", [picker.fetched])}
          value={picker.query}
        />
        <button className="pf-link" disabled={!visibleNew.length} onClick={() => setPicker({ ...picker, selected: new Set([...picker.selected, ...visibleNew]) })} title={t("勾选搜索结果里所有新增的模型")} type="button">{t("全选新增")}</button>
        <button className="pf-link" disabled={!visibleNew.some((model) => picker.selected.has(model))} onClick={() => setPicker({ ...picker, selected: new Set([...picker.selected].filter((model) => !visibleNew.includes(model))) })} title={t("取消勾选搜索结果里新增的模型，已在列表里的不动")} type="button">{t("全不选新增")}</button>
      </div>
      <div className="pf-picker-list">
        {visible.slice(0, PICKER_LIMIT).map((row) => {
          const label = tag(row);
          return (
            <label data-in-list={row.inList} key={row.model}>
              <input checked={picker.selected.has(row.model)} onChange={() => toggle(row.model)} type="checkbox" />
              <span>{row.model}</span>
              {label ? <span className="pf-picker-tag" data-tone={label.tone}>{label.text}</span> : null}
            </label>
          );
        })}
        {!visible.length ? <span className="pf-picker-empty">{t("没有匹配的模型")}</span> : null}
        {visible.length > PICKER_LIMIT ? (
          <span className="pf-picker-empty">{tf("只显示前 {0} 个，共 {1} 个；用搜索缩小范围。", [PICKER_LIMIT, visible.length])}</span>
        ) : null}
      </div>
      <div className="pf-picker-foot">
        <span className="pf-sync-summary">{tf("新增 {0} · 移除 {1}", [outcome.added, outcome.removed])}</span>
        <button className="pf-link" onClick={() => setPicker(null)} type="button">{t("取消")}</button>
        <Button disabled={!outcome.models.length} onClick={() => onApply(outcome.models, outcome.added, outcome.removed)} size="sm" type="button">
          {tf("使用选中的 {0} 个", [outcome.models.length])}
        </Button>
      </div>
    </div>
  );
}
