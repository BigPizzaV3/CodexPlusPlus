(() => {
  const helperBase = window.__CODEX_SESSION_DELETE_HELPER__ || "http://127.0.0.1:57321";
  const buttonClass = "codex-delete-button";
  const exportButtonClass = "codex-export-button";
  const projectMoveButtonClass = "codex-project-move-button";
  const projectMoveOverlayClass = "codex-project-move-overlay";
  const actionButtonClass = "codex-session-action-button";
  const actionGroupClass = "codex-session-actions";
  const timelineClass = "codex-conversation-timeline";
  const timelineTrackClass = "codex-conversation-timeline-track";
  const timelineMarkerClass = "codex-conversation-timeline-marker";
  const timelineTooltipClass = "codex-conversation-timeline-tooltip";
  const timelineTargetClass = "codex-conversation-timeline-target";
  const zedRemoteButtonClass = "codex-zed-remote-button";
  const zedRemoteOpenInMenuItemClass = "codex-zed-open-in-menu-item";
  const zedRemoteToastClass = "codex-zed-remote-toast";
  const zedRemoteOpenVersion = "1";
  const zedRemoteOpenInMenuVersion = "1";
  const zedRemoteOpenInMenuActivationWindowMs = 600;
  const timelineQuestionLimit = 40;
  const timelineMinTopPercent = 2;
  const timelineMaxTopPercent = 98;
  const timelineMaxMarkerGapPercent = 3.5;
  const projectMoveProjectionKey = "codexProjectMoveProjection";
  const legacyProjectMoveOverridesKey = "codexProjectMoveOverrides";
  const projectMoveProjectionTtlMs = 24 * 60 * 60 * 1000;
  const projectMoveProjectionSettleMs = 5 * 60 * 1000;
  const projectMoveRefreshDelaysMs = [50, 250, 750, 1500];
  const chatsSortRefreshIntervalMs = 1500;
  const chatsSortDbRefreshIntervalMs = 5000;
  const styleId = "codex-delete-style";
  const codexDeleteStyleVersion = "9";
  const codexPlusMenuId = "codex-plus-menu";
  const codexPlusMenuFloatingClass = "codex-plus-menu-floating";
  const codexDeleteVersion = "7";
  const codexExportVersion = "1";
  const codexProjectMoveVersion = "1";
  const codexActionGroupVersion = "2";
  const codexArchiveRowActionsVersion = "1";
  const codexArchiveDeleteAllVersion = "2";
  const codexConversationTimelineVersion = "2";
  const codexThreadScrollVersion = "1";
  const codexPlusVersion = "1.0.7";
  const codexPlusMenuVersion = `7:${codexPlusVersion}`;
  const codexPlusTriggerVersion = `6:${codexPlusVersion}`;
  const codexPlusSettingsKey = "codexPlusSettings";
  const codexThreadScrollKey = "codexThreadScroll";
  const codexThreadScrollMaxEntries = 120;
  const codexThreadScrollSaveThrottleMs = 120;
  const codexThreadScrollRestoreWindowMs = 3200;
  const codexThreadScrollRestoreDelaysMs = [0, 80, 220, 500, 1000, 1800, 2800];
  const codexThreadScrollUserIntentWindowMs = 1200;
  const codexThreadScrollProgrammaticGuardVersion = "dispatcher:2";
  const codexThreadScrollRouteHooksVersion = "dispatcher:2";
  const codexThreadScrollListenerVersion = "4";
  const codexThreadScrollUserIntentVersion = "dispatcher:2";
  window.__codexProjectMoveRuntimeId = (window.__codexProjectMoveRuntimeId || 0) + 1;
  const codexProjectMoveRuntimeId = window.__codexProjectMoveRuntimeId;
  clearTimeout(window.__codexProjectMoveProjectionTimer);
  clearTimeout(window.__codexProjectMoveChatsSortTimer);
  window.__codexProjectMoveProjectionTimer = null;
  window.__codexProjectMoveChatsSortTimer = null;
  clearTimeout(window.__codexThreadScrollSaveTimer);
  window.__codexThreadScrollSaveTimer = null;
  (window.__codexThreadScrollRestoreTimers || []).forEach((timer) => clearTimeout(timer));
  window.__codexThreadScrollRestoreTimers = [];
  (window.__codexThreadScrollSyncTimers || []).forEach((timer) => clearTimeout(timer));
  window.__codexThreadScrollSyncTimers = [];
  window.__codexThreadScrollRestoreRevision = (window.__codexThreadScrollRestoreRevision || 0) + 1;
  window.__codexThreadScrollSyncRevision = (window.__codexThreadScrollSyncRevision || 0) + 1;
  window.__codexConversationTimelineNodeCounter = window.__codexConversationTimelineNodeCounter || 0;
  const selectors = {
    sidebarThread: "[data-app-action-sidebar-thread-id]",
    threadTitle: "[data-thread-title]",
    appHeader: ".app-header-tint",
    nativeMenuBar: ".flex.items-center.gap-0\\.5, [class*=\"flex items-center gap-0.5\"]",
    archiveNav: 'button[aria-label="已归档对话"], button[aria-label="Archived conversations"]',
    disabledInstallButton: 'button:disabled.w-full.justify-center, [role="button"][aria-disabled="true"].cursor-not-allowed',
    pluginNavButton: 'nav[role="navigation"] button.h-token-nav-row.w-full',
    pluginSvgPath: 'svg path[d^="M7.94562 14.0277"]',
  };

  function installStyle() {
    const existingStyle = document.getElementById(styleId);
    if (existingStyle?.dataset.codexDeleteStyleVersion === codexDeleteStyleVersion) return;
    existingStyle?.remove();
    const style = document.createElement("style");
    style.id = styleId;
    style.dataset.codexDeleteStyleVersion = codexDeleteStyleVersion;
    style.textContent = `
      .${actionGroupClass} {
        position: absolute;
        right: 28px;
        top: 50%;
        transform: translateY(-50%);
        z-index: 20;
        opacity: 0;
        display: inline-flex;
        align-items: center;
        gap: 6px;
      }
      .${actionButtonClass},
      .codex-archive-row-button {
        border: 1px solid #ef4444;
        border-radius: 6px;
        background: #f3f4f6;
        color: #374151;
        font-size: 12px;
        line-height: 16px;
        padding: 1px 6px;
        cursor: pointer;
      }
      .codex-archive-row-button {
        border-radius: 7px;
        font: 12px system-ui, sans-serif;
        line-height: 16px;
        padding: 3px 8px;
      }
      .${buttonClass},
      .codex-archive-row-button.${buttonClass} {
        border-color: #ef4444;
        background: #fee2e2;
        color: #991b1b;
      }
      .${exportButtonClass},
      .codex-archive-row-button.${exportButtonClass} {
        border-color: #93c5fd;
        background: #dbeafe;
        color: #1d4ed8;
      }
      .${projectMoveButtonClass} {
        border-color: #10a37f;
        background: #d1fae5;
        color: #065f46;
      }
      .${zedRemoteButtonClass} {
        border: 1px solid #10a37f;
        border-radius: 7px;
        background: #d1fae5;
        color: #065f46;
        font: 12px system-ui, sans-serif;
        line-height: 16px;
        margin-left: 6px;
        padding: 2px 7px;
        cursor: pointer;
      }
      .${zedRemoteButtonClass}:hover,
      .${zedRemoteButtonClass}:focus-visible {
        background: #a7f3d0;
        outline: none;
      }
      .${zedRemoteOpenInMenuItemClass} {
        cursor: pointer;
      }
      .codex-zed-open-in-menu-icon {
        width: 18px;
        height: 18px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        object-fit: contain;
      }
      .${zedRemoteToastClass} {
        position: fixed;
        right: 18px;
        bottom: 58px;
        z-index: 2147483000;
        max-width: min(420px, calc(100vw - 36px));
        border-radius: 8px;
        background: #111827;
        color: #ffffff;
        font: 13px system-ui, sans-serif;
        line-height: 18px;
        padding: 10px 12px;
        box-shadow: 0 8px 30px rgba(0,0,0,.25);
        pointer-events: none;
      }
      [data-codex-delete-row="true"]:hover .${actionGroupClass} { opacity: 1; }
      [data-codex-delete-row="true"].codex-archive-confirm-visible .${actionGroupClass} { right: 66px; }
      .${projectMoveOverlayClass} {
        position: fixed;
        inset: 0;
        z-index: 2147483200;
        background: rgba(15,23,42,.28);
      }
      .codex-project-move-panel {
        position: fixed;
        width: min(360px, calc(100vw - 32px));
        max-height: min(520px, calc(100vh - 32px));
        overflow: hidden;
        border: 1px solid rgba(15,23,42,.14);
        border-radius: 10px;
        background: #ffffff;
        color: #111827;
        font: 13px system-ui, sans-serif;
        box-shadow: 0 18px 60px rgba(15,23,42,.25);
      }
      .codex-project-move-header { border-bottom: 1px solid #e5e7eb; padding: 10px 12px; }
      .codex-project-move-title { font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-project-move-list { max-height: min(440px, calc(100vh - 110px)); overflow-y: auto; padding: 6px; }
      .codex-project-move-item {
        display: block;
        width: 100%;
        border: 0;
        border-radius: 7px;
        background: transparent;
        color: #111827;
        padding: 8px 9px;
        text-align: left;
        cursor: pointer;
      }
      .codex-project-move-item:hover,
      .codex-project-move-item:focus-visible { background: #f3f4f6; outline: none; }
      .codex-project-move-item-title { font-weight: 550; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-project-move-item-path { margin-top: 2px; color: #6b7280; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-project-move-empty { padding: 18px 12px; color: #6b7280; text-align: center; }
      .codex-project-move-hidden { display: none !important; }
      [data-codex-project-move-injected-list="true"] { display: flex; flex-direction: column; }
      .codex-archive-delete-all {
        border: 1px solid #ef4444;
        border-radius: 7px;
        background: #fee2e2;
        color: #991b1b;
        font: 12px system-ui, sans-serif;
        line-height: 16px;
        padding: 3px 8px;
        cursor: pointer;
      }
      .codex-archive-action-bar {
        position: fixed;
        right: 28px;
        top: 86px;
        z-index: 2147482999;
        box-shadow: 0 8px 24px rgba(0,0,0,.18);
      }
      .codex-delete-toast {
        position: fixed;
        right: 18px;
        bottom: 18px;
        z-index: 2147483000;
        padding: 10px 12px;
        border-radius: 8px;
        background: #111827;
        color: white;
        font: 13px system-ui, sans-serif;
        box-shadow: 0 8px 30px rgba(0,0,0,.25);
        pointer-events: none;
      }
      .codex-delete-toast button { margin-left: 10px; pointer-events: auto; }
      .codex-delete-confirm-overlay {
        position: fixed;
        inset: 0;
        z-index: 2147483200;
        display: flex;
        align-items: center;
        justify-content: center;
        background: rgba(15,23,42,.28);
      }
      .codex-delete-confirm-content {
        width: min(420px, calc(100vw - 48px));
        border: 1px solid rgba(15,23,42,.12);
        border-radius: 12px;
        background: #ffffff;
        color: #111827;
        font: 14px system-ui, sans-serif;
        box-shadow: 0 24px 80px rgba(15,23,42,.22);
        padding: 18px;
      }
      .codex-delete-confirm-title { font-size: 16px; font-weight: 650; }
      .codex-delete-confirm-message { margin-top: 8px; color: #4b5563; line-height: 1.45; }
      .codex-delete-confirm-actions {
        display: flex;
        justify-content: flex-end;
        gap: 10px;
        margin-top: 18px;
      }
      .codex-delete-confirm-actions button {
        border: 1px solid #d1d5db;
        border-radius: 7px;
        padding: 6px 12px;
        background: #ffffff;
        color: #111827;
        font: 13px system-ui, sans-serif;
        cursor: pointer;
      }
      .codex-delete-confirm-actions [data-codex-delete-confirm="true"] {
        border-color: #ef4444;
        background: #dc2626;
        color: #ffffff;
      }
      #${codexPlusMenuId}.${codexPlusMenuFloatingClass} {
        position: fixed;
        top: var(--codex-plus-menu-top, 0);
        right: var(--codex-plus-menu-right, 140px);
        left: auto;
        z-index: 2147483645;
        height: var(--codex-plus-menu-height, 30px);
        color: #d1d5db;
        font: 13px system-ui, sans-serif;
        text-align: right;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      #${codexPlusMenuId} {
        display: inline-flex;
        align-items: center;
        height: 100%;
        flex: 0 0 auto;
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-trigger {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 4px;
        border: 0;
        background: transparent;
        color: inherit;
        font: inherit;
        height: 100%;
        line-height: 1;
        padding: 0 8px;
        cursor: pointer;
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-overlay {
        position: fixed;
        inset: 0;
        z-index: 2147483646;
        display: flex;
        align-items: center;
        justify-content: center;
        background: rgba(0,0,0,.45);
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-content {
        width: min(520px, calc(100vw - 48px));
        max-height: min(680px, calc(100vh - 40px));
        display: flex;
        flex-direction: column;
        overflow: hidden;
        border: 1px solid rgba(255,255,255,.12);
        border-radius: 18px;
        background: #2b2b2b;
        color: #f3f4f6;
        font: 14px system-ui, sans-serif;
        box-shadow: 0 24px 80px rgba(0,0,0,.45);
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-content[data-codex-plus-active-tab="support"] { width: min(820px, calc(100vw - 48px)); }
      .codex-plus-modal-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 16px 20px 8px;
        flex: 0 0 auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-title { display: flex; align-items: center; gap: 8px; font-size: 18px; font-weight: 650; }
      .codex-plus-backend-indicator { width: 9px; height: 9px; border-radius: 999px; background: #a1a1aa; display: inline-block; }
      .codex-plus-backend-indicator[data-status="ok"] { background: #34d399; box-shadow: 0 0 8px rgba(52,211,153,.75); }
      .codex-plus-backend-indicator[data-status="failed"] { background: #ef4444; box-shadow: 0 0 8px rgba(239,68,68,.75); }
      .codex-plus-backend-indicator[data-status="checking"] { background: #fbbf24; }
      .codex-plus-modal-close {
        border: 0;
        background: transparent;
        color: #d1d5db;
        font-size: 20px;
        cursor: pointer;
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-body {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
        overscroll-behavior: contain;
        scrollbar-gutter: stable;
        padding: 4px 20px 16px;
        scrollbar-width: thin;
        scrollbar-color: rgba(255,255,255,.28) transparent;
      }
      .codex-plus-modal-body::-webkit-scrollbar { width: 10px; }
      .codex-plus-modal-body::-webkit-scrollbar-track { background: transparent; }
      .codex-plus-modal-body::-webkit-scrollbar-thumb {
        border: 2px solid transparent;
        border-radius: 999px;
        background: rgba(255,255,255,.28);
        background-clip: padding-box;
      }
      .codex-plus-modal-body::-webkit-scrollbar-thumb:hover { background: rgba(255,255,255,.38); background-clip: padding-box; }
      .codex-plus-row {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 12px;
        padding: 10px 0;
        border-top: 1px solid rgba(255,255,255,.1);
      }
      .codex-plus-row:first-child { border-top: 0; }
      .codex-plus-row-title { font-weight: 550; line-height: 1.35; }
      .codex-plus-row-description { margin-top: 2px; color: #a1a1aa; font-size: 12px; line-height: 1.4; }
      .codex-plus-toggle {
        width: 42px;
        height: 24px;
        border: 0;
        border-radius: 999px;
        background: #52525b;
        padding: 2px;
      }
      .codex-plus-toggle span {
        display: block;
        width: 20px;
        height: 20px;
        border-radius: 999px;
        background: white;
        transition: transform .12s ease;
      }
      .codex-plus-toggle,
      .codex-plus-action-button,
      .codex-plus-issue-button,
      .codex-plus-backend-status {
        flex-shrink: 0;
        align-self: center;
      }
      .codex-plus-toggle[data-enabled="true"] { background: #10a37f; }
      .codex-plus-toggle[data-enabled="true"] span { transform: translateX(18px); }
      .codex-plus-about { color: #a1a1aa; line-height: 1.5; }
      .codex-plus-tabs { display: flex; gap: 8px; padding: 0 20px 6px; flex: 0 0 auto; }
      .codex-plus-tab-button { border: 1px solid rgba(255,255,255,.14); border-radius: 999px; background: transparent; color: #d1d5db; font: 12px system-ui, sans-serif; padding: 5px 10px; }
      .codex-plus-tab-button[data-active="true"] { background: #10a37f; color: white; border-color: #10a37f; }
      .codex-plus-panel[hidden] { display: none; }
      .codex-plus-action-button,
      .codex-plus-issue-button { border: 1px solid rgba(255,255,255,.18); border-radius: 7px; background: #3f3f46; color: #f3f4f6; font: 12px system-ui, sans-serif; padding: 6px 8px; }
      .codex-plus-backend-status { display: grid; gap: 4px; min-width: 132px; justify-items: end; }
      .codex-plus-backend-label { color: #a1a1aa; font-size: 12px; }
      .codex-plus-backend-label[data-status="ok"] { color: #34d399; }
      .codex-plus-backend-label[data-status="failed"] { color: #f87171; }
      .codex-plus-backend-repair { border: 1px solid rgba(255,255,255,.18); border-radius: 7px; background: #3f3f46; color: #f3f4f6; font: 12px system-ui, sans-serif; padding: 6px 8px; }
      .codex-plus-backend-repair[hidden] { display: none; }
      .codex-plus-model-compat-warning { margin-top: 6px; color: #fbbf24; font-size: 12px; line-height: 1.45; }
      .codex-plus-user-script-warning { margin-top: 4px; color: #fbbf24; font-size: 12px; }
      .codex-plus-user-script-dirs { margin-top: 6px; color: #a1a1aa; font-size: 11px; line-height: 1.4; word-break: break-all; }
      .codex-plus-user-script-list { margin-top: 8px; display: grid; gap: 6px; }
      .codex-plus-user-script-item { display: flex; align-items: center; justify-content: space-between; gap: 8px; border: 1px solid rgba(255,255,255,.08); border-radius: 8px; padding: 6px 8px; }
      .codex-plus-user-script-name { font-size: 12px; }
      .codex-plus-user-script-meta { margin-top: 2px; color: #a1a1aa; font-size: 11px; }
      .codex-plus-user-script-error { margin-top: 2px; color: #f87171; font-size: 11px; word-break: break-all; }
      .codex-plus-user-script-actions { display: grid; justify-items: end; gap: 8px; min-width: 120px; }
      .codex-plus-user-script-reload { border: 1px solid rgba(255,255,255,.18); border-radius: 7px; background: #3f3f46; color: #f3f4f6; font: 12px system-ui, sans-serif; padding: 6px 8px; }
      .codex-plus-sponsor-text { color: #d1d5db; font-size: 13px; line-height: 1.55; margin: 4px 0 12px; }
      .codex-plus-ad-section { display: grid; gap: 10px; margin-top: 12px; }
      .codex-plus-ad-section:first-of-type { margin-top: 0; }
      .codex-plus-ad-section-title { color: #f8fafc; font-size: 15px; margin: 0; }
      .codex-plus-ad-list { display: grid; gap: 14px; }
      .codex-plus-ad-card { border: 1px solid rgba(96,165,250,.26); border-radius: 16px; background: linear-gradient(135deg, rgba(37,99,235,.18), rgba(255,255,255,.05)); box-shadow: 0 14px 36px rgba(0,0,0,.22); }
      .codex-plus-ad-content { padding: 14px; }
      .codex-plus-ad-title { margin: 0; color: #f8fafc; font-size: 17px; line-height: 1.35; }
      .codex-plus-ad-description { margin: 6px 0 10px; color: #dbeafe; font-size: 13px; line-height: 1.55; }
      .codex-plus-ad-highlights { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px; }
      .codex-plus-ad-highlights span { border: 1px solid rgba(255,255,255,.14); border-radius: 999px; background: rgba(255,255,255,.08); color: #f3f4f6; font-size: 12px; padding: 4px 8px; }
      .codex-plus-ad-link { display: inline-flex; align-items: center; justify-content: center; border-radius: 9px; background: #2563eb; color: #ffffff; font-size: 13px; font-weight: 650; text-decoration: none; padding: 8px 12px; }
      .codex-plus-ad-empty { border: 1px dashed rgba(255,255,255,.16); border-radius: 12px; color: #9ca3af; font-size: 13px; padding: 12px; text-align: center; }
      .codex-plus-sponsor-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
      .codex-plus-sponsor-card { border: 1px solid rgba(255,255,255,.1); border-radius: 12px; padding: 10px; background: rgba(255,255,255,.04); text-align: center; }
      .codex-plus-sponsor-card-title { color: #f3f4f6; font-size: 13px; margin-bottom: 8px; }
      .codex-plus-sponsor-qr { display: block; width: 100%; max-width: 340px; border-radius: 8px; margin: 0 auto; background: white; }
      .${timelineClass} {
        position: fixed;
        top: calc(72px + 12px);
        right: 12px;
        bottom: calc(28px + 12px);
        width: 24px;
        z-index: 2147482500;
        pointer-events: none;
      }
      .${timelineTrackClass} {
        position: absolute;
        top: 0;
        bottom: 0;
        left: 50%;
        width: 2px;
        transform: translateX(-50%);
        border-radius: 999px;
        background: rgba(209, 213, 219, .55);
      }
      .${timelineMarkerClass} {
        position: absolute;
        left: 50%;
        width: 12px;
        height: 12px;
        border: 0;
        border-radius: 999px;
        transform: translate(-50%, -50%);
        background: #d1d5db;
        cursor: pointer;
        pointer-events: auto;
        box-shadow: 0 0 0 2px rgba(255, 255, 255, .92);
      }
      .${timelineMarkerClass}:hover,
      .${timelineMarkerClass}:focus-visible,
      .${timelineMarkerClass}.codex-conversation-timeline-marker-active {
        background: #8b8b8b;
        outline: none;
      }
      .${timelineTooltipClass} {
        position: absolute;
        right: 20px;
        top: 50%;
        display: block;
        box-sizing: border-box;
        width: max-content;
        max-width: min(320px, calc(100vw - 72px));
        transform: translateY(-50%);
        border-radius: 8px;
        background: rgba(80, 80, 80, .92);
        color: #ffffff;
        font: 600 13px system-ui, sans-serif;
        line-height: 18px;
        padding: 10px 12px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        box-shadow: 0 8px 24px rgba(0, 0, 0, .18);
        opacity: 0;
        visibility: hidden;
        pointer-events: none;
      }
      .${timelineMarkerClass}:hover .${timelineTooltipClass},
      .${timelineMarkerClass}:focus-visible .${timelineTooltipClass} {
        opacity: 1;
        visibility: visible;
        z-index: 2147482501;
      }
      .${timelineTargetClass} {
        animation: codex-conversation-timeline-pulse 1.2s ease-out;
      }
      @keyframes codex-conversation-timeline-pulse {
        0% { box-shadow: 0 0 0 0 rgba(16, 163, 127, .35); }
        100% { box-shadow: 0 0 0 14px rgba(16, 163, 127, 0); }
      }
    `;
    document.documentElement.appendChild(style);
  }

  function defaultCodexPlusSettings() {
    return {
      pluginEntryUnlock: true,
      forcePluginInstall: true,
      sessionDelete: true,
      markdownExport: true,
      projectMove: true,
      conversationTimeline: true,
      zedRemoteOpen: true,
      threadScrollRestore: true,
      nativeMenuPlacement: true,
      modelWhitelistUnlock: true,
    };
  }

  function codexPlusSettings() {
    try {
      return { ...defaultCodexPlusSettings(), ...JSON.parse(localStorage.getItem(codexPlusSettingsKey) || "{}") };
    } catch {
      return defaultCodexPlusSettings();
    }
  }

  function setCodexPlusSetting(key, value) {
    const next = { ...codexPlusSettings(), [key]: value };
    localStorage.setItem(codexPlusSettingsKey, JSON.stringify(next));
    if (key === "threadScrollRestore" && !value) {
      clearThreadScrollRestoreTimers();
      clearThreadScrollSyncTimers();
      window.__codexThreadScrollRestoreRevision = (window.__codexThreadScrollRestoreRevision || 0) + 1;
      window.__codexThreadScrollSyncRevision = (window.__codexThreadScrollSyncRevision || 0) + 1;
      clearThreadScrollRestoreLock();
      bindThreadScrollListener(null);
    }
    renderCodexPlusMenu();
    scan();
  }

  function renderCodexPlusMenu() {
    document.querySelectorAll(".codex-plus-toggle[data-codex-plus-setting]").forEach((button) => {
      const key = button.getAttribute("data-codex-plus-setting");
      button.dataset.enabled = String(!!codexPlusSettings()[key]);
    });
    renderCodexModelCompatibilityWarning();
  }

  let codexPlusBackendSettings = { providerSyncEnabled: false };

  async function loadBackendSettings() {
    try {
      const settings = await postJson("/settings/get", {});
      codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
      refreshCodexPlusBackendToggles();
    } catch (_) {
      refreshCodexPlusBackendToggles();
    }
  }

  async function setBackendSetting(key, value) {
    codexPlusBackendSettings = { ...codexPlusBackendSettings, [key]: value };
    refreshCodexPlusBackendToggles();
    try {
      const settings = await postJson("/settings/set", { [key]: value });
      codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
    } finally {
      refreshCodexPlusBackendToggles();
    }
  }

  function refreshCodexPlusBackendToggles() {
    document.querySelectorAll(".codex-plus-toggle[data-codex-backend-setting]").forEach((button) => {
      const key = button.getAttribute("data-codex-backend-setting");
      button.dataset.enabled = String(!!codexPlusBackendSettings[key]);
    });
  }

  let codexPlusUserScripts = { enabled: true, builtin_dir: "", user_dir: "", scripts: [] };
  let codexPlusBackendStatus = { status: "checking", message: "正在检查后端…" };

  function renderBackendStatus() {
    const status = codexPlusBackendStatus.status || "failed";
    const label = document.querySelector("[data-codex-backend-status]");
    if (label) {
      label.dataset.status = status;
      label.textContent = codexPlusBackendStatus.message || (status === "ok" ? "后端已连接" : "后端已断开");
    }
    document.querySelectorAll("[data-codex-backend-indicator]").forEach((indicator) => {
      indicator.dataset.status = status;
      indicator.title = status === "ok" ? "后端已连接" : status === "checking" ? "正在检查后端" : "后端已断开";
    });
    const repair = document.querySelector("[data-codex-backend-repair]");
    if (repair) repair.hidden = status === "ok" || status === "checking";
  }

  function withBackendTimeout(request) {
    return Promise.race([
      request,
      new Promise((resolve) => setTimeout(() => resolve({ status: "failed", message: "后端已断开" }), 2000)),
    ]);
  }

  async function checkBackendStatus() {
    codexPlusBackendStatus = await withBackendTimeout(postJson("/backend/status", {}));
    renderBackendStatus();
  }

  async function repairBackend() {
    codexPlusBackendStatus = { status: "checking", message: "正在修复后端…" };
    renderBackendStatus();
    try {
      codexPlusBackendStatus = await postJson("/backend/repair", {});
    } catch (error) {
      codexPlusBackendStatus = { status: "failed", message: "后端修复失败" };
    }
    renderBackendStatus();
  }

  function scheduleBackendHeartbeat() {
    clearInterval(window.__codexPlusBackendHeartbeat);
    window.__codexPlusBackendHeartbeat = setInterval(checkBackendStatus, 5000);
    checkBackendStatus();
  }

  function userScriptStatusLabel(status) {
    return { loaded: "已加载", failed: "失败", disabled: "已禁用", not_loaded: "未加载", loading: "加载中" }[status] || status || "未知";
  }

  function renderUserScripts() {
    const enabledToggle = document.querySelector("[data-codex-user-scripts-enabled]");
    if (enabledToggle) enabledToggle.dataset.enabled = String(!!codexPlusUserScripts.enabled);
    const dirs = document.querySelector("[data-codex-user-script-dirs]");
    if (dirs) dirs.textContent = `内置：${codexPlusUserScripts.builtin_dir || "未找到"}  用户：${codexPlusUserScripts.user_dir || "未找到"}`;
    const list = document.querySelector("[data-codex-user-script-list]");
    if (!list) return;
    if (!codexPlusUserScripts.scripts?.length) {
      list.textContent = "未发现用户脚本。";
      return;
    }
    list.innerHTML = codexPlusUserScripts.scripts.map((script) => `
      <div class="codex-plus-user-script-item">
        <div>
          <div class="codex-plus-user-script-name">${escapeHtml(script.name || script.key)}</div>
          <div class="codex-plus-user-script-meta">${script.source === "builtin" ? "内置" : "用户"} · ${userScriptStatusLabel(script.status)}</div>
          ${script.error ? `<div class="codex-plus-user-script-error">${escapeHtml(script.error)}</div>` : ""}
        </div>
        <button type="button" class="codex-plus-toggle" data-codex-user-script-key="${escapeHtml(script.key)}" data-enabled="${String(!!script.enabled)}"><span></span></button>
      </div>
    `).join("");
  }

  async function loadUserScripts(path = "/user-scripts/list", payload = {}) {
    const result = await postJson(path, payload);
    if (result?.scripts) {
      codexPlusUserScripts = result;
      renderUserScripts();
    }
  }

  const codexPlusAdsUrl = "/ads";
  let codexPlusAds = [];
  let codexPlusAdsLoaded = false;

  function isCodexPlusAdExpired(ad) {
    if (!ad.expires_at) return false;
    const expiresAt = Date.parse(ad.expires_at);
    return Number.isFinite(expiresAt) && expiresAt < Date.now();
  }

  function normalizeCodexPlusAds(payload) {
    if (!payload || !Array.isArray(payload.ads)) return [];
    return payload.ads.filter((ad) => {
      return ad && ["sponsor", "normal"].includes(ad.type) && ad.title && ad.description && ad.url && !isCodexPlusAdExpired(ad);
    }).map((ad) => ({
      id: String(ad.id || ad.title),
      type: ad.type,
      title: String(ad.title),
      description: String(ad.description),
      url: String(ad.url),
      expires_at: ad.expires_at ? String(ad.expires_at) : "",
      highlights: Array.isArray(ad.highlights) ? ad.highlights.map((item) => String(item)).filter(Boolean) : [],
    }));
  }

  function renderCodexPlusAdGroup(type, emptyText) {
    const ads = codexPlusAds.filter((ad) => ad.type === type);
    if (!ads.length) return `<div class="codex-plus-ad-empty">${escapeHtml(emptyText)}</div>`;
    return ads.map((ad) => `
      <article class="codex-plus-ad-card">
        <div class="codex-plus-ad-content">
          <h3 class="codex-plus-ad-title">${escapeHtml(ad.title)}</h3>
          <p class="codex-plus-ad-description">${escapeHtml(ad.description)}</p>
          <div class="codex-plus-ad-highlights">
            ${ad.highlights.map((item) => `<span>${escapeHtml(item)}</span>`).join("")}
          </div>
          <a class="codex-plus-ad-link" href="${escapeHtml(ad.url)}" target="_blank" rel="noreferrer">访问 ${escapeHtml(new URL(ad.url).hostname)}</a>
        </div>
      </article>
    `).join("");
  }

  function renderCodexPlusAds() {
    if (!codexPlusAdsLoaded) return `<div class="codex-plus-ad-empty">推荐内容加载中…</div>`;
    if (!codexPlusAds.length) return `<div class="codex-plus-ad-empty">暂无推荐内容。</div>`;
    return `
      <section class="codex-plus-ad-section">
        <h3 class="codex-plus-ad-section-title">赞助商推荐</h3>
        <div class="codex-plus-ad-list">${renderCodexPlusAdGroup("sponsor", "暂无赞助商推荐。")}</div>
      </section>
      <section class="codex-plus-ad-section">
        <h3 class="codex-plus-ad-section-title">普通推荐</h3>
        <div class="codex-plus-ad-list">${renderCodexPlusAdGroup("normal", "暂无普通推荐。")}</div>
      </section>
    `;
  }

  async function fetchCodexPlusAds() {
    try {
      codexPlusAds = normalizeCodexPlusAds(await postJson("/ads", {}));
    } catch (_) {
      codexPlusAds = [];
    } finally {
      codexPlusAdsLoaded = true;
      const panel = document.querySelector('[data-codex-plus-panel="sponsor"] .codex-plus-ad-remote');
      if (panel) panel.innerHTML = renderCodexPlusAds();
    }
  }

  function selectCodexPlusTab(tab) {
    document.querySelectorAll(".codex-plus-modal-content").forEach((modal) => {
      modal.dataset.codexPlusActiveTab = tab;
    });
    document.querySelectorAll("[data-codex-plus-tab]").forEach((button) => {
      button.dataset.active = String(button.getAttribute("data-codex-plus-tab") === tab);
    });
    document.querySelectorAll("[data-codex-plus-panel]").forEach((panel) => {
      panel.hidden = panel.getAttribute("data-codex-plus-panel") !== tab;
    });
    if (tab === "userScripts") loadUserScripts();
  }

  function openCodexPlusModal() {
    document.querySelectorAll(".codex-plus-modal-overlay").forEach((node) => node.remove());
    document.querySelectorAll('[data-codex-plus-dialog="true"]').forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = "codex-plus-modal-overlay";
    overlay.innerHTML = `
      <div class="codex-plus-modal-content" role="dialog" aria-modal="true" aria-label="Codex++">
        <div class="codex-plus-modal-header">
          <div class="codex-plus-modal-title"><span class="codex-plus-backend-indicator" data-codex-backend-indicator="true" data-status="checking"></span><span>Codex++ ${codexPlusVersion}</span></div>
          <button type="button" class="codex-plus-modal-close" aria-label="关闭">×</button>
        </div>
        <div class="codex-plus-tabs" role="tablist" aria-label="Codex++">
          <button type="button" class="codex-plus-tab-button" data-codex-plus-tab="home" data-active="true">主页</button>
          <button type="button" class="codex-plus-tab-button" data-codex-plus-tab="userScripts" data-active="false">用户脚本</button>
          <button type="button" class="codex-plus-tab-button" data-codex-plus-tab="sponsor" data-active="false">推荐内容</button>
          <button type="button" class="codex-plus-tab-button" data-codex-plus-tab="support" data-active="false">请作者喝咖啡</button>
        </div>
        <div class="codex-plus-modal-body">
          <div class="codex-plus-panel" data-codex-plus-panel="home">
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">后端连接</div><div class="codex-plus-row-description">每 5 秒检查一次 launcher 后端状态；断开时可尝试修复后端运行。</div></div>
              <div class="codex-plus-backend-status">
                <div class="codex-plus-backend-label" data-codex-backend-status="true" data-status="checking">正在检查后端…</div>
                <button type="button" class="codex-plus-backend-repair" data-codex-backend-repair="true" hidden>修复后端运行</button>
              </div>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">插件选项解锁</div><div class="codex-plus-row-description">让 API Key 模式显示并启用插件入口。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="pluginEntryUnlock"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">特殊插件强制安装</div><div class="codex-plus-row-description">解除 App unavailable / 应用不可用导致的前端安装禁用。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="forcePluginInstall"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">会话删除</div><div class="codex-plus-row-description">在会话列表悬停显示删除按钮，并支持撤销。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="sessionDelete"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">Markdown 导出</div><div class="codex-plus-row-description">在会话列表显示导出按钮，按本地 rollout 导出带时间戳的 Markdown。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="markdownExport"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">会话项目移动</div><div class="codex-plus-row-description">在会话列表悬停显示移动按钮，可移动到普通对话或其他本地项目。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="projectMove"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">对话 Timeline</div><div class="codex-plus-row-description">在对话右侧显示用户提问时间线，悬停查看摘要，点击跳转。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="conversationTimeline"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">Zed Remote open</div><div class="codex-plus-row-description">Open supported remote SSH file references in Zed without patching Codex.app.</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="zedRemoteOpen"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">切换对话保留位置</div><div class="codex-plus-row-description">开启后在不同 thread 之间切换时恢复到上一次浏览位置，不再自动跳到底部。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="threadScrollRestore"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">Provider 同步</div><div class="codex-plus-row-description">切换供应商（model_provider）时不丢任何历史会话，避免历史对话因为供应商切换而消失。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-backend-setting="providerSyncEnabled"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">原生菜单栏位置</div><div class="codex-plus-row-description">把 Codex++ 菜单插入顶部原生菜单栏；默认关闭以避免页面重渲染冲突。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="nativeMenuPlacement"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">模型白名单解锁</div><div class="codex-plus-row-description">从环境变量和 Codex config.toml 中的中转站 /v1/models 拉取模型，并补进模型选择列表。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="modelWhitelistUnlock"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">打开 DevTools</div><div class="codex-plus-row-description">打开当前 Codex 页面开发者工具，方便查看用户脚本报错。</div></div>
              <button type="button" class="codex-plus-action-button" data-codex-open-devtools="true">打开 DevTools</button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">关于 Codex++</div><div class="codex-plus-about">Codex++ 是通过外部 launcher 注入的增强菜单，不修改 Codex App 原始安装文件。<br>GitHub: <a href="https://github.com/BigPizzaV3/CodexPlusPlus" target="_blank" rel="noreferrer">https://github.com/BigPizzaV3/CodexPlusPlus</a></div></div>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">提出问题</div><div class="codex-plus-row-description">打开 GitHub Issues 反馈问题或建议。</div></div>
              <button type="button" class="codex-plus-issue-button" data-codex-plus-issue="true">提出问题</button>
            </div>
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="userScripts" hidden>
            <div class="codex-plus-row" data-codex-user-scripts-section="true">
              <div>
                <div class="codex-plus-row-title">用户脚本</div>
                <div class="codex-plus-row-description">启用用户脚本：自动加载内置目录和用户配置目录中的 .js 文件。</div>
                <div class="codex-plus-user-script-warning">禁用后需重载页面或重启 Codex++ 才能完全移除已执行效果。</div>
                <div class="codex-plus-user-script-dirs" data-codex-user-script-dirs="true">正在读取脚本目录…</div>
                <div class="codex-plus-user-script-list" data-codex-user-script-list="true">正在读取用户脚本…</div>
              </div>
              <div class="codex-plus-user-script-actions">
                <button type="button" class="codex-plus-toggle" data-codex-user-scripts-enabled="true"><span></span></button>
                <button type="button" class="codex-plus-user-script-reload" data-codex-user-scripts-reload="true">重新加载用户脚本</button>
              </div>
            </div>
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="sponsor" hidden>
            <div class="codex-plus-sponsor-text">推荐内容分为赞助商推荐和普通推荐。赞助商推荐来自支持 Codex++ 继续维护的合作方；普通推荐用于展示适合 Codex 用户的服务与信息。</div>
            <div class="codex-plus-ad-remote">
              ${renderCodexPlusAds()}
            </div>
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="support" hidden>
            <div class="codex-plus-sponsor-text">如果 Codex++ 帮到了你，可以请我喝杯咖啡，或者随手赞赏支持一下继续维护。</div>
            <div class="codex-plus-sponsor-grid">
              <div class="codex-plus-sponsor-card">
                <div class="codex-plus-sponsor-card-title">支付宝</div>
                <img class="codex-plus-sponsor-qr" src="${window.__CODEX_PLUS_SPONSOR_IMAGES__?.alipay || `${helperBase}/assets/sponsor-alipay.jpg`}" alt="支付宝赞赏码">
              </div>
              <div class="codex-plus-sponsor-card">
                <div class="codex-plus-sponsor-card-title">微信</div>
                <img class="codex-plus-sponsor-qr" src="${window.__CODEX_PLUS_SPONSOR_IMAGES__?.wechat || `${helperBase}/assets/sponsor-wechat.jpg`}" alt="微信赞赏码">
              </div>
            </div>
          </div>
        </div>
      </div>
    `;
    const closeButton = overlay.querySelector(".codex-plus-modal-close");
    closeButton?.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      overlay.remove();
    }, true);
    overlay.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      if (event.target === overlay || target?.closest(".codex-plus-modal-close")) {
        overlay.remove();
        return;
      }
      const tabButton = target?.closest("[data-codex-plus-tab]");
      if (tabButton) {
        selectCodexPlusTab(tabButton.getAttribute("data-codex-plus-tab"));
        return;
      }
      if (target?.closest("[data-codex-open-devtools]")) {
        postJson("/devtools/open", {});
        return;
      }
      if (target?.closest("[data-codex-backend-repair]")) {
        repairBackend();
        return;
      }
      const issueButton = target?.closest("[data-codex-plus-issue]");
      if (issueButton) {
        const issueUrl = "https://github.com/BigPizzaV3/CodexPlusPlus/issues";
        window.open(issueUrl, "_blank");
        return;
      }
      const userScriptsEnabled = target?.closest("[data-codex-user-scripts-enabled]");
      if (userScriptsEnabled) {
        loadUserScripts("/user-scripts/set-enabled", { enabled: userScriptsEnabled.dataset.enabled !== "true" });
        return;
      }
      const userScriptToggle = target?.closest("[data-codex-user-script-key]");
      if (userScriptToggle) {
        loadUserScripts("/user-scripts/set-script-enabled", { key: userScriptToggle.getAttribute("data-codex-user-script-key"), enabled: userScriptToggle.dataset.enabled !== "true" });
        return;
      }
      if (target?.closest("[data-codex-user-scripts-reload]")) {
        loadUserScripts("/user-scripts/reload", {});
        return;
      }
      const toggle = target?.closest("[data-codex-plus-setting]");
      if (toggle) {
        const key = toggle.getAttribute("data-codex-plus-setting");
        setCodexPlusSetting(key, !codexPlusSettings()[key]);
        return;
      }
      const backendToggle = target?.closest("[data-codex-backend-setting]");
      if (backendToggle) {
        const key = backendToggle.getAttribute("data-codex-backend-setting");
        setBackendSetting(key, !codexPlusBackendSettings[key]);
        return;
      }
    }, true);
    document.body.appendChild(overlay);
    if (!codexPlusAdsLoaded) fetchCodexPlusAds();
    selectCodexPlusTab("home");
    renderCodexPlusMenu();
    refreshCodexPlusBackendToggles();
    renderBackendStatus();
    loadBackendSettings();
    loadUserScripts();
  }

  function findNativeMenuInsertionPoint() {
    if (!codexPlusSettings().nativeMenuPlacement) return null;
    const header = document.querySelector(selectors.appHeader);
    const menuBar = header?.querySelector(selectors.nativeMenuBar);
    if (!menuBar) return null;
    const buttons = Array.from(menuBar.querySelectorAll("button")).filter((button) => !button.closest(`#${codexPlusMenuId}`));
    return { parent: menuBar, before: buttons[buttons.length - 1]?.nextSibling || null, nativeButtonClass: buttons[buttons.length - 1]?.className || "" };
  }

  function pageHasCodexAppChrome() {
    return !!(
      document.querySelector(selectors.appHeader) ||
      document.querySelector("header") ||
      document.querySelector(selectors.sidebarThread) ||
      document.querySelector('[data-app-action-sidebar-section-heading="Chats"], [data-app-action-sidebar-section-heading="Projects"]') ||
      document.querySelector(selectors.pluginNavButton)
    );
  }

  function removeDuplicateCodexPlusMenus(keep) {
    document.querySelectorAll(`#${codexPlusMenuId}, [data-codex-plus-menu="true"]`).forEach((node) => {
      if (node !== keep) node.remove();
    });
    Array.from(document.querySelectorAll("button")).forEach((button) => {
      if (/^Codex\+\+ \d+\.\d+\.\d+/.test((button.textContent || "").trim()) && !button.closest(`#${codexPlusMenuId}`)) {
        button.remove();
      }
    });
  }

  function setCodexPlusTriggerLabel(trigger) {
    if (!trigger) return;
    const label = `Codex++ ${codexPlusVersion}`;
    const textNode = Array.from(trigger.childNodes).find((node) => node.nodeType === Node.TEXT_NODE);
    if (textNode) {
      textNode.nodeValue = label;
    } else {
      trigger.appendChild(document.createTextNode(label));
    }
  }

  function configureCodexPlusTrigger(menu, trigger, nativeButtonClass) {
    if (!trigger) return;
    if (nativeButtonClass) trigger.className = nativeButtonClass;
    setCodexPlusTriggerLabel(trigger);
    if (trigger.dataset.codexPlusTriggerInstalled === codexPlusTriggerVersion) return;
    trigger.dataset.codexPlusTriggerInstalled = codexPlusTriggerVersion;
    trigger.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      openCodexPlusModal();
    }, true);
  }

  function numericCssValue(value) {
    const parsed = Number.parseFloat(value || "");
    return Number.isFinite(parsed) ? parsed : 0;
  }

  function setCssPropIfChanged(menu, prop, value) {
    if (menu.style.getPropertyValue(prop) !== value) {
      menu.style.setProperty(prop, value);
    }
  }

  function headerTitleRegion(header) {
    const candidates = Array.from(header?.querySelectorAll?.('[data-state], [class*="truncate"], [class*="text-base"]') || []);
    return candidates.find((node) => {
      if (!node?.querySelector?.('[data-state], button')) return false;
      if (!node.textContent?.trim()) return false;
      return node.closest?.(".draggable") || node.closest?.('[class*="grid-cols-[minmax(0,1fr)]"]');
    }) || null;
  }

  function isHeaderToolbarButton(button, header, rect) {
    if (!button || button.closest?.(`#${codexPlusMenuId}`)) return false;
    if (!(rect.width > 0 && rect.height > 0 && rect.left > window.innerWidth / 2)) return false;
    const buttonCluster = button.closest(".ms-auto.flex.shrink-0.items-center");
    if (buttonCluster && header?.contains(buttonCluster)) return true;
    const titleRegion = headerTitleRegion(header);
    if (titleRegion?.contains?.(button)) return false;
    return !!button.closest?.('[class*="ms-auto"][class*="shrink-0"][class*="items-center"]');
  }

  function updateFloatingCodexPlusMenuPosition(menu) {
    if (!menu?.classList?.contains(codexPlusMenuFloatingClass)) return;
    const header = document.querySelector(selectors.appHeader) || document.querySelector("header");
    if (!header) return;
    const toolbarButtons = Array.from(header.querySelectorAll("button"))
      .map((button) => ({ button, rect: button.getBoundingClientRect() }))
      .filter(({ button, rect }) => isHeaderToolbarButton(button, header, rect))
      .sort((left, right) => left.rect.left - right.rect.left);
    const anchor = toolbarButtons[0];
    if (anchor) {
      const measuredGap = toolbarButtons[1] ? toolbarButtons[1].rect.left - toolbarButtons[0].rect.right : 0;
      const styles = anchor.button.parentElement ? getComputedStyle(anchor.button.parentElement) : null;
      const gap = Math.max(numericCssValue(styles?.columnGap || styles?.gap), measuredGap, 0);
      setCssPropIfChanged(menu, "--codex-plus-menu-top", `${anchor.rect.top}px`);
      setCssPropIfChanged(menu, "--codex-plus-menu-height", `${anchor.rect.height}px`);
      setCssPropIfChanged(menu, "--codex-plus-menu-right", `${Math.max(0, window.innerWidth - anchor.rect.left + gap)}px`);
      return;
    }

    const headerRect = header.getBoundingClientRect();
    if (headerRect.height) {
      setCssPropIfChanged(menu, "--codex-plus-menu-top", `${headerRect.top}px`);
      setCssPropIfChanged(menu, "--codex-plus-menu-height", `${headerRect.height}px`);
    }
    menu.style.removeProperty("--codex-plus-menu-right");
  }

  function installCodexPlusMenu() {
    const existing = document.getElementById(codexPlusMenuId);
    removeDuplicateCodexPlusMenus(existing);
    if (!pageHasCodexAppChrome()) {
      existing?.remove();
      return;
    }
    let insertionPoint = findNativeMenuInsertionPoint();
    if (existing && existing.dataset.codexPlusMenuVersion !== codexPlusMenuVersion) {
      existing.remove();
      insertionPoint = findNativeMenuInsertionPoint();
    } else if (existing && insertionPoint && existing.parentElement === insertionPoint.parent) {
      configureCodexPlusTrigger(existing, existing.querySelector("button"), insertionPoint.nativeButtonClass);
      removeDuplicateCodexPlusMenus(existing);
      return;
    }
    const menu = document.createElement("div");
    menu.id = codexPlusMenuId;
    menu.dataset.codexPlusMenu = "true";
    menu.dataset.codexPlusMenuVersion = codexPlusMenuVersion;
    const trigger = document.createElement("button");
    trigger.type = "button";
    trigger.textContent = `Codex++ ${codexPlusVersion}`;
    const indicator = document.createElement("span");
    indicator.className = "codex-plus-backend-indicator";
    indicator.dataset.codexBackendIndicator = "true";
    indicator.dataset.status = codexPlusBackendStatus.status || "checking";
    trigger.prepend(indicator);
    const nativeButtonClass = insertionPoint?.nativeButtonClass || "codex-plus-trigger";
    configureCodexPlusTrigger(menu, trigger, nativeButtonClass);
    menu.appendChild(trigger);
    if (insertionPoint) {
      menu.className = "";
      const safeBefore = insertionPoint.before?.parentElement === insertionPoint.parent ? insertionPoint.before : null;
      insertionPoint.parent.insertBefore(menu, safeBefore);
    } else {
      menu.className = codexPlusMenuFloatingClass;
      document.documentElement.appendChild(menu);
      updateFloatingCodexPlusMenuPosition(menu);
    }
    removeDuplicateCodexPlusMenus(menu);
  }

  function reactFiberFrom(element) {
    const fiberKey = Object.keys(element).find((key) => key.startsWith("__reactFiber"));
    return fiberKey ? element[fiberKey] : null;
  }

  function authContextValueFrom(element) {
    for (let fiber = reactFiberFrom(element); fiber; fiber = fiber.return) {
      for (const value of [fiber.memoizedProps?.value, fiber.pendingProps?.value]) {
        if (value && typeof value === "object" && typeof value.setAuthMethod === "function" && "authMethod" in value) {
          return value;
        }
      }
    }
    return null;
  }

  function spoofChatGPTAuthMethod(element) {
    const auth = authContextValueFrom(element);
    if (!auth || auth.authMethod === "chatgpt") return false;
    auth.setAuthMethod("chatgpt");
    return true;
  }

  function pluginEntryButton() {
    const byIcon = document.querySelector(`${selectors.pluginNavButton} ${selectors.pluginSvgPath}`)?.closest("button");
    if (byIcon) return byIcon;
    return Array.from(document.querySelectorAll(selectors.pluginNavButton))
      .find((button) => /^(插件|Plugins)(\s+-\s+.*)?$/i.test((button.textContent || "").trim())) || null;
  }

  function labelUnlockedPluginEntry(button) {
    const labelTextNode = Array.from(button.querySelectorAll("span, div")).reverse()
      .flatMap((node) => Array.from(node.childNodes))
      .find((node) => node.nodeType === 3 && /^(插件|Plugins)( - 已解锁| - Unlocked)?$/i.test((node.nodeValue || "").trim()));
    if (!labelTextNode) return;
    const current = (labelTextNode.nodeValue || "").trim();
    labelTextNode.nodeValue = /^Plugins/i.test(current) ? "Plugins - Unlocked" : "插件 - 已解锁";
  }

  function enablePluginEntry() {
    if (!codexPlusSettings().pluginEntryUnlock) return;
    const pluginButton = pluginEntryButton();
    if (!pluginButton) return;
    spoofChatGPTAuthMethod(pluginButton);
    pluginButton.disabled = false;
    pluginButton.removeAttribute("disabled");
    pluginButton.style.display = "";
    pluginButton.querySelectorAll("*").forEach((node) => {
      node.style.display = "";
    });
    labelUnlockedPluginEntry(pluginButton);
    const reactPropsKey = Object.keys(pluginButton).find((key) => key.startsWith("__reactProps"));
    if (reactPropsKey) {
      pluginButton[reactPropsKey].disabled = false;
    }
    if (pluginButton.dataset.codexPluginEnabled === "true") return;
    pluginButton.dataset.codexPluginEnabled = "true";
    pluginButton.addEventListener("click", () => {
      spoofChatGPTAuthMethod(pluginButton);
    }, true);
  }

  function pluginInstallCandidates() {
    return Array.from(document.querySelectorAll(selectors.disabledInstallButton));
  }

  function installButtonLabel(element) {
    return (element.textContent || "").trim();
  }

  function unblockButtonElement(button) {
    button.disabled = false;
    button.removeAttribute("disabled");
    button.removeAttribute("aria-disabled");
    button.classList.remove("disabled", "opacity-50", "cursor-not-allowed", "pointer-events-none");
    button.style.pointerEvents = "auto";
    button.tabIndex = 0;
    const reactPropsKey = Object.keys(button).find((key) => key.startsWith("__reactProps"));
    if (reactPropsKey) {
      button[reactPropsKey].disabled = false;
      button[reactPropsKey]["aria-disabled"] = false;
    }
  }

  function labelForcedInstallButton(button) {
    const textNode = Array.from(button.childNodes).find((node) => node.nodeType === 3 && (/^安装\s/.test((node.nodeValue || "").trim()) || /^Install\s/.test((node.nodeValue || "").trim()) || (node.nodeValue || "").trim() === "强制安装"));
    if (textNode) {
      textNode.nodeValue = "强制安装";
    }
  }

  function unblockPluginInstallButtons() {
    if (!codexPlusSettings().forcePluginInstall) return;
    pluginInstallCandidates().forEach((button) => {
      const text = installButtonLabel(button);
      if (!/^安装\s/.test(text) && !/^Install\s/.test(text) && text !== "强制安装") return;
      unblockButtonElement(button);
      labelForcedInstallButton(button);
    });
  }

  let cachedSessionRows = [];
  let cachedSessionRowsAt = 0;

  function sessionRows(forceRefresh = false) {
    const now = Date.now();
    if (!forceRefresh && now - cachedSessionRowsAt < 150) {
      cachedSessionRows = cachedSessionRows.filter((row) => row.isConnected);
      if (cachedSessionRows.length > 0) return cachedSessionRows;
    }

    cachedSessionRows = Array.from(document.querySelectorAll(selectors.sidebarThread));
    cachedSessionRowsAt = now;
    return cachedSessionRows;
  }

  function archivePageHintVisible() {
    if (window.location.href.includes("archive")) return true;
    if (document.querySelector('[data-codex-archive-page-row="true"], [data-codex-archive-delete-all]')) return true;
    const archiveNav = document.querySelector(selectors.archiveNav);
    if (archiveNav?.className?.includes?.("bg-token-list-hover-background")) return true;
    return !!Array.from(document.querySelectorAll("h1, h2, h3")).find((element) => (element.textContent || "").trim() === "已归档对话");
  }

  function archiveRowFromUnarchiveButton(button) {
    return button.closest('[data-codex-archive-page-row="true"]')
      || button.closest('[role="listitem"], [role="row"]')
      || button.closest(".flex.w-full.items-center.justify-between")
      || button.parentElement;
  }

  function archivedPageRows() {
    if (!archivePageHintVisible()) return [];
    const rows = Array.from(document.querySelectorAll("button")).filter((button) => (button.textContent || "").trim() === "取消归档").map(archiveRowFromUnarchiveButton).filter(Boolean);
    rows.forEach((row) => {
      row.dataset.codexArchivePageRow = "true";
      row.setAttribute("data-codex-archive-page-row", "true");
    });
    return rows;
  }

  function archivedSessionRows() {
    if (!archivePageHintVisible()) return [];
    return sessionRows().filter((row) => row.querySelector('button[aria-label="取消归档对话"]') || row.outerHTML.includes("取消归档") || row.outerHTML.includes("unarchive"));
  }

  function archivedRows() {
    if (!archivePageHintVisible()) return [];
    return [...archivedSessionRows(), ...archivedPageRows()];
  }

  function archivedPageVisible() {
    return archivePageHintVisible() && archivedRows().length > 0;
  }

  function sessionRefFromRow(row) {
    const href = row.getAttribute("href") || row.querySelector("a")?.getAttribute("href") || "";
    const idMatch = href.match(/(?:session|conversation|thread)[=/:-]([A-Za-z0-9_.-]+)/i) || href.match(/([A-Za-z0-9_-]{8,})$/);
    const codexThreadId = row.getAttribute("data-app-action-sidebar-thread-id") || "";
    const fallbackId = row.getAttribute("data-session-id") || row.getAttribute("data-testid") || "";
    const sessionId = codexThreadId || (idMatch && idMatch[1]) || fallbackId;
    const titleNode = row.querySelector(`${selectors.threadTitle}, .truncate.select-none, .truncate.text-base`);
    const rawTitle = (titleNode?.textContent || (titleNode ? "" : (row.textContent || "Untitled session")));
    const title = (titleNode ? rawTitle : rawTitle.replace(/\s*(导出|删除|移动|移出项目)(\s*(导出|删除|移动|移出项目))*$/g, "")).trim().slice(0, 160);
    return { session_id: sessionId, title };
  }

  function locationThreadId() {
    const source = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    const match = source.match(/(?:session|conversation|thread)(?:\/|=|:|-)([A-Za-z0-9_.-]+)/i)
      || source.match(/\/([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})(?:[/?#]|$)/)
      || source.match(/\/([A-Za-z0-9_-]{24,})(?:[/?#]|$)/);
    return match ? decodeURIComponent(match[1]) : "";
  }

  function validThreadScrollSessionKey(sessionId) {
    const key = projectMoveSessionKey(sessionId);
    if (!key || key === "__proto__" || key === "prototype" || key === "constructor") return "";
    return /^[A-Za-z0-9_.-]{8,128}$/.test(key) ? key : "";
  }

  function currentSessionRef() {
    const rows = sessionRows();
    for (const row of rows) {
      const ref = sessionRefFromRow(row);
      if (ref.session_id && isCurrentSessionRow(row, ref)) return ref;
    }
    return { session_id: locationThreadId(), title: "" };
  }

  function finiteNonNegativeNumber(value) {
    const numeric = Number(value);
    return Number.isFinite(numeric) && numeric >= 0 ? numeric : 0;
  }

  function finiteScrollNumber(value) {
    const numeric = Number(value);
    return Number.isFinite(numeric) ? numeric : 0;
  }

  function readThreadScrollEntries() {
    if (window.__codexThreadScrollEntries && typeof window.__codexThreadScrollEntries === "object") {
      return { ...window.__codexThreadScrollEntries };
    }
    try {
      const parsed = JSON.parse(localStorage.getItem(codexThreadScrollKey) || "{}");
      const rawEntries = parsed?.version === codexThreadScrollVersion && parsed?.entries && typeof parsed.entries === "object"
        ? parsed.entries
        : parsed && typeof parsed === "object"
          ? parsed
          : {};
      const entries = Object.create(null);
      Object.entries(rawEntries).forEach(([key, value]) => {
        const safeKey = validThreadScrollSessionKey(key);
        if (!safeKey || !value || typeof value !== "object") return;
        entries[safeKey] = {
          top: finiteScrollNumber(value.top),
          scrollHeight: finiteNonNegativeNumber(value.scrollHeight),
          clientHeight: finiteNonNegativeNumber(value.clientHeight),
          at: finiteNonNegativeNumber(value.at),
        };
      });
      window.__codexThreadScrollEntries = entries;
      return { ...entries };
    } catch {
      window.__codexThreadScrollEntries = Object.create(null);
      return {};
    }
  }

  function writeThreadScrollEntries(entries) {
    const pruned = Object.create(null);
    Object.entries(entries || {})
      .sort((left, right) => finiteNonNegativeNumber(right[1]?.at) - finiteNonNegativeNumber(left[1]?.at))
      .slice(0, codexThreadScrollMaxEntries)
      .forEach(([key, value]) => {
        const safeKey = validThreadScrollSessionKey(key);
        if (safeKey) pruned[safeKey] = value;
      });
    window.__codexThreadScrollEntries = pruned;
    localStorage.setItem(codexThreadScrollKey, JSON.stringify({ version: codexThreadScrollVersion, entries: pruned }));
  }

  function currentThreadScroller() {
    const explicit = document.querySelector(".thread-scroll-container");
    if (explicit?.isConnected) return explicit;
    const root = conversationTimelineRoot();
    if (!root?.isConnected) return document.scrollingElement || document.documentElement;
    const style = getComputedStyle(root);
    if (/(auto|scroll)/.test(style.overflowY) && root.scrollHeight > root.clientHeight) return root;
    return nearestTimelineScroller(root);
  }

  function threadScrollRuntime() {
    if (!window.__codexThreadScrollRuntime || typeof window.__codexThreadScrollRuntime !== "object") {
      window.__codexThreadScrollRuntime = {
        activeSessionId: "",
        activeScroller: null,
        scrollListener: null,
        scrollListenerUsesWindow: false,
        lastSavedTop: -1,
        lastSavedHeight: -1,
        lastSavedClientHeight: -1,
        restoreLock: null,
        applyingRestore: false,
        pendingNavigation: null,
        userScrollIntentUntil: 0,
        userCancelledRestoreSessionId: "",
      };
    }
    return window.__codexThreadScrollRuntime;
  }

  function clearThreadScrollRestoreTimers() {
    (window.__codexThreadScrollRestoreTimers || []).forEach((timer) => clearTimeout(timer));
    window.__codexThreadScrollRestoreTimers = [];
  }

  function clearThreadScrollSyncTimers() {
    (window.__codexThreadScrollSyncTimers || []).forEach((timer) => clearTimeout(timer));
    window.__codexThreadScrollSyncTimers = [];
  }

  function clearThreadScrollRestoreLock() {
    const runtime = threadScrollRuntime();
    runtime.restoreLock = null;
  }

  function cancelThreadScrollRestoreForUserIntent() {
    const runtime = threadScrollRuntime();
    const cancelledSessionId = validThreadScrollSessionKey(runtime.restoreLock?.sessionId)
      || validThreadScrollSessionKey(currentSessionRef().session_id)
      || validThreadScrollSessionKey(runtime.activeSessionId);
    runtime.userScrollIntentUntil = Date.now() + codexThreadScrollUserIntentWindowMs;
    runtime.userCancelledRestoreSessionId = cancelledSessionId;
    window.__codexThreadScrollRestoreRevision = (window.__codexThreadScrollRestoreRevision || 0) + 1;
    window.__codexThreadScrollSyncRevision = (window.__codexThreadScrollSyncRevision || 0) + 1;
    clearThreadScrollRestoreTimers();
    clearThreadScrollSyncTimers();
    clearThreadScrollRestoreLock();
  }

  function userScrollIntentActive() {
    return finiteNonNegativeNumber(threadScrollRuntime().userScrollIntentUntil) > Date.now();
  }

  function threadScrollRestoreCancelledForSession(sessionId = threadScrollRuntime().activeSessionId) {
    const key = validThreadScrollSessionKey(sessionId);
    return !!key && threadScrollRuntime().userCancelledRestoreSessionId === key;
  }

  function activeThreadScrollRestoreLock(sessionId = threadScrollRuntime().activeSessionId) {
    const runtime = threadScrollRuntime();
    const key = validThreadScrollSessionKey(sessionId);
    const lock = runtime.restoreLock;
    if (!lock || !key || lock.sessionId !== key) return null;
    if (lock.expiresAt <= Date.now()) {
      clearThreadScrollRestoreLock();
      return null;
    }
    return lock;
  }

  function currentThreadScrollRestoreLock() {
    const sessionId = threadScrollRuntime().restoreLock?.sessionId;
    return sessionId ? activeThreadScrollRestoreLock(sessionId) : null;
  }

  function threadScrollIsReversed(scroller) {
    return getComputedStyle(scroller).flexDirection === "column-reverse";
  }

  function threadScrollRange(scroller) {
    const extent = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    return threadScrollIsReversed(scroller)
      ? { min: -extent, max: 0, bottom: 0 }
      : { min: 0, max: extent, bottom: extent };
  }

  function startThreadScrollRestoreLock(sessionId, entry) {
    const key = validThreadScrollSessionKey(sessionId);
    if (!key || !entry) {
      clearThreadScrollRestoreLock();
      return null;
    }
    const runtime = threadScrollRuntime();
    runtime.restoreLock = {
      sessionId: key,
      targetTop: finiteScrollNumber(entry.top),
      expiresAt: Date.now() + codexThreadScrollRestoreWindowMs,
    };
    return runtime.restoreLock;
  }

  function prepareThreadScrollRestoreLock(sessionId) {
    const key = validThreadScrollSessionKey(sessionId);
    const entry = key ? readThreadScrollEntries()[key] : null;
    if (entry) startThreadScrollRestoreLock(key, entry);
  }

  function threadScrollTargetTop(scroller, targetTop) {
    const range = threadScrollRange(scroller);
    return Math.max(range.min, Math.min(range.max, finiteScrollNumber(targetTop)));
  }

  function threadScrollNearBottom(scroller, top) {
    const range = threadScrollRange(scroller);
    return Math.abs(range.bottom - finiteScrollNumber(top)) <= Math.max(24, scroller.clientHeight * 0.15);
  }

  function threadScrollGuardScroller(scroller) {
    if (!scroller) return null;
    const runtime = threadScrollRuntime();
    const rootScroller = document.scrollingElement || document.documentElement || document.body;
    const normalizedScroller = scroller === document.body || scroller === document.documentElement ? rootScroller : scroller;
    if (normalizedScroller === runtime.activeScroller) return normalizedScroller;
    const currentScroller = currentThreadScroller();
    if (normalizedScroller === currentScroller) return normalizedScroller;
    return null;
  }

  function shouldBlockThreadScrollAutobottom(scroller, top) {
    const runtime = threadScrollRuntime();
    const lock = currentThreadScrollRestoreLock();
    if (!lock || !codexPlusSettings().threadScrollRestore) return false;
    const guardScroller = threadScrollGuardScroller(scroller);
    if (runtime.applyingRestore || !guardScroller) return false;
    const targetTop = threadScrollTargetTop(guardScroller, lock.targetTop);
    return Math.abs(finiteScrollNumber(top) - targetTop) > 8 && threadScrollNearBottom(guardScroller, top);
  }

  function scrollToRequestedTop(args, scroller) {
    if (!args.length) return null;
    const first = args[0];
    if (typeof first === "object" && first !== null) {
      return first.top == null ? null : finiteScrollNumber(first.top);
    }
    if (args.length >= 2) return finiteScrollNumber(args[1]);
    return scroller?.scrollTop ?? null;
  }

  function scrollByRequestedTop(args, scroller) {
    if (!args.length || !scroller) return null;
    const first = args[0];
    let delta = null;
    if (typeof first === "object" && first !== null) {
      delta = first.top == null ? null : Number(first.top);
    } else if (args.length >= 2) {
      delta = Number(args[1]);
    }
    return Number.isFinite(delta) ? finiteScrollNumber(scroller.scrollTop + delta) : null;
  }

  function shouldBlockThreadScrollIntoView(element) {
    const runtime = threadScrollRuntime();
    const lock = currentThreadScrollRestoreLock();
    if (runtime.applyingRestore || !lock || !element) return false;
    const activeScroller = threadScrollGuardScroller(runtime.activeScroller) || threadScrollGuardScroller(currentThreadScroller());
    if (!activeScroller || element === activeScroller || !activeScroller.contains?.(element)) return false;
    if (threadScrollIsReversed(activeScroller) && shouldBlockThreadScrollAutobottom(activeScroller, 0)) return true;
    const elementRect = element.getBoundingClientRect?.();
    if (!elementRect) return false;
    const elementBottomTop = activeScroller.scrollTop + elementRect.bottom - timelineScrollerViewportTop(activeScroller) - activeScroller.clientHeight;
    return shouldBlockThreadScrollAutobottom(activeScroller, elementBottomTop);
  }

  function findScrollTopDescriptor() {
    const htmlElementPrototype = typeof HTMLElement === "undefined" ? null : HTMLElement.prototype;
    for (const prototype of [Element.prototype, htmlElementPrototype].filter(Boolean)) {
      const descriptor = Object.getOwnPropertyDescriptor(prototype, "scrollTop");
      if (descriptor?.get && descriptor?.set) return { prototype, descriptor };
    }
    return null;
  }

  function threadScrollNativePrototypeSnapshot() {
    if (window.__codexThreadScrollNativePrototypeSnapshot) return window.__codexThreadScrollNativePrototypeSnapshot;
    const snapshot = {};
    let frame = null;
    try {
      frame = document.createElement("iframe");
      frame.style.display = "none";
      frame.tabIndex = -1;
      frame.setAttribute("aria-hidden", "true");
      (document.documentElement || document.body)?.appendChild(frame);
      const frameWindow = frame.contentWindow;
      if (frameWindow?.Element?.prototype) {
        snapshot.elementScrollTo = frameWindow.Element.prototype.scrollTo;
        snapshot.elementScroll = frameWindow.Element.prototype.scroll;
        snapshot.elementScrollBy = frameWindow.Element.prototype.scrollBy;
        snapshot.scrollIntoView = frameWindow.Element.prototype.scrollIntoView;
      }
      if (frameWindow?.Window?.prototype) {
        snapshot.windowScrollTo = frameWindow.Window.prototype.scrollTo;
        snapshot.windowScroll = frameWindow.Window.prototype.scroll;
        snapshot.windowScrollBy = frameWindow.Window.prototype.scrollBy;
      }
      const frameHtmlElementPrototype = frameWindow?.HTMLElement?.prototype;
      const candidates = [
        { framePrototype: frameWindow?.Element?.prototype, prototype: Element.prototype },
        { framePrototype: frameHtmlElementPrototype, prototype: typeof HTMLElement === "undefined" ? null : HTMLElement.prototype },
      ];
      for (const candidate of candidates) {
        const descriptor = candidate.framePrototype ? Object.getOwnPropertyDescriptor(candidate.framePrototype, "scrollTop") : null;
        if (descriptor?.get && descriptor?.set && candidate.prototype) {
          snapshot.scrollTop = { prototype: candidate.prototype, descriptor };
          break;
        }
      }
    } catch {
      snapshot.unavailable = true;
    } finally {
      frame?.remove?.();
    }
    window.__codexThreadScrollNativePrototypeSnapshot = snapshot;
    return snapshot;
  }

  function threadScrollFunctionLooksGuarded(fn) {
    return typeof fn === "function" && (fn.name || "").startsWith("codexThreadScrollGuarded");
  }

  function threadScrollDescriptorLooksGuarded(descriptor) {
    try {
      return String(descriptor?.set || "").includes("__codexThreadScrollHandlers")
        || String(descriptor?.set || "").includes("shouldBlockThreadScrollAutobottom");
    } catch {
      return false;
    }
  }

  function threadScrollOriginalFunction(originals, key, current, nativeFunction) {
    if (typeof originals[key] === "function" && !threadScrollFunctionLooksGuarded(originals[key])) return originals[key];
    if (threadScrollFunctionLooksGuarded(current) && typeof nativeFunction === "function") return nativeFunction;
    return typeof originals[key] === "function" ? originals[key] : current;
  }

  function threadScrollOriginalScrollTopDescriptor(originals) {
    const current = findScrollTopDescriptor();
    const nativeScrollTop = threadScrollNativePrototypeSnapshot().scrollTop;
    if (originals.scrollTop && !threadScrollDescriptorLooksGuarded(originals.scrollTop.descriptor)) return originals.scrollTop;
    if (current && threadScrollDescriptorLooksGuarded(current.descriptor) && nativeScrollTop) return nativeScrollTop;
    return originals.scrollTop || current;
  }

  function installThreadScrollProgrammaticScrollGuard() {
    if (window.__codexThreadScrollProgrammaticGuardInstalled === codexThreadScrollProgrammaticGuardVersion) return;
    window.__codexThreadScrollProgrammaticGuardInstalled = codexThreadScrollProgrammaticGuardVersion;
    window.__codexThreadScrollOriginals = window.__codexThreadScrollOriginals || {};
    const originals = window.__codexThreadScrollOriginals;
    const nativeSnapshot = threadScrollNativePrototypeSnapshot();
    const originalElementScrollTo = threadScrollOriginalFunction(originals, "elementScrollTo", Element.prototype.scrollTo, nativeSnapshot.elementScrollTo);
    originals.elementScrollTo = originalElementScrollTo;
    if (typeof originalElementScrollTo === "function") {
      Element.prototype.scrollTo = function codexThreadScrollGuardedScrollTo(...args) {
        const top = scrollToRequestedTop(args, this);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(this, top)) return;
        return originalElementScrollTo.apply(this, args);
      };
    }
    const originalElementScroll = threadScrollOriginalFunction(originals, "elementScroll", Element.prototype.scroll, nativeSnapshot.elementScroll);
    originals.elementScroll = originalElementScroll;
    if (typeof originalElementScroll === "function") {
      Element.prototype.scroll = function codexThreadScrollGuardedScroll(...args) {
        const top = scrollToRequestedTop(args, this);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(this, top)) return;
        return originalElementScroll.apply(this, args);
      };
    }
    const originalElementScrollBy = threadScrollOriginalFunction(originals, "elementScrollBy", Element.prototype.scrollBy, nativeSnapshot.elementScrollBy);
    originals.elementScrollBy = originalElementScrollBy;
    if (typeof originalElementScrollBy === "function") {
      Element.prototype.scrollBy = function codexThreadScrollGuardedScrollBy(...args) {
        const top = scrollByRequestedTop(args, this);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(this, top)) return;
        return originalElementScrollBy.apply(this, args);
      };
    }
    const originalScrollIntoView = threadScrollOriginalFunction(originals, "scrollIntoView", Element.prototype.scrollIntoView, nativeSnapshot.scrollIntoView);
    originals.scrollIntoView = originalScrollIntoView;
    if (typeof originalScrollIntoView === "function") {
      Element.prototype.scrollIntoView = function codexThreadScrollGuardedScrollIntoView(...args) {
        if (window.__codexThreadScrollHandlers?.shouldBlockIntoView?.(this)) return;
        return originalScrollIntoView.apply(this, args);
      };
    }
    const originalWindowScrollTo = threadScrollOriginalFunction(originals, "windowScrollTo", window.scrollTo, nativeSnapshot.windowScrollTo);
    originals.windowScrollTo = originalWindowScrollTo;
    if (typeof originalWindowScrollTo === "function") {
      window.scrollTo = function codexThreadScrollGuardedWindowScrollTo(...args) {
        const scroller = document.scrollingElement || document.documentElement || document.body;
        const top = scrollToRequestedTop(args, scroller);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(scroller, top)) return;
        return originalWindowScrollTo.apply(this, args);
      };
    }
    const originalWindowScroll = threadScrollOriginalFunction(originals, "windowScroll", window.scroll, nativeSnapshot.windowScroll);
    originals.windowScroll = originalWindowScroll;
    if (typeof originalWindowScroll === "function") {
      window.scroll = function codexThreadScrollGuardedWindowScroll(...args) {
        const scroller = document.scrollingElement || document.documentElement || document.body;
        const top = scrollToRequestedTop(args, scroller);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(scroller, top)) return;
        return originalWindowScroll.apply(this, args);
      };
    }
    const originalWindowScrollBy = threadScrollOriginalFunction(originals, "windowScrollBy", window.scrollBy, nativeSnapshot.windowScrollBy);
    originals.windowScrollBy = originalWindowScrollBy;
    if (typeof originalWindowScrollBy === "function") {
      window.scrollBy = function codexThreadScrollGuardedWindowScrollBy(...args) {
        const scroller = document.scrollingElement || document.documentElement || document.body;
        const top = scrollByRequestedTop(args, scroller);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(scroller, top)) return;
        return originalWindowScrollBy.apply(this, args);
      };
    }
    const scrollTop = threadScrollOriginalScrollTopDescriptor(originals);
    originals.scrollTop = scrollTop;
    if (scrollTop?.descriptor?.configurable !== false) {
      try {
        Object.defineProperty(scrollTop.prototype, "scrollTop", {
          configurable: true,
          enumerable: scrollTop.descriptor.enumerable,
          get() {
            return scrollTop.descriptor.get.call(this);
          },
          set(value) {
            if (window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(this, value)) return;
            return scrollTop.descriptor.set.call(this, value);
          },
        });
      } catch {
        window.__codexThreadScrollTopGuardUnavailable = true;
      }
    }
  }

  function bindThreadScrollListener(scroller) {
    const runtime = threadScrollRuntime();
    const currentUsesWindow = !runtime.activeScroller || runtime.activeScroller === document.scrollingElement || runtime.activeScroller === document.documentElement || runtime.activeScroller === document.body;
    const nextUsesWindow = !scroller || scroller === document.scrollingElement || scroller === document.documentElement || scroller === document.body;
    let listenerReplaced = false;
    if (runtime.scrollListener && runtime.scrollListenerVersion !== codexThreadScrollListenerVersion) {
      const currentTarget = currentUsesWindow ? window : runtime.activeScroller;
      currentTarget?.removeEventListener?.("scroll", runtime.scrollListener, true);
      runtime.scrollListener = null;
      runtime.scrollListenerVersion = "";
      listenerReplaced = true;
    }
    runtime.scrollListener = runtime.scrollListener || (() => scheduleThreadScrollSave());
    runtime.scrollListenerVersion = codexThreadScrollListenerVersion;
    if (!listenerReplaced && runtime.activeScroller === scroller && runtime.scrollListenerUsesWindow === nextUsesWindow) return;
    if (runtime.activeScroller) {
      const target = currentUsesWindow ? window : runtime.activeScroller;
      target.removeEventListener("scroll", runtime.scrollListener, true);
    }
    runtime.activeScroller = scroller;
    runtime.scrollListenerUsesWindow = nextUsesWindow;
    if (!scroller || !codexPlusSettings().threadScrollRestore) return;
    const target = nextUsesWindow ? window : scroller;
    target.addEventListener("scroll", runtime.scrollListener, true);
  }

  function saveThreadScrollPositionNow(sessionId = threadScrollRuntime().activeSessionId, scroller = threadScrollRuntime().activeScroller) {
    if (!codexPlusSettings().threadScrollRestore) return;
    const runtime = threadScrollRuntime();
    const key = validThreadScrollSessionKey(sessionId);
    if (!key || !scroller) return;
    if (activeThreadScrollRestoreLock(key)) return;
    const snapshot = {
      top: finiteScrollNumber(scroller.scrollTop),
      scrollHeight: finiteNonNegativeNumber(scroller.scrollHeight),
      clientHeight: finiteNonNegativeNumber(scroller.clientHeight),
      at: Date.now(),
    };
    if (
      Math.abs(runtime.lastSavedTop - snapshot.top) < 2 &&
      runtime.lastSavedHeight === snapshot.scrollHeight &&
      runtime.lastSavedClientHeight === snapshot.clientHeight
    ) {
      return;
    }
    const entries = readThreadScrollEntries();
    entries[key] = snapshot;
    writeThreadScrollEntries(entries);
    runtime.lastSavedTop = snapshot.top;
    runtime.lastSavedHeight = snapshot.scrollHeight;
    runtime.lastSavedClientHeight = snapshot.clientHeight;
  }

  function scheduleThreadScrollSave() {
    if (!codexPlusSettings().threadScrollRestore || window.__codexThreadScrollSaveTimer) return;
    window.__codexThreadScrollSaveTimer = setTimeout(() => {
      window.__codexThreadScrollSaveTimer = null;
      saveThreadScrollPositionNow();
    }, codexThreadScrollSaveThrottleMs);
  }

  function restoreThreadScrollPosition(sessionId) {
    const runtime = threadScrollRuntime();
    const key = validThreadScrollSessionKey(sessionId);
    if (!codexPlusSettings().threadScrollRestore || !key || runtime.activeSessionId !== key || userScrollIntentActive() || threadScrollRestoreCancelledForSession(key)) return;
    const lock = activeThreadScrollRestoreLock(key);
    const entry = lock || readThreadScrollEntries()[key];
    if (!entry) return;
    const scroller = currentThreadScroller();
    if (!scroller) return;
    bindThreadScrollListener(scroller);
    const targetTop = threadScrollTargetTop(scroller, lock ? lock.targetTop : entry.top);
    if (Math.abs(scroller.scrollTop - targetTop) <= 1) return;
    runtime.applyingRestore = true;
    try {
      if (typeof scroller.scrollTo === "function") {
        scroller.scrollTo({ top: targetTop, behavior: "auto" });
      } else {
        scroller.scrollTop = targetTop;
      }
    } finally {
      runtime.applyingRestore = false;
    }
    runtime.lastSavedTop = targetTop;
    runtime.lastSavedHeight = finiteNonNegativeNumber(scroller.scrollHeight);
    runtime.lastSavedClientHeight = finiteNonNegativeNumber(scroller.clientHeight);
  }

  function scheduleThreadScrollRestore(sessionId) {
    clearThreadScrollRestoreTimers();
    const key = validThreadScrollSessionKey(sessionId);
    if (!codexPlusSettings().threadScrollRestore || !key || userScrollIntentActive() || threadScrollRestoreCancelledForSession(key)) return;
    const entry = readThreadScrollEntries()[key];
    if (!entry) {
      clearThreadScrollRestoreLock();
      return;
    }
    startThreadScrollRestoreLock(key, entry);
    const restoreRevision = (window.__codexThreadScrollRestoreRevision || 0) + 1;
    window.__codexThreadScrollRestoreRevision = restoreRevision;
    window.__codexThreadScrollRestoreTimers = codexThreadScrollRestoreDelaysMs.map((delay) => {
      return setTimeout(() => {
        if (window.__codexThreadScrollRestoreRevision !== restoreRevision) return;
        restoreThreadScrollPosition(key);
      }, delay);
    });
  }

  function syncThreadScrollState(forceRestore = false) {
    const runtime = threadScrollRuntime();
    const currentRef = currentSessionRef();
    const nextSessionId = validThreadScrollSessionKey(currentRef.session_id);
    if (!nextSessionId) return;
    if (!codexPlusSettings().threadScrollRestore) {
      bindThreadScrollListener(null);
      clearThreadScrollRestoreTimers();
      clearThreadScrollRestoreLock();
      runtime.activeSessionId = nextSessionId;
      return;
    }
    if (runtime.activeSessionId !== nextSessionId) prepareThreadScrollRestoreLock(nextSessionId);
    const nextScroller = currentThreadScroller();
    bindThreadScrollListener(nextScroller);
    if (runtime.activeSessionId !== nextSessionId) {
      runtime.lastSavedTop = -1;
      runtime.lastSavedHeight = -1;
      runtime.lastSavedClientHeight = -1;
      clearThreadScrollRestoreLock();
      runtime.activeSessionId = nextSessionId;
      runtime.pendingNavigation = null;
      runtime.userScrollIntentUntil = 0;
      if (runtime.userCancelledRestoreSessionId !== nextSessionId) runtime.userCancelledRestoreSessionId = "";
      scheduleThreadScrollRestore(nextSessionId);
      return;
    }
    runtime.activeSessionId = nextSessionId;
    if (forceRestore && !userScrollIntentActive() && !threadScrollRestoreCancelledForSession(nextSessionId)) scheduleThreadScrollRestore(nextSessionId);
  }

  function scheduleThreadScrollSyncAttempts(forceRestore = true) {
    const currentKey = validThreadScrollSessionKey(currentSessionRef().session_id) || validThreadScrollSessionKey(threadScrollRuntime().activeSessionId);
    if (userScrollIntentActive() || threadScrollRestoreCancelledForSession(currentKey)) return;
    clearThreadScrollSyncTimers();
    const syncRevision = (window.__codexThreadScrollSyncRevision || 0) + 1;
    window.__codexThreadScrollSyncRevision = syncRevision;
    window.__codexThreadScrollSyncTimers = codexThreadScrollRestoreDelaysMs.map((delay) => {
      return setTimeout(() => {
        if (window.__codexThreadScrollSyncRevision !== syncRevision) return;
        scheduleThreadScrollSync(forceRestore);
      }, delay);
    });
  }

  function captureThreadScrollNavigation(targetSessionId) {
    if (!codexPlusSettings().threadScrollRestore) return;
    const runtime = threadScrollRuntime();
    const targetKey = validThreadScrollSessionKey(targetSessionId);
    const sessionChanged = !!targetKey && targetKey !== runtime.activeSessionId;
    if (sessionChanged) {
      runtime.userScrollIntentUntil = 0;
      runtime.userCancelledRestoreSessionId = "";
    }
    const pending = runtime.pendingNavigation;
    const duplicatePendingTarget = !!targetKey && pending?.targetSessionId === targetKey && Date.now() - finiteNonNegativeNumber(pending.at) < 5000;
    if (!duplicatePendingTarget) saveThreadScrollPositionNow();
    if (targetKey) {
      runtime.pendingNavigation = { fromSessionId: runtime.activeSessionId, targetSessionId: targetKey, at: Date.now() };
      prepareThreadScrollRestoreLock(targetKey);
    }
    scheduleThreadScrollSyncAttempts(true);
  }

  function editableThreadScrollTarget(element) {
    return !!element?.closest?.("input, textarea, select, [contenteditable='true'], [contenteditable='']");
  }

  function eventTargetsActiveThreadScroller(event) {
    const runtime = threadScrollRuntime();
    const scroller = threadScrollGuardScroller(runtime.activeScroller) || threadScrollGuardScroller(currentThreadScroller());
    if (!scroller) return false;
    const target = event?.target;
    if (!target || target === document || target === window) return true;
    return target === scroller || scroller.contains?.(target) || scroller.contains?.(document.activeElement);
  }

  function markThreadScrollUserIntent(event) {
    if (!codexPlusSettings().threadScrollRestore || !eventTargetsActiveThreadScroller(event)) return;
    cancelThreadScrollRestoreForUserIntent();
  }

  function markThreadScrollKeyboardIntent(event) {
    if (editableThreadScrollTarget(event.target)) return;
    if (!["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " ", "Spacebar"].includes(event.key)) return;
    markThreadScrollUserIntent(event);
  }

  function markThreadScrollPointerIntent(event) {
    const scroller = threadScrollGuardScroller(threadScrollRuntime().activeScroller) || threadScrollGuardScroller(currentThreadScroller());
    if (event.target === scroller) markThreadScrollUserIntent(event);
  }

  function updateThreadScrollHandlers() {
    window.__codexThreadScrollHandlers = {
      shouldBlockAutobottom: shouldBlockThreadScrollAutobottom,
      shouldBlockIntoView: shouldBlockThreadScrollIntoView,
      markUserIntent: markThreadScrollUserIntent,
      markKeyboardIntent: markThreadScrollKeyboardIntent,
      markPointerIntent: markThreadScrollPointerIntent,
      captureNavigation: captureThreadScrollNavigation,
      saveNow: saveThreadScrollPositionNow,
      prepareRestoreLock: prepareThreadScrollRestoreLock,
      scheduleSyncAttempts: scheduleThreadScrollSyncAttempts,
    };
  }

  function installThreadScrollUserIntentCapture() {
    if (window.__codexThreadScrollUserIntentInstalled === codexThreadScrollUserIntentVersion) return;
    document.removeEventListener("wheel", window.__codexThreadScrollWheelIntentHandler, true);
    document.removeEventListener("touchmove", window.__codexThreadScrollTouchIntentHandler, true);
    document.removeEventListener("keydown", window.__codexThreadScrollKeyIntentHandler, true);
    document.removeEventListener("pointerdown", window.__codexThreadScrollPointerIntentHandler, true);
    window.__codexThreadScrollWheelIntentHandler = (event) => window.__codexThreadScrollHandlers?.markUserIntent?.(event);
    window.__codexThreadScrollTouchIntentHandler = (event) => window.__codexThreadScrollHandlers?.markUserIntent?.(event);
    window.__codexThreadScrollKeyIntentHandler = (event) => window.__codexThreadScrollHandlers?.markKeyboardIntent?.(event);
    window.__codexThreadScrollPointerIntentHandler = (event) => window.__codexThreadScrollHandlers?.markPointerIntent?.(event);
    document.addEventListener("wheel", window.__codexThreadScrollWheelIntentHandler, { capture: true, passive: true });
    document.addEventListener("touchmove", window.__codexThreadScrollTouchIntentHandler, { capture: true, passive: true });
    document.addEventListener("keydown", window.__codexThreadScrollKeyIntentHandler, true);
    document.addEventListener("pointerdown", window.__codexThreadScrollPointerIntentHandler, true);
    window.__codexThreadScrollUserIntentInstalled = codexThreadScrollUserIntentVersion;
  }

  function installThreadScrollNavigationCapture() {
    document.removeEventListener("pointerdown", window.__codexThreadScrollNavigationHandler, true);
    document.removeEventListener("click", window.__codexThreadScrollClickNavigationHandler, true);
    document.removeEventListener("keydown", window.__codexThreadScrollKeyboardHandler, true);
    const navigationHandler = (event) => {
      if (!codexPlusSettings().threadScrollRestore) return;
      const row = event.target?.closest?.(selectors.sidebarThread);
      if (!row) return;
      window.__codexThreadScrollHandlers?.captureNavigation?.(sessionRefFromRow(row).session_id);
    };
    const clickHandler = (event) => {
      if (!codexPlusSettings().threadScrollRestore) return;
      const row = event.target?.closest?.(selectors.sidebarThread);
      if (!row) return;
      window.__codexThreadScrollHandlers?.captureNavigation?.(sessionRefFromRow(row).session_id);
    };
    const keyboardHandler = (event) => {
      if (!codexPlusSettings().threadScrollRestore) return;
      if (event.key !== "Enter" && event.key !== " ") return;
      const row = event.target?.closest?.(selectors.sidebarThread);
      if (!row) return;
      window.__codexThreadScrollHandlers?.captureNavigation?.(sessionRefFromRow(row).session_id);
    };
    window.__codexThreadScrollNavigationHandler = navigationHandler;
    window.__codexThreadScrollClickNavigationHandler = clickHandler;
    window.__codexThreadScrollKeyboardHandler = keyboardHandler;
    document.addEventListener("pointerdown", navigationHandler, true);
    document.addEventListener("click", clickHandler, true);
    document.addEventListener("keydown", keyboardHandler, true);
  }

  function scheduleThreadScrollSync(forceRestore = false) {
    if (window.__codexThreadScrollSyncPending) return;
    window.__codexThreadScrollSyncPending = true;
    setTimeout(() => {
      window.__codexThreadScrollSyncPending = false;
      syncThreadScrollState(forceRestore);
    }, 0);
  }

  function installThreadScrollRouteHooks() {
    if (window.__codexThreadScrollRouteHooksInstalled === codexThreadScrollRouteHooksVersion) return;
    window.__codexThreadScrollRouteHooksInstalled = codexThreadScrollRouteHooksVersion;
    window.__codexThreadScrollOriginals = window.__codexThreadScrollOriginals || {};
    const originals = window.__codexThreadScrollOriginals;
    ["pushState", "replaceState"].forEach((method) => {
      const prototypeMethod = typeof History !== "undefined" ? History.prototype?.[method] : null;
      const currentMethod = history[method];
      const storedMethod = originals[`history_${method}`];
      const original = (storedMethod?.name === "codexThreadScrollPatchedHistory" && typeof prototypeMethod === "function" ? prototypeMethod : storedMethod)
        || (currentMethod?.name === "codexThreadScrollPatchedHistory" && typeof prototypeMethod === "function" ? prototypeMethod : currentMethod);
      originals[`history_${method}`] = original;
      if (typeof original !== "function") return;
      history[method] = function codexThreadScrollPatchedHistory(...args) {
        window.__codexThreadScrollHandlers?.saveNow?.();
        const result = original.apply(this, args);
        window.__codexThreadScrollHandlers?.captureNavigation?.(locationThreadId());
        return result;
      };
    });
    window.removeEventListener("popstate", window.__codexThreadScrollPopStateHandler, true);
    window.removeEventListener("hashchange", window.__codexThreadScrollHashChangeHandler, true);
    document.removeEventListener("visibilitychange", window.__codexThreadScrollVisibilityHandler, true);
    window.__codexThreadScrollPopStateHandler = () => {
      window.__codexThreadScrollHandlers?.saveNow?.();
      window.__codexThreadScrollHandlers?.captureNavigation?.(locationThreadId());
    };
    window.__codexThreadScrollHashChangeHandler = () => {
      window.__codexThreadScrollHandlers?.saveNow?.();
      window.__codexThreadScrollHandlers?.captureNavigation?.(locationThreadId());
    };
    window.__codexThreadScrollVisibilityHandler = () => {
      if (document.visibilityState === "hidden") window.__codexThreadScrollHandlers?.saveNow?.();
    };
    window.addEventListener("popstate", window.__codexThreadScrollPopStateHandler, true);
    window.addEventListener("hashchange", window.__codexThreadScrollHashChangeHandler, true);
    document.addEventListener("visibilitychange", window.__codexThreadScrollVisibilityHandler, true);
  }

  async function postJson(path, payload) {
    if (!window.__codexSessionDeleteBridge) {
      return { status: "failed", message: "桥接不可用，请重启启动器" };
    }
    return await window.__codexSessionDeleteBridge(path, payload);
  }

  let codexModelCatalog = { status: "loading", model: "", default_model: "", model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
  let codexModelCatalogLoadedAt = 0;
  let codexModelCatalogPromise = null;
  const codexPlusModelListRequestIds = new Set();

  function codexPlusModelUnlockEnabled() {
    return !!codexPlusSettings().modelWhitelistUnlock;
  }

  function codexPlusModelNames() {
    return uniqueValues([
      codexModelCatalog.default_model,
      codexModelCatalog.model,
      ...(Array.isArray(codexModelCatalog.models) ? codexModelCatalog.models : []),
    ]);
  }

  function codexResponsesApiStatus() {
    const status = codexModelCatalog.responses_api;
    return status && typeof status === "object" ? status : { status: "unknown", message: "" };
  }

  function codexModelCompatibilityWarningText() {
    if (!codexPlusModelUnlockEnabled()) return "";
    const responsesApi = codexResponsesApiStatus();
    if (responsesApi.status !== "unsupported") return "";
    const provider = codexModelCatalog.provider_name || codexModelCatalog.model_provider || "当前模型供应商";
    const detail = responsesApi.message ? `：${responsesApi.message}` : "";
    return `${provider} 不支持 Codex 使用的 /v1/responses 接口，模型可能能显示，但发起对话会失败。请换支持 Responses API 的中转，或使用兼容转换代理${detail}`;
  }

  function modelCompatibilityWarningElement() {
    const toggle = document.querySelector('[data-codex-plus-setting="modelWhitelistUnlock"]');
    const row = toggle?.closest?.(".codex-plus-row");
    if (!row) return null;
    let warning = row.querySelector("[data-codex-model-compat-warning]");
    if (warning) return warning;
    warning = document.createElement("div");
    warning.className = "codex-plus-model-compat-warning";
    warning.dataset.codexModelCompatWarning = "true";
    const description = row.querySelector(".codex-plus-row-description");
    (description?.parentElement || row).appendChild(warning);
    return warning;
  }

  function renderCodexModelCompatibilityWarning() {
    const text = codexModelCompatibilityWarningText();
    document.querySelectorAll("[data-codex-model-compat-warning]").forEach((warning) => {
      warning.hidden = !text;
      warning.textContent = text;
    });
    if (!text) return;
    const warning = modelCompatibilityWarningElement();
    if (!warning) return;
    warning.hidden = false;
    warning.textContent = text;
  }

  function maybeShowCodexModelCompatibilityWarning() {
    const text = codexModelCompatibilityWarningText();
    if (!text) return;
    const responsesApi = codexResponsesApiStatus();
    const key = `${codexModelCatalog.model_provider || ""}:${responsesApi.endpoint || ""}:${responsesApi.message || ""}`;
    if (window.__codexPlusResponsesApiWarningKey === key) return;
    window.__codexPlusResponsesApiWarningKey = key;
    showToast(text, null);
  }

  async function loadCodexModelCatalog(force = false) {
    if (!force && codexModelCatalogPromise) return codexModelCatalogPromise;
    if (!force && codexModelCatalogLoadedAt && Date.now() - codexModelCatalogLoadedAt < 10000) return codexModelCatalog;
    codexModelCatalogPromise = postJson("/codex-model-catalog", {})
      .then((result) => {
        codexModelCatalog = result && typeof result === "object" ? result : { status: "failed", model: "", default_model: "", model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
        codexModelCatalogLoadedAt = Date.now();
        renderCodexPlusMenu();
        maybeShowCodexModelCompatibilityWarning();
        patchCodexModelWhitelist();
        return codexModelCatalog;
      })
      .catch((error) => {
        codexModelCatalog = { status: "failed", message: String(error?.message || error), model: "", default_model: "", model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
        codexModelCatalogLoadedAt = Date.now();
        return codexModelCatalog;
      })
      .finally(() => {
        codexModelCatalogPromise = null;
      });
    return codexModelCatalogPromise;
  }

  function modelReasoningEfforts() {
    return ["minimal", "low", "medium", "high", "xhigh"].map((reasoningEffort) => ({ reasoningEffort, description: `${reasoningEffort} effort` }));
  }

  function codexPlusModelDescriptor(modelName) {
    return {
      model: modelName,
      name: modelName,
      displayName: modelName,
      description: codexModelCatalog.provider_name || codexModelCatalog.model_provider || "Custom model",
      hidden: false,
      isDefault: (codexModelCatalog.default_model || codexModelCatalog.model) === modelName,
      defaultReasoningEffort: "medium",
      supportedReasoningEfforts: modelReasoningEfforts(),
    };
  }

  function modelArrayLooksPatchable(value, allowEmpty = false) {
    return Array.isArray(value)
      && (allowEmpty || value.length > 0)
      && value.every((item) => item && typeof item === "object" && typeof item.model === "string");
  }

  function patchModelArray(models, allowEmpty = false) {
    if (!modelArrayLooksPatchable(models, allowEmpty)) return false;
    const customModels = codexPlusModelNames();
    if (!customModels.length) return false;
    let changed = false;
    const existing = new Map(models.map((item) => [item.model, item]));
    models.forEach((item) => {
      if (customModels.includes(item.model) && item.hidden !== false) {
        item.hidden = false;
        changed = true;
      }
    });
    customModels.forEach((modelName) => {
      if (!existing.has(modelName)) {
        models.push(codexPlusModelDescriptor(modelName));
        changed = true;
      }
    });
    return changed;
  }

  function patchModelContainer(value) {
    if (!value || typeof value !== "object") return false;
    let changed = false;
    if (patchModelArray(value.models, "defaultModel" in value || "availableModels" in value)) changed = true;
    if (patchModelArray(value.data)) changed = true;
    if (patchModelArray(value.result)) changed = true;
    if (patchModelArray(value.pages?.[0]?.data)) changed = true;
    if (patchModelArray(value.result?.data)) changed = true;
    if (patchModelArray(value.result?.models)) changed = true;
    if (patchModelArray(value.message?.result?.data)) changed = true;
    if (patchModelArray(value.message?.result?.models)) changed = true;
    const names = codexPlusModelNames();
    if (value.availableModels instanceof Set) {
      names.forEach((name) => {
        if (!value.availableModels.has(name)) {
          value.availableModels.add(name);
          changed = true;
        }
      });
    }
    if (Array.isArray(value.availableModels)) {
      names.forEach((name) => {
        if (!value.availableModels.includes(name)) {
          value.availableModels.push(name);
          changed = true;
        }
      });
    }
    if (value.defaultModel == null && names.length > 0) {
      value.defaultModel = codexPlusModelDescriptor(names[0]);
      changed = true;
    } else if (typeof value.defaultModel === "string" && names.includes(value.defaultModel) && value.model == null) {
      value.model = value.defaultModel;
      changed = true;
    }
    return changed;
  }

  function patchStatsigModelDynamicConfig(config) {
    const names = codexPlusModelNames();
    const value = config?.value;
    if (!names.length || !value || typeof value !== "object") return config;
    const availableModels = Array.isArray(value.available_models) ? [...value.available_models] : [];
    let changed = false;
    names.forEach((name) => {
      if (!availableModels.includes(name)) {
        availableModels.push(name);
        changed = true;
      }
    });
    const nextValue = {
      ...value,
      available_models: availableModels,
      default_model: names[0] || value.default_model,
    };
    if (!changed && nextValue.default_model === value.default_model) return config;
    try {
      config.value = nextValue;
    } catch {
      return { ...config, value: nextValue };
    }
    return config;
  }

  function statsigClients() {
    const root = window.__STATSIG__ || globalThis.__STATSIG__;
    if (!root || typeof root !== "object") return [];
    const clients = [root.firstInstance, typeof root.instance === "function" ? root.instance() : null];
    if (root.instances && typeof root.instances === "object") clients.push(...Object.values(root.instances));
    return clients.filter((client, index, array) => client && typeof client === "object" && array.indexOf(client) === index);
  }

  function patchStatsigModelWhitelist() {
    statsigClients().forEach((client) => {
      if (client.__codexPlusModelWhitelistPatched || typeof client.getDynamicConfig !== "function") return;
      const originalGetDynamicConfig = client.getDynamicConfig.bind(client);
      client.getDynamicConfig = (name, options) => {
        const result = originalGetDynamicConfig(name, options);
        return name === "107580212" ? patchStatsigModelDynamicConfig(result) : result;
      };
      client.__codexPlusModelWhitelistPatched = true;
      try {
        patchStatsigModelDynamicConfig(client.getDynamicConfig("107580212", { disableExposureLog: true }));
      } catch {
      }
    });
  }

  function patchObjectGraphForModels(root, visited, depth = 0) {
    if (!root || typeof root !== "object" || visited.has(root) || depth > 5) return false;
    visited.add(root);
    let changed = patchModelContainer(root);
    if (root instanceof Element || root === window || root === document || root === document.body || root === document.documentElement) return changed;
    for (const key of Object.keys(root)) {
      if (key === "ownerDocument" || key === "parentElement" || key === "parentNode" || key === "children" || key === "childNodes") continue;
      let value;
      try {
        value = root[key];
      } catch {
        continue;
      }
      if (value && typeof value === "object" && patchObjectGraphForModels(value, visited, depth + 1)) changed = true;
    }
    return changed;
  }

  function reactFiberKeys(element) {
    return Object.keys(element).filter((key) => key.startsWith("__reactFiber") || key.startsWith("__reactInternalInstance") || key.startsWith("__reactProps"));
  }

  function patchReactModelState() {
    const visited = new WeakSet();
    const nodes = [document.body, ...document.querySelectorAll("button, [role='menu'], [role='dialog'], [data-radix-popper-content-wrapper]")].filter(Boolean);
    let changed = false;
    for (const node of nodes.slice(0, 220)) {
      for (const key of reactFiberKeys(node)) {
        if (patchObjectGraphForModels(node[key], visited)) changed = true;
      }
    }
    return changed;
  }

  function patchAppServerModelMessages() {
    if (window.__codexPlusModelMessagePatchInstalled) return;
    window.__codexPlusModelMessagePatchInstalled = true;
    const originalDispatchEvent = window.dispatchEvent;
    window.dispatchEvent = function patchedCodexPlusDispatchEvent(event) {
      try {
        const detail = event?.detail;
        const request = detail?.request;
        if (event?.type === "codex-message-from-view" && detail?.type === "mcp-request" && request?.method === "model/list") {
          request.params = { ...(request.params || {}), includeHidden: true };
          if (request.id != null) codexPlusModelListRequestIds.add(String(request.id));
        }
        if (event?.type === "message") patchMcpModelResponseData(event.data);
      } catch (error) {
        window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
        window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
      }
      return originalDispatchEvent.call(this, event);
    };

    window.addEventListener("message", (event) => {
      try {
        patchMcpModelResponseData(event?.data);
      } catch (error) {
        window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
        window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
      }
    }, true);
  }

  function patchMcpModelResponseData(data) {
    if (data?.type !== "mcp-response") return false;
    const message = data.message || data.response;
    const requestId = message.id != null ? String(message.id) : "";
    if (codexPlusModelListRequestIds.size > 0 && !codexPlusModelListRequestIds.has(requestId)) return false;
    codexPlusModelListRequestIds.delete(requestId);
    return patchModelContainer(data) || patchModelContainer(message) || patchModelContainer(message?.result) || patchModelContainer(message?.result?.data);
  }

  function patchCodexModelWhitelist() {
    if (!codexPlusModelUnlockEnabled()) return;
    patchAppServerModelMessages();
    if (!codexPlusModelNames().length) {
      loadCodexModelCatalog();
      return;
    }
    patchStatsigModelWhitelist();
    patchReactModelState();
  }

  function downloadMarkdown(filename, markdown) {
    if (!filename || typeof markdown !== "string") {
      throw new Error("导出结果不完整");
    }
    const blob = new Blob([markdown], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = filename;
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  let codexStateApiPromise = null;
  let chatsSortInFlight = false;
  let chatsSortSignature = "";
  let chatsSortLastFetchAt = 0;

  async function codexStateApi() {
    codexStateApiPromise = codexStateApiPromise || import("./assets/vscode-api-Dc9pX2Bc.js");
    const api = await codexStateApiPromise;
    if (typeof api.n !== "function") throw new Error("Codex 状态 API 不可用");
    return api.n;
  }

  async function codexStateCall(method, params) {
    const call = await codexStateApi();
    return await call(method, params);
  }

  async function getCodexGlobalState(key) {
    const result = await codexStateCall("get-global-state", { params: { key } });
    return result && Object.prototype.hasOwnProperty.call(result, "value") ? result.value : result;
  }

  async function setCodexGlobalState(key, value) {
    return await codexStateCall("set-global-state", { params: { key, value } });
  }

  function objectGlobalState(value) {
    return value && typeof value === "object" && !Array.isArray(value) ? { ...value } : {};
  }

  function uniqueValues(values) {
    return Array.from(new Set(values.filter((value) => typeof value === "string" && value.trim().length > 0)));
  }

  function threadIdVariants(sessionId) {
    if (typeof sessionId !== "string" || !sessionId.trim()) return [];
    const id = sessionId.trim();
    const bareId = id.startsWith("local:") ? id.slice("local:".length) : id;
    return uniqueValues([id, bareId, `local:${bareId}`]);
  }

  function projectMoveSessionKey(sessionId) {
    const variants = threadIdVariants(sessionId);
    const bareId = variants.find((id) => !id.startsWith("local:"));
    return bareId || variants[0] || "";
  }

  function uuidV7TimestampMs(sessionId) {
    const id = projectMoveSessionKey(sessionId).replaceAll("-", "");
    if (!/^[0-9a-fA-F]{12}/.test(id)) return 0;
    const timestamp = Number.parseInt(id.slice(0, 12), 16);
    return Number.isFinite(timestamp) ? timestamp : 0;
  }

  function numericTimestamp(value) {
    const timestamp = Number(value);
    return Number.isFinite(timestamp) && timestamp > 0 ? timestamp : 0;
  }

  function timestampValueToMs(value) {
    const timestamp = numericTimestamp(value);
    if (!timestamp) return 0;
    return timestamp < 1000000000000 ? timestamp * 1000 : timestamp;
  }

  function sortMsForSession(sessionId, preferredValue) {
    return numericTimestamp(preferredValue) || uuidV7TimestampMs(sessionId);
  }

  function timestampMsFromPayload(payload) {
    return numericTimestamp(payload?.updated_at_ms) || timestampValueToMs(payload?.updated_at) || numericTimestamp(payload?.created_at_ms);
  }

  function relativeTimeLabel(timestampMs, nowMs = Date.now()) {
    const timestamp = numericTimestamp(timestampMs);
    if (!timestamp) return "";
    const elapsedSeconds = Math.max(0, Math.floor((nowMs - timestamp) / 1000));
    if (elapsedSeconds < 60) return "刚刚";
    const elapsedMinutes = Math.floor(elapsedSeconds / 60);
    if (elapsedMinutes < 60) return `${elapsedMinutes} 分`;
    const elapsedHours = Math.floor(elapsedMinutes / 60);
    if (elapsedHours < 24) return `${elapsedHours} 小时`;
    const elapsedDays = Math.floor(elapsedHours / 24);
    if (elapsedDays < 7) return `${elapsedDays} 天`;
    const elapsedWeeks = Math.floor(elapsedDays / 7);
    if (elapsedWeeks < 5) return `${elapsedWeeks} 周`;
    const elapsedMonths = Math.floor(elapsedDays / 30);
    if (elapsedMonths < 12) return `${Math.max(1, elapsedMonths)} 月`;
    return `${Math.floor(elapsedDays / 365)} 年`;
  }

  function normalizeWorkspacePath(path) {
    const normalized = String(path || "").trim().replace(/\\/g, "/").replace(/\/+$/, "");
    return normalized || String(path || "").trim();
  }

  function sameWorkspacePath(left, right) {
    const leftPath = normalizeWorkspacePath(left);
    const rightPath = normalizeWorkspacePath(right);
    return !!leftPath && !!rightPath && leftPath === rightPath;
  }

  function displayProjectName(path) {
    const trimmed = String(path || "").replace(/\/+$/, "");
    return trimmed.split(/[\\/]+/).filter(Boolean).pop() || trimmed || "未命名项目";
  }

  function normalizeProjectLabel(value) {
    return String(value || "").replace(/\s+/g, " ").trim();
  }

  function projectsSection() {
    return document.querySelector('[data-app-action-sidebar-section-heading="Projects"]');
  }

  function chatsSection() {
    return document.querySelector('[data-app-action-sidebar-section-heading="Chats"]');
  }

  function projectRowListItem(projectRow) {
    return projectRow.closest?.('[role="listitem"][aria-label]') || projectRow.closest?.('[role="listitem"]') || projectRow;
  }

  function nativeProjectTargets() {
    const section = projectsSection();
    const seen = new Set();
    const targets = [];
    Array.from(document.querySelectorAll('[data-app-action-sidebar-project-row]')).forEach((row) => {
      if (section && !section.contains(row)) return;
      const path = row.getAttribute("data-app-action-sidebar-project-id") || "";
      const normalizedPath = normalizeWorkspacePath(path);
      if (!normalizedPath || seen.has(normalizedPath)) return;
      const label = row.getAttribute("data-app-action-sidebar-project-label") || row.getAttribute("aria-label") || displayProjectName(path);
      seen.add(normalizedPath);
      targets.push({ kind: "project", label: String(label || displayProjectName(path)), description: path, path, normalizedPath, row, listItem: projectRowListItem(row) });
    });
    return targets;
  }

  function serializableProjectTarget(target) {
    return { kind: target.kind, label: target.label, description: target.description, path: target.path, normalizedPath: target.normalizedPath || normalizeWorkspacePath(target.path) };
  }

  function projectMoveTargets() {
    return [
      { kind: "projectless", label: "普通对话", description: "不属于任何项目", path: "", normalizedPath: "" },
      ...nativeProjectTargets().map(serializableProjectTarget),
    ];
  }

  function readLegacyProjectMoveProjection() {
    try {
      const parsed = JSON.parse(localStorage.getItem(legacyProjectMoveOverridesKey) || "{}");
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
      const now = Date.now();
      const next = {};
      for (const [key, value] of Object.entries(parsed)) {
        if (!value || typeof value !== "object" || !value.targetCwd) continue;
        const sessionId = projectMoveSessionKey(value.sessionId || key);
        if (!sessionId) continue;
        next[sessionId] = {
          sessionId,
          targetKind: "project",
          targetCwd: String(value.targetCwd),
          targetLabel: String(value.targetLabel || displayProjectName(value.targetCwd)),
          title: String(value.title || ""),
          sortMs: sortMsForSession(sessionId, value.sortMs || value.updatedAtMs || value.updated_at_ms),
          sortMsTrusted: false,
          at: typeof value.at === "number" ? value.at : now,
        };
      }
      return next;
    } catch {
      return {};
    }
  }

  function readProjectMoveProjection() {
    try {
      const parsed = JSON.parse(localStorage.getItem(projectMoveProjectionKey) || "{}");
      const raw = parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
      const merged = { ...readLegacyProjectMoveProjection(), ...raw };
      const now = Date.now();
      const projection = {};
      for (const [key, value] of Object.entries(merged)) {
        if (!value || typeof value !== "object") continue;
        const sessionId = projectMoveSessionKey(value.sessionId || key);
        if (!sessionId) continue;
        if (typeof value.at === "number" && now - value.at > projectMoveProjectionTtlMs) continue;
        const targetKind = value.targetKind === "projectless" ? "projectless" : "project";
        const targetCwd = String(value.targetCwd || value.path || "");
        if (targetKind === "project" && !targetCwd) continue;
        projection[sessionId] = {
          sessionId,
          targetKind,
          targetCwd,
          targetLabel: String(value.targetLabel || value.label || (targetKind === "projectless" ? "普通对话" : displayProjectName(targetCwd))),
          title: String(value.title || ""),
          sortMs: sortMsForSession(sessionId, value.sortMs || value.updatedAtMs || value.updated_at_ms),
          sortMsTrusted: value.sortMsTrusted === true,
          at: typeof value.at === "number" ? value.at : now,
        };
      }
      return projection;
    } catch {
      return readLegacyProjectMoveProjection();
    }
  }

  function writeProjectMoveProjection(projection) {
    try {
      localStorage.setItem(projectMoveProjectionKey, JSON.stringify(projection || {}));
      localStorage.removeItem(legacyProjectMoveOverridesKey);
    } catch (error) {
      window.__codexProjectMoveProjectionFailures = window.__codexProjectMoveProjectionFailures || [];
      window.__codexProjectMoveProjectionFailures.push(String(error?.stack || error));
    }
  }

  function saveProjectMoveProjection(ref, target, sortMs) {
    const id = projectMoveSessionKey(ref.session_id);
    if (!id || !target) return;
    const projection = readProjectMoveProjection();
    projection[id] = {
      sessionId: id,
      targetKind: target.kind === "projectless" ? "projectless" : "project",
      targetCwd: target.path || "",
      targetLabel: target.label || (target.kind === "projectless" ? "普通对话" : displayProjectName(target.path)),
      title: ref.title || "",
      sortMs: sortMsForSession(ref.session_id, sortMs || target.sortMs),
      sortMsTrusted: target.sortMsTrusted === true,
      at: Date.now(),
    };
    writeProjectMoveProjection(projection);
  }

  function clearProjectMoveProjection(ref) {
    const projection = readProjectMoveProjection();
    const keys = threadIdVariants(ref.session_id).map(projectMoveSessionKey).filter(Boolean);
    let changed = false;
    keys.forEach((key) => {
      if (Object.prototype.hasOwnProperty.call(projection, key)) {
        delete projection[key];
        changed = true;
      }
    });
    if (changed) writeProjectMoveProjection(projection);
  }

  function projectionForSessionId(sessionId, projection = readProjectMoveProjection()) {
    const key = projectMoveSessionKey(sessionId);
    return key ? projection[key] || null : null;
  }

  function projectRowFromListItem(projectItem) {
    if (!projectItem) return null;
    if (projectItem.matches?.("[data-app-action-sidebar-project-row]")) return projectItem;
    return projectItem.querySelector?.("[data-app-action-sidebar-project-row]") || null;
  }

  function targetPath(target) {
    return target?.path || target?.targetCwd || "";
  }

  function targetLabel(target) {
    return target?.label || target?.targetLabel || displayProjectName(targetPath(target));
  }

  function projectItemMatchesTarget(projectItem, target) {
    const projectRow = projectRowFromListItem(projectItem);
    const projectPath = projectRow?.getAttribute?.("data-app-action-sidebar-project-id") || "";
    if (projectPath && sameWorkspacePath(projectPath, targetPath(target))) return true;
    const actual = normalizeProjectLabel(projectRow?.getAttribute?.("data-app-action-sidebar-project-label") || projectItem?.getAttribute?.("aria-label"));
    const labels = uniqueValues([targetLabel(target), displayProjectName(targetPath(target))]).map(normalizeProjectLabel).filter(Boolean);
    return !!actual && labels.includes(actual);
  }

  function findProjectListItem(target) {
    const nativeTarget = nativeProjectTargets().find((project) => sameWorkspacePath(project.path, targetPath(target)));
    if (nativeTarget?.listItem) return nativeTarget.listItem;
    const section = projectsSection();
    if (!section) return null;
    return Array.from(section.querySelectorAll('[role="listitem"][aria-label]')).find((item) => projectItemMatchesTarget(item, target)) || null;
  }

  function closestProjectListItem(row) {
    const item = row.closest?.('[role="listitem"][aria-label]');
    return item?.closest?.('[data-app-action-sidebar-section-heading="Projects"]') ? item : null;
  }

  function rowIsInChats(row) {
    return !!row.closest?.('[data-app-action-sidebar-section-heading="Chats"]');
  }

  function chatsThreadList() {
    return chatsSection()?.querySelector?.('[role="list"][aria-label="对话"], [role="list"]') || null;
  }

  function rowIsUnderTargetProject(row, target) {
    const item = closestProjectListItem(row);
    return !!item && projectItemMatchesTarget(item, target);
  }

  function rowIsUnderTarget(row, target) {
    return target?.targetKind === "projectless" || target?.kind === "projectless" ? rowIsInChats(row) : rowIsUnderTargetProject(row, target);
  }

  function rowListItem(row) {
    return row.closest?.('[role="listitem"]') || row;
  }

  function rowContentRoot(row) {
    return Array.from(row?.children || []).find((child) => String(child.className || "").includes("h-full w-full items-center")) || null;
  }

  function normalizedText(node) {
    return String(node?.textContent || "").replace(/\s+/g, " ").trim();
  }

  function classNameText(node) {
    return String(node?.className || "");
  }

  function isRelativeTimeText(text) {
    const value = String(text || "").replace(/\s+/g, " ").trim();
    return /^(刚刚|just now|\d+\s*(秒|秒钟|分|分钟|小时|天|日|周|星期|个月|月|年|sec|secs|second|seconds|min|mins|minute|minutes|h|hr|hrs|hour|hours|d|day|days|w|wk|wks|week|weeks|mo|mos|month|months|y|yr|yrs|year|years))$/i.test(value);
  }

  function nodeIsThreadTitle(row, node) {
    return Array.from(row?.querySelectorAll?.('[data-thread-title], .truncate.select-none, .truncate.text-base') || [])
      .some((titleNode) => titleNode === node || titleNode.contains(node));
  }

  function closestTimeWrapper(row, node) {
    const root = rowContentRoot(row) || row;
    let current = node?.parentElement || null;
    while (current && current !== root && current !== row) {
      const className = classNameText(current);
      if (current.dataset?.codexProjectMoveTimeWrapper === "true" || (className.includes("ml-[3px]") && className.includes("min-w-[26px]"))) return current;
      current = current.parentElement;
    }
    return null;
  }

  function nodeInsideStatusIcon(row, node) {
    const stop = closestTimeWrapper(row, node) || rowContentRoot(row) || row;
    let current = node || null;
    while (current && current !== stop && current !== row) {
      const className = classNameText(current);
      if (className.includes("animate-spin")) return true;
      if (className.includes("size-5") && className.includes("shrink-0")) return true;
      if (className.includes("contain-paint") && className.includes("contain-layout")) return true;
      current = current.parentElement;
    }
    return false;
  }

  function cleanupManagedStatusIconTimeNodes(row) {
    Array.from(row?.querySelectorAll?.('[data-codex-project-move-time="true"]') || []).forEach((node) => {
      if (!nodeInsideStatusIcon(row, node)) return;
      const text = normalizedText(node);
      delete node.dataset.codexProjectMoveTime;
      delete node.dataset.codexProjectMoveTimeMs;
      if (node.children.length === 0 && isRelativeTimeText(text)) node.textContent = "";
    });
  }

  function nodeLooksLikeTimeLabel(row, node) {
    if (nodeInsideStatusIcon(row, node)) return false;
    if (node?.dataset?.codexProjectMoveTime === "true") return true;
    if (node.children.length > 0) return false;
    const text = normalizedText(node);
    const className = classNameText(node);
    if ((className.includes("tabular-nums") || className.includes("text-token-description-foreground")) && text.length <= 24) return true;
    if (!isRelativeTimeText(text)) return false;
    const rowRect = row?.getBoundingClientRect?.();
    const nodeRect = node?.getBoundingClientRect?.();
    if (!rowRect || !nodeRect || rowRect.width <= 0 || nodeRect.width <= 0) return false;
    return nodeRect.left >= rowRect.left + rowRect.width * 0.45 || nodeRect.right >= rowRect.right - 96;
  }

  function rowTimeLabelCandidates(row) {
    cleanupManagedStatusIconTimeNodes(row);
    const root = rowContentRoot(row) || row;
    const raw = Array.from(root?.querySelectorAll?.("div, span, time, small") || []).filter((node) => {
      if (nodeIsThreadTitle(row, node)) return false;
      return nodeLooksLikeTimeLabel(row, node);
    });
    return raw.filter((node) => !raw.some((other) => other !== node && node.contains(other)));
  }

  function rowTimeLabelNode(row) {
    const candidates = rowTimeLabelCandidates(row);
    return candidates.find((node) => node.dataset?.codexProjectMoveTime !== "true" && !node.closest?.('[data-codex-project-move-time-wrapper="true"]')) || candidates[0] || null;
  }

  function removeTimeLabelNode(row, node) {
    if (!node || !row?.contains?.(node)) return;
    const wrapper = node.closest?.('[data-codex-project-move-time-wrapper="true"]') || closestTimeWrapper(row, node);
    if (wrapper && wrapper !== row && row.contains(wrapper)) {
      wrapper.remove();
      return;
    }
    node.remove();
  }

  function cleanupRowTimeLabels(row, keepNode) {
    if (!keepNode) return;
    rowTimeLabelCandidates(row).forEach((node) => {
      if (node === keepNode) return;
      if (node.dataset?.codexProjectMoveTime === "true" || node.closest?.('[data-codex-project-move-time-wrapper="true"]')) removeTimeLabelNode(row, node);
    });
  }

  function ensureRowTimeLabelNode(row) {
    const existing = rowTimeLabelNode(row);
    if (existing) {
      cleanupRowTimeLabels(row, existing);
      return existing;
    }
    const root = rowContentRoot(row);
    if (!root) return null;
    const wrapper = document.createElement("div");
    wrapper.className = "ml-[3px] flex items-center justify-end gap-1 min-w-[26px]";
    wrapper.dataset.codexProjectMoveTimeWrapper = "true";
    const inner = document.createElement("div");
    const label = document.createElement("div");
    label.className = "text-token-description-foreground text-sm leading-4 empty:hidden tabular-nums overflow-visible truncate text-right group-focus-within:opacity-0 group-hover:opacity-0";
    label.dataset.codexProjectMoveTime = "true";
    inner.appendChild(label);
    wrapper.appendChild(inner);
    root.appendChild(wrapper);
    return label;
  }

  function updateRowTimeLabel(row, sortMs) {
    const label = ensureRowTimeLabelNode(row);
    if (!label) return;
    const timestamp = numericTimestamp(sortMs);
    const text = relativeTimeLabel(timestamp);
    label.dataset.codexProjectMoveTime = "true";
    label.dataset.codexProjectMoveTimeMs = String(timestamp || 0);
    if (text && label.textContent !== text) label.textContent = text;
    cleanupRowTimeLabels(row, label);
  }

  function rowProjectionKind(row) {
    return row?.dataset?.codexProjectMoveTargetKind || rowListItem(row)?.dataset?.codexProjectMoveTargetKind || "";
  }

  function rowSortMs(row, ref = sessionRefFromRow(row), target = null) {
    return sortMsForSession(ref.session_id, target?.sortMs || row?.dataset?.codexProjectMoveSortMs || rowListItem(row)?.dataset?.codexProjectMoveSortMs);
  }

  function threadRowFromListItem(item) {
    if (!item) return null;
    if (item.matches?.("[data-app-action-sidebar-thread-id]")) return item;
    return item.querySelector?.("[data-app-action-sidebar-thread-id]") || null;
  }

  function rowPinned(row) {
    return row?.getAttribute?.("data-app-action-sidebar-thread-pinned") === "true" || rowListItem(row)?.getAttribute?.("data-app-action-sidebar-thread-pinned") === "true";
  }

  function insertRowItemByTime(list, item, row, target) {
    const ref = sessionRefFromRow(row);
    const sortMs = rowSortMs(row, ref, target);
    item.dataset.codexProjectMoveSortMs = String(sortMs || 0);
    row.dataset.codexProjectMoveSortMs = String(sortMs || 0);
    if (target?.sortMsTrusted) updateRowTimeLabel(row, sortMs);
    const pinned = rowPinned(row);
    const sessionKey = projectMoveSessionKey(ref.session_id);
    const existingItems = Array.from(list.children).filter((child) => child !== item);
    let firstNonThreadItem = null;
    for (const child of existingItems) {
      const childRow = threadRowFromListItem(child);
      if (!childRow) {
        firstNonThreadItem = firstNonThreadItem || child;
        continue;
      }
      const childPinned = rowPinned(childRow);
      if (childPinned && !pinned) continue;
      if (!childPinned && pinned) {
        list.insertBefore(item, child);
        return;
      }
      const childRef = sessionRefFromRow(childRow);
      const childSortMs = rowSortMs(childRow, childRef);
      const childKey = projectMoveSessionKey(childRef.session_id);
      if (sortMs > childSortMs || (sortMs === childSortMs && sessionKey > childKey)) {
        list.insertBefore(item, child);
        return;
      }
    }
    if (firstNonThreadItem) {
      list.insertBefore(item, firstNonThreadItem);
      return;
    }
    list.appendChild(item);
  }

  function projectMoveInjectedList(projectItem) {
    let list = projectItem.querySelector('[data-codex-project-move-injected-list="true"]');
    if (!list) {
      const body = Array.from(projectItem.children).find((child) => child.classList?.contains("overflow-hidden")) || projectItem;
      list = document.createElement("div");
      list.setAttribute("role", "list");
      list.setAttribute("data-codex-project-move-injected-list", "true");
      list.className = "flex flex-col";
      body.appendChild(list);
    }
    return list;
  }

  function projectThreadList(projectItem, target) {
    const targetCwd = targetPath(target);
    const projectLists = Array.from(projectItem.querySelectorAll("[data-app-action-sidebar-project-list-id]"));
    return projectLists.find((list) => sameWorkspacePath(list.getAttribute("data-app-action-sidebar-project-list-id"), targetCwd))
      || projectLists[0]
      || projectMoveInjectedList(projectItem);
  }

  function projectEmptyStateNodes(projectItem) {
    const emptyLabels = new Set(["暂无对话", "No conversations"]);
    return Array.from(projectItem.querySelectorAll("div, span")).filter((node) => {
      if (node.classList?.contains("overflow-hidden")) return false;
      if (node.closest('[data-app-action-sidebar-thread-id], [data-codex-project-move-injected-list="true"]')) return false;
      return emptyLabels.has(normalizeProjectLabel(node.textContent));
    });
  }

  function setProjectEmptyStateHidden(projectItem, hidden) {
    projectEmptyStateNodes(projectItem).forEach((node) => {
      if (hidden) {
        node.dataset.codexProjectMoveEmptyHidden = "true";
        node.classList.add("codex-project-move-hidden");
      } else if (node.dataset.codexProjectMoveEmptyHidden === "true") {
        delete node.dataset.codexProjectMoveEmptyHidden;
        node.classList.remove("codex-project-move-hidden");
      }
    });
  }

  function updateProjectMoveEmptyStates() {
    document.querySelectorAll('[data-codex-project-move-injected-list="true"]').forEach((list) => {
      const projectItem = list.closest('[role="listitem"][aria-label]');
      const hasRows = Array.from(list.children).some((child) => child.querySelector?.("[data-app-action-sidebar-thread-id]") || child.matches?.("[data-app-action-sidebar-thread-id]"));
      if (!hasRows) list.remove();
      if (projectItem) setProjectEmptyStateHidden(projectItem, hasRows);
    });
    document.querySelectorAll('[data-codex-project-move-empty-hidden="true"]').forEach((node) => {
      const projectItem = node.closest('[role="listitem"][aria-label]');
      const list = projectItem?.querySelector?.('[data-codex-project-move-injected-list="true"]');
      if (!list || list.children.length === 0) {
        delete node.dataset.codexProjectMoveEmptyHidden;
        node.classList.remove("codex-project-move-hidden");
      }
    });
  }

  function moveRowToProjectList(row, target) {
    const projectItem = findProjectListItem(target);
    if (!projectItem) return false;
    const list = projectThreadList(projectItem, target);
    const item = rowListItem(row);
    if (!list) return false;
    insertRowItemByTime(list, item, row, target);
    cachedSessionRowsAt = 0;
    item.dataset.codexProjectMoveTargetKind = "project";
    item.dataset.codexProjectMoveTargetCwd = targetPath(target);
    row.dataset.codexProjectMoveTargetKind = "project";
    row.dataset.codexProjectMoveTargetCwd = targetPath(target);
    setProjectEmptyStateHidden(projectItem, true);
    return true;
  }

  function moveRowToChats(row, target = null) {
    const list = chatsThreadList();
    if (!list) return false;
    const item = rowListItem(row);
    insertRowItemByTime(list, item, row, target);
    cachedSessionRowsAt = 0;
    item.dataset.codexProjectMoveTargetKind = "projectless";
    row.dataset.codexProjectMoveTargetKind = "projectless";
    delete item.dataset.codexProjectMoveTargetCwd;
    delete row.dataset.codexProjectMoveTargetCwd;
    updateProjectMoveEmptyStates();
    return true;
  }

  function applyProjectMoveProjection() {
    if (!codexPlusSettings().projectMove) return;
    const projection = readProjectMoveProjection();
    const targetRowsById = new Map();
    const settledRefs = [];
    const now = Date.now();
    const rows = sessionRows(true);
    rows.forEach((row) => {
      const ref = sessionRefFromRow(row);
      const target = projectionForSessionId(ref.session_id, projection);
      if (target && rowIsUnderTarget(row, target)) {
        const rowId = projectMoveSessionKey(ref.session_id);
        const hadProjectionKind = !!rowProjectionKind(row);
        const existingRow = targetRowsById.get(rowId);
        if (existingRow && existingRow !== row) {
          const existingIsProjection = !!rowProjectionKind(existingRow);
          const currentIsProjection = !!rowProjectionKind(row);
          const rowToRemove = existingIsProjection && !currentIsProjection ? existingRow : row;
          rowListItem(rowToRemove).remove();
          if (rowToRemove === existingRow) targetRowsById.set(rowId, row);
          if (rowToRemove === row) return;
        } else {
          targetRowsById.set(rowId, row);
        }
        if (!hadProjectionKind && typeof target.at === "number" && now - target.at > projectMoveProjectionSettleMs) settledRefs.push(ref);
        const moved = target.targetKind === "projectless" ? moveRowToChats(row, target) : moveRowToProjectList(row, target);
        if (moved) targetRowsById.set(rowId, row);
        const projectItem = closestProjectListItem(row);
        if (projectItem) setProjectEmptyStateHidden(projectItem, true);
      }
    });
    rows.forEach((row) => {
      const ref = sessionRefFromRow(row);
      const rowId = projectMoveSessionKey(ref.session_id);
      const target = projectionForSessionId(ref.session_id, projection);
      if (!target) {
        const item = rowListItem(row);
        delete row.dataset.codexProjectMoveTargetKind;
        delete row.dataset.codexProjectMoveTargetCwd;
        delete item.dataset.codexProjectMoveTargetKind;
        delete item.dataset.codexProjectMoveTargetCwd;
        return;
      }
      if (rowIsUnderTarget(row, target)) return;
      if (targetRowsById.has(rowId)) {
        rowListItem(row).remove();
        return;
      }
      const moved = target.targetKind === "projectless" ? moveRowToChats(row, target) : moveRowToProjectList(row, target);
      if (moved) targetRowsById.set(rowId, row);
    });
    settledRefs.forEach(clearProjectMoveProjection);
    updateProjectMoveEmptyStates();
  }

  function scheduleProjectMoveProjection() {
    if (!codexPlusSettings().projectMove || window.__codexProjectMoveProjectionTimer) return;
    window.__codexProjectMoveProjectionTimer = setTimeout(() => {
      if (window.__codexProjectMoveRuntimeId !== codexProjectMoveRuntimeId) return;
      window.__codexProjectMoveProjectionTimer = null;
      applyProjectMoveProjection();
    }, 80);
  }

  async function refreshRecentConversationsForHost() {
    try {
      const signals = await import("./assets/app-server-manager-signals-C1h8B-R-.js");
      if (typeof signals.rn === "function") await signals.rn("refresh-recent-conversations-for-host", { hostId: "local", sortKey: "updated_at" });
    } catch (error) {
      window.__codexProjectMoveRefreshFailures = window.__codexProjectMoveRefreshFailures || [];
      window.__codexProjectMoveRefreshFailures.push(String(error?.stack || error));
    }
  }

  function refreshAfterProjectMove() {
    const refreshVisibleSidebar = () => {
      applyProjectMoveProjection();
      scheduleChatsSortCorrection(0);
    };
    refreshVisibleSidebar();
    refreshRecentConversationsForHost().finally(() => {
      projectMoveRefreshDelaysMs.forEach((delay) => setTimeout(refreshVisibleSidebar, delay));
    });
  }

  function visibleChatsRows() {
    const list = chatsThreadList();
    if (!list) return [];
    return Array.from(list.children).map(threadRowFromListItem).filter(Boolean).filter((row) => rowIsInChats(row));
  }

  function chatsSortNeedsCorrection(rows) {
    let previousPinned = true;
    let previousSortMs = Infinity;
    let previousKey = "\uffff";
    for (const row of rows) {
      const pinned = rowPinned(row);
      const ref = sessionRefFromRow(row);
      const sortMs = rowSortMs(row, ref);
      const key = projectMoveSessionKey(ref.session_id);
      if (previousPinned && !pinned) {
        previousPinned = false;
        previousSortMs = sortMs;
        previousKey = key;
        continue;
      }
      if (!previousPinned && pinned) return true;
      if (sortMs > previousSortMs || (sortMs === previousSortMs && key > previousKey)) return true;
      previousSortMs = sortMs;
      previousKey = key;
    }
    return false;
  }

  function reorderChatsRows(rows) {
    const list = chatsThreadList();
    if (!list || rows.length < 2) return;
    const rowItems = new Set(rows.map(rowListItem));
    const firstNonThreadItem = Array.from(list.children).find((child) => !rowItems.has(child) && !threadRowFromListItem(child));
    const orderedRows = [...rows].sort((left, right) => {
      const leftPinned = rowPinned(left);
      const rightPinned = rowPinned(right);
      if (leftPinned !== rightPinned) return leftPinned ? -1 : 1;
      const leftRef = sessionRefFromRow(left);
      const rightRef = sessionRefFromRow(right);
      const leftSortMs = rowSortMs(left, leftRef);
      const rightSortMs = rowSortMs(right, rightRef);
      if (leftSortMs !== rightSortMs) return rightSortMs - leftSortMs;
      return projectMoveSessionKey(rightRef.session_id).localeCompare(projectMoveSessionKey(leftRef.session_id));
    });
    orderedRows.forEach((row) => list.insertBefore(rowListItem(row), firstNonThreadItem || null));
    cachedSessionRowsAt = 0;
  }

  async function applyChatsSortCorrection() {
    if (!codexPlusSettings().projectMove || chatsSortInFlight) return;
    const rows = visibleChatsRows();
    if (rows.length < 2) return;
    const refs = rows.map(sessionRefFromRow).filter((ref) => ref.session_id);
    const signature = refs.map((ref) => projectMoveSessionKey(ref.session_id)).join("|");
    const allRowsHaveSortMs = rows.every((row) => numericTimestamp(row.dataset.codexProjectMoveSortMs || rowListItem(row).dataset.codexProjectMoveSortMs));
    const shouldRefreshSortKeys = signature !== chatsSortSignature || !allRowsHaveSortMs || Date.now() - chatsSortLastFetchAt > chatsSortDbRefreshIntervalMs;
    if (!shouldRefreshSortKeys && !chatsSortNeedsCorrection(rows)) return;
    chatsSortInFlight = true;
    try {
      if (shouldRefreshSortKeys) {
        const result = await postJson("/thread-sort-keys", { sessions: refs }).catch(() => ({ status: "failed", sort_keys: [] }));
        chatsSortLastFetchAt = Date.now();
        const byId = new Map();
        if (result?.status === "ok" && Array.isArray(result?.sort_keys)) {
          result.sort_keys.forEach((item) => {
            const key = projectMoveSessionKey(String(item?.session_id || ""));
            if (key) byId.set(key, item);
          });
        }
        rows.forEach((row) => {
          const ref = sessionRefFromRow(row);
          const payload = byId.get(projectMoveSessionKey(ref.session_id));
          const trustedSortMs = timestampMsFromPayload(payload);
          const sortMs = trustedSortMs || sortMsForSession(ref.session_id, row.dataset.codexProjectMoveSortMs || rowListItem(row).dataset.codexProjectMoveSortMs);
          row.dataset.codexProjectMoveSortMs = String(sortMs || 0);
          rowListItem(row).dataset.codexProjectMoveSortMs = String(sortMs || 0);
          if (trustedSortMs) updateRowTimeLabel(row, trustedSortMs);
        });
      }
      if (chatsSortNeedsCorrection(rows)) reorderChatsRows(rows);
      chatsSortSignature = visibleChatsRows().map((row) => projectMoveSessionKey(sessionRefFromRow(row).session_id)).join("|");
    } finally {
      chatsSortInFlight = false;
    }
  }

  function scheduleChatsSortCorrection(delay = chatsSortRefreshIntervalMs) {
    if (!codexPlusSettings().projectMove || window.__codexProjectMoveChatsSortTimer) return;
    window.__codexProjectMoveChatsSortTimer = setTimeout(() => {
      if (window.__codexProjectMoveRuntimeId !== codexProjectMoveRuntimeId) return;
      window.__codexProjectMoveChatsSortTimer = null;
      applyChatsSortCorrection().catch((error) => {
        window.__codexProjectMoveSortFailures = window.__codexProjectMoveSortFailures || [];
        window.__codexProjectMoveSortFailures.push(String(error?.stack || error));
      }).finally(() => {
        if (codexPlusSettings().projectMove) scheduleChatsSortCorrection();
      });
    }, delay);
  }

  async function setProjectlessThreadIds(ref, mode) {
    const variants = threadIdVariants(ref.session_id);
    if (variants.length === 0) throw new Error("未找到会话 ID");
    const existingIds = await getCodexGlobalState("projectless-thread-ids").catch(() => []);
    const ids = Array.isArray(existingIds) ? existingIds : [];
    const variantSet = new Set(variants);
    const nextIds = mode === "add" ? uniqueValues([...ids, ...variants]) : ids.filter((id) => !variantSet.has(id));
    if (nextIds.length !== ids.length || nextIds.some((id, index) => id !== ids[index])) await setCodexGlobalState("projectless-thread-ids", nextIds);
  }

  async function clearThreadWorkspaceHints(ref) {
    const variants = threadIdVariants(ref.session_id);
    if (variants.length === 0) return;
    const hints = objectGlobalState(await getCodexGlobalState("thread-workspace-root-hints").catch(() => ({})));
    const hintKeys = variants.filter((id) => Object.prototype.hasOwnProperty.call(hints, id));
    if (hintKeys.length > 0) {
      hintKeys.forEach((id) => delete hints[id]);
      await setCodexGlobalState("thread-workspace-root-hints", hints);
    }
  }

  async function moveSessionToProjectless(ref) {
    if (!ref.session_id) throw new Error("未找到会话 ID");
    await setProjectlessThreadIds(ref, "add");
    await clearThreadWorkspaceHints(ref);
    const sortKey = await postJson("/thread-sort-key", ref).catch(() => ({}));
    return { status: "moved", session_id: ref.session_id, updated_at: sortKey?.updated_at, updated_at_ms: sortKey?.updated_at_ms, created_at_ms: sortKey?.created_at_ms };
  }

  function isNativeProjectTarget(target) {
    return target?.kind === "project" && nativeProjectTargets().some((project) => sameWorkspacePath(project.path, target.path));
  }

  async function moveSessionToProject(ref, target) {
    if (!ref.session_id) throw new Error("未找到会话 ID");
    if (!target?.path) throw new Error("目标项目路径为空");
    if (!isNativeProjectTarget(target)) throw new Error("目标项目不在 Codex 项目列表中");
    const result = await postJson("/move-thread-workspace", { ...ref, target_cwd: target.path });
    if (result.status !== "moved") throw new Error(result.message || "移动项目失败");
    await setProjectlessThreadIds(ref, "remove");
    await clearThreadWorkspaceHints(ref);
    return result;
  }

  function showToast(message, undoToken) {
    document.querySelectorAll(".codex-delete-toast").forEach((node) => node.remove());
    const toast = document.createElement("div");
    toast.className = "codex-delete-toast";
    toast.textContent = message;
    if (undoToken) {
      const undo = document.createElement("button");
      undo.textContent = "撤销";
      undo.addEventListener("click", async () => {
        const result = await postJson("/undo", { undo_token: undoToken });
        toast.textContent = result.message || "撤销完成";
        setTimeout(() => toast.remove(), 5000);
      });
      toast.appendChild(undo);
    }
    document.body.appendChild(toast);
    setTimeout(() => toast.remove(), 10000);
  }

  function escapeHtml(value) {
    return String(value)
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#39;");
  }

  function confirmDelete(title) {
    document.querySelectorAll(".codex-delete-confirm-overlay").forEach((node) => node.remove());
    return new Promise((resolve) => {
      const overlay = document.createElement("div");
      overlay.className = "codex-delete-confirm-overlay";
      overlay.innerHTML = `
        <div class="codex-delete-confirm-content" role="dialog" aria-modal="true" aria-label="删除会话">
          <div class="codex-delete-confirm-title">删除会话</div>
          <div class="codex-delete-confirm-message">删除“${escapeHtml(title)}”？</div>
          <div class="codex-delete-confirm-actions">
            <button type="button" data-codex-delete-cancel="true">取消</button>
            <button type="button" data-codex-delete-confirm="true">删除</button>
          </div>
        </div>
      `;
      const finish = (value, event) => {
        event?.preventDefault();
        event?.stopPropagation();
        event?.target?.blur?.();
        overlay.remove();
        resolve(value);
      };
      overlay.addEventListener("click", (event) => {
        if (event.target === overlay || event.target.closest("[data-codex-delete-cancel]")) {
          finish(false, event);
          return;
        }
        if (event.target.closest("[data-codex-delete-confirm]")) {
          finish(true, event);
        }
      }, true);
      overlay.addEventListener("keydown", (event) => {
        if (event.key === "Escape") finish(false, event);
      }, true);
      document.body.appendChild(overlay);
      overlay.querySelector("[data-codex-delete-cancel]")?.focus();
    });
  }

  function rowHref(row) {
    return row.getAttribute("href") || row.querySelector("a")?.getAttribute("href") || "";
  }

  function isCurrentSessionRow(row, ref) {
    if (row.getAttribute("aria-current") === "page" || row.getAttribute("aria-current") === "true") return true;
    const href = rowHref(row);
    if (href) {
      try {
        const url = new URL(href, window.location.href);
        if (url.href === window.location.href || url.pathname === window.location.pathname) return true;
      } catch {
        if (window.location.href.includes(href)) return true;
      }
    }
    return !!ref.session_id && window.location.href.includes(ref.session_id);
  }

  function releaseDeleteFocus(row, button) {
    button.blur();
    if (row.contains(document.activeElement)) {
      document.activeElement.blur();
    }
  }

  function removeDeletedRow(row, button, ref) {
    releaseDeleteFocus(row, button);
    const shouldReload = isCurrentSessionRow(row, ref);
    row.remove();
    if (shouldReload) {
      window.location.reload();
    }
  }

  function isThreadMissingResult(result) {
    const message = String(result?.message || "");
    return result?.status === "failed" && message.includes("Thread not found in local storage");
  }

  async function removeOrphanedProjectedRow(row, button, ref) {
    await setProjectlessThreadIds(ref, "remove").catch(() => {});
    await clearThreadWorkspaceHints(ref).catch(() => {});
    removeDeletedRow(row, button, ref);
  }

  function updateDeleteButtonOffsets() {
    sessionRows().forEach((row) => {
      const hasArchiveConfirm = Array.from(row.querySelectorAll("button")).some((button) => {
        const rect = button.getBoundingClientRect();
        const label = button.getAttribute("aria-label") || "";
        const text = (button.textContent || "").trim();
        if (button.classList.contains(buttonClass) || button.classList.contains(exportButtonClass) || label === "归档对话" || label === "置顶对话") return false;
        return text === "确认" || (text.length > 0 && rect.width > 0 && rect.width <= 36 && rect.x > row.getBoundingClientRect().right - 50);
      });
      row.classList.toggle("codex-archive-confirm-visible", hasArchiveConfirm);
    });
  }

  function openDeleteConfirmForRow(row, button, ref, event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    releaseDeleteFocus(row, button);
    confirmDelete(ref.title).then(async (confirmed) => {
      if (!confirmed) return;
      releaseDeleteFocus(row, button);
      const result = await postJson("/delete", ref);
      if (result.status === "server_deleted" || result.status === "local_deleted") {
        removeDeletedRow(row, button, ref);
        showToast(result.message || "删除成功", result.undo_token);
      } else if (isThreadMissingResult(result)) {
        await removeOrphanedProjectedRow(row, button, ref);
        showToast("已移除本地列表中的失效会话", null);
      } else {
        showToast(result.message || "删除失败", null);
      }
    });
  }

  async function exportMarkdown(ref) {
    const result = await postJson("/export-markdown", ref);
    if (result.status === "exported" && result.filename && typeof result.markdown === "string") {
      downloadMarkdown(result.filename, result.markdown);
      showToast(result.message || "导出成功", null);
      return;
    }
    showToast(result.message || "导出失败", null);
  }

  function sortStateFromMoveResult(result, ref, row) {
    const trustedSortMs = timestampMsFromPayload(result);
    return { sortMs: trustedSortMs || rowSortMs(row, ref), sortMsTrusted: !!trustedSortMs };
  }

  function finishProjectMove(row, button, ref, target, message) {
    releaseDeleteFocus(row, button);
    button.disabled = false;
    button.textContent = "移动";
    saveProjectMoveProjection(ref, target, target.sortMs || rowSortMs(row, ref, target));
    if (target.kind === "projectless") moveRowToChats(row, target);
    refreshAfterProjectMove();
    showToast(message, null);
  }

  async function applyProjectMove(row, button, ref, target) {
    button.disabled = true;
    button.textContent = "移动中";
    try {
      if (target.kind === "projectless") {
        const result = await moveSessionToProjectless(ref);
        finishProjectMove(row, button, ref, { ...target, ...sortStateFromMoveResult(result, ref, row) }, `已移动到普通对话：“${ref.title || ref.session_id}”`);
      } else {
        const result = await moveSessionToProject(ref, target);
        finishProjectMove(row, button, ref, { ...target, ...sortStateFromMoveResult(result, ref, row) }, `已移动到“${target.label}”：“${ref.title || ref.session_id}”`);
      }
    } catch (error) {
      button.disabled = false;
      button.textContent = "移动";
      showToast(`移动失败：${error?.message || error}`, null);
    }
  }

  async function openProjectMoveMenuForRow(row, button, ref, event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    releaseDeleteFocus(row, button);
    document.querySelectorAll(`.${projectMoveOverlayClass}`).forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = projectMoveOverlayClass;
    overlay.innerHTML = `
      <div class="codex-project-move-panel" role="dialog" aria-modal="true" aria-label="移动对话">
        <div class="codex-project-move-header">
          <div class="codex-project-move-title">移动“${escapeHtml(ref.title || ref.session_id)}”</div>
        </div>
        <div class="codex-project-move-list"><div class="codex-project-move-empty">加载项目中...</div></div>
      </div>
    `;
    const panel = overlay.querySelector(".codex-project-move-panel");
    const rect = button.getBoundingClientRect();
    const panelWidth = Math.min(360, Math.max(240, window.innerWidth - 32));
    panel.style.left = `${Math.max(16, Math.min(window.innerWidth - panelWidth - 16, rect.right - panelWidth))}px`;
    panel.style.top = `${Math.max(16, Math.min(window.innerHeight - 120, rect.bottom + 6))}px`;
    const close = () => overlay.remove();
    overlay.addEventListener("click", (clickEvent) => {
      if (clickEvent.target === overlay) close();
    }, true);
    overlay.addEventListener("keydown", (keyEvent) => {
      if (keyEvent.key === "Escape") {
        keyEvent.preventDefault();
        close();
      }
    }, true);
    document.body.appendChild(overlay);
    try {
      const targets = projectMoveTargets();
      const list = overlay.querySelector(".codex-project-move-list");
      if (!list) return;
      list.innerHTML = "";
      if (targets.length === 0) {
        list.innerHTML = `<div class="codex-project-move-empty">没有可用目标</div>`;
        return;
      }
      for (const target of targets) {
        const item = document.createElement("button");
        item.type = "button";
        item.className = "codex-project-move-item";
        item.innerHTML = `
          <div class="codex-project-move-item-title">${escapeHtml(target.label)}</div>
          <div class="codex-project-move-item-path">${escapeHtml(target.description)}</div>
        `;
        item.addEventListener("click", async (selectEvent) => {
          selectEvent.preventDefault();
          selectEvent.stopPropagation();
          close();
          await applyProjectMove(row, button, ref, target);
        }, true);
        list.appendChild(item);
      }
      list.querySelector("button")?.focus();
    } catch (error) {
      close();
      showToast(`加载项目失败：${error?.message || error}`, null);
    }
  }

  function installDeleteButtonEventDelegation() {
    document.removeEventListener("pointerup", window.__codexSessionDeleteDocumentDeleteHandler, true);
    document.removeEventListener("click", window.__codexSessionDeleteDocumentDeleteHandler, true);
    const handler = (event) => {
      const button = event.target?.closest?.(`.${buttonClass}`);
      const row = button?.closest?.("[data-app-action-sidebar-thread-id]");
      if (!button || !row) return;
      const ref = sessionRefFromRow(row);
      if (!ref.session_id) return;
      openDeleteConfirmForRow(row, button, ref, event);
    };
    window.__codexSessionDeleteDocumentDeleteHandler = handler;
    document.addEventListener("pointerup", handler, true);
    document.addEventListener("click", handler, true);
  }

  function actionGroupFromRow(row) {
    return row.querySelector(`.${actionGroupClass}`);
  }

  function removeActionGroups(row) {
    row.querySelectorAll(`.${actionGroupClass}`).forEach((group) => group.remove());
  }

  function stopActionButtonEvent(row, button, event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    releaseDeleteFocus(row, button);
  }

  function installActionButtonEvents(row, button, onActivate) {
    ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
      button.addEventListener(eventName, (event) => stopActionButtonEvent(row, button, event), true);
    });
    button.addEventListener("pointerup", onActivate, true);
    button.addEventListener("click", onActivate, true);
  }

  function refreshActionButton(originalButton, row, onActivate) {
    if (!originalButton.isConnected) return;
    const replacement = originalButton.cloneNode(true);
    installActionButtonEvents(row, replacement, onActivate);
    originalButton.replaceWith(replacement);
  }

  function attachButton(row) {
    const settings = codexPlusSettings();
    if (!settings.sessionDelete && !settings.markdownExport && !settings.projectMove) {
      removeActionGroups(row);
      row.dataset.codexDeleteRow = "false";
      row.dataset.codexProjectMoveRow = "false";
      return;
    }
    const existingGroup = actionGroupFromRow(row);
    const existingDeleteButton = existingGroup?.querySelector(`.${buttonClass}`);
    const existingExportButton = existingGroup?.querySelector(`.${exportButtonClass}`);
    const existingMoveButton = existingGroup?.querySelector(`.${projectMoveButtonClass}`);
    const hasUnexpectedDelete = !settings.sessionDelete && !!existingDeleteButton;
    const hasUnexpectedExport = !settings.markdownExport && !!existingExportButton;
    const hasUnexpectedMove = !settings.projectMove && !!existingMoveButton;
    const missingDelete = settings.sessionDelete && !existingDeleteButton;
    const missingExport = settings.markdownExport && !existingExportButton;
    const missingMove = settings.projectMove && !existingMoveButton;
    const deleteReady = !settings.sessionDelete || existingDeleteButton?.dataset.codexDeleteVersion === codexDeleteVersion;
    const exportReady = !settings.markdownExport || existingExportButton?.dataset.codexExportVersion === codexExportVersion;
    const moveReady = !settings.projectMove || existingMoveButton?.dataset.codexProjectMoveVersion === codexProjectMoveVersion;
    const groupReady = existingGroup?.dataset.codexActionGroupVersion === codexActionGroupVersion;
    if (groupReady && deleteReady && exportReady && moveReady && !hasUnexpectedDelete && !hasUnexpectedExport && !hasUnexpectedMove && !missingDelete && !missingExport && !missingMove) return;
    removeActionGroups(row);
    row.dataset.codexDeleteRow = "false";
    row.dataset.codexProjectMoveRow = "false";
    const ref = sessionRefFromRow(row);
    if (!ref.session_id) return;
    row.dataset.codexDeleteRow = "true";
    row.dataset.codexProjectMoveRow = String(!!settings.projectMove);
    const group = document.createElement("div");
    group.className = actionGroupClass;
    group.dataset.codexActionGroupVersion = codexActionGroupVersion;
    if (settings.projectMove) {
      const moveButton = document.createElement("button");
      moveButton.type = "button";
      moveButton.className = `${actionButtonClass} ${projectMoveButtonClass}`;
      moveButton.dataset.codexProjectMoveVersion = codexProjectMoveVersion;
      moveButton.textContent = "移动";
      const openProjectMove = (event) => openProjectMoveMenuForRow(row, moveButton, ref, event);
      installActionButtonEvents(row, moveButton, openProjectMove);
      group.appendChild(moveButton);
      setTimeout(() => refreshActionButton(moveButton, row, openProjectMove), 0);
    }
    if (settings.markdownExport) {
      const exportButton = document.createElement("button");
      exportButton.type = "button";
      exportButton.className = `${actionButtonClass} ${exportButtonClass}`;
      exportButton.dataset.codexExportVersion = codexExportVersion;
      exportButton.textContent = "导出";
      const openExport = (event) => {
        stopActionButtonEvent(row, exportButton, event);
        exportMarkdown(ref);
      };
      installActionButtonEvents(row, exportButton, openExport);
      group.appendChild(exportButton);
      setTimeout(() => refreshActionButton(exportButton, row, openExport), 0);
    }
    if (settings.sessionDelete) {
      const deleteButton = document.createElement("button");
      deleteButton.type = "button";
      deleteButton.className = `${actionButtonClass} ${buttonClass}`;
      deleteButton.dataset.codexDeleteVersion = codexDeleteVersion;
      deleteButton.textContent = "删除";
      const openDeleteConfirm = (event) => openDeleteConfirmForRow(row, deleteButton, ref, event);
      installActionButtonEvents(row, deleteButton, openDeleteConfirm);
      group.appendChild(deleteButton);
      setTimeout(() => refreshActionButton(deleteButton, row, openDeleteConfirm), 0);
    }
    row.appendChild(group);
  }

  function tryAttachButton(row) {
    try {
      attachButton(row);
    } catch (error) {
      window.__codexSessionDeleteAttachButtonFailures = window.__codexSessionDeleteAttachButtonFailures || [];
      window.__codexSessionDeleteAttachButtonFailures.push(String(error?.stack || error));
    }
  }

  function reactArchivedThreadFromNode(node) {
    const reactKey = Object.keys(node).find((key) => key.startsWith("__reactFiber$") || key.startsWith("__reactInternalInstance$"));
    let fiber = reactKey ? node[reactKey] : null;
    for (let depth = 0; fiber && depth < 20; depth += 1, fiber = fiber.return) {
      const props = fiber.memoizedProps || fiber.pendingProps || {};
      if (props.archivedThread?.id) return props.archivedThread;
      const childThread = props.children?.props?.archivedThread;
      if (childThread?.id) return childThread;
    }
    return null;
  }

  function archivedThreadFromRow(row) {
    for (const node of [row, ...row.querySelectorAll("*")]) {
      const thread = reactArchivedThreadFromNode(node);
      if (thread?.id || thread?.sessionId) return thread;
    }
    return null;
  }

  function archivedRefFromRow(row) {
    const archivedThread = archivedThreadFromRow(row);
    if (archivedThread?.id || archivedThread?.sessionId) {
      return { session_id: archivedThread.id || archivedThread.sessionId, title: archivedThread.title || row.querySelector(".truncate.text-base")?.textContent?.trim() || "Untitled session" };
    }
    const sidebarRef = sessionRefFromRow(row);
    if (sidebarRef.session_id) return sidebarRef;
    const titleNode = row.querySelector(".truncate.text-base, [data-thread-title], a, div");
    const title = ((titleNode || row).textContent || "Untitled session")
      .replace("取消归档", "")
      .replace("删除", "")
      .replace(/\d{4}年\d{1,2}月\d{1,2}日.*$/, "")
      .replace(/\s+·\s+.*$/, "")
      .trim()
      .slice(0, 160);
    return { session_id: "", title };
  }

  async function resolveArchivedThread(row) {
    const ref = archivedRefFromRow(row);
    if (ref.session_id) return ref;
    const resolved = await postJson("/archived-thread", { title: ref.title });
    return resolved?.session_id ? resolved : ref;
  }

  function stopArchivedButtonEvent(event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
  }

  function isArchiveTitleText(value) {
    return value === "已归档对话" || value === "Archived conversations";
  }

  function archiveTitleContainer() {
    const heading = Array.from(document.querySelectorAll("h1, h2, h3"))
      .find((element) => isArchiveTitleText((element.textContent || "").trim()));
    if (heading) return heading;
    return Array.from(document.querySelectorAll("h1, h2, h3, div, span"))
      .find((element) => isArchiveTitleText((element.textContent || "").trim()) && element.getBoundingClientRect().x > 350);
  }

  async function deleteArchivedSessions(rows) {
    let deleted = 0;
    for (const row of rows) {
      const ref = await resolveArchivedThread(row);
      if (!ref.session_id) continue;
      const result = await postJson("/delete", ref);
      if (result.status === "server_deleted" || result.status === "local_deleted") {
        row.remove();
        deleted += 1;
      }
    }
    showToast(`已删除 ${deleted} 个归档会话`, null);
  }

  function attachArchivedPageDeleteButton(row) {
    const settings = codexPlusSettings();
    row.querySelectorAll("[data-codex-archive-row-action]").forEach((button) => button.remove());
    row.dataset.codexArchiveDeleteRow = "false";
    if (!settings.sessionDelete && !settings.markdownExport) return;
    const unarchiveButton = Array.from(row.querySelectorAll("button")).find((button) => (button.textContent || "").trim() === "取消归档");
    if (!unarchiveButton) return;
    row.dataset.codexArchiveDeleteRow = "true";
    row.dataset.codexArchiveRowActionsVersion = codexArchiveRowActionsVersion;
    let insertionPoint = unarchiveButton;
    if (settings.markdownExport) {
      const exportButton = document.createElement("button");
      exportButton.type = "button";
      exportButton.className = `codex-archive-delete-all codex-archive-row-button ${exportButtonClass}`;
      exportButton.dataset.codexArchiveRowAction = "export";
      exportButton.textContent = "导出";
      ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
        exportButton.addEventListener(eventName, stopArchivedButtonEvent, true);
      });
      exportButton.addEventListener("click", async (event) => {
        stopArchivedButtonEvent(event);
        const ref = await resolveArchivedThread(row);
        if (!ref.session_id) {
          showToast("导出失败：未找到归档会话 ID", null);
          return;
        }
        await exportMarkdown(ref);
      }, true);
      insertionPoint.insertAdjacentElement("afterend", exportButton);
      insertionPoint = exportButton;
    }
    if (settings.sessionDelete) {
      const deleteButton = document.createElement("button");
      deleteButton.type = "button";
      deleteButton.className = `codex-archive-delete-all codex-archive-row-button ${buttonClass}`;
      deleteButton.dataset.codexArchiveRowAction = "delete";
      deleteButton.textContent = "删除";
      ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
        deleteButton.addEventListener(eventName, stopArchivedButtonEvent, true);
      });
      deleteButton.addEventListener("click", async (event) => {
        stopArchivedButtonEvent(event);
        const ref = await resolveArchivedThread(row);
        if (!ref.session_id) {
          showToast("删除失败：未找到归档会话 ID", null);
          return;
        }
        if (!(await confirmDelete(ref.title))) return;
        const result = await postJson("/delete", ref);
        if (result.status === "server_deleted" || result.status === "local_deleted") {
          row.remove();
          showToast(result.message || "删除成功", result.undo_token);
        } else {
          showToast(result.message || "删除失败", null);
        }
      }, true);
      insertionPoint.insertAdjacentElement("afterend", deleteButton);
    }
  }

  function installArchivedDeleteAllButton() {
    const existingButton = document.querySelector("[data-codex-archive-delete-all]");
    if (!codexPlusSettings().sessionDelete || !archivedPageVisible()) {
      existingButton?.remove();
      return;
    }
    const rows = archivedRows();
    if (rows.length === 0) {
      existingButton?.remove();
      return;
    }
    if (existingButton?.dataset.codexArchiveDeleteAllVersion === codexArchiveDeleteAllVersion) return;
    existingButton?.remove();
    const button = document.createElement("button");
    button.type = "button";
    button.className = "codex-archive-delete-all codex-archive-action-bar";
    Object.assign(button.style, {
      position: "static",
      marginLeft: "12px",
      verticalAlign: "middle",
      zIndex: "2147482999",
      cursor: "pointer",
      pointerEvents: "auto",
      maxWidth: "fit-content",
      alignSelf: "flex-start",
    });
    button.dataset.codexArchiveDeleteAll = "true";
    button.dataset.codexArchiveDeleteAllVersion = codexArchiveDeleteAllVersion;
    button.textContent = "删除全部归档";
    ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
      button.addEventListener(eventName, stopArchivedButtonEvent, true);
    });
    const openArchivedDeleteAllConfirm = async (event) => {
      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation?.();
      const currentRows = archivedRows();
      if (currentRows.length === 0) return;
      if (!(await confirmDelete(`全部 ${currentRows.length} 个归档会话`))) return;
      await deleteArchivedSessions(currentRows);
    };
    button.addEventListener("pointerup", openArchivedDeleteAllConfirm, true);
    button.addEventListener("click", openArchivedDeleteAllConfirm, true);
    const title = archiveTitleContainer();
    if (title) {
      title.insertAdjacentElement("afterend", button);
    } else {
      document.body.appendChild(button);
    }
  }

  function truncateTimelineQuestion(text) {
    const normalized = String(text || "").replace(/\s+/g, " ").trim();
    const chars = Array.from(normalized);
    if (chars.length <= timelineQuestionLimit) return normalized;
    return `${chars.slice(0, timelineQuestionLimit).join("")}…`;
  }

  function conversationTimelineRoot() {
    return document.querySelector(".thread-scroll-container") || document.querySelector("main") || document.querySelector('[role="main"]');
  }

  function timelineQuestionSelector() {
    return [
      '[data-message-author-role="user"]',
      '[data-testid="conversation-turn"][data-message-author-role="user"]',
      '[data-testid="conversation-turn"] [data-message-author-role="user"]',
      '[class*="user-message"]',
      '[class*="UserMessage"]',
    ].join(", ");
  }

  function nodeOrAncestorLooksLikeCodexUserBubble(node) {
    if (node.nodeType !== 1) return false;
    const className = String(node.className || "");
    if (className.includes("bg-token-foreground/5") && node.parentElement?.classList?.contains("items-end")) return true;
    const bubble = node.closest?.("[class*='bg-token-foreground/5']");
    return !!bubble?.parentElement?.classList?.contains("items-end");
  }

  function nodeLooksLikeCodexUserBubble(node) {
    if (nodeOrAncestorLooksLikeCodexUserBubble(node)) return true;
    return !!node.querySelector?.(".group.flex.w-full.flex-col.items-end.justify-end.gap-1 > [class*='bg-token-foreground/5']");
  }

  function nodeLooksLikeTimelineQuestion(node) {
    if (node.nodeType !== 1 || isExtensionUiNode(node)) return false;
    const questionSelector = timelineQuestionSelector();
    return !!node.matches?.(questionSelector) || !!node.closest?.(questionSelector) || !!node.querySelector?.(questionSelector) || nodeLooksLikeCodexUserBubble(node);
  }

  function conversationTimelineQuestionCandidates(root) {
    const explicitCandidates = Array.from(root.querySelectorAll([
      '[data-message-author-role="user"]',
      '[data-testid="conversation-turn"][data-message-author-role="user"]',
      '[data-testid="conversation-turn"] [data-message-author-role="user"]',
      '[class*="user-message"]',
      '[class*="UserMessage"]',
    ].join(", ")));
    const codexUserBubbles = Array.from(root.querySelectorAll(".group.flex.w-full.flex-col.items-end.justify-end.gap-1")).flatMap((group) => {
      return Array.from(group.children).filter((child) => String(child.className || "").includes("bg-token-foreground/5"));
    });
    return [...explicitCandidates, ...codexUserBubbles];
  }

  function extractTimelineQuestionText(node) {
    const clone = node.cloneNode(true);
    clone.querySelectorAll("button, svg, [aria-hidden='true'], .sr-only").forEach((child) => child.remove());
    return clone.textContent.replace(/\s+/g, " ").trim();
  }

  function timelineNodeId(node) {
    if (!node.__codexConversationTimelineNodeId) {
      window.__codexConversationTimelineNodeCounter += 1;
      node.__codexConversationTimelineNodeId = String(window.__codexConversationTimelineNodeCounter);
    }
    return node.__codexConversationTimelineNodeId;
  }

  function visibleTimelineNode(node) {
    if (!node.isConnected) return false;
    const style = getComputedStyle(node);
    if (style.display === "none" || style.visibility === "hidden") return false;
    const rect = node.getBoundingClientRect();
    return rect.width > 0 || rect.height > 0 || !!node.textContent?.trim();
  }

  function conversationTimelineQuestions() {
    const root = conversationTimelineRoot();
    if (!root?.matches?.('.thread-scroll-container, main, [role="main"]')) return [];
    const seen = new Set();
    return conversationTimelineQuestionCandidates(root).flatMap((node) => {
      if (node.closest('[data-app-action-sidebar-thread-id]')) return [];
      if (isExtensionUiNode(node)) return [];
      const target = node.closest('[data-testid="conversation-turn"]') || node;
      if (seen.has(target)) return [];
      seen.add(target);
      if (!visibleTimelineNode(target)) return [];
      const text = extractTimelineQuestionText(node);
      if (!text) return [];
      return [{ node: target, text, nodeId: timelineNodeId(target) }];
    });
  }

  function timelineScrollerViewportTop(scroller) {
    if (scroller === document.scrollingElement || scroller === document.documentElement || scroller === document.body) return 0;
    return scroller.getBoundingClientRect().top;
  }

  function timelineScrollableHeight(scroller) {
    return Math.max(1, scroller.scrollHeight - scroller.clientHeight);
  }

  function timelineRawMarkerTop(question, scroller) {
    const scrollOffset = scroller.scrollTop + question.node.getBoundingClientRect().top - timelineScrollerViewportTop(scroller);
    const percent = (scrollOffset / timelineScrollableHeight(scroller)) * 100;
    return Math.max(timelineMinTopPercent, Math.min(timelineMaxTopPercent, percent));
  }

  function timelineMarkerTops(questions, scroller) {
    if (questions.length <= 1) return [50];
    const minGap = Math.min(timelineMaxMarkerGapPercent, (timelineMaxTopPercent - timelineMinTopPercent) / Math.max(questions.length - 1, 1));
    const tops = questions.map((question) => timelineRawMarkerTop(question, scroller));
    for (let index = 1; index < tops.length; index += 1) {
      tops[index] = Math.max(tops[index], tops[index - 1] + minGap);
    }
    for (let index = tops.length - 1; index >= 0; index -= 1) {
      const maxForIndex = timelineMaxTopPercent - ((tops.length - 1 - index) * minGap);
      tops[index] = Math.min(tops[index], maxForIndex);
    }
    return tops.map((top) => Math.max(timelineMinTopPercent, Math.min(timelineMaxTopPercent, top)));
  }

  function removeConversationTimeline() {
    document.querySelectorAll(`.${timelineClass}`).forEach((node) => node.remove());
  }

  function nearestTimelineScroller(node) {
    for (let current = node?.parentElement; current; current = current.parentElement) {
      const style = getComputedStyle(current);
      if (/(auto|scroll)/.test(style.overflowY) && current.scrollHeight > current.clientHeight) return current;
    }
    return document.querySelector(".thread-scroll-container") || document.scrollingElement || document.documentElement;
  }

  function scrollTimelineTarget(node) {
    const scroller = nearestTimelineScroller(node);
    const nodeRect = node.getBoundingClientRect();
    const nextTop = scroller.scrollTop + nodeRect.top - timelineScrollerViewportTop(scroller) - (scroller.clientHeight / 2) + (nodeRect.height / 2);
    scroller.scrollTo({ top: nextTop, behavior: "smooth" });
  }

  function highlightTimelineTarget(node) {
    node.classList.remove(timelineTargetClass);
    void node.offsetWidth;
    node.classList.add(timelineTargetClass);
    clearTimeout(node.__codexConversationTimelineHighlightTimer);
    node.__codexConversationTimelineHighlightTimer = setTimeout(() => {
      node.classList.remove(timelineTargetClass);
    }, 1300);
  }

  function createConversationTimelineMarker(question) {
    const marker = document.createElement("button");
    marker.type = "button";
    marker.className = timelineMarkerClass;
    marker.style.top = `${question.markerTop}%`;
    marker.setAttribute("aria-label", `跳转到：${truncateTimelineQuestion(question.text)}`);
    const tooltip = document.createElement("span");
    tooltip.className = timelineTooltipClass;
    tooltip.id = `codex-conversation-timeline-tooltip-${question.nodeId}`;
    tooltip.setAttribute("role", "tooltip");
    tooltip.textContent = truncateTimelineQuestion(question.text);
    marker.setAttribute("aria-describedby", tooltip.id);
    marker.appendChild(tooltip);
    const activateMarker = (event) => {
      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation?.();
      document.querySelectorAll(`.${timelineMarkerClass}.codex-conversation-timeline-marker-active`).forEach((node) => {
        node.classList.remove("codex-conversation-timeline-marker-active");
      });
      marker.classList.add("codex-conversation-timeline-marker-active");
      scrollTimelineTarget(question.node);
      highlightTimelineTarget(question.node);
    };
    marker.addEventListener("pointerup", activateMarker, true);
    marker.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") activateMarker(event);
    }, true);
    return marker;
  }

  function prepareTimelineQuestions(questions) {
    if (questions.length === 0) return [];
    const scroller = nearestTimelineScroller(questions[0].node);
    const tops = timelineMarkerTops(questions, scroller);
    return questions.map((question, index) => ({ ...question, markerTop: Number(tops[index].toFixed(3)) }));
  }

  function timelineSignature(questions) {
    return questions.map((question) => `${question.nodeId}:${Math.round(question.markerTop * 10)}:${truncateTimelineQuestion(question.text)}`).join("|");
  }

  function refreshConversationTimeline() {
    if (!codexPlusSettings().conversationTimeline) {
      removeConversationTimeline();
      return;
    }
    const questions = prepareTimelineQuestions(conversationTimelineQuestions());
    if (questions.length === 0) {
      removeConversationTimeline();
      return;
    }
    const signature = timelineSignature(questions);
    const existing = document.querySelector(`.${timelineClass}`);
    if (
      existing?.dataset.codexConversationTimelineVersion === codexConversationTimelineVersion &&
      existing?.dataset.codexConversationTimelineSignature === signature
    ) {
      return;
    }
    removeConversationTimeline();
    const container = document.createElement("div");
    container.className = timelineClass;
    container.dataset.codexConversationTimelineVersion = codexConversationTimelineVersion;
    container.dataset.codexConversationTimelineSignature = signature;
    const track = document.createElement("div");
    track.className = timelineTrackClass;
    container.appendChild(track);
    questions.forEach((question) => {
      container.appendChild(createConversationTimelineMarker(question));
    });
    document.body.appendChild(container);
  }

  function scanLightweight() {
    installStyle();
    installCodexPlusMenu();
    scheduleBackendHeartbeat();
    patchCodexModelWhitelist();
    installDeleteButtonEventDelegation();
    updateThreadScrollHandlers();
    installThreadScrollProgrammaticScrollGuard();
    installThreadScrollNavigationCapture();
    installThreadScrollUserIntentCapture();
    installThreadScrollRouteHooks();
    scheduleThreadScrollSync(true);
  }

  let zedRemoteStatusPromise = null;
  const zedRemoteMissingHostMessage = "Cannot determine remote SSH host for this file";

  function showZedRemoteToast(message) {
    document.querySelectorAll(`.${zedRemoteToastClass}`).forEach((node) => node.remove());
    const toast = document.createElement("div");
    toast.className = zedRemoteToastClass;
    toast.textContent = message;
    document.body.appendChild(toast);
    setTimeout(() => toast.remove(), 3200);
  }

  async function loadZedRemoteStatus() {
    zedRemoteStatusPromise = zedRemoteStatusPromise || postJson("/zed-remote/status", {});
    return zedRemoteStatusPromise;
  }

  async function resolveZedRemoteHost(hostId) {
    const result = await postJson("/zed-remote/resolve-host", { hostId });
    return result?.status === "ok" && result.ssh ? result.ssh : null;
  }

  function zedRemoteString(value) {
    return typeof value === "string" || typeof value === "number" ? String(value).trim() : "";
  }

  function zedRemoteTruthy(value) {
    if (value === true) return true;
    if (typeof value === "string") return /^(true|1|yes|enabled|ssh)$/i.test(value.trim());
    return false;
  }

  function zedRemoteHasTrustedSshSignal(source, hostConfig) {
    return zedRemoteTruthy(source?.supportsSsh) || zedRemoteTruthy(hostConfig?.supportsSsh);
  }

  function zedRemoteContextFromObject(source) {
    if (!source || typeof source !== "object") return null;
    const hostConfig = source.hostConfig || source.sshHostConfig || source.remoteHostConfig || source.ssh || {};
    const host = zedRemoteString(source.remoteHost || source.sshHost || source.host || source.hostname || source.hostName || hostConfig.host || hostConfig.hostname || hostConfig.hostName || hostConfig.sshHost);
    const hostId = zedRemoteString(source.hostId);
    const cwd = zedRemoteString(source.cwd || source.workspaceRoot || source.rootPath || source.remoteWorkspaceRoot || hostConfig.remoteWorkspaceRoot || hostConfig.workspaceRoot || hostConfig.rootPath);
    if ((!host || !zedRemoteHasTrustedSshSignal(source, hostConfig)) && !(hostId.startsWith("remote-ssh-") && cwd.startsWith("/"))) return null;
    const user = zedRemoteString(source.remoteUser || source.sshUser || source.user || source.username || hostConfig.user || hostConfig.username || hostConfig.sshUser);
    const port = zedRemoteString(source.remotePort || source.sshPort || source.port || hostConfig.port || hostConfig.sshPort);
    const workspaceRoot = cwd;
    return { hostId, ssh: { user, host, port }, workspaceRoot };
  }

  function zedRemoteWalkObject(root, visitor, options = {}) {
    const maxDepth = options.maxDepth || 6;
    const maxNodes = options.maxNodes || 180;
    const visited = new WeakSet();
    const stack = [{ value: root, depth: 0 }];
    let scanned = 0;
    while (stack.length && scanned < maxNodes) {
      const { value, depth } = stack.pop();
      if (!value || typeof value !== "object" || visited.has(value) || depth > maxDepth) continue;
      visited.add(value);
      scanned += 1;
      const result = visitor(value);
      if (result) return result;
      if (value instanceof Element || value === window || value === document || value === document.body || value === document.documentElement) continue;
      for (const key of Object.keys(value).slice(0, 80)) {
        if (key === "ownerDocument" || key === "parentElement" || key === "parentNode" || key === "children" || key === "childNodes") continue;
        let child;
        try {
          child = value[key];
        } catch {
          continue;
        }
        if (child && typeof child === "object") stack.push({ value: child, depth: depth + 1 });
      }
    }
    return null;
  }

  function zedRemoteReactKeys(element) {
    return Object.keys(element).filter((key) => key.startsWith("__reactFiber") || key.startsWith("__reactInternalInstance") || key.startsWith("__reactProps"));
  }

  function zedRemoteContextFromElement(element) {
    for (const key of zedRemoteReactKeys(element)) {
      const context = zedRemoteWalkObject(element[key], zedRemoteContextFromObject);
      if (context) return context;
    }
    return null;
  }

  function zedRemoteContextForElement(element) {
    for (let node = element; node && node !== document.body; node = node.parentElement) {
      const context = zedRemoteContextFromElement(node);
      if (context) return context;
    }
    return zedRemoteContext();
  }

  function zedRemoteHostIdFromText(text) {
    const source = String(text || "");
    const match = source.match(/\bremote-ssh-[A-Za-z0-9:_-]+\b/);
    return match ? match[0] : "";
  }

  function zedRemoteWorkspaceRootForPath(path) {
    const source = String(path || "").trim();
    const projects = Array.from(document.querySelectorAll(selectors.sidebarThread))
      .map((row) => ({
        label: (row.textContent || "").replace(/\s+/g, " ").trim(),
        selected: row.getAttribute("aria-current") === "page" || row.getAttribute("data-selected") === "true" || row.getAttribute("data-active") === "true" || row.className.includes("selected"),
      }))
      .filter((row) => row.label);
    const selected = projects.find((row) => row.selected)?.label || "";
    for (const label of [selected, ...projects.map((row) => row.label)]) {
      const name = label.match(/^([A-Za-z0-9._-]+)/)?.[1];
      if (name && source.includes(`/repo/${name}/`)) return source.slice(0, source.indexOf(`/repo/${name}/`) + `/repo/${name}`.length);
    }
    const repoIndex = source.indexOf("/bin/repo/");
    if (repoIndex >= 0) {
      const afterRepo = source.slice(repoIndex + "/bin/repo/".length);
      const project = afterRepo.split("/")[0];
      if (project) return source.slice(0, repoIndex + "/bin/repo/".length + project.length);
    }
    return source;
  }

  function zedRemoteFallbackContextForElement(element) {
    const pathText = (element.textContent || "").trim();
    if (!pathText.startsWith("/")) return null;
    const root = element.closest("main") || document.body;
    const hostId = zedRemoteHostIdFromText(root?.textContent || "") || "remote-ssh-codex-managed:remote";
    return { hostId, ssh: { user: "", host: "", port: "" }, workspaceRoot: zedRemoteWorkspaceRootForPath(pathText) };
  }

  function zedRemoteContextFromSerializedState(text) {
    const source = String(text || "");
    if (!source.includes("hostConfig") || !source.includes("supportsSsh") || !source.includes("remoteWorkspaceRoot")) return null;
    const trimmed = source.trim();
    if (/^[{[]/.test(trimmed)) {
      try {
        const parsed = JSON.parse(trimmed);
        const context = zedRemoteWalkObject(parsed, zedRemoteContextFromObject, { maxDepth: 10, maxNodes: 300 });
        if (context) return context;
      } catch {
      }
    }
    if (!/['"]supportsSsh['"]\s*:\s*true/.test(source)) return null;
    const fieldValue = (name) => {
      const match = source.match(new RegExp(`["']${name}["']\\s*:\\s*["']([^"']+)["']`));
      return match ? match[1] : "";
    };
    const host = fieldValue("host") || fieldValue("hostname") || fieldValue("hostName") || fieldValue("sshHost") || fieldValue("remoteHost");
    if (!host) return null;
    return {
      ssh: {
        user: fieldValue("user") || fieldValue("username") || fieldValue("sshUser") || fieldValue("remoteUser"),
        host,
        port: fieldValue("port") || fieldValue("sshPort") || fieldValue("remotePort"),
      },
      workspaceRoot: fieldValue("remoteWorkspaceRoot") || fieldValue("workspaceRoot") || fieldValue("rootPath"),
    };
  }

  function zedRemoteContext() {
    const settings = codexPlusSettings();
    if (!settings.zedRemoteOpen) return null;
    const explicitNodes = document.querySelectorAll("[data-host-config], [data-ssh-host], [data-remote-host], [data-remote-workspace-root], [data-supports-ssh]");
    for (const node of explicitNodes) {
      if (!(node instanceof HTMLElement)) continue;
      const data = node.dataset;
      const context = zedRemoteContextFromObject({
        hostConfig: data.hostConfig ? { host: data.hostConfig, supportsSsh: true } : {},
        supportsSsh: data.supportsSsh || data.supportsSshRemote,
        sshHost: data.sshHost,
        remoteHost: data.remoteHost,
        host: data.host,
        sshUser: data.sshUser,
        remoteUser: data.remoteUser,
        user: data.user,
        sshPort: data.sshPort,
        remotePort: data.remotePort,
        port: data.port,
        remoteWorkspaceRoot: data.remoteWorkspaceRoot,
        workspaceRoot: data.workspaceRoot,
      });
      if (context) return context;
    }
    const reactNodes = [document.body, ...document.querySelectorAll("[data-remote-path], [data-file-path], [data-path], [data-open-in-targets], [data-open-file], span.inline-markdown, code, [class*='inlineMarkdown'], a, button, [role='button'], [role='menuitem'], [role='treeitem']")].filter(Boolean);
    for (const node of reactNodes.slice(0, 260)) {
      if (!(node instanceof HTMLElement) || isExtensionUiNode(node)) continue;
      const context = zedRemoteContextFromElement(node);
      if (context) return context;
    }
    const scripts = Array.from(document.querySelectorAll("script[type='application/json'], script[data-state], script#__NEXT_DATA__, script:not([src])"));
    for (const script of scripts.slice(0, 40)) {
      const context = zedRemoteContextFromSerializedState(script.textContent || "");
      if (context) return context;
    }
    return null;
  }

  function zedRemoteAbsolutePath(value, workspaceRoot) {
    const text = String(value || "").trim();
    if (!text) return "";
    if (text.startsWith("/")) return text;
    if (workspaceRoot && !text.includes("://") && !text.startsWith("~")) {
      return `${workspaceRoot.replace(/\/+$/, "")}/${text.replace(/^\.\//, "")}`;
    }
    return "";
  }

  function zedRemoteMetadataRemotePath(source) {
    if (!source || typeof source !== "object") return "";
    return zedRemoteString(source.remotePath || source.remote_path || source.path || source.filePath || source.file_path || source.openFile?.remotePath || source.openFile?.path);
  }

  function zedRemotePathFromElementMetadata(element) {
    const dataPath = element.dataset.remotePath || element.dataset.filePath || element.dataset.path || "";
    if (dataPath) return dataPath;
    for (const key of zedRemoteReactKeys(element)) {
      const path = zedRemoteWalkObject(element[key], zedRemoteMetadataRemotePath, { maxDepth: 6, maxNodes: 120 });
      if (path) return path;
    }
    return "";
  }

  function zedRemoteInlinePathFromElement(element, context) {
    if (!context?.hostId && !context?.ssh?.host) return "";
    const text = (element.textContent || "").trim();
    if (!text || text.length > 600 || !text.startsWith("/")) return "";
    const path = zedRemoteAbsolutePath(text, context.workspaceRoot || "");
    if (!path) return "";
    if (context.workspaceRoot && !path.startsWith(`${context.workspaceRoot.replace(/\/+$/, "")}/`) && path !== context.workspaceRoot) return "";
    return path;
  }

  function zedRemoteAnchorHasOpenFileMetadata(anchor) {
    if (!(anchor instanceof HTMLAnchorElement)) return false;
    if (anchor.dataset.remotePath || anchor.dataset.filePath || anchor.dataset.path || anchor.dataset.openInTargets || anchor.dataset.openFile || anchor.dataset.codexOpenFile) return true;
    const label = `${anchor.getAttribute("aria-label") || ""} ${anchor.getAttribute("data-testid") || ""} ${anchor.getAttribute("rel") || ""}`;
    return /open[-_\s]?file|open-in-targets|remote/i.test(label) && !!zedRemotePathFromElementMetadata(anchor);
  }

  function zedRemoteFileCandidates(context) {
    const candidates = [];
    const seen = new Set();
    const addCandidate = (node, candidateContext, rawPath) => {
      if (!candidateContext?.ssh?.host && !candidateContext?.hostId) return;
      const path = zedRemoteAbsolutePath(rawPath, candidateContext.workspaceRoot || "");
      if (!path || seen.has(path)) return;
      seen.add(path);
      candidates.push({ node, request: { ssh: candidateContext.ssh, hostId: candidateContext.hostId || "", path } });
    };
    const selectors = "[data-remote-path], [data-file-path], [data-path], [data-open-in-targets], [data-open-file], [data-codex-open-file], a[data-remote-path], a[data-file-path], a[data-path]";
    document.querySelectorAll(selectors).forEach((node) => {
      if (!(node instanceof HTMLElement) || isExtensionUiNode(node)) return;
      if (node instanceof HTMLAnchorElement && !zedRemoteAnchorHasOpenFileMetadata(node)) return;
      addCandidate(node, zedRemoteContextForElement(node) || context, zedRemotePathFromElementMetadata(node));
    });
    document.querySelectorAll("span.inline-markdown, code, [class*='inlineMarkdown']").forEach((node) => {
      if (!(node instanceof HTMLElement) || isExtensionUiNode(node)) return;
      const candidateContext = zedRemoteContextForElement(node) || context || zedRemoteFallbackContextForElement(node);
      if (!candidateContext?.hostId && !candidateContext?.ssh?.host) return;
      const path = zedRemoteInlinePathFromElement(node, candidateContext);
      if (path) addCandidate(node, candidateContext, path);
    });
    return candidates;
  }

  function zedRemoteBestOpenRequest(context = zedRemoteContext() || {}) {
    const candidates = zedRemoteFileCandidates(context);
    if (candidates.length) return candidates[0].request;
    const workspaceRoot = zedRemoteAbsolutePath(context.workspaceRoot || "", "");
    if (!workspaceRoot || (!context?.ssh?.host && !context?.hostId)) return null;
    return { ssh: context.ssh, hostId: context.hostId || "", path: workspaceRoot };
  }

  async function openZedRemote(request) {
    let nextRequest = request;
    if (!nextRequest?.ssh?.host && nextRequest?.hostId) {
      const ssh = await resolveZedRemoteHost(nextRequest.hostId);
      nextRequest = ssh ? { ...nextRequest, ssh } : nextRequest;
    }
    if (!nextRequest?.ssh?.host) {
      showZedRemoteToast(zedRemoteMissingHostMessage);
      return;
    }
    try {
      const result = await postJson("/zed-remote/open", nextRequest);
      if (result?.status === "ok") {
        showZedRemoteToast("Opened in Zed Remote");
        return;
      }
      showZedRemoteToast(result?.message || "Cannot open this file in Zed Remote");
    } catch (error) {
      showZedRemoteToast(error?.message || "Cannot open this file in Zed Remote");
    }
  }

  function openBestZedRemoteTarget() {
    const request = zedRemoteBestOpenRequest();
    if (!request) {
      showZedRemoteToast("Cannot find a remote workspace or file for Zed Remote");
      return;
    }
    openZedRemote(request);
  }

  function attachZedRemoteButton(candidate) {
    const anchor = candidate.node;
    if (anchor.dataset.codexZedRemoteVersion === zedRemoteOpenVersion) return;
    anchor.dataset.codexZedRemoteVersion = zedRemoteOpenVersion;
    const button = document.createElement("button");
    button.type = "button";
    button.className = zedRemoteButtonClass;
    button.textContent = "Open in Zed Remote";
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      openZedRemote(candidate.request);
    }, true);
    anchor.insertAdjacentElement("afterend", button);
  }

  function removeZedRemoteButtons() {
    document.querySelectorAll(`[data-codex-zed-remote-version]`).forEach((node) => {
      delete node.dataset.codexZedRemoteVersion;
    });
    document.querySelectorAll(`.${zedRemoteButtonClass}`).forEach((node) => node.remove());
  }

  function createZedRemoteOpenInMenuItem(referenceItem) {
    const item = document.createElement("div");
    item.className = referenceItem?.className || "no-drag text-token-foreground outline-hidden rounded-lg px-[var(--padding-row-x)] py-[var(--padding-row-y)] text-sm group hover:bg-token-list-hover-background focus:bg-token-list-hover-background cursor-interaction flex flex-col";
    item.classList.add(zedRemoteOpenInMenuItemClass);
    item.setAttribute("role", referenceItem?.getAttribute("role") || "menuitem");
    item.setAttribute("tabindex", referenceItem?.getAttribute("tabindex") || "-1");
    item.setAttribute("data-orientation", referenceItem?.getAttribute("data-orientation") || "vertical");
    item.innerHTML = `
      <div class="flex w-full items-center gap-1.5">
        <span class="inline-flex size-[18px] items-center justify-center leading-none shrink-0 opacity-75 group-focus:opacity-100 group-hover:opacity-100">
          <img alt="" class="codex-zed-open-in-menu-icon icon-sm" src="apps/zed.png">
        </span>
        <span class="flex-1 min-w-0 truncate">Zed</span>
      </div>
    `;
    bindZedRemoteOpenInMenuItem(item, "injected");
    return item;
  }

  function zedRemoteOpenInMenuActivationIsDuplicate(target) {
    if (!(target instanceof HTMLElement)) return false;
    const now = Date.now();
    const activatedAt = Number(target.dataset.codexZedOpenInMenuActivatedAt || 0);
    if (activatedAt && now - activatedAt < zedRemoteOpenInMenuActivationWindowMs) return true;
    target.dataset.codexZedOpenInMenuActivatedAt = String(now);
    return false;
  }

  function activateZedRemoteOpenInMenuItem(event) {
    if (!codexPlusSettings().zedRemoteOpen) return;
    if (event?.type === "keydown" && !["Enter", " "].includes(event.key)) return;
    const request = zedRemoteBestOpenRequest();
    if (!request) return;
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    if (zedRemoteOpenInMenuActivationIsDuplicate(event?.currentTarget)) return;
    openZedRemote(request);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true }));
  }

  function bindZedRemoteOpenInMenuItem(item, source) {
    item.setAttribute("data-codex-zed-open-in-menu", source);
    if (item.dataset.codexZedOpenInMenuBound === zedRemoteOpenInMenuVersion) return;
    item.dataset.codexZedOpenInMenuBound = zedRemoteOpenInMenuVersion;
    item.dataset.codexZedOpenInMenuVersion = zedRemoteOpenInMenuVersion;
    item.addEventListener("pointerup", activateZedRemoteOpenInMenuItem, true);
    item.addEventListener("click", activateZedRemoteOpenInMenuItem, true);
    item.addEventListener("keydown", activateZedRemoteOpenInMenuItem, true);
  }

  function removeZedRemoteOpenInMenuItems() {
    document.querySelectorAll(`.${zedRemoteOpenInMenuItemClass}, [data-codex-zed-open-in-menu="injected"]`).forEach((node) => node.remove());
  }

  function refreshZedRemoteOpenInMenus(context) {
    removeZedRemoteOpenInMenuItems();
    if (!codexPlusSettings().zedRemoteOpen) return;
    const request = zedRemoteBestOpenRequest(context || zedRemoteContext() || {});
    if (!request) return;
    document.querySelectorAll('[role="menu"]').forEach((menu) => {
      if (!(menu instanceof HTMLElement) || isExtensionUiNode(menu)) return;
      const items = Array.from(menu.querySelectorAll('[role="menuitem"]')).filter((item) => !isExtensionUiNode(item));
      const menuText = items.map((item) => (item.textContent || "").trim()).join(" ");
      if (!/\b(VS Code|Cursor|Antigravity)\b/.test(menuText)) return;
      const existingZedItem = items.find((item) => (item.textContent || "").trim() === "Zed");
      if (existingZedItem) {
        bindZedRemoteOpenInMenuItem(existingZedItem, "native");
        return;
      }
      const referenceItem = items.find((item) => /^(VS Code|Cursor|Antigravity)$/.test((item.textContent || "").trim()));
      if (!referenceItem) return;
      referenceItem.parentElement?.appendChild(createZedRemoteOpenInMenuItem(referenceItem));
    });
  }

  async function refreshZedRemoteOpenControls() {
    if (!codexPlusSettings().zedRemoteOpen) {
      removeZedRemoteButtons();
      removeZedRemoteOpenInMenuItems();
      return;
    }
    const context = zedRemoteContext() || {};
    try {
      const status = await loadZedRemoteStatus();
      if (!status?.platformSupported || (!status.zedAppFound && !status.zedCliFound)) {
        removeZedRemoteButtons();
        removeZedRemoteOpenInMenuItems();
        return;
      }
    } catch (_) {
      removeZedRemoteButtons();
      removeZedRemoteOpenInMenuItems();
      return;
    }
    removeZedRemoteButtons();
    const candidates = zedRemoteFileCandidates(context);
    candidates.forEach(attachZedRemoteButton);
    refreshZedRemoteOpenInMenus(context);
  }

  function scanDeferred() {
    enablePluginEntry();
    unblockPluginInstallButtons();
    patchCodexModelWhitelist();
    sessionRows().forEach(tryAttachButton);
    updateDeleteButtonOffsets();
    scheduleProjectMoveProjection();
    scheduleChatsSortCorrection();
    archivedPageRows().forEach(attachArchivedPageDeleteButton);
    installArchivedDeleteAllButton();
    refreshConversationTimeline();
    refreshZedRemoteOpenControls();
    scheduleThreadScrollSync(true);
  }

  function runScanStep(step) {
    try {
      step();
    } catch (error) {
      window.__codexSessionDeleteScanFailures = window.__codexSessionDeleteScanFailures || [];
      window.__codexSessionDeleteScanFailures.push(String(error?.stack || error));
    }
  }

  function scan() {
    runScanStep(scanLightweight);
    requestAnimationFrame(() => runScanStep(scanDeferred));
  }

  function isExtensionUiNode(node) {
    return !!node?.closest?.([
      ".codex-delete-toast",
      ".codex-delete-confirm-overlay",
      ".codex-plus-modal-overlay",
      `.${projectMoveOverlayClass}`,
      `.${timelineClass}`,
      ".codex-conversation-timeline",
      ".codex-zed-remote-button",
      ".codex-zed-remote-toast",
      "#codex-plus-menu",
    ].join(", "));
  }

  const scanRelevantSelector = [
    selectors.sidebarThread,
    '[data-app-action-sidebar-section-heading="Chats"]',
    '[data-app-action-sidebar-section-heading="Projects"]',
    '[data-codex-project-move-row="true"]',
    '[data-codex-archive-page-row="true"]',
    "[data-codex-archive-delete-all]",
    '[data-message-author-role]',
    '[data-testid="conversation-turn"]',
    '[class*="user-message"]',
    '[class*="UserMessage"]',
    "span.inline-markdown",
    "[class*='inlineMarkdown']",
    "code",
    selectors.appHeader,
    "header",
    selectors.archiveNav,
    selectors.disabledInstallButton,
  ].join(", ");

  function nodeSelfOrAncestorMatchesScanRelevance(node) {
    if (node.nodeType !== 1) return false;
    if (isExtensionUiNode(node)) return false;
    const questionSelector = timelineQuestionSelector();
    return !!node.matches?.(scanRelevantSelector) ||
      !!node.closest?.(scanRelevantSelector) ||
      !!node.matches?.(questionSelector) ||
      !!node.closest?.(questionSelector) ||
      nodeOrAncestorLooksLikeCodexUserBubble(node);
  }

  function isScanRelevantNode(node) {
    if (node.nodeType !== 1) return false;
    if (isExtensionUiNode(node)) return false;
    return nodeSelfOrAncestorMatchesScanRelevance(node) || !!node.querySelector?.(scanRelevantSelector) || nodeLooksLikeTimelineQuestion(node);
  }

  function isChatContentMutation(mutation) {
    const target = mutation.target;
    if (!target?.closest?.('[data-message-author-role], [data-testid="conversation-turn"], main .prose')) return false;
    return !Array.from(mutation.addedNodes).some((node) => node.nodeType === 1 && isScanRelevantNode(node)) &&
      !Array.from(mutation.removedNodes).some((node) => node.nodeType === 1 && isScanRelevantNode(node));
  }

  function shouldScheduleScan(mutations) {
    if (!mutations) return true;
    return mutations.some((mutation) => {
      if (isChatContentMutation(mutation)) return false;
      const target = mutation.target;
      if (isExtensionUiNode(target)) return false;
      if (target?.nodeType === 1 && target.matches?.('[role="menu"], [data-radix-popper-content-wrapper]')) return true;
      if (target?.nodeType === 1 && nodeSelfOrAncestorMatchesScanRelevance(target)) return true;
      const changedNodes = [...Array.from(mutation.addedNodes), ...Array.from(mutation.removedNodes)];
      if (changedNodes.some((node) => node.nodeType === 1 && (
        node.matches?.('[role="menu"], [data-radix-popper-content-wrapper]') ||
        node.querySelector?.('[role="menu"], [data-radix-popper-content-wrapper]')
      ))) return true;
      return changedNodes.some((node) => node.nodeType === 1 && isScanRelevantNode(node));
    });
  }

  function runScheduledScan() {
    window.__codexSessionDeleteScanPending = false;
    clearTimeout(window.__codexSessionDeleteScanTimer);
    window.__codexSessionDeleteScanTimer = null;
    scan();
  }

  function scheduleScan(mutations) {
    if (mutations?.some((mutation) => mutation.type === "attributes" && mutation.attributeName === "aria-current" && mutation.target?.matches?.(selectors.sidebarThread))) {
      scheduleThreadScrollSyncAttempts(true);
    }
    if (!shouldScheduleScan(mutations)) return;
    if (window.__codexSessionDeleteScanPending) return;
    window.__codexSessionDeleteScanPending = true;
    window.__codexSessionDeleteScanTimer = setTimeout(runScheduledScan, 200);
  }

  scan();
  window.__codexProjectMoveApplyProjection = applyProjectMoveProjection;
  window.__codexProjectMoveReadProjection = readProjectMoveProjection;
  window.__codexProjectMoveTargets = projectMoveTargets;
  window.__codexProjectMoveSortChats = applyChatsSortCorrection;
  window.removeEventListener("resize", window.__codexPlusResizeHandler);
  let codexPlusResizeRafId = 0;
  window.__codexPlusResizeHandler = () => {
    cancelAnimationFrame(codexPlusResizeRafId);
    codexPlusResizeRafId = requestAnimationFrame(() => {
      updateFloatingCodexPlusMenuPosition(document.getElementById(codexPlusMenuId));
      runScanStep(refreshConversationTimeline);
    });
  };
  window.addEventListener("resize", window.__codexPlusResizeHandler);
  window.__codexSessionDeleteObserver?.disconnect();
  window.__codexSessionDeleteObserver = new MutationObserver(scheduleScan);
  window.__codexSessionDeleteObserver.observe(document.body || document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ["aria-current"] });
})();
