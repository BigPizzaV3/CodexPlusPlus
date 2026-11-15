/** 远程控制：在浏览器里查看与操控 Codex 桌面里的会话（发消息、插话、停止、审批、回答提问）。 */
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Copy, ExternalLink, KeyRound, Smartphone } from "lucide-react";
import { PageHeader } from "./ManagementPages";
import { Switch } from "./gateway-ui";
import { CxBadge, CxBanner, CxCard, CxRow, CxSection } from "./cx-layout";
import { t } from "./i18n";

export type RemoteControlStatus = {
  enabled: boolean;
  running: boolean;
  port: number;
  url?: string | null;
  error?: string | null;
};

export function remoteControlRequest(action: "status" | "enable" | "disable" | "resetToken") {
  return invoke<RemoteControlStatus>("remote_control_request", { action });
}

export function RemoteControlPage() {
  const [status, setStatus] = useState<RemoteControlStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const run = useCallback(async (action: Parameters<typeof remoteControlRequest>[0]) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await remoteControlRequest(action));
    } catch (reason) {
      setError(String(reason));
      try {
        setStatus(await remoteControlRequest("status"));
      } catch {}
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void run("status");
  }, [run]);

  const url = status?.url ?? null;
  const copy = async () => {
    if (!url) return;
    await navigator.clipboard.writeText(url);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  const openInBrowser = async () => {
    if (!url) return;
    try {
      await invoke("open_external_url", { url });
    } catch (reason) {
      setError(String(reason));
    }
  };

  return (
    <div className="remote-control-page">
      <PageHeader icon={<Smartphone size={20} strokeWidth={1.5} />} title={t("远程控制")} />
      {error || status?.error ? (
        <div role="alert"><CxBanner icon={<AlertTriangle />} title={error ?? status?.error ?? ""} tone="warn" /></div>
      ) : null}
      <CxSection title={t("远程控制")} aside={<CxBadge tone="warn">{t("实验")}</CxBadge>}>
        <CxCard>
          <CxRow
            title={t("启用远程控制")}
            detail={t("在浏览器里查看 Codex 的会话、发消息、停止，并处理命令审批和模型提问。目前只监听本机 127.0.0.1。")}
            badges={status?.running ? <CxBadge tone="ok">{t("运行中")}</CxBadge> : null}
          >
            <Switch
              label={t("启用远程控制")}
              on={Boolean(status?.enabled)}
              pending={busy}
              disabled={busy || !status}
              onToggle={() => void run(status?.enabled ? "disable" : "enable")}
            />
          </CxRow>
          {url ? (
            <CxRow
              title={t("访问链接")}
              detail={<code className="remote-control-url">{url.replace(/#token=.*/, "#token=…")}</code>}
              tooltip={t("链接里带着访问令牌，拿到链接就能操控 Codex，不要发给别人。")}
            >
              <button className="mg-button" type="button" onClick={() => void copy()}>
                <Copy size={14} />{copied ? t("已复制") : t("复制")}
              </button>
              <button className="mg-button" type="button" onClick={() => void openInBrowser()}>
                <ExternalLink size={14} />{t("在浏览器打开")}
              </button>
            </CxRow>
          ) : null}
          {status?.enabled ? (
            <CxRow
              title={t("更换令牌")}
              detail={t("之前打开的页面会立刻失效，需要用新链接重新打开。")}
            >
              <button className="mg-button" type="button" disabled={busy} onClick={() => void run("resetToken")}>
                <KeyRound size={14} />{t("更换")}
              </button>
            </CxRow>
          ) : null}
        </CxCard>
      </CxSection>
      <CxSection title={t("使用说明")}>
        <CxCard>
          <CxRow
            title={t("需要从 Codex++ 启动 Codex")}
            detail={t("会话在 Codex 桌面里执行。没在桌面打开的会话，发消息时会自动在桌面打开它，这一步需要 Codex 由 Codex++ 启动。")}
          />
          <CxRow
            title={t("审批与提问")}
            detail={t("命令审批、模型提问会同时出现在桌面和网页上，任一端处理即可。")}
          />
          <CxRow
            title={t("手机访问")}
            detail={t("下一步开放局域网与中继访问；目前只能在本机浏览器里使用。")}
          />
        </CxCard>
      </CxSection>
    </div>
  );
}
