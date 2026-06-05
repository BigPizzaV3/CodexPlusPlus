import { Suspense } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./i18n";
import "./styles.css";

const app = document.getElementById("app");

function Fallback() {
  return <div className="shell">Loading...</div>;
}

if (app instanceof HTMLElement) {
  createRoot(app).render(
    <Suspense fallback={<Fallback />}>
      <App />
    </Suspense>,
  );
}
