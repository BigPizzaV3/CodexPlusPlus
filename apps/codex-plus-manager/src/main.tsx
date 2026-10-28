import { createRoot } from "react-dom/client";
import { App } from "./App";
import { CuPipWindow } from "./CuPipWindow";
import { TrayPanel } from "./TrayPanel";
import "./styles.css";

/* ── Bundled fonts (offline, no Google Fonts request) ──
     Fontsource packages ship woff2 files that Vite bundles into dist/.
     CSS @font-face declarations are injected at build time.
     界面字体已对齐 DSH：改走系统字体栈（--font-sans），不再打包 Inter；
     这里只保留 DSH 代码字体栈里排第二的 JetBrains Mono 作等宽回退。      */
import "@fontsource/jetbrains-mono";

const app = document.getElementById("app");

if (app instanceof HTMLElement) {
  // 同一个前端也用于 Computer Use 的悬浮预览窗口（`#cu-pip`）和托盘快捷面板（`#tray-panel`）。
  createRoot(app).render(window.location.hash === "#tray-panel" ? <TrayPanel /> : window.location.hash === "#cu-pip" ? <CuPipWindow /> : <App />);
}
