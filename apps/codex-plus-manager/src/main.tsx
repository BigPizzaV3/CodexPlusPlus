// 旧版系统 WebKit 的 API 兜底必须最先执行（#2753），见 legacy-webkit.ts。
import "./legacy-webkit";
import { Component, type ErrorInfo, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { CuPipWindow } from "./CuPipWindow";
import { TrayPanel } from "./TrayPanel";
import { applyMacTitlebarClass } from "./mac-titlebar";
import "./styles.css";
// Cursor 风格层必须最后加载，压过前面的组件样式。
import "./cursor-ui.css";
import "./cx-pages-gateway.css";
import "./cx-pages-resources.css";
import "./cx-pages-agents.css";
import "./cx-pages-settings.css";
import "./cx-tray.css";

/* ── Bundled fonts (offline, no Google Fonts request) ──
     Fontsource packages ship woff2 files that Vite bundles into dist/.
     CSS @font-face declarations are injected at build time.
     界面字体已对齐 DSH：改走系统字体栈（--font-sans），不再打包 Inter；
     这里只保留 DSH 代码字体栈里排第二的 JetBrains Mono 作等宽回退。      */
import "@fontsource/jetbrains-mono";

/**
 * 启动兜底：渲染期抛错时 React 会卸掉整棵树，窗口只剩深色底，看起来就是「黑屏」（#2753）。
 * 这里接住错误，显示原因和重新加载按钮。样式全部内联，不依赖 styles.css 是否生效。
 */
class StartupErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: unknown) {
    return { error: error instanceof Error ? error : new Error(String(error)) };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error("[Codex++] 界面渲染失败", error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div role="alert" style={{ boxSizing: "border-box", minHeight: "100vh", padding: "32px", background: "#111318", color: "#e8e9ed", font: "14px/1.6 -apple-system, BlinkMacSystemFont, sans-serif" }}>
        <h1 style={{ margin: "0 0 8px", fontSize: "18px" }}>Codex++ 界面加载失败 / Codex++ failed to load</h1>
        <p style={{ margin: "0 0 12px", color: "#a9adb8" }}>
          可以先重新加载；如果仍然失败，请把 macOS 更新到最新版本，并把下面的错误信息附到 GitHub issue 里。
          <br />
          Try reloading. If it keeps failing, update macOS and attach the error below to a GitHub issue.
        </p>
        <pre style={{ margin: "0 0 16px", padding: "12px", maxHeight: "40vh", overflow: "auto", whiteSpace: "pre-wrap", wordBreak: "break-all", borderRadius: "8px", background: "#1b1e25", color: "#f2b8b5", font: "12px/1.5 monospace" }}>
          {error.stack || error.message || String(error)}
        </pre>
        <button onClick={() => window.location.reload()} style={{ padding: "6px 14px", border: "1px solid #3a3f4b", borderRadius: "6px", background: "#2a2f3a", color: "#e8e9ed", cursor: "pointer" }} type="button">
          重新加载 / Reload
        </button>
      </div>
    );
  }
}

const app = document.getElementById("app");

if (app instanceof HTMLElement) {
  // 只有主窗口去掉了系统标题栏；托盘面板、悬浮预览本来就没有标题栏。
  if (window.location.hash !== "#tray-panel" && window.location.hash !== "#cu-pip") applyMacTitlebarClass();
  // 同一个前端也用于 Computer Use 的悬浮预览窗口（`#cu-pip`）和托盘快捷面板（`#tray-panel`）。
  createRoot(app).render(
    <StartupErrorBoundary>
      {window.location.hash === "#tray-panel" ? <TrayPanel /> : window.location.hash === "#cu-pip" ? <CuPipWindow /> : <App />}
    </StartupErrorBoundary>,
  );
}
