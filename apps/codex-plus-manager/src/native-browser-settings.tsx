import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { CheckCircle2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { browserConnectionLabel, browserHeaderLabel, nativeBrowserStatusLabel, type NativeBrowserDiagnostics } from "./native-browser-status";

export const nativeBrowserConsent =
  "启用原生 Edge / Chrome 请求标识兼容？\n\n" +
  "受控标签页的目标网站可收到包含会话标识的 x-browser-agent 请求头。" +
  "原生扩展会保留开启状态；关闭此兼容选项或恢复服务文件不会关闭扩展已保存的标识设置。\n\n" +
  "此选项不代替站点授权、企业策略或浏览器操作审批。仅适配指定 Windows 原生运行时的 Edge / Chrome 稳定版扩展。\n\n" +
  "保存后需由你关闭并重新启动 Codex / Codex++ 才能完成验收，不会自动重启。";

export function NativeBrowserStatusView() {
  const [result, setResult] = useState<NativeBrowserDiagnostics>({
    compatibility: { state: "not_started", detail: "" },
    connection: { state: "checking", browsers: [], failedChecks: 0 },
  });
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    try {
      const next = await invoke<NativeBrowserDiagnostics>("native_browser_status");
      if (mounted.current) setResult(next);
    } catch {
      if (mounted.current) setResult({
        compatibility: { state: "unavailable", detail: "" },
        connection: { state: "check_failed", browsers: [], failedChecks: 1 },
      });
    } finally {
      inFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    const updateVisible = () => { if (!document.hidden) void refresh(); };
    updateVisible();
    const timer = window.setInterval(updateVisible, 15_000);
    document.addEventListener("visibilitychange", updateVisible);
    return () => {
      mounted.current = false;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", updateVisible);
    };
  }, [refresh]);
  const { compatibility, connection } = result;
  return (
    <div className="feature-action-row items-start">
      <div className="min-w-0 flex-1">
        <div role="status" aria-live="polite" aria-busy={busy}>
          <div className="flex items-center gap-2">
            {connection.state === "available" && connection.browsers.length > 0
              ? <CheckCircle2 className="size-4 shrink-0 text-green-600" aria-hidden="true" /> : null}
            <small>{browserConnectionLabel(connection)}</small>
          </div>
          {connection.browsers.map(browser => <small className="block" key={`${browser.family}-${browser.headerEnabled}`}>
            {browser.family === "edge" ? "Edge" : "Chrome"} · {browserHeaderLabel(browser.headerEnabled)}
          </small>)}
          {compatibility.state === "blocked"
            ? <small className="block">兼容补丁文件检查或恢复受阻</small> : null}
        </div>
        <details className="mt-2 min-w-0 text-xs">
          <summary className="cursor-pointer">兼容补丁详情</summary>
          <small className="block">{nativeBrowserStatusLabel(compatibility.state)}</small>
          {compatibility.detail ? <small className="block break-all">{compatibility.detail}</small> : null}
          {connection.failedChecks > 0
            ? <small className="block">{connection.failedChecks} 个原生连接端点未通过检测</small> : null}
        </details>
      </div>
      <Button className="shrink-0" variant="outline" size="icon" disabled={busy} onClick={() => void refresh()}
        aria-label="刷新浏览器状态" title="刷新浏览器状态">
        <RefreshCw />
      </Button>
    </div>
  );
}
