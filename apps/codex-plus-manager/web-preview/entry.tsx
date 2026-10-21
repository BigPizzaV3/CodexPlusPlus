import { createRoot } from "react-dom/client";
import "./tailwind-source.css";
import "../src/styles.css";
import "@fontsource/jetbrains-mono";

const previewLanguage = new URLSearchParams(window.location.search).get("lang");
if (previewLanguage === "en" || previewLanguage === "zh") {
  window.localStorage.setItem("codex-plus-lang", previewLanguage);
}

async function start() {
  const { App } = await import("../src/App");
  const app = document.getElementById("app");
  if (app instanceof HTMLElement) {
    createRoot(app).render(<App />);
  }
}

void start();
