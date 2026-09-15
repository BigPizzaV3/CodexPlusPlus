import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { nativeBrowserStatusLabel } from "./native-browser-status";

export const nativeBrowserConsent =
  "启用原生 Edge / Chrome 请求标识兼容？\n\n" +
  "受控标签页的目标网站可收到包含会话标识的 x-browser-agent 请求头。" +
  "原生扩展会保留开启状态；关闭此兼容选项或恢复服务文件不会关闭扩展已保存的标识设置。\n\n" +
  "此选项不代替站点授权、企业策略或浏览器操作审批。仅适配指定 Windows 原生运行时的 Edge / Chrome 稳定版扩展。\n\n" +
  "保存后需由你关闭并重新启动 Codex / Codex++ 才能完成验收，不会自动重启。";

export function NativeBrowserStatusView() {
  const [state, setState] = useState("not_started");
  const [detail, setDetail] = useState("");
  const [busy, setBusy] = useState(false);
  const refresh = async () => {
    setBusy(true);
    try {
      const result = await invoke<{ state: string; detail: string }>("native_browser_status");
      setState(result.state);
      setDetail(result.detail);
    } catch {
      setState("unavailable");
      setDetail("");
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => { void refresh(); }, []);
  return (
    <div className="feature-action-row">
      <div role="status">
        <small>{nativeBrowserStatusLabel(state)}</small>
        {state === "blocked" && detail ? <small>{detail}</small> : null}
      </div>
      <Button variant="outline" size="icon" disabled={busy} onClick={() => void refresh()}
        aria-label="刷新原生浏览器兼容状态" title="刷新原生浏览器兼容状态">
        <RefreshCw />
      </Button>
    </div>
  );
}
