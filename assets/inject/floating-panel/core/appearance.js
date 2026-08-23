/* Floating-panel appearance: shell CSS, materials, typography, and motion rules. */

  // One stylesheet owns the shell, materials, views, responsive layout, and reduced-motion states.
  function installStyle() {
    const existing = document.getElementById(STYLE_ID);
    if (existing?.dataset.codexStepwiseStyleVersion === SCRIPT_VERSION) return;
    existing?.remove();

    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.dataset.codexStepwiseStyleVersion = SCRIPT_VERSION;
    style.textContent = `
      [${ROOT_ATTR}="true"] {
        --csw-surface-opaque: var(--color-background-elevated-primary-opaque, var(--color-token-dropdown-background, var(--main-surface-primary, #FAFAFA)));
        --csw-text: var(--color-token-text-primary, var(--color-token-foreground, var(--text-primary, #202020)));
        --csw-muted: var(--color-token-text-tertiary, var(--color-token-description-foreground, #6F6F6F));
        --csw-faint: color-mix(in srgb, var(--csw-text) 34%, transparent);
        --csw-accent: var(--color-token-charts-blue, #4D8DFF);
        --csw-danger: #dc5d67;
        --csw-ready: var(--csw-accent);
        --csw-hover: var(--color-token-list-hover-background, color-mix(in srgb, var(--csw-text) 6%, transparent));
        --csw-divider: color-mix(in srgb, var(--csw-text) 9%, transparent);
        --csw-glass-x: 28%;
        --csw-glass-y: 22%;
        --csw-glass-strength: 0;
        --csw-glass-rim-width: 140%;
        --csw-glass-rim-height: 120%;
        --csw-glass-px: 0px;
        --csw-glass-py: 0px;
        --csw-glass-angle: -40deg;
        --csw-glass-edge: rgba(108, 128, 152, 0.4);
        --csw-glass-edge-hi: rgba(168, 190, 214, 0.7);
        --csw-hover-core: 0.13;
        --csw-hover-mid: 0.04;
        --csw-hover-layer-opacity: 0.85;
        --csw-hover-rim-gain: 10%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 190, 210, 230;
        --csw-frost-noise: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='180' height='180' viewBox='0 0 180 180'%3E%3Cfilter id='n' color-interpolation-filters='sRGB'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.008' numOctaves='2' seed='92' stitchTiles='stitch'/%3E%3CfeGaussianBlur stdDeviation='2'/%3E%3CfeComponentTransfer%3E%3CfeFuncA type='table' tableValues='0 .055'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)' opacity='.55'/%3E%3C/svg%3E");
        --csw-eye-x: 0px;
        --csw-eye-y: 0px;
        --csw-curious-eye-x: 0px;
        --csw-curious-eye-y: 0px;
        --csw-panel-width: ${PANEL_WIDTH}px;
        --csw-panel-height: ${PANEL_HEIGHT}px;
        --csw-item-font: ${DEFAULT_FONT}px;
        --csw-chrome-font: 12px;
        --csw-icon-font: 16px;
        color: var(--csw-text);
        font-family: var(--csw-font-family, -apple-system, system-ui, "Segoe UI", sans-serif);
        font-size: var(--csw-item-font);
        font-weight: var(--csw-font-weight, 400);
        line-height: 1.4;
        inset: 0;
        letter-spacing: 0;
        pointer-events: none;
        position: fixed;
        z-index: 2147483000;
      }

      [${ROOT_ATTR}="true"][data-hidden="true"] {
        display: none !important;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] {
        --csw-surface-opaque: var(--color-background-elevated-primary-opaque, var(--color-token-dropdown-background, #2B2B2B));
        --csw-text: var(--color-token-text-primary, var(--color-token-foreground, #F3F3F3));
        --csw-muted: var(--color-token-text-tertiary, var(--color-token-description-foreground, #AAAAAA));
        --csw-faint: color-mix(in srgb, var(--csw-text) 32%, transparent);
        --csw-accent: var(--color-token-charts-blue, #4D8DFF);
        --csw-danger: #ff7f89;
        --csw-ready: var(--csw-accent);
        --csw-hover: var(--color-token-list-hover-background, rgba(255, 255, 255, 0.078));
        --csw-divider: rgba(255, 255, 255, 0.09);
        --csw-glass-strength: 0;
        --csw-glass-edge: rgba(132, 154, 180, 0.36);
        --csw-glass-edge-hi: rgba(178, 200, 224, 0.62);
        color: var(--csw-text);
      }

      [${PAYLOAD_ATTR}="true"],
      [${PAYLOAD_ATTR}="block"] {
        display: none !important;
      }

      .csw-popover {
        height: var(--csw-panel-height);
        isolation: isolate;
        pointer-events: none;
        position: fixed;
        width: var(--csw-panel-width);
      }

      .csw-material-layer {
        inset: 0;
        isolation: isolate;
        pointer-events: none;
        position: absolute;
        z-index: 0;
      }

      .csw-material-layer,
      .csw-material-layer * {
        pointer-events: none;
      }

      .csw-glass {
        -webkit-backdrop-filter: blur(18px) saturate(165%) contrast(1.04) brightness(1.04);
        backdrop-filter: blur(18px) saturate(165%) contrast(1.04) brightness(1.04);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 68%, transparent);
        background-image: linear-gradient(160deg, rgba(255, 255, 255, 0.11) 0%, rgba(255, 255, 255, 0.032) 48%, rgba(150, 170, 195, 0.032) 100%);
        border: 0;
        border-radius: ${CHIP_RADIUS}px;
        box-shadow: none;
        box-sizing: border-box;
        height: ${CHIP_HEIGHT}px;
        left: var(--csw-chip-left, ${Math.max(0, (PANEL_WIDTH - CHIP_WIDTH) / 2)}px);
        overflow: hidden;
        pointer-events: none;
        position: absolute;
        top: 0;
        transition: background-color 0.2s ease, box-shadow 0.2s ease, backdrop-filter 0.2s ease, -webkit-backdrop-filter 0.2s ease;
        width: ${CHIP_WIDTH}px;
        z-index: 1;
      }

      .csw-rim {
        background: var(--csw-glass-edge);
        border-radius: ${CHIP_RADIUS}px;
        box-sizing: border-box;
        height: ${CHIP_HEIGHT}px;
        left: var(--csw-chip-left, ${Math.max(0, (PANEL_WIDTH - CHIP_WIDTH) / 2)}px);
        -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        -webkit-mask-composite: xor;
        mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        mask-composite: exclude;
        padding: 1px;
        pointer-events: none;
        position: absolute;
        top: 0;
        width: ${CHIP_WIDTH}px;
        z-index: 4;
      }

      .csw-popover[data-csw-hot-hover] .csw-rim {
        background: conic-gradient(
          from calc(var(--csw-glass-angle, -40deg) + 90deg),
          color-mix(in srgb, var(--csw-glass-edge-hi) var(--csw-hover-rim-gain, 0%), var(--csw-glass-edge)) 0deg,
          var(--csw-glass-edge) 64deg,
          var(--csw-glass-edge) 296deg,
          color-mix(in srgb, var(--csw-glass-edge-hi) var(--csw-hover-rim-gain, 0%), var(--csw-glass-edge)) 360deg
        );
      }

      .csw-popover[data-morphing="true"] .csw-glass,
      .csw-popover[data-morphing="true"] .csw-rim,
      .csw-popover[data-morphing="true"] .csw-displacement-texture {
        will-change: left, top, width, height, border-radius;
      }

      .csw-popover[data-snap-right="true"] {
        transition: left 180ms cubic-bezier(.22, .72, 0, 1), top 180ms cubic-bezier(.22, .72, 0, 1);
      }

      .csw-popover[data-snap-right="true"] .csw-fab,
      .csw-popover[data-snap-right="true"] .csw-glass,
      .csw-popover[data-snap-right="true"] .csw-rim,
      .csw-popover[data-snap-right="true"] .csw-displacement-texture {
        transition-property: left, top;
        transition-duration: 180ms;
        transition-timing-function: cubic-bezier(.22, .72, 0, 1);
      }

      .csw-completion-beam {
        box-sizing: border-box;
        color: var(--csw-text);
        -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        -webkit-mask-composite: xor;
        mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        mask-composite: exclude;
        opacity: 0;
        overflow: hidden;
        padding: 1px;
        pointer-events: none;
        position: absolute;
        z-index: 5;
      }

      .csw-completion-beam::before {
        background: conic-gradient(
          from 0deg,
          transparent 0deg,
          transparent 302deg,
          color-mix(in srgb, currentColor 22%, transparent) 320deg,
          color-mix(in srgb, currentColor 72%, transparent) 338deg,
          transparent 360deg
        );
        content: "";
        inset: -170%;
        opacity: 0;
        position: absolute;
        transform: rotate(-64deg);
        transform-origin: center;
        will-change: opacity, transform;
      }

      .csw-popover[data-morphing="false"][data-completion-beam="true"] .csw-completion-beam {
        opacity: 1;
      }

      .csw-popover[data-morphing="false"][data-completion-beam="true"] .csw-completion-beam::before {
        animation: csw-completion-beam-sweep ${COMPLETION_BEAM_MS}ms cubic-bezier(.22, .78, .18, 1) 1 both;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-glass {
        -webkit-backdrop-filter: blur(18px) saturate(165%) contrast(1.05) brightness(1.08);
        backdrop-filter: blur(18px) saturate(165%) contrast(1.05) brightness(1.08);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 60%, transparent);
        background-image: linear-gradient(160deg, rgba(160, 185, 215, 0.055) 0%, rgba(255, 255, 255, 0.012) 48%, rgba(30, 45, 70, 0.045) 100%);
      }

      .csw-popover[data-material="matte"] {
        --csw-hover-core: 0.07;
        --csw-hover-mid: 0.022;
        --csw-hover-layer-opacity: 0.75;
        --csw-hover-rim-gain: 6%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 205, 215, 225;
      }

      .csw-popover[data-material="frosted"] {
        --csw-hover-core: 0.13;
        --csw-hover-mid: 0.04;
        --csw-hover-layer-opacity: 0.85;
        --csw-hover-rim-gain: 10%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 190, 210, 230;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="frosted"] {
        --csw-hover-core: 0.11;
        --csw-hover-mid: 0.034;
      }

      .csw-popover[data-material="clear"] {
        --csw-hover-core: 0.12;
        --csw-hover-mid: 0.038;
        --csw-hover-layer-opacity: 0.65;
        --csw-hover-rim-gain: 8%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 174, 214, 255;
      }

      .csw-popover[data-material="liquid"] {
        --csw-hover-core: 0.15;
        --csw-hover-mid: 0.045;
        --csw-hover-layer-opacity: 0.72;
        --csw-hover-rim-gain: 9%;
        --csw-hover-core-color: 255, 255, 255;
        --csw-hover-mid-color: 145, 205, 255;
      }

      .csw-popover[data-material="crystal"] {
        --csw-hover-core: 0.14;
        --csw-hover-mid: 0.045;
        --csw-hover-layer-opacity: 0.68;
        --csw-hover-rim-gain: 9%;
        --csw-hover-core-color: 242, 252, 255;
        --csw-hover-mid-color: 122, 199, 255;
      }

      .csw-popover[data-material="frosted"] .csw-glass {
        -webkit-backdrop-filter: blur(15px) saturate(124%) contrast(1.02);
        backdrop-filter: blur(15px) saturate(124%) contrast(1.02);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 18%, transparent);
        background-image: var(--csw-frost-noise);
        background-blend-mode: soft-light;
        background-repeat: no-repeat;
        background-size: cover;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="frosted"] .csw-glass {
        -webkit-backdrop-filter: blur(15px) saturate(118%) contrast(1.03) brightness(1.03);
        backdrop-filter: blur(15px) saturate(118%) contrast(1.03) brightness(1.03);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 24%, transparent);
        background-image: var(--csw-frost-noise);
        background-blend-mode: soft-light;
      }

      .csw-popover[data-open="true"] {
        --csw-glass-rim-width: 92%;
        --csw-glass-rim-height: 72%;
      }

      .csw-popover[data-open="false"][data-material="frosted"] .csw-glass {
        -webkit-backdrop-filter: blur(15px) saturate(124%) contrast(1.02);
        backdrop-filter: blur(15px) saturate(124%) contrast(1.02);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 12%, transparent);
        background-image: var(--csw-frost-noise);
        background-blend-mode: soft-light;
        background-repeat: no-repeat;
        background-size: cover;
        box-shadow: none;
        filter: none !important;
        isolation: auto;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-open="false"][data-material="frosted"] .csw-glass {
        -webkit-backdrop-filter: blur(15px) saturate(118%) contrast(1.03) brightness(1.03);
        backdrop-filter: blur(15px) saturate(118%) contrast(1.03) brightness(1.03);
        background-color: color-mix(in srgb, var(--csw-surface-opaque) 18%, transparent);
        background-image: var(--csw-frost-noise);
        background-blend-mode: soft-light;
        box-shadow: none;
      }

      .csw-popover[data-material="matte"] .csw-glass {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background-color: var(--csw-surface-opaque);
        background-image: none;
      }

      .csw-popover[data-material="clear"] .csw-glass {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background-color: transparent;
        background-image: none;
        isolation: isolate;
      }

      .csw-clear-texture {
        -webkit-backdrop-filter: none;
        backdrop-filter: url(#${CLEAR_FILTER_ID});
        background-color: transparent;
        background-image: none;
        border-radius: inherit;
        display: none;
        filter: none;
        inset: -12px;
        opacity: 1;
        pointer-events: none;
        position: absolute;
        transform: translateZ(0);
        will-change: backdrop-filter;
        z-index: 0;
      }

      .csw-popover[data-material="clear"] .csw-clear-texture {
        display: block;
      }

      .csw-clear-distortion {
        -webkit-backdrop-filter: none;
        -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        -webkit-mask-composite: xor;
        backdrop-filter: none;
        background: transparent;
        border-radius: inherit;
        box-sizing: border-box;
        display: none;
        filter: none;
        inset: 1px;
        mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
        mask-composite: exclude;
        opacity: 1;
        padding: 1px;
        pointer-events: none;
        position: absolute;
        transform: translate3d(var(--csw-glass-px, 0px), var(--csw-glass-py, 0px), 0);
        transition: opacity 0.18s ease, transform 0.14s cubic-bezier(0.23, 1, 0.32, 1);
        will-change: transform;
        z-index: 0;
      }

      .csw-popover[data-material="clear"] .csw-clear-distortion {
        display: block;
      }

      .csw-popover[data-material="liquid"] .csw-glass,
      .csw-popover[data-material="crystal"] .csw-glass {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background-color: rgba(255, 255, 255, 0);
        background-image: none;
        isolation: isolate;
      }

      .csw-popover[data-material="liquid"] .csw-glass::before,
      .csw-popover[data-material="crystal"] .csw-glass::before {
        box-shadow: none;
        mix-blend-mode: screen;
        transform: translate3d(var(--csw-glass-px, 0px), var(--csw-glass-py, 0px), 0);
        z-index: 1;
      }

      .csw-displacement-texture {
        border-radius: ${CHIP_RADIUS}px;
        display: none;
        height: ${CHIP_HEIGHT}px;
        isolation: isolate;
        left: var(--csw-chip-left, ${Math.max(0, (PANEL_WIDTH - CHIP_WIDTH) / 2)}px);
        overflow: hidden;
        pointer-events: none;
        position: absolute;
        top: 0;
        width: ${CHIP_WIDTH}px;
        z-index: 0;
      }

      .csw-displacement-texture::before {
        border-radius: inherit;
        content: "";
        inset: 0;
        pointer-events: none;
        position: absolute;
      }

      .csw-popover[data-material="liquid"] .csw-displacement-texture,
      .csw-popover[data-material="crystal"] .csw-displacement-texture {
        display: block;
      }

      .csw-popover[data-material="crystal"] .csw-displacement-texture {
        overflow: visible;
      }

      .csw-popover[data-material="liquid"] .csw-displacement-texture::before {
        -webkit-backdrop-filter: url(#${LIQUID_FILTER_ID}) blur(0.6px) saturate(112%) contrast(1.02);
        backdrop-filter: url(#${LIQUID_FILTER_ID}) blur(0.6px) saturate(112%) contrast(1.02);
        background-color: rgba(255, 255, 255, 0.24);
        -webkit-filter: none;
        filter: none;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="liquid"] .csw-displacement-texture::before {
        background-color: rgba(20, 24, 30, 0.34);
      }

      .csw-popover[data-material="crystal"] .csw-displacement-texture::before {
        -webkit-backdrop-filter: blur(7px);
        backdrop-filter: blur(7px);
        background-color: rgba(255, 255, 255, 0);
        border-radius: 0;
        clip-path: inset(48px round ${PANEL_RADIUS}px);
        inset: -48px;
        -webkit-filter: url(#${CRYSTAL_FILTER_ID});
        filter: url(#${CRYSTAL_FILTER_ID});
      }

      .csw-popover[data-material-animating="true"] .csw-glass {
        transition-duration: 260ms;
      }

      .csw-glass::before {
        background: radial-gradient(
          var(--csw-glass-rim-width, 140%) var(--csw-glass-rim-height, 120%) at var(--csw-glass-x, 28%) var(--csw-glass-y, 22%),
          rgba(var(--csw-hover-core-color, 255, 255, 255), calc(var(--csw-hover-core, 0.13) * var(--csw-glass-strength, 0))) 0%,
          rgba(var(--csw-hover-mid-color, 190, 210, 230), calc(var(--csw-hover-mid, 0.04) * var(--csw-glass-strength, 0))) 22%,
          transparent 50%
        );
        border-radius: inherit;
        content: "";
        inset: 0;
        mix-blend-mode: screen;
        opacity: var(--csw-hover-layer-opacity, 0.85);
        pointer-events: none;
        position: absolute;
        transform: translate3d(var(--csw-glass-px, 0px), var(--csw-glass-py, 0px), 0);
        transition: transform 0.14s cubic-bezier(0.23, 1, 0.32, 1);
        will-change: transform;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-glass {
        box-shadow: none;
      }

      .csw-popover[data-morphing="true"] .csw-glass {
        cursor: pointer;
        pointer-events: auto;
        transition: none;
      }

      .csw-popover[data-resizing="true"],
      .csw-popover[data-resizing="true"] .csw-glass,
      .csw-popover[data-resizing="true"] .csw-rim,
      .csw-popover[data-resizing="true"] .csw-panel {
        transition: none !important;
      }

      .csw-fab {
        align-items: center;
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 999px;
        box-sizing: border-box;
        color: var(--csw-text);
        cursor: grab;
        display: flex;
        height: ${CHIP_HEIGHT}px;
        justify-content: center;
        padding: 0;
        pointer-events: auto;
        position: absolute;
        user-select: none;
        width: ${CHIP_WIDTH}px;
        z-index: 3;
      }

      .csw-fab[data-expression="hidden"] {
        display: none;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-fab {
        opacity: 0;
        pointer-events: none;
        visibility: hidden;
      }

      .csw-popover[data-morphing="true"] .csw-fab {
        opacity: 1;
        pointer-events: none;
        transform: none;
        visibility: visible;
      }

      .csw-fab:active {
        cursor: grabbing;
        transform: scale(0.96);
      }

      .csw-fab:focus-visible {
        outline: 2px solid color-mix(in srgb, var(--csw-accent) 76%, transparent);
        outline-offset: 4px;
      }

      .csw-fab-face {
        align-items: center;
        display: flex;
        gap: 16px;
        height: 27px;
        justify-content: center;
        position: relative;
        width: 52px;
        z-index: 1;
      }

      .csw-status-stage {
        align-items: center;
        display: flex;
        height: 28px;
        justify-content: center;
        position: relative;
        width: 58px;
        z-index: 1;
      }

      .csw-source-track {
        height: var(--csw-source-track-height, ${CHIP_HEIGHT}px);
        left: 50%;
        pointer-events: none;
        position: absolute;
        top: 50%;
        transform: translate(-50%, -50%);
        width: ${CHIP_WIDTH}px;
        z-index: 2;
      }

      .csw-fab-eye {
        background: currentColor;
        border-radius: 999px;
        display: block;
        height: 14px;
        position: relative;
        transform: translate3d(var(--csw-eye-x, 0px), var(--csw-eye-y, 0px), 0);
        transform-origin: center;
        transition: background 170ms ease, border-color 170ms ease, height 170ms ease, transform 170ms ease, width 170ms ease;
        will-change: transform;
        width: 8px;
      }

      .csw-fab-happy-arc {
        display: none;
        height: 100%;
        overflow: visible;
        width: 100%;
      }

      .csw-fab-happy-arc path {
        fill: none;
        stroke: currentColor;
        stroke-linecap: round;
        stroke-linejoin: round;
        stroke-width: 2.6;
        vector-effect: non-scaling-stroke;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="idle"] .csw-fab-eye {
        animation: csw-face-blink 4.8s infinite;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="answering"] .csw-fab-eye {
        height: 13px;
        transition-duration: 70ms;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="surprise"] .csw-fab-eye {
        animation: csw-face-star 1.25s ease-in-out infinite;
        background: currentColor;
        border: 0;
        border-radius: 0;
        clip-path: polygon(50% 0, 61% 36%, 100% 50%, 61% 64%, 50% 100%, 39% 64%, 0 50%, 39% 36%);
        height: 18px;
        width: 18px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="generating"] .csw-fab-eye {
        animation: csw-face-generate-bob .92s cubic-bezier(.45, 0, .2, 1) infinite;
        animation-delay: 0s;
        height: 14px;
        width: 8px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="ready"] .csw-fab-eye {
        animation: csw-face-happy-lift 1.8s ease-in-out infinite;
        background: transparent;
        border: 0;
        border-radius: 0;
        height: 12px;
        width: 18px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="ready"] .csw-fab-happy-arc {
        display: block;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="ready"] .csw-fab-eye::before,
      :is(.csw-fab, .csw-head-face)[data-expression="ready"] .csw-fab-eye::after {
        content: none;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="empty"] .csw-fab-eye {
        animation: csw-face-calm-breathe 3.6s ease-in-out infinite;
        height: 3px;
        width: 16px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye {
        animation: csw-face-error-breathe 3.8s ease-in-out infinite;
        background: transparent;
        color: var(--csw-text);
        height: 14px;
        width: 14px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye::before,
      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye::after {
        background: currentColor;
        border-radius: 999px;
        content: "";
        height: 2.5px;
        left: 0;
        position: absolute;
        top: 5.75px;
        width: 14px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye::before {
        transform: rotate(45deg);
      }

      :is(.csw-fab, .csw-head-face)[data-expression="error"] .csw-fab-eye::after {
        transform: rotate(-45deg);
      }

      :is(.csw-fab, .csw-head-face)[data-expression="curious"] .csw-fab-eye {
        animation: none;
        background: transparent;
        border: 3px solid currentColor;
        border-radius: 50%;
        clip-path: none;
        height: 17px;
        width: 17px;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="curious"] .csw-fab-eye::before {
        content: none;
      }

      :is(.csw-fab, .csw-head-face)[data-expression="curious"] .csw-fab-eye::after {
        animation: none;
        background: currentColor;
        border-radius: 50%;
        content: "";
        height: 5px;
        left: 50%;
        position: absolute;
        top: 50%;
        transform: translate(-50%, -50%) translate3d(var(--csw-curious-eye-x, 0px), var(--csw-curious-eye-y, 0px), 0);
        transition: transform 90ms cubic-bezier(.2, .8, .2, 1);
        width: 5px;
      }

      .csw-fab-badge {
        display: none;
      }

      .csw-fab[data-count="0"] .csw-fab-badge {
        display: none;
      }

      .csw-fab:not([data-expression="ready"]) .csw-fab-badge {
        display: none;
      }

      .csw-panel {
        -webkit-backdrop-filter: none !important;
        -webkit-filter: none !important;
        backdrop-filter: none !important;
        border-radius: ${PANEL_RADIUS}px;
        box-sizing: border-box;
        container-name: csw-panel;
        container-type: inline-size;
        display: flex;
        filter: none !important;
        flex-direction: column;
        height: 100%;
        opacity: 0;
        overflow: hidden;
        pointer-events: none;
        position: absolute;
        inset: 0;
        visibility: hidden;
        will-change: clip-path;
        z-index: 2;
      }

      .csw-panel *,
      .csw-panel *::before,
      .csw-panel *::after {
        -webkit-backdrop-filter: none !important;
        -webkit-filter: none !important;
        backdrop-filter: none !important;
        filter: none !important;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-panel {
        opacity: 1;
        pointer-events: auto;
        visibility: visible;
      }

      .csw-popover[data-morphing="true"] .csw-panel {
        opacity: 1;
        pointer-events: none;
        visibility: visible;
      }

      .csw-head {
        align-items: center;
        cursor: grab;
        display: grid;
        flex: 0 0 auto;
        grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
        min-height: 48px;
        padding: 8px 10px;
        touch-action: none;
        user-select: none;
      }

      .csw-head[data-dragging="true"] {
        cursor: grabbing;
      }

      .csw-head-side {
        align-items: center;
        cursor: default;
        display: flex;
        min-width: 0;
        opacity: 0;
        pointer-events: auto;
        transform: translateY(-2px) scale(.98);
        transition:
          opacity .15s cubic-bezier(.23, 1, .32, 1),
          transform .15s cubic-bezier(.23, 1, .32, 1);
        will-change: opacity, transform;
      }

      .csw-head-side .csw-icon {
        pointer-events: none;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-head:hover .csw-head-side,
      .csw-popover[data-open="true"][data-morphing="false"] .csw-head:has(:focus-visible) .csw-head-side,
      .csw-head[data-dragging="true"] .csw-head-side {
        opacity: 1;
        transform: translateY(0) scale(1);
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-head:hover .csw-head-side .csw-icon,
      .csw-popover[data-open="true"][data-morphing="false"] .csw-head:has(:focus-visible) .csw-head-side .csw-icon {
        pointer-events: auto;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-view-tabs .csw-icon {
        pointer-events: auto;
      }

      .csw-head-left {
        justify-content: flex-start;
      }

      .csw-head-right {
        align-items: center;
        display: flex;
        gap: 2px;
        justify-content: flex-end;
      }

      .csw-head-face {
        align-items: center;
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 999px;
        color: var(--csw-text);
        cursor: grab;
        display: flex;
        height: 32px;
        justify-content: center;
        padding: 0;
        position: relative;
        touch-action: none;
        transition:
          background-color 140ms ease,
          transform 140ms cubic-bezier(.23, 1, .32, 1);
        user-select: none;
        width: ${CHIP_WIDTH}px;
      }

      .csw-head-face:hover {
        background: color-mix(in srgb, var(--csw-text) 4%, transparent);
      }

      .csw-head-face:active {
        background: color-mix(in srgb, var(--csw-text) 6%, transparent);
        transform: scale(.97);
      }

      .csw-source-dot {
        background: color-mix(in srgb, var(--csw-text) 72%, transparent);
        border-radius: 999px;
        box-shadow: none;
        height: 4px;
        left: var(--csw-source-x, 50%);
        opacity: .72;
        pointer-events: none;
        position: absolute;
        top: var(--csw-source-y, 50%);
        transform: translate(-50%, -50%);
        transition: opacity .15s ease;
        width: 4px;
      }

      .csw-source-dot[data-direction="single"] { opacity: 0; }

      :is(.csw-fab, .csw-head-face)[data-expression="generating"] .csw-source-track {
        opacity: 0;
      }

      .csw-head[data-dragging="true"] .csw-head-face {
        cursor: grabbing;
      }

      .csw-head-face:focus-visible {
        background: color-mix(in srgb, var(--csw-text) 5%, transparent);
      }

      .csw-popover[data-morphing="true"] .csw-head-face {
        opacity: 0;
        visibility: hidden;
      }

      .csw-tabs {
        align-items: center;
        display: flex;
        gap: 2px;
      }

      .csw-view-tabs {
        background: color-mix(in srgb, var(--csw-text) 3.5%, transparent);
        border-radius: 10px;
        isolation: isolate;
        padding: 2px;
        position: relative;
      }

      .csw-view-indicator {
        background: color-mix(in srgb, var(--csw-surface-opaque) 74%, transparent);
        border-radius: 8px;
        height: 28px;
        left: 2px;
        opacity: 0;
        pointer-events: none;
        position: absolute;
        top: 2px;
        transform: translate3d(0, 0, 0);
        transition:
          transform ${VIEW_INDICATOR_MS}ms cubic-bezier(.23, 1, .32, 1),
          opacity 110ms cubic-bezier(.23, 1, .32, 1);
        width: 28px;
        will-change: opacity, transform;
        z-index: 0;
      }

      .csw-icon {
        align-items: center;
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 8px;
        color: var(--csw-muted);
        cursor: pointer;
        display: inline-flex;
        font: inherit;
        font-size: var(--csw-chrome-font);
        font-weight: var(--csw-label-weight, 500);
        height: 28px;
        justify-content: center;
        padding: 0;
        transition: background-color 140ms ease-out, color 140ms ease-out, transform 140ms cubic-bezier(.23, 1, .32, 1);
        width: 28px;
      }

      .csw-icon:active {
        transform: scale(.94);
      }

      .csw-view-tabs .csw-icon {
        cursor: grab;
        position: relative;
        touch-action: none;
        transform: scale(1);
        transition:
          color ${VIEW_INDICATOR_MS}ms cubic-bezier(.23, 1, .32, 1),
          transform ${VIEW_INDICATOR_MS}ms cubic-bezier(.23, 1, .32, 1);
        z-index: 1;
      }

      .csw-view-tabs .csw-icon:active {
        transform: scale(.9);
      }

      .csw-view-tabs[data-reordering="true"] .csw-icon[data-dragging="true"] {
        cursor: grabbing;
        opacity: .78;
        z-index: 2;
      }

      .csw-icon[data-active="true"],
      .csw-icon:hover {
        background: var(--csw-hover);
        color: var(--csw-text);
      }

      .csw-view-tabs .csw-icon[data-active="true"] {
        background: transparent;
        box-shadow: none;
        transform: scale(1);
      }

      .csw-icon:disabled {
        cursor: not-allowed;
        opacity: .42;
      }

      .csw-icon svg {
        display: block;
        height: var(--csw-icon-font);
        transform-origin: center;
        width: var(--csw-icon-font);
      }

      .csw-icon[data-view="next"] svg {
        transform: scale(1.08);
      }

      .csw-icon[data-action="refresh"] svg {
        transform: scale(.95);
      }

      .csw-icon[data-view="settings"] svg {
        transform: scale(.86);
      }

      .csw-body {
        display: flex;
        flex-direction: column;
        flex: 1 1 auto;
        min-height: 0;
        overflow: auto;
        overflow-anchor: none;
        padding: 2px 16px 14px;
        position: relative;
        scrollbar-color: color-mix(in srgb, var(--csw-text) 18%, transparent) transparent;
        scrollbar-gutter: stable;
        scrollbar-width: thin;
      }

      .csw-mouth-stage {
        display: flex;
        flex: 1 1 auto;
        flex-direction: column;
        min-height: 100%;
        transform-origin: 50% 0;
        will-change: opacity, transform;
      }

      .csw-body[data-view-transition="true"] {
        overflow: hidden;
      }

      .csw-view-transition-layer {
        inset: 0;
        overflow: hidden;
        pointer-events: none;
        position: absolute;
        z-index: 2;
      }

      .csw-view-transition-copy {
        left: 16px;
        margin: 0;
        pointer-events: none;
        position: absolute;
        right: 16px;
      }

      .csw-mouth-stage[data-mouth-stage="settings"] {
        height: 100%;
      }

      .csw-body[data-view-body="next"] {
        overflow: auto;
      }

      .csw-popover[data-content-fade="true"] .csw-body[data-view-body="next"],
      .csw-popover[data-content-fade="true"] .csw-body[data-view-body="outline"] {
        -webkit-mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 max(0px, calc(100% - var(--csw-content-fade-size, 24px))),
          transparent 100%
        );
        mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 max(0px, calc(100% - var(--csw-content-fade-size, 24px))),
          transparent 100%
        );
        -webkit-mask-repeat: no-repeat;
        mask-repeat: no-repeat;
      }

      .csw-mouth-stage[data-mouth-stage="next"] {
        height: auto;
        min-height: 100%;
      }

      .csw-next-layout {
        display: grid;
        flex: 1 1 auto;
        gap: 16px;
        grid-template-rows: max-content minmax(clamp(168px, 28vh, 240px), auto);
        height: auto;
        min-height: 100%;
        width: 100%;
      }

      .csw-next-layout::after {
        content: "";
        height: 8px;
      }

      .csw-list {
        display: grid;
        align-content: start;
        align-self: start;
        flex: 0 0 auto;
        gap: 6px;
        grid-auto-rows: max-content;
        height: max-content;
        min-height: max-content;
        overflow: visible;
        padding: 4px 2px 6px;
        width: 100%;
      }

      .csw-row {
        align-items: start;
        appearance: none;
        background: transparent;
        border: 0;
        border-top: 0;
        border-radius: 13px;
        color: inherit;
        cursor: pointer;
        display: grid;
        gap: 12px;
        grid-template-columns: minmax(0, 1fr) 18px;
        isolation: isolate;
        box-sizing: border-box;
        min-height: 64px;
        min-width: 0;
        overflow: hidden;
        padding: 13px 10px;
        position: relative;
        text-align: left;
        transition: background 140ms ease-out, color 140ms ease-out, transform 90ms ease-out;
        width: 100%;
      }

      .csw-row:active {
        transform: scale(.985);
      }

      .csw-row::before {
        background: var(--csw-row-surface);
        border-radius: inherit;
        content: "";
        inset: 0;
        opacity: 0;
        pointer-events: none;
        position: absolute;
        transition: background 180ms ease-out, opacity 160ms ease-out;
        z-index: -1;
      }

      .csw-popover[data-material="frosted"] {
        --csw-row-surface: color-mix(in srgb, var(--csw-surface-opaque) 28%, transparent);
        --csw-row-selected: color-mix(in srgb, var(--csw-accent) 8.5%, transparent);
      }

      .csw-popover[data-material="clear"] {
        --csw-row-surface: color-mix(in srgb, var(--csw-text) 3.5%, transparent);
        --csw-row-selected: color-mix(in srgb, var(--csw-accent) 8%, transparent);
      }

      .csw-popover[data-material="liquid"],
      .csw-popover[data-material="crystal"] {
        --csw-row-surface: color-mix(in srgb, var(--csw-text) 5%, transparent);
        --csw-row-selected: color-mix(in srgb, var(--csw-accent) 9%, transparent);
      }

      .csw-popover[data-material="matte"] {
        --csw-row-surface: color-mix(in srgb, var(--csw-surface-opaque) 82%, transparent);
        --csw-row-selected: color-mix(in srgb, var(--csw-accent) 7%, transparent);
      }

      .csw-row:first-child {
        border-top: 0;
      }

      .csw-row:hover,
      .csw-row:focus-visible,
      .csw-row:focus-within {
        background: transparent;
        color: var(--csw-text);
        outline: 1px solid color-mix(in srgb, var(--csw-text) 12%, transparent);
        outline-offset: -1px;
      }

      .csw-row:hover::before,
      .csw-row:focus-visible::before,
      .csw-row:focus-within::before {
        opacity: 1;
      }

      .csw-row[data-preview-active="true"] {
        background: transparent;
        color: var(--csw-text);
        outline: 1px solid color-mix(in srgb, var(--csw-accent) 34%, transparent);
        outline-offset: -1px;
      }

      .csw-row[data-preview-active="true"]::before {
        background: var(--csw-row-selected);
        opacity: 1;
      }

      .csw-row:active {
        transform: scale(.992);
      }

      .csw-row-copy {
        display: block;
        min-width: 0;
        overflow: hidden;
      }

      .csw-row-label {
        color: var(--csw-text);
        display: block;
        font-size: var(--csw-item-font);
        font-weight: var(--csw-label-weight, 600);
        line-height: 1.3;
        margin-bottom: 3px;
        overflow-wrap: anywhere;
      }

      .csw-row-prompt {
        color: var(--csw-muted);
        display: -webkit-box;
        font-size: max(10px, calc(var(--csw-item-font) - 1px));
        line-height: 1.46;
        max-height: 2.92em;
        overflow: hidden;
        overflow-wrap: anywhere;
        white-space: normal;
        -webkit-box-orient: vertical;
        -webkit-line-clamp: 2;
      }

      .csw-list[data-label-only="true"] .csw-row-prompt {
        display: none;
      }

      .csw-list[data-label-only="true"] .csw-row {
        align-items: center;
        min-height: 44px;
        padding-block: 10px;
      }

      .csw-list[data-label-only="true"] .csw-row-label {
        margin-bottom: 0;
      }

      .csw-list[data-label-only="true"] .csw-row-arrow {
        align-self: center;
      }

      .csw-row-arrow {
        color: var(--csw-faint);
        font-size: 17px;
        line-height: 1;
        text-align: center;
        transition: color 160ms ease, transform 160ms ease;
      }

      .csw-row:hover .csw-row-arrow,
      .csw-row:focus-visible .csw-row-arrow,
      .csw-row[data-preview-active="true"] .csw-row-arrow {
        color: var(--csw-accent);
        transform: translateX(2px);
      }

      .csw-prompt-preview {
        --csw-prompt-edge-fade-size: clamp(36px, 16%, 64px);
        background: transparent;
        border: 0;
        border-radius: 20px;
        box-shadow: none;
        isolation: isolate;
        min-height: 0;
        overflow: hidden;
        position: relative;
      }

      .csw-prompt-preview::before {
        -webkit-mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 calc(100% - var(--csw-prompt-edge-fade-size)),
          transparent 100%
        );
        -webkit-mask-repeat: no-repeat;
        background:
          linear-gradient(180deg, color-mix(in srgb, var(--csw-text) 4.5%, transparent), transparent),
          color-mix(in srgb, var(--csw-surface-opaque) 22%, transparent);
        border: 1px solid color-mix(in srgb, var(--csw-text) 7%, transparent);
        border-radius: inherit;
        box-sizing: border-box;
        content: "";
        inset: 0;
        mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 calc(100% - var(--csw-prompt-edge-fade-size)),
          transparent 100%
        );
        mask-repeat: no-repeat;
        pointer-events: none;
        position: absolute;
        z-index: 0;
      }

      .csw-prompt-preview-scroll {
        -webkit-mask-image: none;
        height: 100%;
        mask-image: none;
        overflow: auto;
        overscroll-behavior: contain;
        padding: 16px 18px 28px;
        position: relative;
        scrollbar-color: color-mix(in srgb, var(--csw-text) 20%, transparent) transparent;
        scrollbar-width: thin;
        z-index: 1;
      }

      .csw-prompt-preview[data-scroll-fade="true"] .csw-prompt-preview-scroll {
        -webkit-mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 max(0px, calc(100% - 18px)),
          transparent 100%
        );
        mask-image: linear-gradient(
          to bottom,
          #000 0,
          #000 max(0px, calc(100% - 18px)),
          transparent 100%
        );
        -webkit-mask-repeat: no-repeat;
        mask-repeat: no-repeat;
      }

      .csw-prompt-preview-content {
        opacity: 1;
        transform: translateY(0);
        transition:
          opacity 120ms ease-out,
          transform 150ms cubic-bezier(.22, .8, .2, 1);
      }

      .csw-prompt-preview[data-switching="true"] .csw-prompt-preview-content {
        opacity: 0;
        transform: translateY(4px);
      }

      .csw-prompt-preview-kicker {
        color: var(--csw-accent);
        display: block;
        font-size: max(9px, calc(var(--csw-item-font) - 3px));
        font-weight: var(--csw-label-weight, 600);
        letter-spacing: .045em;
        line-height: 1.2;
        margin-bottom: 7px;
      }

      .csw-prompt-preview-title {
        color: var(--csw-text);
        display: block;
        font-size: clamp(12px, calc(var(--csw-item-font) + 1px), 25px);
        font-weight: var(--csw-label-weight, 600);
        letter-spacing: -.012em;
        line-height: 1.35;
        margin-bottom: 9px;
      }

      .csw-prompt-preview-body {
        color: var(--csw-muted);
        display: block;
        font-size: var(--csw-item-font);
        line-height: 1.65;
        overflow-wrap: anywhere;
        white-space: pre-wrap;
      }

      .csw-empty {
        align-items: center;
        background: transparent;
        border: 0;
        border-top: 0;
        border-radius: 0;
        color: var(--csw-muted);
        display: grid;
        flex: 1 1 auto;
        align-content: center;
        justify-items: center;
        min-height: 0;
        min-width: 0;
        max-width: 100%;
        padding: 24px 12px;
        text-align: center;
      }

      .csw-empty-title {
        color: var(--csw-text);
        font-size: clamp(12px, calc(var(--csw-item-font) + 1px), 25px);
        font-weight: 720;
        line-height: 1.25;
        max-width: 100%;
        min-width: 0;
        overflow-wrap: anywhere;
      }

      .csw-empty[data-state="manual"] .csw-empty-title {
        color: var(--csw-muted);
        font-size: clamp(11px, var(--csw-item-font), 20px);
        font-weight: 500;
        letter-spacing: 0.01em;
      }

      .csw-progress {
        align-items: center;
        color: var(--csw-muted);
        display: flex;
        flex: 1 1 auto;
        gap: clamp(10px, calc(var(--csw-item-font) - 1px), 18px);
        justify-content: center;
        min-height: 0;
        min-width: 0;
        max-width: 100%;
        padding: 16px 5px;
      }

      .csw-progress-ring {
        animation: csw-progress-spin .82s linear infinite;
        border: 2px solid color-mix(in srgb, var(--csw-text) 11%, transparent);
        border-radius: 999px;
        border-top-color: var(--csw-accent);
        flex: 0 0 auto;
        height: clamp(18px, calc(var(--csw-item-font) + 7px), 31px);
        width: clamp(18px, calc(var(--csw-item-font) + 7px), 31px);
      }

      .csw-progress-copy {
        display: grid;
        gap: 2px;
        min-width: 0;
        max-width: 100%;
      }

      .csw-progress-title {
        animation: csw-progress-text-shimmer 1.8s linear infinite;
        background-image: linear-gradient(
          90deg,
          color-mix(in srgb, var(--csw-text) 62%, var(--csw-muted)) 34%,
          var(--csw-text) 50%,
          color-mix(in srgb, var(--csw-text) 62%, var(--csw-muted)) 66%
        );
        background-position: 100% 50%;
        background-size: 220% 100%;
        -webkit-background-clip: text;
        background-clip: text;
        color: transparent;
        font-size: clamp(11px, var(--csw-item-font), 24px);
        font-weight: var(--csw-label-weight, 600);
        line-height: 1.25;
        overflow-wrap: anywhere;
        -webkit-text-fill-color: transparent;
      }

      .csw-outline-list {
        display: grid;
        align-content: start;
        flex: 0 0 auto;
        grid-auto-rows: max-content;
        min-height: max-content;
        width: 100%;
      }

      .csw-outline-view {
        display: flex;
        flex: 1 1 auto;
        flex-direction: column;
        min-height: 100%;
      }

      .csw-outline-toolbar {
        align-items: center;
        display: flex;
        flex: 0 0 auto;
        gap: 4px;
        justify-content: flex-end;
        margin-top: auto;
        opacity: 0;
        padding: 6px 0 0;
        pointer-events: none;
        position: sticky;
        bottom: 0;
        transform: translateY(4px);
        transition: opacity 150ms ease-out, transform 180ms ease-out;
        z-index: 2;
      }

      .csw-popover[data-open="true"]:hover .csw-outline-toolbar,
      .csw-popover[data-open="true"]:focus-within .csw-outline-toolbar {
        opacity: 1;
        pointer-events: auto;
        transform: translateY(0);
      }

      .csw-outline-nav-button {
        align-items: center;
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 8px;
        color: var(--csw-muted);
        cursor: pointer;
        display: inline-flex;
        height: 26px;
        justify-content: center;
        padding: 0;
        transition: background 160ms ease-out, color 160ms ease-out, transform 90ms ease-out;
        width: 26px;
      }

      .csw-outline-nav-button svg {
        display: block;
        height: 13px;
        width: 13px;
      }

      .csw-outline-nav-button:hover,
      .csw-outline-nav-button:focus-visible {
        background: color-mix(in srgb, var(--csw-text) 8%, transparent);
        color: var(--csw-text);
        outline: none;
      }

      .csw-outline-nav-button:active {
        transform: scale(.94);
      }

      .csw-outline-row {
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 13px;
        box-sizing: border-box;
        color: var(--csw-text);
        cursor: pointer;
        display: block;
        isolation: isolate;
        min-height: 38px;
        padding: 8px 12px 8px calc(12px + var(--csw-outline-indent, 0px));
        position: relative;
        text-align: left;
        transition: color 140ms ease-out, transform 90ms ease-out;
        width: 100%;
      }

      .csw-outline-row::before {
        background: var(--csw-row-surface);
        border-radius: inherit;
        content: "";
        inset: 0;
        opacity: 0;
        pointer-events: none;
        position: absolute;
        transition: background 180ms ease-out, opacity 160ms ease-out;
        z-index: -1;
      }

      .csw-outline-row:first-child {
        border-top: 0;
      }

      .csw-outline-row:hover,
      .csw-outline-row:focus-visible {
        background: transparent;
        color: var(--csw-text);
        outline: 1px solid color-mix(in srgb, var(--csw-text) 12%, transparent);
        outline-offset: -1px;
      }

      .csw-outline-row:hover::before,
      .csw-outline-row:focus-visible::before {
        opacity: 1;
      }

      .csw-outline-row[data-active="true"] {
        background: transparent;
        color: var(--csw-text);
        outline: 1px solid color-mix(in srgb, var(--csw-accent) 34%, transparent);
        outline-offset: -1px;
      }

      .csw-outline-row[data-active="true"]::before {
        background: var(--csw-row-selected);
        opacity: 1;
      }

      .csw-outline-row:active {
        transform: scale(.992);
      }

      .csw-outline-row[data-outline-id] {
        align-items: center;
        column-gap: 8px;
        display: grid;
        grid-template-columns: minmax(0, 1fr);
        min-height: calc(var(--csw-item-font) * 1.35 + 16px);
        padding-left: calc(12px + var(--csw-outline-indent, 0px) + var(--csw-outline-hanging-indent, 0px));
      }

      .csw-outline-row[data-outline-id][data-numbered="true"] {
        grid-template-columns: max-content minmax(0, 1fr);
      }

      .csw-outline-heading-marker {
        display: none;
      }

      .csw-outline-heading-marker::before {
        display: none;
      }

      .csw-outline-prefix {
        align-self: center;
        font-variant-numeric: tabular-nums;
        min-width: 0;
        white-space: nowrap;
      }

      .csw-outline-label {
        min-width: 0;
        overflow-wrap: anywhere;
      }

      .csw-outline-row[data-numbered="false"] .csw-outline-prefix {
        display: none;
      }

      .csw-outline-row[data-numbered="false"] .csw-outline-label {
        grid-column: 1;
      }

      .csw-outline-row[data-numbered="true"] .csw-outline-label {
        grid-column: 2;
      }

      .csw-outline-label {
        align-self: center;
        line-height: 1.35;
      }

      .csw-outline-text,
      .csw-outline-prefix,
      .csw-outline-label {
        font-size: var(--csw-item-font);
        line-height: 1.35;
        position: relative;
        z-index: 1;
      }

      .${HIGHLIGHT_CLASS} {
        outline: 2px solid color-mix(in srgb, var(--csw-accent) 70%, transparent) !important;
        outline-offset: 4px !important;
        border-radius: 6px !important;
        transition: outline-color 0.2s ease;
      }

      .csw-resize-handle {
        appearance: none;
        -webkit-appearance: none;
        background: none;
        border: 0;
        bottom: 0;
        box-shadow: none;
        color: transparent;
        cursor: nwse-resize;
        display: none;
        font-size: 0;
        height: 28px;
        line-height: 0;
        outline: 0;
        padding: 0;
        pointer-events: auto;
        position: absolute;
        touch-action: none;
        user-select: none;
        width: 28px;
        z-index: 5;
      }

      .csw-popover[data-open="true"][data-morphing="false"] .csw-resize-handle {
        display: block;
      }

      .csw-popover[data-view="settings"] .csw-resize-handle {
        display: none !important;
      }

      .csw-resize-handle[data-corner="bl"] {
        cursor: nesw-resize;
        left: 0;
      }

      .csw-resize-handle[data-corner="br"] {
        right: 0;
      }

      .csw-settings {
        display: grid;
        height: 100%;
        min-height: 0;
        padding-top: 4px;
      }

      .csw-settings-surface {
        -webkit-backdrop-filter: blur(18px) saturate(145%);
        backdrop-filter: blur(18px) saturate(145%);
        background:
          linear-gradient(
            180deg,
            color-mix(in srgb, #000 2%, transparent) 0%,
            transparent 16%,
            transparent 82%,
            color-mix(in srgb, #fff 7%, transparent) 100%
          ),
          color-mix(in srgb, var(--csw-surface-opaque) 82%, transparent);
        border: 1px solid color-mix(in srgb, var(--csw-text) 6%, transparent);
        border-radius: 22px;
        box-shadow: none;
        display: grid;
        font-size: var(--csw-chrome-font);
        grid-template-rows: minmax(0, 1fr) auto;
        min-height: 0;
        overflow: hidden;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-settings-surface {
        background:
          linear-gradient(
            180deg,
            color-mix(in srgb, #000 8%, transparent) 0%,
            transparent 18%,
            transparent 82%,
            color-mix(in srgb, #fff 3%, transparent) 100%
          ),
          color-mix(in srgb, var(--csw-surface-opaque) 76%, transparent);
        border-color: color-mix(in srgb, #fff 6%, transparent);
        box-shadow: none;
      }

      .csw-popover[data-material="clear"] .csw-settings-surface {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background: transparent;
        border-color: color-mix(in srgb, var(--csw-glass-edge-hi) 24%, var(--csw-glass-edge));
        box-shadow: none;
      }

      .csw-popover[data-material="liquid"] .csw-settings-surface,
      .csw-popover[data-material="crystal"] .csw-settings-surface {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background: rgba(255, 255, 255, 0.2);
        border-color: rgba(255, 255, 255, 0.14);
        box-shadow: none;
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="liquid"] .csw-settings-surface,
      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="crystal"] .csw-settings-surface {
        background: rgba(20, 24, 30, 0.28);
        border-color: rgba(255, 255, 255, 0.11);
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="clear"] .csw-settings-surface {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background: transparent;
        border-color: color-mix(in srgb, rgba(205, 228, 255, 0.5) 20%, var(--csw-glass-edge));
        box-shadow: none;
      }

      .csw-popover[data-material="matte"] .csw-settings-surface {
        -webkit-backdrop-filter: none;
        backdrop-filter: none;
        background:
          linear-gradient(
            180deg,
            color-mix(in srgb, #000 1.5%, transparent) 0%,
            transparent 18%,
            transparent 82%,
            color-mix(in srgb, #fff 5%, transparent) 100%
          ),
          color-mix(in srgb, var(--csw-surface-opaque) 98%, transparent);
      }

      [${ROOT_ATTR}="true"][data-theme="dark"] .csw-popover[data-material="matte"] .csw-settings-surface {
        background:
          linear-gradient(
            180deg,
            color-mix(in srgb, #000 8%, transparent) 0%,
            transparent 18%,
            transparent 82%,
            color-mix(in srgb, #fff 3%, transparent) 100%
          ),
          color-mix(in srgb, var(--csw-surface-opaque) 96%, #000 2%);
      }

      .csw-settings-hero {
        align-items: center;
        display: grid;
        gap: 18px;
        grid-template-columns: minmax(0, 1fr) minmax(230px, 238px);
        min-height: 0;
        padding: 18px 18px 14px;
      }

      .csw-model-pane {
        align-self: center;
        display: flex;
        flex-direction: column;
        justify-content: center;
        min-width: 0;
        padding: 8px 6px;
      }

      .csw-metric-label,
      .csw-control-label {
        color: var(--csw-muted);
        font-size: 11px;
        font-weight: var(--csw-label-weight, 500);
        letter-spacing: .015em;
      }

      .csw-metric-label {
        font-synthesis: none;
        font-weight: 400;
      }

      .csw-model-value {
        color: var(--csw-text);
        font-size: 34px;
        font-weight: 580;
        letter-spacing: -.035em;
        line-height: 1.08;
        margin-top: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .csw-settings-surface[data-loading="true"] .csw-model-value {
        color: var(--csw-muted);
        font-size: 24px;
        letter-spacing: -.02em;
      }

      .csw-runtime-line {
        align-items: center;
        color: var(--csw-muted);
        display: flex;
        font-size: 12px;
        gap: 7px;
        margin-top: 10px;
        min-width: 0;
      }

      .csw-runtime-copy {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .csw-runtime-dot {
        background: var(--csw-faint);
        border-radius: 999px;
        flex: 0 0 auto;
        height: 7px;
        width: 7px;
      }

      .csw-runtime-dot[data-tone="busy"] {
        animation: csw-status-breathe 1.2s ease-in-out infinite;
        background: var(--csw-accent);
      }

      .csw-runtime-dot[data-tone="ready"] {
        background: var(--csw-ready);
      }

      .csw-runtime-dot[data-tone="error"] {
        background: var(--csw-danger);
      }

      .csw-runtime-grid {
        align-items: center;
        display: grid;
        gap: 14px;
        grid-template-columns: minmax(0, max-content) minmax(0, 1fr);
        min-width: 0;
        padding: 0;
        width: 100%;
      }

      .csw-click-mode,
      .csw-generation-mode {
        align-items: baseline;
        display: inline-flex;
        gap: 6px;
        max-width: 100%;
        min-width: 0;
        position: relative;
      }

      .csw-generation-mode {
        white-space: nowrap;
      }

      .csw-metric {
        align-items: baseline;
        display: inline-flex;
        gap: 6px;
        min-width: 0;
        padding: 0;
      }

      .csw-metric-value,
      .csw-metric-action {
        color: color-mix(in srgb, var(--csw-text) 76%, transparent);
        font-size: 11px;
        font-synthesis: none;
        font-variant-numeric: tabular-nums;
        font-weight: 400;
        letter-spacing: .005em;
        line-height: 1.25;
        min-width: 0;
        overflow: visible;
        text-overflow: clip;
      }

      .csw-metric-value,
      .csw-generation-mode .csw-metric-action {
        white-space: nowrap;
      }

      .csw-click-mode .csw-metric-action {
        overflow-wrap: anywhere;
        white-space: normal;
      }

      .csw-metric-value[data-enabled="true"],
      .csw-metric-action {
        color: var(--csw-text);
      }

      button.csw-metric-action {
        appearance: none;
        background: transparent;
        border: 0;
        border-radius: 0;
        box-shadow: none;
        color: var(--csw-text);
        cursor: pointer;
        font-family: inherit;
        font-size: 11px;
        font-weight: 400;
        margin: 0;
        max-width: 100%;
        min-width: 0;
        outline: none;
        padding: 0;
        text-align: left;
      }

      button.csw-metric-action:hover,
      button.csw-metric-action:focus-visible {
        background: transparent;
        color: color-mix(in srgb, var(--csw-text) 88%, var(--csw-accent) 12%);
      }

      button.csw-metric-action:active {
        color: color-mix(in srgb, var(--csw-text) 78%, var(--csw-accent) 22%);
      }

      button.csw-metric-action:disabled {
        cursor: not-allowed;
        opacity: .34;
      }

      .csw-control-deck {
        align-self: center;
        background: transparent;
        border: 0;
        border-radius: 0;
        box-shadow: none;
        display: grid;
        gap: 10px;
        grid-auto-rows: 32px;
        justify-self: end;
        min-width: 0;
        overflow: visible;
        padding: 0;
        width: 238px;
      }

      .csw-settings-footer {
        align-items: center;
        background: transparent;
        border: 0;
        border-radius: 0;
        border-top: 1px solid color-mix(in srgb, var(--csw-text) 8%, transparent);
        box-shadow: none;
        display: grid;
        gap: 12px;
        grid-template-columns: minmax(0, 1fr) auto;
        margin: 0 18px 12px;
        min-height: 50px;
        overflow: visible;
        padding: 9px 2px 0;
      }

      .csw-control-group {
        align-items: center;
        display: grid;
        gap: 10px;
        grid-template-columns: 76px minmax(0, 152px);
        min-width: 0;
        padding: 0;
      }

      .csw-control-group + .csw-control-group {
        border-top: 0;
      }

      .csw-control-label {
        align-self: center;
        font-size: 12px;
        line-height: 1;
        text-align: left;
      }

      .csw-control-row,
      .csw-stepper {
        align-items: center;
        box-sizing: border-box;
        justify-self: end;
        min-width: 0;
        width: 152px;
      }

      .csw-control-row {
        background: color-mix(in srgb, var(--csw-text) 2%, transparent);
        border: 1px solid color-mix(in srgb, var(--csw-text) 7%, transparent);
        border-radius: 11px;
        box-shadow: none;
        display: grid;
        gap: 0;
        grid-template-columns: minmax(0, 1fr);
        height: 32px;
        overflow: hidden;
      }

      .csw-control-button,
      .csw-step-button,
      .csw-command-button {
        appearance: none;
        background: transparent;
        border: 0;
        color: var(--csw-text);
        cursor: pointer;
        font: inherit;
      }

      .csw-control-button {
        align-items: center;
        border-radius: 0;
        display: flex;
        font-size: 13px;
        font-weight: var(--csw-label-weight, 500);
        gap: 5px;
        height: 30px;
        justify-content: center;
        line-height: 30px;
        max-width: none;
        overflow: hidden;
        padding: 0 8px;
        text-overflow: ellipsis;
        white-space: nowrap;
        width: 100%;
      }

      .csw-stepper {
        display: grid;
        background: color-mix(in srgb, var(--csw-text) 2%, transparent);
        border: 1px solid color-mix(in srgb, var(--csw-text) 7%, transparent);
        border-radius: 11px;
        box-shadow: none;
        grid-template-columns: 30px minmax(0, 1fr) 30px;
        height: 32px;
        overflow: hidden;
      }

      .csw-step-button {
        align-items: center;
        border-radius: 0;
        color: var(--csw-muted);
        display: flex;
        font-size: 18px;
        height: 30px;
        justify-content: center;
        line-height: 1;
        min-width: 30px;
        padding: 0;
      }

      .csw-step-value {
        align-items: center;
        border-left: 1px solid var(--csw-divider);
        border-right: 1px solid var(--csw-divider);
        color: var(--csw-text);
        display: flex;
        font-size: 13px;
        font-variant-numeric: tabular-nums;
        font-weight: 620;
        justify-content: center;
        min-width: 0;
        text-align: center;
      }

      .csw-control-button:hover,
      .csw-step-button:hover,
      .csw-command-button:hover {
        background: var(--csw-hover);
      }

      .csw-command-deck {
        align-items: center;
        display: flex;
        flex: 0 0 auto;
        gap: 3px;
        justify-self: end;
        min-width: 0;
        padding-left: 0;
      }

      .csw-command-button {
        align-items: center;
        border-radius: 7px;
        color: var(--csw-muted);
        display: flex;
        gap: 5px;
        justify-content: center;
        height: 30px;
        margin: 0;
        min-width: 0;
        padding: 0 7px;
      }

      .csw-command-button:hover {
        color: var(--csw-text);
      }

      .csw-command-button:disabled,
      .csw-step-button:disabled {
        cursor: not-allowed;
        opacity: .32;
      }

      .csw-step-button:disabled:hover {
        background: transparent;
      }

      .csw-command-icon {
        align-items: center;
        display: flex;
        height: 17px;
        justify-content: center;
        width: 17px;
      }

      .csw-command-icon svg {
        height: 16px;
        width: 16px;
      }

      .csw-command-icon[data-busy="true"] svg {
        animation: csw-progress-spin .9s linear infinite;
      }

      .csw-command-label {
        font-size: 12px;
        line-height: 1;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .csw-settings-notice {
        align-items: center;
        color: var(--csw-muted);
        display: flex;
        font-size: 11px;
        grid-column: 1 / -1;
        line-height: 1.4;
        max-width: 100%;
        min-height: 22px;
        min-width: 0;
        overflow-wrap: anywhere;
        padding: 2px 2px 0;
        white-space: normal;
      }

      .csw-settings-notice[data-tone="warn"] {
        color: color-mix(in srgb, var(--csw-danger) 78%, var(--csw-muted));
      }

      .csw-icon:focus-visible,
      .csw-head-face:focus-visible,
      .csw-control-button:focus-visible,
      .csw-metric-action:focus-visible,
      .csw-step-button:focus-visible,
      .csw-command-button:focus-visible,
      .csw-row:focus-visible {
        outline: 2px solid color-mix(in srgb, var(--csw-accent) 72%, transparent);
        outline-offset: 2px;
      }

      @container csw-panel (max-width: 440px) {
        .csw-list {
          padding-inline: 0;
        }

        .csw-row {
          gap: 8px;
          grid-template-columns: minmax(0, 1fr) 16px;
          padding: 11px 8px;
        }

        .csw-row-arrow {
          font-size: 16px;
        }

        .csw-prompt-preview-scroll {
          padding: 14px 14px 26px;
        }

        .csw-settings-hero {
          align-content: start;
          gap: 10px;
          grid-template-columns: 1fr;
          grid-template-rows: auto auto;
          overflow-x: hidden;
          overflow-y: auto;
          overscroll-behavior: contain;
          padding: 12px 14px 10px;
          scrollbar-color: color-mix(in srgb, var(--csw-text) 16%, transparent) transparent;
          scrollbar-width: thin;
        }

        .csw-model-pane {
          align-self: start;
          min-height: 72px;
          padding: 3px 4px 1px;
        }

        .csw-model-value {
          font-size: 27px;
        }

        .csw-settings-surface[data-loading="true"] .csw-model-value {
          font-size: 20px;
        }

        .csw-runtime-line {
          font-size: 11px;
          margin-top: 6px;
        }

        .csw-control-deck {
          gap: 8px;
          justify-self: stretch;
          min-height: 0;
          width: 100%;
        }

        .csw-control-group {
          gap: 8px;
          grid-template-columns: minmax(76px, auto) minmax(0, 1fr);
        }

        .csw-control-label {
          font-size: 11px;
        }

        .csw-control-row,
        .csw-stepper {
          width: 100%;
        }

        .csw-settings-footer {
          column-gap: 8px;
          display: grid;
          grid-template-columns: minmax(max-content, 1fr) auto;
          min-height: 0;
          padding: 8px 0 0;
          row-gap: 6px;
        }

        .csw-runtime-grid {
          gap: 12px;
          grid-template-columns: minmax(0, max-content) minmax(0, 1fr);
          justify-content: flex-start;
          min-height: 30px;
          width: 100%;
        }

        .csw-command-deck {
          gap: 2px;
          justify-content: flex-end;
        }

        .csw-command-button {
          flex: 0 0 30px;
          height: 30px;
          padding: 0;
          width: 30px;
        }

        .csw-command-label {
          display: none;
        }

        .csw-settings-notice {
          grid-column: 1 / -1;
          min-width: 0;
          padding-top: 0;
        }
      }

      @container csw-panel (max-width: 404px) {
        .csw-settings-footer {
          margin: 0 14px 10px;
        }
      }

      @container csw-panel (max-width: 360px) {
        .csw-runtime-grid {
          gap: 6px;
          grid-template-columns: minmax(0, 1fr);
        }

        .csw-generation-mode,
        .csw-click-mode {
          width: 100%;
        }
      }

      @container csw-panel (max-width: 320px) {
        .csw-row {
          gap: 6px;
          grid-template-columns: minmax(0, 1fr) 14px;
          padding-inline: 6px;
        }

        .csw-prompt-preview {
          border-radius: 16px;
        }

        .csw-settings-footer {
          column-gap: 6px;
          margin: 0 12px 10px;
        }

        .csw-runtime-grid {
          gap: 8px;
        }

        .csw-metric {
          gap: 5px;
          white-space: nowrap;
        }

        .csw-command-deck {
          gap: 0;
        }

        .csw-command-button {
          flex: 0 0 28px;
          height: 28px;
          width: 28px;
        }
      }

      @keyframes csw-face-blink {
        0%, 45%, 49%, 100% { transform: scaleY(1); }
        47% { transform: scaleY(0.12); }
      }

      @keyframes csw-face-star {
        0%, 100% { transform: scale(.9) rotate(0deg); }
        50% { transform: scale(1.08) rotate(8deg); }
      }

      @keyframes csw-face-generate-bob {
        0%, 100% { transform: translate3d(0, 2px, 0) scaleY(.9); }
        50% { transform: translate3d(0, -4px, 0) scaleY(1.06); }
      }

      @keyframes csw-face-happy-lift {
        0%, 100% { transform: translate3d(0, 1px, 0) scale(.97); }
        50% { transform: translate3d(0, -1px, 0) scale(1.03); }
      }

      @keyframes csw-face-calm-breathe {
        0%, 100% { opacity: .74; transform: scaleX(.94); }
        50% { opacity: 1; transform: scaleX(1); }
      }

      @keyframes csw-face-error-breathe {
        0%, 100% { opacity: .78; transform: scale(.96); }
        50% { opacity: 1; transform: scale(1); }
      }

      @keyframes csw-status-breathe {
        0%, 100% { opacity: .45; transform: scale(.86); }
        50% { opacity: 1; transform: scale(1); }
      }

      @keyframes csw-progress-spin {
        to { transform: rotate(360deg); }
      }

      @keyframes csw-progress-text-shimmer {
        to { background-position: -100% 50%; }
      }

      @media (prefers-reduced-motion: reduce) {
        .csw-progress-title {
          animation: none !important;
          background: none;
          color: var(--csw-text);
          -webkit-text-fill-color: currentColor;
        }

        .csw-view-indicator,
        .csw-view-tabs .csw-icon,
        .csw-outline-toolbar {
          transition: none !important;
        }
      }

      @media (hover: none), (pointer: coarse) {
        .csw-outline-toolbar {
          opacity: 1;
          pointer-events: auto;
          transform: none;
        }
      }

      @media (max-width: 520px) {
        .csw-head {
          padding-left: 14px;
          padding-right: 14px;
        }

        .csw-body {
          padding-left: 13px;
          padding-right: 13px;
        }

      }

      .csw-body,
      .csw-prompt-preview-scroll,
      .csw-settings-hero {
        scrollbar-color: transparent transparent;
        scrollbar-gutter: auto;
        scrollbar-width: none;
      }

      .csw-body::-webkit-scrollbar,
      .csw-prompt-preview-scroll::-webkit-scrollbar,
      .csw-settings-hero::-webkit-scrollbar {
        display: none;
        height: 0;
        width: 0;
      }

      .csw-popover[data-material="clear"],
      .csw-popover[data-material="clear"] *,
      .csw-popover[data-material="clear"]::before,
      .csw-popover[data-material="clear"]::after,
      .csw-popover[data-material="clear"] *::before,
      .csw-popover[data-material="clear"] *::after {
        text-shadow: none !important;
      }

      .csw-popover[data-material="matte"] .csw-prompt-preview::before {
        background:
          linear-gradient(180deg, color-mix(in srgb, var(--csw-text) 2.5%, transparent), transparent),
          color-mix(in srgb, var(--csw-surface-opaque) 14%, transparent);
        border-color: color-mix(in srgb, var(--csw-text) 4.5%, transparent);
      }

      .csw-popover[data-material] .csw-settings-surface {
        -webkit-backdrop-filter: none !important;
        backdrop-filter: none !important;
        background: transparent !important;
        border: 0 !important;
        border-radius: 0 !important;
        box-shadow: none !important;
      }

      @media (prefers-reduced-motion: reduce) {
        .csw-completion-beam,
        .csw-completion-beam::before {
          animation: none !important;
          opacity: 0 !important;
        }

        :is(.csw-fab, .csw-head-face) .csw-fab-eye,
        :is(.csw-fab, .csw-head-face) .csw-fab-eye::before,
        :is(.csw-fab, .csw-head-face) .csw-fab-eye::after {
          animation: none !important;
        }

        [${ROOT_ATTR}="true"] *,
        [${ROOT_ATTR}="true"] *::before,
        [${ROOT_ATTR}="true"] *::after {
          animation-duration: 1ms !important;
          animation-iteration-count: 1 !important;
          transition-duration: 1ms !important;
        }
      }

      @keyframes csw-completion-beam-sweep {
        0% {
          opacity: 0;
          transform: rotate(-64deg);
        }
        12% {
          opacity: .34;
        }
        72% {
          opacity: .52;
        }
        100% {
          opacity: 0;
          transform: rotate(296deg);
        }
      }

    `;
    document.head.appendChild(style);
  }
