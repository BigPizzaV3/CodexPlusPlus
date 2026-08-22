(() => {
  "use strict";

  const API_KEY = "__codexFloatingPanelVisual";
  const PANEL_KEY = "__codexFloatingPanel";
  const MATERIAL_KEY = "codex-floating-panel-material-v3";
  const ROOT_ATTR = "data-codex-floating-panel-root";
  const STYLE_ID = "codex-floating-panel-visual-style";
  const FILTER_ID = "codex-floating-panel-noise-filter";
  const MATERIALS = ["frosted", "clear", "liquid", "crystal", "matte"];
  const instanceId = `visual-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  let observer = null;
  let themeObserver = null;
  let destroyed = false;
  let applyFrame = 0;

  function root() {
    return document.querySelector(`[${ROOT_ATTR}="true"]`);
  }

  function normalizeMaterial(value) {
    return MATERIALS.includes(value) ? value : "frosted";
  }

  function material() {
    return normalizeMaterial(localStorage.getItem(MATERIAL_KEY));
  }

  function setMaterial(value) {
    const next = normalizeMaterial(value);
    try {
      localStorage.setItem(MATERIAL_KEY, next);
    } catch {}
    apply();
    return next;
  }

  function cycle() {
    const current = material();
    const next = MATERIALS[(MATERIALS.indexOf(current) + 1) % MATERIALS.length];
    return setMaterial(next);
  }

  function hostTypography() {
    const candidate = document.querySelector("[data-codex-intelligence-trigger], textarea, [contenteditable='true'], body");
    if (!candidate) return null;
    const style = getComputedStyle(candidate);
    return {
      family: style.fontFamily || "-apple-system, system-ui, sans-serif",
      size: style.fontSize || "13px",
      weight: style.fontWeight || "400",
    };
  }

  function installFilters() {
    if (document.getElementById(FILTER_ID)) return;
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.id = FILTER_ID;
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("width", "0");
    svg.setAttribute("height", "0");
    svg.style.position = "absolute";
    svg.innerHTML = `<defs>
      <filter id="cfp-noise-filter" x="-10%" y="-10%" width="120%" height="120%">
        <feTurbulence type="fractalNoise" baseFrequency=".8" numOctaves="2" seed="17" result="noise" />
        <feColorMatrix in="noise" type="saturate" values="0" result="mono" />
        <feComponentTransfer in="mono"><feFuncA type="table" tableValues="0 .12" /></feComponentTransfer>
      </filter>
    </defs>`;
    document.body?.appendChild(svg);
  }

  function installStyle() {
    if (document.getElementById(STYLE_ID)) return;
    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      [${ROOT_ATTR}="true"] {
        --cfp-font-family: -apple-system, system-ui, "Segoe UI", sans-serif;
        --cfp-font-size: 13px;
        --cfp-font-weight: 400;
        --cfp-material-tint: transparent;
        --cfp-material-opacity: .92;
        --cfp-material-blur: 24px;
        --cfp-material-saturation: 1.08;
        --cfp-material-noise: 0;
        font-family: var(--cfp-font-family);
        font-size: var(--cfp-font-size);
        font-weight: var(--cfp-font-weight);
      }
      [${ROOT_ATTR}="true"] .cfp-shell {
        background-color: color-mix(in srgb, var(--cfp-bg) calc(var(--cfp-material-opacity) * 100%), transparent);
        background-image: linear-gradient(135deg, var(--cfp-material-tint), transparent 62%);
        border-color: color-mix(in srgb, var(--cfp-border) 84%, var(--cfp-material-tint));
        box-shadow: none;
        -webkit-backdrop-filter: blur(var(--cfp-material-blur)) saturate(var(--cfp-material-saturation));
        backdrop-filter: blur(var(--cfp-material-blur)) saturate(var(--cfp-material-saturation));
        isolation: isolate;
        overflow: hidden;
      }
      [${ROOT_ATTR}="true"] .cfp-shell::before {
        background: rgba(255,255,255,var(--cfp-material-noise));
        content: "";
        inset: 0;
        opacity: .55;
        pointer-events: none;
        position: absolute;
        filter: url(#cfp-noise-filter);
      }
      [${ROOT_ATTR}="true"] .cfp-shell > * { position: relative; z-index: 1; }
      [${ROOT_ATTR}="true"][data-material="frosted"] { --cfp-material-tint: rgba(255,255,255,.18); --cfp-material-opacity: .88; --cfp-material-blur: 20px; --cfp-material-noise: .06; }
      [${ROOT_ATTR}="true"][data-material="clear"] { --cfp-material-tint: rgba(255,255,255,.08); --cfp-material-opacity: .34; --cfp-material-blur: 10px; --cfp-material-saturation: 1.2; --cfp-material-noise: .04; }
      [${ROOT_ATTR}="true"][data-material="liquid"] { --cfp-material-tint: rgba(111,169,255,.2); --cfp-material-opacity: .54; --cfp-material-blur: 16px; --cfp-material-saturation: 1.25; --cfp-material-noise: .09; }
      [${ROOT_ATTR}="true"][data-material="crystal"] { --cfp-material-tint: rgba(206,229,255,.24); --cfp-material-opacity: .62; --cfp-material-blur: 12px; --cfp-material-saturation: 1.3; --cfp-material-noise: .08; }
      [${ROOT_ATTR}="true"][data-material="matte"] { --cfp-material-tint: rgba(80,87,101,.12); --cfp-material-opacity: .82; --cfp-material-blur: 4px; --cfp-material-saturation: .92; --cfp-material-noise: .025; }
      [${ROOT_ATTR}="true"] .cfp-status-face[data-pending="true"],
      [${ROOT_ATTR}="true"] .cfp-status-face[data-pending="true"] ~ .cfp-capsule-copy .cfp-capsule-label {
        background: linear-gradient(90deg, currentColor 35%, var(--cfp-accent) 50%, currentColor 65%);
        background-clip: text;
        background-size: 220% 100%;
        color: transparent;
        -webkit-background-clip: text;
        animation: cfp-shimmer 1.5s linear infinite;
      }
      @keyframes cfp-shimmer { from { background-position: 180% 0; } to { background-position: -20% 0; } }
      @media (prefers-reduced-motion: reduce) {
        [${ROOT_ATTR}="true"] .cfp-status-face[data-pending="true"],
        [${ROOT_ATTR}="true"] .cfp-status-face[data-pending="true"] ~ .cfp-capsule-copy .cfp-capsule-label { animation: none; color: var(--cfp-muted); }
      }
    `;
    document.documentElement.appendChild(style);
  }

  function apply() {
    if (destroyed) return;
    const panelRoot = root();
    if (!panelRoot) return;
    panelRoot.dataset.material = material();
    const typography = hostTypography();
    if (typography) {
      panelRoot.style.setProperty("--cfp-font-family", typography.family);
      panelRoot.style.setProperty("--cfp-font-size", typography.size);
      panelRoot.style.setProperty("--cfp-font-weight", typography.weight);
    }
    const pending = String(Boolean(panelRoot.querySelector(".cfp-capsule-label")?.textContent?.includes("正在")));
    const statusFace = panelRoot.querySelector(".cfp-status-face");
    if (statusFace?.dataset.pending !== pending) statusFace?.setAttribute("data-pending", pending);
    if (panelRoot.dataset.visualLayer !== "true") panelRoot.dataset.visualLayer = "true";
  }

  function scheduleApply() {
    if (applyFrame || destroyed) return;
    applyFrame = window.requestAnimationFrame(() => {
      applyFrame = 0;
      apply();
    });
  }

  function destroy() {
    destroyed = true;
    if (applyFrame) window.cancelAnimationFrame(applyFrame);
    observer?.disconnect();
    themeObserver?.disconnect();
    document.getElementById(STYLE_ID)?.remove();
    document.getElementById(FILTER_ID)?.remove();
    if (window[API_KEY]?.instanceId === instanceId) delete window[API_KEY];
  }

  function start() {
    if (destroyed) return;
    installFilters();
    installStyle();
    apply();
    observer = new MutationObserver(scheduleApply);
    observer.observe(document.body, { childList: true, subtree: true, characterData: true });
    themeObserver = new MutationObserver(scheduleApply);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style"] });
  }

  window[API_KEY] = { version: "2.0.0", instanceId, destroy, apply, current: () => ({ material: material() }), cycle, setMaterial };
  if (document.body) start();
  else document.addEventListener("DOMContentLoaded", start, { once: true });
})();
