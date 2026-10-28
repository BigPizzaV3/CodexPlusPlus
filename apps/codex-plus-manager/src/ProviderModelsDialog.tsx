/** 供应商的模型列表：手动增删、设默认模型，或用存下的凭据从上游刷新（订阅账号也能刷）。
 * 刷新只列出新增 / 已下架的模型，由用户决定加不加、删不删，保存后才写回。 */
import { useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { Plus, RefreshCw, Star, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import { maskAccountName, splitModels, type GatewayPoolEntry } from "./gateway-model";
import { Avatar } from "./gateway-ui";
import { useEscapeLayer } from "./use-escape-layer";
import "./ProviderFormDialog.css";

/** 后端 `ModelDiff`，随状态一起摊平在结果里。 */
type CheckResult = { status: string; message: string; upstream?: string[]; added?: string[]; removed?: string[] };

const same = (left: string, right: string) => left.toLowerCase() === right.toLowerCase();

export function ProviderModelsDialog({
  entry,
  busy,
  onSave,
  onCancel,
}: {
  entry: GatewayPoolEntry;
  busy: boolean;
  onSave: (models: string[], defaultModel: string) => void;
  onCancel: () => void;
}) {
  const [models, setModels] = useState<string[]>(entry.models);
  const [defaultModel, setDefaultModel] = useState(entry.models[0] ?? "");
  // 后台检查已经发现的差异直接带进来，不用再点刷新。
  const [added, setAdded] = useState<string[]>(entry.upstreamNew ?? []);
  const [gone, setGone] = useState<string[]>(entry.upstreamGone ?? []);
  const [input, setInput] = useState("");
  const [checking, setChecking] = useState(false);
  const [note, setNote] = useState<{ tone: "ok" | "error"; text: string } | null>(null);
  const pending = added.filter((model) => !models.some((item) => same(item, model)));
  const goneHere = gone.filter((model) => models.some((item) => same(item, model)));

  useEscapeLayer(true, onCancel);

  const add = (list: string[]) => setModels((previous) => [...previous, ...list.filter((model) => !previous.some((item) => same(item, model)))]);
  const remove = (model: string) => setModels((previous) => previous.filter((item) => item !== model));
  const commitInput = () => {
    if (input.trim()) add(splitModels(input));
    setInput("");
  };

  const refresh = async () => {
    setChecking(true);
    setNote(null);
    try {
      const result = await invoke<CheckResult>("check_gateway_provider_models", { id: entry.id });
      if (result.status === "ok" && result.upstream) {
        const upstream = result.upstream;
        // 和对话框里当前的列表比（可能已经手动改过）。
        setAdded(upstream.filter((model) => !models.some((item) => same(item, model))));
        setGone(models.filter((model) => !upstream.some((item) => same(item, model))));
        setNote({ tone: "ok", text: tf("上游有 {0} 个模型", [upstream.length]) });
      } else {
        setNote({ tone: "error", text: result.message });
      }
    } catch (error) {
      setNote({ tone: "error", text: String(error) });
    } finally {
      setChecking(false);
    }
  };

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <form
        className="pf-modal"
        onSubmit={(event) => {
          event.preventDefault();
          onSave(models, models.some((model) => model === defaultModel) ? defaultModel : models[0] ?? "");
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{tf("{0} 的模型", [maskAccountName(entry.name)])}</strong>
            <small>{t("Agent 请求这些模型名时会路由到这个供应商")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={onCancel} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            <h4>
              {t("模型")}<em>{models.length}</em>
              <button className="pf-action" disabled={checking} onClick={() => void refresh()} title={t("用已保存的 Key 或登录重新获取")} type="button">
                <RefreshCw className={checking ? "animate-spin" : ""} />{checking ? t("获取中…") : t("从上游刷新")}
              </button>
            </h4>
            {note ? <p className="pf-note" data-tone={note.tone}>{note.text}</p> : null}

            {pending.length ? (
              <div className="pf-model-diff" data-kind="added">
                <span>{tf("上游新增 {0}", [pending.length])}</span>
                {pending.map((model) => (
                  <button className="pf-chip pf-chip-new" key={model} onClick={() => add([model])} title={t("加入列表")} type="button">
                    <Plus />{model}
                  </button>
                ))}
                <button className="pf-text-action" onClick={() => add(pending)} type="button">{t("全部加入")}</button>
              </div>
            ) : null}
            {goneHere.length ? (
              <div className="pf-model-diff" data-kind="gone">
                <span>{tf("上游已没有 {0}", [goneHere.length])}</span>
                <button className="pf-text-action" onClick={() => setModels((previous) => previous.filter((model) => !goneHere.some((item) => same(item, model))))} type="button">
                  {t("全部移除")}
                </button>
              </div>
            ) : null}

            <div className="pf-chips">
              {models.map((model) => (
                <span className="pf-chip" data-default={model === defaultModel} data-gone={goneHere.some((item) => same(item, model))} key={model}>
                  <button aria-label={tf("设 {0} 为默认模型", [model])} className="pf-star" onClick={() => setDefaultModel(model)} title={model === defaultModel ? t("默认模型") : t("设为默认模型")} type="button">
                    <Star />
                  </button>
                  <span title={goneHere.some((item) => same(item, model)) ? t("上游已没有这个模型") : undefined}>{model}</span>
                  <button aria-label={tf("移除 {0}", [model])} className="pf-chip-remove" onClick={() => remove(model)} type="button"><X /></button>
                </span>
              ))}
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
                placeholder={models.length ? t("添加模型…") : t("输入模型名回车添加，或从上游刷新")}
                value={input}
              />
            </div>
            <p className="pf-hint">{t("星标是默认模型。新加的模型保存后会按公开价格表自动填价格。")}</p>
          </section>
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{models.length ? "" : t("至少保留一个模型")}</span>
          <Button onClick={onCancel} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button disabled={busy || !models.length} size="sm" type="submit">{busy ? t("保存中") : t("保存")}</Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}
