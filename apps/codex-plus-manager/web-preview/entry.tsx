import { createRoot } from "react-dom/client";
import "./tailwind-source.css";
import "@fontsource/jetbrains-mono";

const previewLanguage = new URLSearchParams(window.location.search).get("lang");
if (previewLanguage === "en" || previewLanguage === "zh") {
  window.localStorage.setItem("codex-plus-lang", previewLanguage);
}

async function start() {
  const app = document.getElementById("app");
  if (!(app instanceof HTMLElement)) return;
  if (window.location.hash === "#flow-demo") {
    const { FlowDemo } = await import("./FlowDemo");
    createRoot(app).render(<FlowDemo />);
    return;
  }
  if (new URLSearchParams(window.location.search).has("flowPreview")) {
    window.localStorage.setItem("codex-plus-tray-tab", "flow");
  }
  // 和正式版 main.tsx 一样，`#tray-panel` 打开托盘快捷面板。
  if (window.location.hash === "#tray-panel") {
    const { TrayPanel } = await import("../src/TrayPanel");
    // 和正式版一样加载 Cursor 风格层（main.tsx 里是全局导入，托盘窗口也用它）。
    await import("../src/cursor-ui.css");
    await import("../src/cx-tray.css");
    createRoot(app).render(<TrayPanel />);
    return;
  }
  const { App } = await import("../src/App");
  // 和正式版一样，Cursor 风格层在组件样式之后加载。
  await import("../src/cursor-ui.css");
  await Promise.all([import("../src/cx-pages-gateway.css"), import("../src/cx-pages-resources.css"), import("../src/cx-pages-agents.css"), import("../src/cx-pages-settings.css")]);
  // 和正式版 main.tsx 一样判断是否去掉系统标题栏；预览里用 ?macTitlebar=1 模拟。
  const { applyMacTitlebarClass } = await import("../src/mac-titlebar");
  applyMacTitlebarClass();
  createRoot(app).render(<App />);
}

void start();
