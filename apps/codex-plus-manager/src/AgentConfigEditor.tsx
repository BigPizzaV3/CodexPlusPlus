import { createContext, useContext, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import { useEscapeLayer } from "./use-escape-layer";
import { useModalFocus } from "./use-modal-focus";

type ConfigView = { source: string; inputFormat: string; contents: string; editable: boolean };

export type ConfigTarget = { agent: string; name: string };
export const OpenAgentConfig = createContext<((target: ConfigTarget) => void) | null>(null);

export function AgentConfigButton({ agent, name }: ConfigTarget) {
  const open = useContext(OpenAgentConfig);
  return (
    <Button size="sm" type="button" variant="ghost" onClick={() => open?.({ agent, name })}>
      {agent === "codex" ? t("编辑通用配置") : t("编辑配置")}
    </Button>
  );
}

export function AgentConfigDialog({ agent, name, onClose }: ConfigTarget & {
  onClose: () => void;
}) {
  const [view, setView] = useState<ConfigView | null>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const editorId = useId();
  const close = () => { if (!saving) onClose(); };
  useEscapeLayer(true, close);
  useModalFocus(root);
  useEffect(() => {
    let current = true;
    invoke<ConfigView>("read_agent_config", { agent }).then(result => {
      if (current) { setView(result); setDraft(result.contents); }
    }).catch(reason => { if (current) setError(String(reason)); });
    return () => { current = false; };
  }, [agent]);
  const save = async () => {
    if (!view?.editable || saving) return;
    setSaving(true);
    setError("");
    try {
      await invoke<ConfigView>("save_agent_config", {
        agent,
        original: { source: view.source, contents: view.contents },
        contents: draft,
      });
      onClose();
    } catch (reason) { setError(String(reason)); } finally { setSaving(false); }
  };
  const title = tf("编辑 {0} 配置", [name]);
  return createPortal(<div className="gw-modal-backdrop gw-portal">
    <div role="dialog" aria-modal="true" aria-label={title} ref={root} tabIndex={-1} className="gw-modal">
      <header><strong>{title}</strong></header>
      <div className="gw-modal-body">
        {!view && !error ? <p role="status">{t("正在读取…")}</p> : null}
        {view ? <>
          <p>{t("配置来源")}：<code>{view.source}</code></p>
          {view.editable ? <>
            <label htmlFor={editorId}>{tf("输入格式：{0}", [view.inputFormat])}</label>
            <textarea id={editorId} className="relay-file-textarea" rows={12} spellCheck={false} disabled={saving}
              value={draft} onChange={event => setDraft(event.currentTarget.value)} />
          </> : <p>{t("配置只读")}</p>}
        </> : null}
        {error ? <p role="alert" className="gw-disconnect-error">{error}</p> : null}
      </div>
      <footer><Button size="sm" type="button" variant="ghost" disabled={saving} onClick={close}>{t("关闭")}</Button>
        {view?.editable ? <Button size="sm" type="button" disabled={saving || draft === view.contents} onClick={() => void save()}>{t("保存")}</Button> : null}
      </footer>
    </div>
  </div>, document.body);
}
