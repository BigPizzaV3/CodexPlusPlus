  // Optional right-side layout for Codex's native message navigation rail.
  // The native page recreates the rail and preview card during navigation, so
  // this uses stable data attributes instead of retaining DOM node references.
  (() => {
    const styleId = "codex-plus-avatar-overlay-layout-style";
    const root = document.documentElement;
    const enabledAttribute = "data-codex-plus-activity-overlay-right";

    function installStyle() {
      const existing = document.getElementById(styleId);
      if (existing) return;
      const style = document.createElement("style");
      style.id = styleId;
      style.dataset.codexPlusExt = "builtin-avatar-overlay-layout";
      style.textContent = `
        /* The native rail is mounted inside an absolute nav with left-3. */
        html[${enabledAttribute}="true"] nav:has([data-thread-user-message-navigation-rail-list]) {
          left: auto !important;
          right: max(12px, env(safe-area-inset-right, 0px)) !important;
        }

        /* Native marker lines grow from their left edge. Once the rail is on
           the right, the same level indicator must grow toward the left. */
        html[${enabledAttribute}="true"] [data-thread-user-message-navigation-rail-list] [data-thread-user-message-navigation-item-id] > span > span > span {
          transform-origin: 100% 50% !important;
        }

        /* Floating preview cards are positioned by Radix beside the old left
           rail. Use the individual translate property so Floating UI's own
           transform stays intact while the card moves to the opposite side. */
        html[${enabledAttribute}="true"] [data-radix-popper-content-wrapper]:has([data-thread-user-message-navigation-tooltip-preview]) {
          translate: calc(-100% - 8px) 0 !important;
        }
      `;
      document.head?.appendChild(style);
    }

    function sync() {
      installStyle();
      const enabled = typeof codexPlusSettings === "function"
        && codexPlusSettings().activityOverlayRight === true;
      root.toggleAttribute(enabledAttribute, enabled);
      root.setAttribute(enabledAttribute, String(enabled));
    }

    installStyle();
    sync();
    globalThis.__CODEX_PLUS_ACTIVITY_OVERLAY_SYNC__ = sync;
    let attempts = 0;
    const timer = window.setInterval(() => {
      sync();
      if (++attempts >= 60 && codexPlusBackendSettingsLoaded) window.clearInterval(timer);
    }, 250);
  })();
