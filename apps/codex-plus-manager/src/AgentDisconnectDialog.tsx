import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { FileDiff, Loader2, RefreshCw, ShieldCheck, Unplug, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import type { GatewayAgentView, GatewayDisconnectPreview, PickerItem } from "./gateway-model";
import { disconnectFileLabel, disconnectPreviewReady } from "./gateway-agent-safety";
import { useEscapeLayer } from "./use-escape-layer";
import { useModalFocus } from "./use-modal-focus";

type PreviewResult = { status: string; message: string; preview?: GatewayDisconnectPreview | null };
type Loaded = { state: "loading" } | { state: "error"; message: string } | { state: "ready"; preview: GatewayDisconnectPreview };

export function AgentDisconnectDialog({
  agent,
  ownModel,
  busy,
  onApply,
  onClose,
}: {
  agent: Pick<GatewayAgentView, "agent" | "name">;
  ownModel?: PickerItem | null;
  busy: boolean;
  onApply: (previewToken: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const [loaded, setLoaded] = useState<Loaded>({ state: "loading" });
  const [applying, setApplying] = useState(false);
  const latest = useRef(0);
  const applyingRef = useRef(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const close = () => { if (!busy && !applyingRef.current) onClose(); };
  useEscapeLayer(true, close);
  useModalFocus(rootRef);

  const load = async () => {
    if (applyingRef.current) return;
    const current = ++latest.current;
    setLoaded({ state: "loading" });
    try {
      const result = await invoke<PreviewResult>("preview_gateway_disconnect", { agent: agent.agent });
      if (current !== latest.current) return;
      if (result.status !== "ok" || !result.preview || result.preview.agent !== agent.agent) {
        setLoaded({ state: "error", message: result.message || t("无法获取断开预览") });
        return;
      }
      setLoaded({ state: "ready", preview: result.preview });
    } catch (error) {
      if (current === latest.current) setLoaded({ state: "error", message: String(error) });
    }
  };

  useEffect(() => {
    void load();
    return () => { latest.current += 1; };
    // 每个 Agent 打开时读取；刷新会作废尚未返回的旧预览。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agent.agent]);

  const preview = loaded.state === "ready" ? loaded.preview : null;
  const blocked = busy || applying;
  const apply = async () => {
    if (busy || applyingRef.current || !disconnectPreviewReady(preview, agent.agent)) return;
    applyingRef.current = true;
    setApplying(true);
    const current = ++latest.current;
    try {
      const done = await onApply(preview.previewToken);
      if (current !== latest.current) return;
      if (done) onClose();
      else setLoaded({ state: "error", message: t("操作未完成，请重新获取预览后再确认。") });
    } catch (error) {
      if (current === latest.current) setLoaded({ state: "error", message: String(error) });
    } finally {
      if (current === latest.current) {
        applyingRef.current = false;
        setApplying(false);
      }
    }
  };
  const title = ownModel ? tf("断开并直连 {0}", [agent.name]) : tf("断开 {0} 并还原", [agent.name]);

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" onMouseDown={(event) => { if (event.target === event.currentTarget) close(); }}>
      <div aria-busy={blocked || loaded.state === "loading"} aria-label={title} aria-modal="true" className="gw-modal gw-disconnect-modal" ref={rootRef} role="dialog" tabIndex={-1}>
        <header>
          <Unplug aria-hidden="true" className="h-4 w-4" />
          <strong>{title}</strong>
          <button aria-label={t("关闭")} className="gw-icon-button" disabled={blocked} onClick={close} type="button"><X className="h-4 w-4" /></button>
        </header>
        <div className="gw-modal-body gw-disconnect-body">
          {ownModel ? (
            <p className="gw-disconnect-note">{tf("还原后将选择 {0}，使用 {1} 的登录或 Key 直连。下面仅预览接入配置的还原差异。", [ownModel.label, ownModel.direct || agent.name])}</p>
          ) : null}
          {loaded.state === "loading" ? (
            <p className="gw-disconnect-note" role="status"><Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />{t("正在检查文件与还原差异…")}</p>
          ) : null}
          {loaded.state === "error" ? <p className="gw-disconnect-error" role="alert">{t(loaded.message)}</p> : null}
          {preview ? (
            <>
              {preview.warnings.length ? <ul className="gw-disconnect-warnings">{preview.warnings.map((warning, index) => <li key={index}>{t(warning)}</li>)}</ul> : null}
              {preview.files.map((file, index) => (
                <section className="gw-disconnect-file" key={`${file.path}:${index}`}>
                  <div className="gw-disconnect-file-head">
                    <FileDiff aria-hidden="true" className="h-4 w-4" />
                    <code>{file.path}</code>
                    <span>{t(disconnectFileLabel(file.kind))}</span>
                  </div>
                  {file.redacted ? <p className="gw-disconnect-redacted"><ShieldCheck aria-hidden="true" className="h-3.5 w-3.5" />{t("敏感值已隐藏")}</p> : null}
                  <pre className="gw-disconnect-diff" tabIndex={0}>{file.diff ? file.diff.split("\n").map((line, lineIndex) => (
                    <span data-kind={line.startsWith("+") && !line.startsWith("+++") ? "add" : line.startsWith("-") && !line.startsWith("---") ? "remove" : "context"} key={lineIndex}>{line}{"\n"}</span>
                  )) : t("文件内容无需修改")}</pre>
                </section>
              ))}
              {!preview.files.length ? <p className="gw-disconnect-note">{t("没有需要改写的配置文件。")}</p> : null}
              {!preview.canDisconnect ? <p className="gw-disconnect-error" role="alert">{t("当前无法安全还原，请先处理上面的文件问题。")}</p> : null}
            </>
          ) : null}
        </div>
        <footer>
          {loaded.state !== "loading" ? <Button disabled={blocked} onClick={() => void load()} size="sm" type="button" variant="ghost"><RefreshCw className="h-3.5 w-3.5" />{t("重新获取预览")}</Button> : null}
          <span className="gw-disconnect-foot-space" />
          <Button data-autofocus disabled={blocked} onClick={close} size="sm" type="button" variant="ghost">{t("取消")}</Button>
          <Button className="gw-disconnect-confirm" disabled={blocked || !disconnectPreviewReady(preview, agent.agent)} onClick={() => void apply()} size="sm" type="button">
            {applying ? <Loader2 aria-hidden="true" className="h-3.5 w-3.5 animate-spin" /> : <Unplug aria-hidden="true" className="h-3.5 w-3.5" />}
            {applying ? t("正在还原…") : ownModel ? t("断开并直连") : t("断开并还原")}
          </Button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}
