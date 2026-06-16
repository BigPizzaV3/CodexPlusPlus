import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";
import i18n from "./i18n";
import { I18nextProvider } from "react-i18next";

/* ── Bundled fonts (offline, no Google Fonts request) ──
     Fontsource packages ship woff2 files that Vite bundles into dist/.
     CSS @font-face declarations are injected at build time.              */
import "@fontsource/inter";
import "@fontsource/jetbrains-mono";

const app = document.getElementById("app");

if (app instanceof HTMLElement) {
  createRoot(app).render(
    <I18nextProvider i18n={i18n}>
      <App />
    </I18nextProvider>
  );
}
