/** 供应商池的「测试」弹窗：选模型（和 Key），给这家供应商发一条很短的请求，显示结果与排查提示。
 *
 * 请求走网关同一条路（协议转换、这家的代理与 Key、账号类先换 token），但不计入路由与统计：
 * 不占限速名额、不记冷却 / 熔断 / 测速、不改会话固定，请求日志和用量里也没有它。
 * 后台最多等 60 秒；关掉弹窗或点「取消」会中止进行中的测试。 */
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { CheckCircle2, Loader2, X, XCircle, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import { AppSelect } from "./AppSelect";
import { maskAccountName, type GatewayPoolEntry } from "./gateway-model";
import { Avatar } from "./gateway-ui";
import { useEscapeLayer } from "./use-escape-layer";
import { formatProbeMs, probeHints, probeWireLabel, type ProviderProbeResult } from "./provider-test";
import "./ProviderFormDialog.css";

type ProbeResponse = Partial<ProviderProbeResult> & { status: string; message: string };

/** 「按网关顺序」：不指定 Key，用当前选中的那个。 */
const CURRENT_KEY = "";

export function ProviderTestDialog({
  entry,
  keyId,
  onClose,
}: {
  entry: GatewayPoolEntry;
  /** 从 Key 列表点进来时只用这个 Key；为空时用当前选中的 Key。 */
  keyId: string | null;
  onClose: () => void;
}) {
  const imageModels = (entry.imageModels ?? []).map((model) => model.toLowerCase());
  const models = entry.models.filter((model) => !imageModels.includes(model.toLowerCase()));
  const keys = entry.keys ?? [];
  const [model, setModel] = useState(models[0] ?? "");
  const [key, setKey] = useState(keyId ?? CURRENT_KEY);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<ProviderProbeResult | null>(null);
  const [error, setError] = useState("");
  // 每次开始测试加一：取消或关掉之后回来的旧结果不再显示。
  const seq = useRef(0);
  const runningRef = useRef(false);

  const cancel = () => {
    seq.current += 1;
    if (runningRef.current) void invoke("cancel_gateway_provider_test").catch(() => undefined);
    runningRef.current = false;
    setRunning(false);
  };
  const close = () => {
    cancel();
    onClose();
  };
  useEscapeLayer(true, close);
  // 弹窗被卸载（例如整页切走）时也中止后台测试。
  useEffect(() => () => {
    seq.current += 1;
    if (runningRef.current) void invoke("cancel_gateway_provider_test").catch(() => undefined);
  }, []);

  const start = async () => {
    if (!model || runningRef.current) return;
    const current = ++seq.current;
    runningRef.current = true;
    setRunning(true);
    setResult(null);
    setError("");
    try {
      const response = await invoke<ProbeResponse>("test_gateway_provider", { id: entry.id, model, keyId: key || null });
      if (current !== seq.current) return;
      if (response.status === "ok") setResult(response as ProviderProbeResult);
      else if (response.status !== "cancelled") setError(t(response.message));
    } catch (failure) {
      if (current === seq.current) setError(String(failure));
    } finally {
      if (current === seq.current) {
        runningRef.current = false;
        setRunning(false);
      }
    }
  };

  const keyOptions = [
    { value: CURRENT_KEY, label: t("当前选中的 Key") },
    ...keys.map((item) => ({ value: item.id, label: item.masked })),
  ];
  const usedKey = result?.keyId ? keys.find((item) => item.id === result.keyId)?.masked ?? "" : "";
  const hints = result ? probeHints(result, Boolean(entry.account)) : [];

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) close(); }}>
      <form
        className="pf-modal pf-extras-modal pt-modal"
        onSubmit={(event) => {
          event.preventDefault();
          void start();
        }}
      >
        <header className="pf-head">
          <Avatar baseUrl={entry.baseUrl} name={entry.name} />
          <div>
            <strong>{tf("测试 {0}", [maskAccountName(entry.name)])}</strong>
            <small>{t("发一条很短的请求，不计入路由、限速与统计")}</small>
          </div>
          <button aria-label={t("关闭")} className="pf-icon" onClick={close} type="button"><X /></button>
        </header>

        <div className="pf-body">
          <section className="pf-section">
            <div className="pf-field">
              <span>{t("模型")}</span>
              {models.length ? (
                <AppSelect
                  ariaLabel={t("测试用的模型")}
                  disabled={running}
                  onChange={setModel}
                  options={models.map((item) => ({ value: item, label: item }))}
                  value={model}
                />
              ) : (
                <span className="pf-note" data-tone="error">{t("这个供应商还没有模型，先在「模型」一行添加")}</span>
              )}
            </div>
            {keys.length > 1 ? (
              <div className="pf-field">
                <span>Key</span>
                <AppSelect ariaLabel={t("测试用的 Key")} disabled={running} onChange={setKey} options={keyOptions} value={key} />
              </div>
            ) : null}
            <p className="pf-note">{t("按这家的协议、代理和 Key 发送，让模型只回一个词；最多等 60 秒。")}</p>
          </section>

          {running ? (
            <section className="pt-result" data-state="running">
              <div className="pt-status">
                <Loader2 aria-hidden="true" className="animate-spin" />
                <strong>{t("测试中…")}</strong>
              </div>
            </section>
          ) : null}

          {result ? (
            <section className="pt-result" data-state={result.ok ? "ok" : "failed"}>
              <div className="pt-status">
                {result.ok ? <CheckCircle2 aria-hidden="true" /> : <XCircle aria-hidden="true" />}
                <strong>{result.ok ? t("成功") : t("失败")}</strong>
                {result.httpStatus ? <code className="pt-chip">HTTP {result.httpStatus}</code> : null}
                <span className="pt-chip">{probeWireLabel(result.wireApi)}</span>
              </div>
              <dl className="pt-metrics">
                <div><dt>{t("首字")}</dt><dd>{formatProbeMs(result.firstTokenMs)}</dd></div>
                <div><dt>{t("总耗时")}</dt><dd>{formatProbeMs(result.totalMs)}</dd></div>
                <div><dt>{t("模型")}</dt><dd className="pt-mono">{result.model}</dd></div>
                {usedKey ? <div><dt>Key</dt><dd className="pt-mono">{usedKey}</dd></div> : null}
              </dl>
              {result.ok ? (
                result.reply ? (
                  <div className="pt-block">
                    <span>{t("回复")}</span>
                    <pre className="pt-text">{result.reply}{result.replyTruncated ? "…" : ""}</pre>
                  </div>
                ) : (
                  <p className="pf-note">{t("上游没有返回文字（输出额度可能都用在思考上了），但请求本身是通的。")}</p>
                )
              ) : (
                <>
                  {hints.length ? (
                    <ul className="pt-hints">
                      {hints.map((hint) => <li key={hint.text}>{hint.args.length ? tf(hint.text, hint.args) : t(hint.text)}</li>)}
                    </ul>
                  ) : null}
                  {result.error ? (
                    <div className="pt-block">
                      <span>{t("错误原文")}</span>
                      <pre className="pt-text pt-error">{result.error}</pre>
                    </div>
                  ) : null}
                </>
              )}
            </section>
          ) : null}
        </div>

        <footer className="pf-foot">
          <span className="pf-error">{error}</span>
          {running ? (
            <Button onClick={cancel} size="sm" type="button" variant="ghost">{t("取消测试")}</Button>
          ) : (
            <Button onClick={close} size="sm" type="button" variant="ghost">{t("关闭")}</Button>
          )}
          <Button disabled={running || !model} size="sm" type="submit">
            <Zap className="h-3.5 w-3.5" />
            {result || error ? t("再测一次") : t("开始测试")}
          </Button>
        </footer>
      </form>
    </div>,
    document.body,
  );
}
