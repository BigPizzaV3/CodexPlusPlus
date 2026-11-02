import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { managerDataRequest } from "./extension-manager-pages";
import {
  handleManagerRequest,
  isManagerRequest,
  managerPanelSrcdoc,
  MANAGER_PANEL_MIN_HEIGHT,
  type ManagerTheme,
} from "./extension-manager-bridge";

/// 拓展的管理器面板：拓展包 `manager.page`（或设置页 `settings.page`）的 HTML 放进沙箱 iframe，
/// 经 postMessage 只调本拓展的配置、后台、资源和清单声明过的数据路由。
export function ExtensionManagerPanel({ scriptKey, config, onSaveConfig, onToast, dataSources = [], onNavigate, fullPage = false, kind = "manager" }: {
  scriptKey: string;
  config: Record<string, unknown>;
  onSaveConfig: (values: Record<string, unknown>) => Promise<boolean>;
  onToast: (message: string) => void;
  /** 清单 manager.data 声明过的数据名。 */
  dataSources?: string[];
  /** 页面里的 codexPlusManager.navigate(target)；返回是否认得这个页面。 */
  onNavigate?: (target: string) => boolean;
  /** 作为整页（侧栏入口、顶替内置页面）显示：高度至少占满内容区。 */
  fullPage?: boolean;
  /** "settings"：拓展自己的设置页（拓展停用时也能打开）。 */
  kind?: "manager" | "settings";
}) {
  const frame = useRef<HTMLIFrameElement | null>(null);
  const [page, setPage] = useState<{ title: string; html: string } | null>(null);
  const [error, setError] = useState("");
  const [height, setHeight] = useState(240);
  const [theme, setTheme] = useState<ManagerTheme>(() => (document.querySelector(".shell")?.classList.contains("dark") ? "dark" : "light"));
  // 回调里读最新的配置与回调，不随每次渲染重挂监听。
  const latest = useRef({ config, onSaveConfig, onToast, theme, dataSources, onNavigate });
  latest.current = { config, onSaveConfig, onToast, theme, dataSources, onNavigate };

  useEffect(() => {
    let cancelled = false;
    setError("");
    invoke<{ title: string; html: string }>("extension_manager_page", { key: scriptKey, kind })
      .then((result) => { if (!cancelled) setPage(result); })
      .catch((reason) => { if (!cancelled) setError(String(reason)); });
    return () => { cancelled = true; };
  }, [scriptKey, kind]);

  // 主题跟着管理器外壳（.shell.dark）走，变了推给面板页。
  useEffect(() => {
    const shell = frame.current?.closest(".shell") ?? document.querySelector(".shell");
    const read = (): ManagerTheme => (shell?.classList.contains("dark") ? "dark" : "light");
    setTheme(read());
    if (!shell) return;
    const observer = new MutationObserver(() => {
      const next = read();
      setTheme(next);
      frame.current?.contentWindow?.postMessage({ __codexPlusManager: 1, event: "theme", value: next }, "*");
    });
    observer.observe(shell, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, [page]);

  // 配置在别处改了（设置表单、Codex 页面）也推给面板页。
  useEffect(() => {
    frame.current?.contentWindow?.postMessage({ __codexPlusManager: 1, event: "config", value: config }, "*");
  }, [config]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const target = frame.current?.contentWindow;
      // 只认本面板 iframe 发来的请求；沙箱页的 origin 是 "null"，靠 source 判断归属。
      if (!target || event.source !== target || !isManagerRequest(event.data)) return;
      void handleManagerRequest(event.data, {
        config: () => latest.current.config,
        saveConfig: (values) => latest.current.onSaveConfig(values),
        backendCall: (method, params) => invoke("extension_manager_backend_call", { key: scriptKey, method, params }),
        asset: (path) => invoke<string>("extension_manager_asset", { key: scriptKey, path, kind }),
        toast: (message) => latest.current.onToast(message),
        resize: (next) => setHeight(next),
        theme: () => latest.current.theme,
        data: async (name, params) => {
          const request = managerDataRequest(name, params, latest.current.dataSources, scriptKey);
          return invoke(request.command, request.args);
        },
        navigate: (next) => latest.current.onNavigate?.(next) ?? false,
        call: (route, payload) => invoke("extension_page_call", { key: scriptKey, route, payload }),
      }).then((reply) => target.postMessage(reply, "*"));
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [scriptKey, kind]);

  // srcdoc 只随页面内容变（主题变化经消息推送），否则每次换主题都会重载面板。
  const srcdoc = useMemo(() => (page ? managerPanelSrcdoc(page.html, latest.current.theme) : ""), [page]);

  if (error) return <div className="extension-manager-panel is-error" role="alert">{t("面板加载失败：")}{error}</div>;
  if (!page) return <div className="extension-manager-panel is-loading">{t("正在加载面板…")}</div>;
  return (
    <div className={`extension-manager-panel${fullPage ? " is-full-page" : ""}`}>
      <iframe
        ref={frame}
        title={page.title}
        // 允许脚本和表单交互，外发由 CSP 限制；没有同源、弹窗、顶层跳转权限。iframe 自导航仍须信任作者。
        sandbox="allow-scripts allow-forms"
        srcDoc={srcdoc}
        style={{ width: "100%", height: Math.max(fullPage ? 480 : MANAGER_PANEL_MIN_HEIGHT, height), border: 0, display: "block" }}
      />
    </div>
  );
}
