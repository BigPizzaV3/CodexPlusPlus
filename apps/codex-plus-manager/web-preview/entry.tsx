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
    createRoot(app).render(<TrayPanel />);
    return;
  }
  const { App } = await import("../src/App");
  createRoot(app).render(<App />);
}

void start();
