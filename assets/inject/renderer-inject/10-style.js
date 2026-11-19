  /**
   * 装上 / 换掉样式表。版本 = 手写版本号前缀 + 样式全文的哈希：改了任何 CSS，重新注入（不刷新页面）时
   * 都会换成新样式表。以前只比手写版本号，改样式忘了 +1 时新 JS 配旧样式，新界面全没样式。
   * 样式全文与哈希每次注入只算一次（挂在函数上）：installStyle 在每轮轻量扫描里都会调。
   */
  function installStyle() {
    if (!installStyle.sheet) {
      const text = `
      .${actionGroupClass} {
        position: absolute;
        right: var(--codex-session-actions-right, 28px);
        top: 50%;
        transform: translateY(-50%);
        z-index: 20;
        opacity: 0;
        pointer-events: none;
        display: inline-flex;
        align-items: center;
        gap: 2px;
        background: transparent;
      }
      .${actionButtonClass} {
        flex: 0 0 auto;
        -webkit-app-region: no-drag;
      }
      /* 外观优先套用同一行原生「置顶 / 归档」按钮的类名与 data-* 属性（见
         90-action-groups.js 的 syncNativeSessionActionStyle），套上后带
         data-codex-native-style，下面这组兜底规则就不再生效。兜底按 Codex 原生
         secondary / transparent / 3xs 图标按钮复刻：20px 方形、14px 图标、
         默认三级文字色，悬停只提亮颜色、不铺底色。 */
      .${actionButtonClass}:not([data-codex-native-style]) {
        position: relative;
        width: var(--control-size-4xs, 20px);
        height: var(--control-size-4xs, 20px);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border: 0;
        border-radius: var(--control-radius-sm, var(--radius-sm, 6px));
        background: transparent;
        color: var(--color-text-secondary-ghost, var(--color-text-tertiary, var(--color-token-text-tertiary, var(--codex-plus-text-tertiary, rgba(255,255,255,.5)))));
        font: inherit;
        padding: 0;
        cursor: default;
        text-align: center;
        transition: color var(--transition-duration-basic, .15s) var(--transition-ease-basic, ease);
      }
      .${actionButtonClass}:not([data-codex-native-style]) svg {
        display: block;
        width: var(--control-icon-size-xs, 14px);
        height: var(--control-icon-size-xs, 14px);
      }
      .codex-session-action-inner {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 100%;
        height: 100%;
        pointer-events: none;
      }
      [data-codex-plus-ext="side-chat-save"] > .codex-plus-side-chat-save-spinner {
        display: inline-block;
        width: 12px;
        height: 12px;
        box-sizing: border-box;
        border: 1.5px solid currentColor;
        border-right-color: transparent;
        border-radius: 50%;
        margin-inline-start: 4px;
        vertical-align: -1px;
        animation: codex-plus-side-chat-save-spin .8s linear infinite;
      }
      @keyframes codex-plus-side-chat-save-spin { to { transform: rotate(360deg); } }
      .${actionButtonClass}:not([data-codex-native-style]):hover,
      .${actionButtonClass}:not([data-codex-native-style]):focus-visible,
      .${actionButtonClass}:not([data-codex-native-style])[aria-expanded="true"] {
        background: transparent;
        color: var(--color-text-secondary-ghost-hover, var(--color-text, var(--color-token-text-primary, #f4f4f5)));
        outline: none;
      }
      .${moreMenuClass} {
        position: fixed;
        z-index: 2147483201;
        min-width: 104px;
        border: 1px solid var(--codex-plus-border);
        border-radius: var(--border-radius-lg, 8px);
        background: var(--codex-plus-bg-elevated);
        color: var(--codex-plus-text);
        box-shadow: var(--ui-menu-shadow, var(--shadow-300, 0 8px 24px rgba(0,0,0,.16)));
        padding: 4px;
      }
      .${moreMenuClass}[hidden] { display: none !important; }
      .${moreMenuClass}.codex-session-more-menu-open-up {
        transform: translateY(calc(-100% - 34px));
      }
      .codex-session-more-menu-item {
        width: 100%;
        border: 0;
        border-radius: var(--border-radius-sm, 6px);
        background: transparent;
        color: inherit;
        cursor: default;
        display: flex;
        align-items: center;
        gap: 8px;
        font: inherit;
        font-size: 13px;
        line-height: 18px;
        padding: 6px 8px;
        text-align: left;
      }
      .codex-session-more-menu-item:hover,
      .codex-session-more-menu-item:focus-visible {
        background: var(--codex-plus-bg-hover);
        outline: none;
      }
      .codex-session-more-menu-icon {
        width: 16px;
        text-align: center;
      }
      .${threadIdBadgeClass} {
        flex: 0 0 auto;
        display: inline-flex;
        align-items: center;
        max-width: 152px;
        margin-right: 8px;
        color: var(--text-secondary, var(--color-token-text-secondary, var(--codex-plus-text-secondary, rgba(142,142,160,.95))));
        font: 11px/1.1 ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace;
        letter-spacing: .01em;
        opacity: .9;
        white-space: nowrap;
        user-select: text;
      }
      ${selectors.sidebarThread} [data-codex-thread-id-badge-wrap="true"] {
        display: inline-flex;
        align-items: center;
        min-width: 0;
        max-width: 100%;
      }
      ${selectors.sidebarThread} [data-codex-thread-id-badge-wrap="true"] ${selectors.threadTitle},
      ${selectors.sidebarThread} [data-codex-thread-id-badge-wrap="true"] .truncate.select-none,
      ${selectors.sidebarThread} [data-codex-thread-id-badge-wrap="true"] .truncate.text-base {
        min-width: 0;
      }
      .codex-archive-row-button {
        border: 1px solid var(--color-token-border-light, var(--color-token-border-default, rgba(0,0,0,.12)));
        border-radius: var(--border-radius-sm, 6px);
        background: var(--color-token-bg-secondary, transparent);
        color: var(--color-token-text-secondary, inherit);
        font: inherit;
        font-size: 13px;
        line-height: 16px;
        padding: 3px 8px;
        cursor: pointer;
      }
      .codex-archive-row-button.${buttonClass} {
        border-color: var(--color-border-danger, #dc2626);
        background: var(--color-background-danger-soft, rgba(220,38,38,.1));
        color: var(--color-text-danger, #dc2626);
      }
      .codex-archive-row-button.${exportButtonClass} {
        border-color: var(--color-token-border-light, var(--color-token-border-default, rgba(0,0,0,.12)));
        background: var(--color-token-bg-secondary, transparent);
        color: var(--color-token-text-primary, inherit);
      }
      [data-codex-delete-row="true"]:hover .${actionGroupClass} {
        opacity: 1;
        pointer-events: auto;
      }
      [data-codex-delete-row="true"]:hover ${selectors.threadTitle},
      [data-codex-delete-row="true"]:focus-within ${selectors.threadTitle},
      [data-codex-delete-row="true"].codex-session-more-open ${selectors.threadTitle} {
        flex: 0 1 auto;
        width: var(--codex-session-title-max-width, auto);
        max-width: var(--codex-session-title-max-width, 100%);
        overflow: hidden;
      }
      [data-codex-delete-row="true"].codex-session-more-open .${actionGroupClass} {
        opacity: 1;
        pointer-events: auto;
        z-index: 2147483201;
      }
      [data-codex-delete-row="true"].codex-archive-confirm-visible .${actionGroupClass} {
        right: max(66px, var(--codex-session-actions-right, 28px));
      }
      .${actionTooltipClass} {
        position: fixed;
        z-index: 2147483201;
        max-width: min(220px, calc(100vw - 32px));
        border: 1px solid var(--codex-plus-border);
        border-radius: var(--border-radius-md, 6px);
        background: var(--codex-plus-bg-elevated);
        color: var(--codex-plus-text);
        font: inherit;
        font-size: 13px;
        line-height: 16px;
        padding: 6px 8px;
        box-shadow: var(--tooltip-box-shadow, var(--shadow-200, 0 4px 12px rgba(0,0,0,.14)));
        pointer-events: none;
        white-space: nowrap;
      }
      .codex-archive-delete-all {
        border: 1px solid var(--color-border-danger, #dc2626);
        border-radius: var(--border-radius-sm, 6px);
        background: var(--color-background-danger-soft, rgba(220,38,38,.1));
        color: var(--color-text-danger, #dc2626);
        font: inherit;
        font-size: 13px;
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
        border: 1px solid var(--codex-plus-border);
        border-radius: var(--border-radius-lg, 8px);
        background: var(--codex-plus-bg-elevated);
        color: var(--codex-plus-text);
        font: inherit;
        font-size: 13px;
        box-shadow: var(--ui-menu-shadow, var(--shadow-300, 0 8px 24px rgba(0,0,0,.16)));
        pointer-events: none;
      }
      .codex-delete-toast button { margin-left: 10px; pointer-events: auto; }
      /* 拓展与内置提示共用的类型配色。不传 type 时保持上面的默认外观。 */
      .codex-delete-toast[data-toast-type="success"] { border-color: var(--codex-plus-success, #2f9e63); }
      .codex-delete-toast[data-toast-type="warn"] { border-color: var(--codex-plus-warn, #b7791f); }
      .codex-delete-toast[data-toast-type="error"] { border-color: var(--codex-plus-error, #c53030); }
      .codex-delete-confirm-overlay {
        position: fixed;
        inset: 0;
        z-index: 2147483200;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--modal-backdrop-dim-shadow, rgba(0,0,0,.32));
        backdrop-filter: blur(1px);
      }
      .codex-delete-confirm-content {
        width: min(420px, calc(100vw - 48px));
        border: 1px solid var(--codex-plus-border);
        border-radius: var(--border-radius-xl, 12px);
        background: var(--codex-plus-bg-elevated);
        color: var(--codex-plus-text);
        font: inherit;
        font-size: 14px;
        box-shadow: var(--shadow-400, 0 16px 48px rgba(0,0,0,.2));
        padding: 20px;
      }
      .codex-delete-confirm-title { font-size: 16px; font-weight: 650; }
      .codex-delete-confirm-message { margin-top: 8px; color: var(--codex-plus-text-secondary); line-height: 1.45; }
      .codex-delete-confirm-actions {
        display: flex;
        justify-content: flex-end;
        gap: 10px;
        margin-top: 18px;
      }
      .codex-delete-confirm-actions button {
        min-height: 32px;
        border: 1px solid var(--codex-plus-border);
        border-radius: var(--border-radius-lg, 8px);
        padding: 5px 12px;
        background: var(--codex-plus-bg-secondary);
        color: var(--codex-plus-text);
        font: inherit;
        font-size: 13px;
        cursor: pointer;
      }
      .codex-delete-confirm-actions button:hover,
      .codex-delete-confirm-actions button:focus-visible {
        background: var(--codex-plus-bg-hover);
        outline: none;
      }
      .codex-delete-confirm-actions [data-codex-delete-confirm="true"] {
        border-color: var(--color-border-danger, #dc2626);
        background: var(--color-background-danger-solid, #dc2626);
        color: var(--color-text-danger-solid, #fff);
      }
      .codex-plus-modal-overlay {
        position: fixed;
        top: 0;
        left: 0;
        /*
         * overlay 自身带 zoom（见 applyCodexPlusZoom），而它的 inset: 0 与 100vw
         * 都按未缩放的视口算，再乘 zoom 就溢出（实测 zoom=1.2 时 100vw 得到 2072px，
         * 视口只有 1727px）。用 calc(100vw / var(--codex-plus-zoom)) 抵消；zoom 缺失
         * 时分母回落到 1，行为与改造前一致。
         * 内部子元素用百分比即可——它们在缩放空间里，百分比本来就对。
         * 注意：本段在 JS 模板字符串里，注释中不能出现反引号，否则会提前闭合。
         */
        width: calc(100vw / var(--codex-plus-zoom, 1));
        height: calc(100vh / var(--codex-plus-zoom, 1));
        z-index: 2147483646;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--modal-backdrop-dim-shadow, rgba(0,0,0,.32));
        backdrop-filter: blur(1px);
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-content {
        width: min(520px, calc(100% - 48px));
        max-height: min(680px, calc(100% - 40px));
        display: flex;
        flex-direction: column;
        overflow: hidden;
        border: 1px solid var(--codex-plus-border);
        border-radius: var(--border-radius-xl, 12px);
        background: var(--codex-plus-bg-primary);
        color: var(--codex-plus-text);
        font: inherit;
        font-size: 14px;
        box-shadow: var(--shadow-400, 0 16px 48px rgba(0,0,0,.2));
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 16px 20px 8px;
        flex: 0 0 auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-title { display: flex; align-items: center; gap: 8px; font-size: 18px; font-weight: 600; }
      .codex-plus-backend-indicator { width: 8px; height: 8px; border-radius: 999px; background: var(--codex-plus-text-tertiary); display: inline-block; }
      .codex-plus-backend-indicator[data-status="ok"] { background: var(--codex-plus-success); }
      .codex-plus-backend-indicator[data-status="failed"] { background: var(--codex-plus-danger); }
      .codex-plus-backend-indicator[data-status="checking"] { background: var(--codex-plus-warning); }
      .codex-plus-backend-indicator[data-status="degraded"] { background: var(--codex-plus-warning); }
      #${codexPlusSidebarNavId} {
        position: relative;
        flex: 0 0 auto;
      }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-icon {
        width: 20px;
        height: 20px;
        flex: 0 0 20px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
      }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-icon svg {
        width: 19px;
        height: 19px;
        display: block;
      }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status {
        width: 7px;
        height: 7px;
        margin-left: auto;
        border-radius: 999px;
        background: #a1a1aa;
        opacity: .9;
      }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status[data-status="ok"] {
        background: #34d399;
        box-shadow: 0 0 7px rgba(52,211,153,.7);
      }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status[data-status="failed"] { background: #ef4444; }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status[data-status="checking"] { background: #fbbf24; }
      #${codexPlusSidebarNavId} button[data-active="true"] {
        background: var(--codex-plus-bg-selected);
        color: var(--codex-plus-text);
      }
      .codex-plus-plugin-sidebar { position: relative; flex: 0 0 auto; }
      .codex-plus-plugin-sidebar .codex-plus-sidebar-nav-icon { width: 20px; height: 20px; flex: 0 0 20px; display: inline-flex; align-items: center; justify-content: center; }
      .codex-plus-plugin-sidebar .codex-plus-sidebar-nav-icon svg { width: 19px; height: 19px; }
      .codex-plus-plugin-sidebar-label { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; min-width: 0; }
      .codex-plus-plugin-sidebar button[data-active="true"] { background: var(--codex-plus-bg-selected); color: var(--codex-plus-text); }
      .codex-plus-plugin-market-panel { max-width: 1140px; width: 100%; margin: 0 auto; box-sizing: border-box; }
      .codex-plus-plugin-intro { margin: 0 0 18px; color: var(--codex-plus-text-secondary); font-size: 13px; line-height: 1.6; }
      .codex-plus-plugin-toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; margin-bottom: 14px; }
      .codex-plus-plugin-sources { display: flex; gap: 4px; flex: 1 1 auto; }
      .codex-plus-plugin-sources button { border: 1px solid var(--codex-plus-border); border-radius: 7px; padding: 7px 12px; color: var(--codex-plus-text-secondary); background: transparent; cursor: pointer; }
      .codex-plus-plugin-sources button[data-active="true"] { background: var(--codex-plus-bg-selected); color: var(--codex-plus-text); }
      .codex-plus-plugin-search { flex: 1 1 240px; min-width: 120px; border: 1px solid var(--codex-plus-border); border-radius: 8px; background: var(--codex-plus-bg-secondary); color: var(--codex-plus-text); padding: 9px 12px; font-size: 14px; }
      .codex-plus-plugin-filter { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--codex-plus-text-secondary); }
      .codex-plus-plugin-status { font-size: 12px; color: var(--codex-plus-text-secondary); white-space: pre-wrap; margin-bottom: 14px; }
      .codex-plus-plugin-results { display: flex; flex-direction: column; gap: 10px; }
      .codex-plus-plugin-card { display: flex; align-items: flex-start; gap: 18px; padding: 16px; border: 1px solid var(--codex-plus-border); border-radius: 10px; background: var(--codex-plus-bg-secondary); }
      .codex-plus-plugin-card-info { flex: 1 1 auto; min-width: 0; }
      .codex-plus-plugin-name { font-size: 15px; font-weight: 600; overflow-wrap: anywhere; }
      .codex-plus-plugin-id, .codex-plus-plugin-meta { color: var(--codex-plus-text-tertiary); font-size: 11px; margin-top: 4px; overflow-wrap: anywhere; }
      .codex-plus-plugin-description { color: var(--codex-plus-text-secondary); font-size: 13px; line-height: 1.55; margin: 7px 0; display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere; }
      .codex-plus-plugin-tags { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 8px; }
      .codex-plus-plugin-tags span { border-radius: 5px; padding: 2px 6px; background: var(--codex-plus-bg-selected); color: var(--codex-plus-text-secondary); font-size: 10px; overflow-wrap: anywhere; }
      .codex-plus-plugin-install { flex: 0 0 auto; white-space: nowrap; }
      .codex-plus-plugin-market-page button:disabled { opacity: .55; cursor: default; }
      /* 「供应商」页（43-provider-page.js）：左侧列表 + 右侧详情，13px 紧凑，沿用 --codex-plus-* 变量。 */
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="provider"] { background: var(--codex-plus-settings-page-bg, var(--codex-plus-bg-primary)) !important; }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="provider"] .codex-plus-modal-body { box-sizing: border-box; padding: 0; height: 100%; overflow: hidden; }
      .codex-plus-provider-panel { height: 100%; font-size: 13px; line-height: 18px; color: var(--codex-plus-text); position: relative; }
      .codex-plus-provider-shell { display: flex; flex-direction: column; height: 100%; min-height: 0; }
      /* Tabs → segment control（分段选择器），更像原生 App。 */
      .codex-plus-provider-tabs { display: flex; align-items: center; padding: 8px 14px; border-bottom: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); flex: 0 0 auto; background: var(--codex-plus-nav-bg, var(--codex-plus-bg-secondary)); }
      .codex-plus-provider-tab-group { display: inline-flex; gap: 1px; padding: 2px; background: rgba(127,127,127,.1); border-radius: 7px; border: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); }
      .codex-plus-provider-tabs button { display: inline-flex; align-items: center; gap: 5px; padding: 4px 12px; border: 0; border-radius: 5px; background: transparent; color: var(--codex-plus-text-secondary); font: inherit; font-size: 12px; cursor: pointer; white-space: nowrap; }
      .codex-plus-provider-tabs button:hover:not([aria-selected="true"]) { background: rgba(127,127,127,.08); }
      .codex-plus-provider-tabs button[aria-selected="true"] { background: var(--codex-plus-bg-primary); color: var(--codex-plus-text); font-weight: 600; box-shadow: 0 1px 3px rgba(0,0,0,.12), 0 0 0 .5px rgba(0,0,0,.07); }
      .codex-plus-provider-layout { display: flex; flex: 1; min-height: 0; }
      .codex-plus-provider-item[data-disabled="true"] .codex-plus-provider-item-name { color: var(--codex-plus-text-tertiary, var(--codex-plus-text-secondary)); text-decoration: line-through; }
      .codex-plus-provider-badge[data-tone="off"] { color: var(--codex-plus-danger, #dc2626); }
      .codex-plus-provider-detail > .codex-plus-provider-note { margin-bottom: 12px; }
      .codex-plus-gateway-modal { position: absolute; inset: 0; z-index: 5; display: flex; align-items: center; justify-content: center; background: rgba(0,0,0,.35); }
      .codex-plus-gateway-modal-box { box-sizing: border-box; width: min(720px, calc(100% - 48px)); max-height: calc(100% - 48px); overflow-y: auto; display: flex; flex-direction: column; gap: 10px; padding: 18px 20px; border-radius: 10px; border: 1px solid var(--codex-plus-border); background: var(--codex-plus-bg-primary); box-shadow: 0 12px 40px rgba(0,0,0,.35); }
      .codex-plus-gateway-modal-box .codex-plus-provider-actions { margin-top: 4px; }
      .codex-plus-gateway-warnings { margin: 0; padding-left: 18px; color: var(--codex-plus-warning, #d97706); font-size: 12px; }
      .codex-plus-gateway-diffs { display: flex; flex-direction: column; gap: 8px; }
      .codex-plus-gateway-diff-head { display: flex; justify-content: space-between; gap: 8px; font-size: 12px; color: var(--codex-plus-text-secondary); }
      .codex-plus-gateway-diff pre { margin: 4px 0 0; max-height: 220px; overflow: auto; padding: 8px; border-radius: 6px; background: var(--codex-plus-bg-secondary); font: 12px/17px ui-monospace, SFMono-Regular, Menlo, monospace; white-space: pre; }
      .codex-plus-gateway-diff pre span[data-kind="add"] { color: var(--codex-plus-success, #10a37f); }
      .codex-plus-gateway-diff pre span[data-kind="remove"] { color: var(--codex-plus-danger, #dc2626); }
      .codex-plus-provider-sidebar { box-sizing: border-box; width: 240px; flex: 0 0 240px; overflow-y: auto; padding: 10px 8px; border-right: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); background: var(--codex-plus-nav-bg, var(--codex-plus-bg-secondary)); }
      .codex-plus-provider-sidebar-header { display: flex; align-items: center; justify-content: space-between; padding: 2px 8px 8px; font-size: 12px; font-weight: 600; color: var(--codex-plus-text-secondary); letter-spacing: .02em; }
      .codex-plus-provider-list { display: flex; flex-direction: column; gap: 1px; }
      /* 侧栏条目：照 Codex 原生侧栏，选中只用圆角底色，不加彩条。 */
      .codex-plus-provider-item { display: flex; align-items: center; gap: 6px; padding: 7px 8px 7px 10px; border-radius: 8px; cursor: pointer; }
      .codex-plus-provider-item:hover { background: var(--codex-plus-bg-hover); }
      .codex-plus-provider-item[data-selected="true"] { background: var(--codex-plus-nav-selected-bg, var(--codex-plus-bg-selected)); }
      /* 活跃供应商的绿点指示器 */
      .codex-plus-provider-item-dot { display: inline-block; width: 6px; height: 6px; border-radius: 50%; background: var(--codex-plus-success, #10a37f); vertical-align: middle; margin-right: 4px; margin-bottom: 1px; flex-shrink: 0; }
      .codex-plus-provider-item-main { flex: 1; min-width: 0; }
      .codex-plus-provider-item-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-plus-provider-item-meta { display: flex; gap: 4px; margin-top: 3px; }
      .codex-plus-provider-item-actions { display: none; gap: 2px; }
      .codex-plus-provider-item:hover .codex-plus-provider-item-actions { display: flex; }
      .codex-plus-provider-item-actions button { width: 20px; height: 20px; padding: 0; border: 0; border-radius: 4px; background: transparent; color: var(--codex-plus-text-secondary); cursor: pointer; }
      .codex-plus-provider-item-actions button:hover:not(:disabled) { background: var(--codex-plus-bg-hover); }
      .codex-plus-provider-item-actions button:disabled { opacity: .3; cursor: default; }
      .codex-plus-provider-badge { display: inline-block; padding: 0 6px; border-radius: 4px; font-size: 11px; line-height: 16px; color: var(--codex-plus-text-secondary); background: var(--codex-plus-bg-elevated, var(--codex-plus-bg-hover)); }
      .codex-plus-provider-badge[data-tone="active"] { color: var(--codex-plus-success, #10a37f); }
      .codex-plus-provider-badge[data-tone="new"] { color: var(--codex-plus-warning, #d97706); }
      .codex-plus-provider-detail { flex: 1; min-width: 0; overflow-y: auto; box-sizing: border-box; padding: 14px 20px 24px; }
      /* 详情头部：标题 + 按钮分组 */
      .codex-plus-provider-detail-header { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; padding-bottom: 12px; border-bottom: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); }
      .codex-plus-provider-title { font-size: 15px; font-weight: 600; line-height: 22px; }
      .codex-plus-provider-subtitle, .codex-plus-provider-help { color: var(--codex-plus-text-tertiary, var(--codex-plus-text-secondary)); font-size: 12px; }
      .codex-plus-provider-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
      .codex-plus-provider-actions .codex-plus-action-button[data-primary="true"]:not(:disabled) { background: var(--codex-plus-text); color: var(--codex-plus-bg-primary); }
      .codex-plus-provider-actions .codex-plus-action-button[data-danger="true"]:not(:disabled) { color: var(--codex-plus-danger, #dc2626); }
      .codex-plus-provider-page .codex-plus-action-button:disabled { opacity: .5; cursor: default; }
      /* Section 分组：标题改成 macOS 系统偏好设置风格（11px 大写小标签） */
      .codex-plus-provider-section { padding: 16px 0; border-bottom: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); display: flex; flex-direction: column; gap: 8px; }
      .codex-plus-provider-section:last-of-type { border-bottom: 0; padding-bottom: 4px; }
      .codex-plus-provider-section > summary { cursor: pointer; }
      details.codex-plus-provider-section:not([open]) { gap: 0; }
      .codex-plus-provider-section-title { font-size: 11px; font-weight: 600; letter-spacing: .07em; text-transform: uppercase; color: var(--codex-plus-text-tertiary, var(--codex-plus-text-secondary)); }
      .codex-plus-provider-field { display: flex; flex-direction: column; gap: 4px; min-width: 0; flex: 1; }
      .codex-plus-provider-label { color: var(--codex-plus-text-secondary); font-size: 12px; }
      .codex-plus-provider-input { box-sizing: border-box; width: 100%; min-height: 28px; padding: 4px 8px; border: 1px solid var(--codex-plus-border); border-radius: 6px; background: var(--codex-plus-bg-primary); color: var(--codex-plus-text); font: inherit; }
      .codex-plus-provider-input:focus { outline: none; border-color: var(--codex-plus-focus, #2563eb); }
      .codex-plus-provider-code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; line-height: 17px; resize: vertical; }
      .codex-plus-provider-row { display: flex; gap: 12px; }
      /* Key 列表：悬停高亮，让点替换 / 删除的目标更清晰 */
      .codex-plus-provider-key { display: flex; align-items: center; gap: 8px; padding: 4px 6px; border-radius: 5px; }
      .codex-plus-provider-key:hover { background: var(--codex-plus-bg-hover); }
      .codex-plus-provider-key label { display: flex; align-items: center; gap: 6px; flex: 1; min-width: 0; cursor: pointer; }
      .codex-plus-provider-key code { color: var(--codex-plus-text-secondary); font-size: 12px; }
      /* Codex 的表单 reset 会把原生单选框 / 下拉箭头隐藏掉，这里只恢复本页自己的控件。 */
      .codex-plus-provider-key input[type="radio"] { appearance: auto !important; -webkit-appearance: radio !important; display: inline-block !important; position: static !important; width: 14px !important; height: 14px !important; margin: 0 !important; opacity: 1 !important; clip: auto !important; accent-color: var(--codex-plus-text); cursor: pointer; }
      select.codex-plus-provider-input { appearance: auto !important; -webkit-appearance: menulist !important; cursor: pointer; }
      /* 设置页「语音输入服务」「下一步建议服务」两节的行内控件（46-service-settings.js） */
      .codex-plus-service-controls { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 8px; flex-shrink: 0; max-width: 60%; }
      .codex-plus-service-input { width: min(320px, 46vw); }
      .codex-plus-service-input-short { width: 180px; }
      .codex-plus-service-input:disabled { opacity: .55; cursor: not-allowed; }
      .codex-plus-service-status { color: var(--codex-plus-text-secondary); font-size: 12px; }
      .codex-plus-service-status[data-tone="error"] { color: var(--codex-plus-danger, #dc2626); }
      .codex-plus-provider-key button, .codex-plus-provider-link { border: 0; padding: 0; background: transparent; color: var(--codex-plus-text-secondary); font: inherit; font-size: 12px; cursor: pointer; text-decoration: underline; text-underline-offset: 2px; align-self: flex-start; }
      .codex-plus-provider-key-add { display: flex; gap: 6px; }
      .codex-plus-provider-key-add input { flex: 1; min-width: 0; }
      .codex-plus-provider-key-add input[data-codex-provider-new-key-name] { flex: 0 0 120px; }
      /* 状态消息：从内容区挪到底部状态栏，这里只留内联用（如弹窗里）。 */
      .codex-plus-provider-status { margin: 0 0 10px; padding: 7px 10px; border-radius: 6px; background: var(--codex-plus-bg-elevated, var(--codex-plus-bg-hover)); white-space: pre-wrap; font-size: 12px; }
      .codex-plus-provider-status[data-tone="error"] { color: var(--codex-plus-danger, #dc2626); background: var(--codex-plus-danger-bg, rgba(220,38,38,.08)); }
      .codex-plus-provider-status[data-tone="ok"] { color: var(--codex-plus-success, #10a37f); }
      /* 底部状态栏：固定在面板底部，不再插入内容区。 */
      .codex-plus-provider-footer { flex: 0 0 auto; min-height: 24px; padding: 3px 16px; border-top: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); background: var(--codex-plus-nav-bg, var(--codex-plus-bg-secondary)); display: flex; align-items: center; font-size: 11px; color: var(--codex-plus-text-tertiary, var(--codex-plus-text-secondary)); user-select: none; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
      .codex-plus-provider-footer[data-tone="ok"] { color: var(--codex-plus-success, #10a37f); }
      .codex-plus-provider-footer[data-tone="error"] { color: var(--codex-plus-danger, #dc2626); }
      .codex-plus-provider-footer[data-tone="warn"] { color: var(--codex-plus-warning, #d97706); }
      /* 网关 / 直连模式 banner：更醒目的全宽横幅（比普通 note 更有存在感）。 */
      .codex-plus-provider-mode-bar { padding: 7px 16px; font-size: 12px; display: flex; align-items: center; gap: 8px; }
      .codex-plus-provider-mode-bar[data-codex-gateway-mode="gateway"] { background: rgba(16,163,127,.07); border-bottom: 1px solid rgba(16,163,127,.18); color: var(--codex-plus-text); }
      .codex-plus-provider-mode-bar[data-codex-gateway-mode="direct"] { background: var(--codex-plus-bg-elevated, var(--codex-plus-bg-hover)); border-bottom: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); color: var(--codex-plus-text-secondary); }
      .codex-plus-provider-mode-bar button { margin-left: auto; }
      .codex-plus-provider-note { padding: 8px 10px; border-radius: 6px; background: var(--codex-plus-bg-elevated, var(--codex-plus-bg-hover)); color: var(--codex-plus-text-secondary); font-size: 12px; }
      .codex-plus-provider-empty { padding: 48px 24px; color: var(--codex-plus-text-secondary); }
      /* 详情改版：照 Codex 原生设置页——居中窄栏、14px 小节标题、圆角卡片里一行一项（左标题右控件）。 */
      .codex-plus-provider-detail { padding: 20px 32px 32px; }
      .codex-plus-provider-detail > * { max-width: 760px; margin-left: auto; margin-right: auto; }
      .codex-plus-provider-detail-header { align-items: center; padding: 0 0 4px; border-bottom: 0; }
      .codex-plus-provider-title { font-size: 20px; font-weight: 500; line-height: 28px; }
      .codex-plus-provider-group { gap: 0; padding: 0; margin-top: 24px; border-bottom: 0; }
      .codex-plus-provider-group > .codex-plus-provider-section-title { display: flex; align-items: center; min-height: 32px; padding: 0 0 6px; color: var(--codex-plus-text); font-size: 14px; font-weight: 500; letter-spacing: 0; line-height: 21px; text-transform: none; }
      .codex-plus-provider-card { box-sizing: border-box; min-width: 0; padding: 0 16px; border: 1px solid var(--codex-plus-settings-card-border); border-radius: var(--codex-plus-card-radius); background: var(--codex-plus-settings-card-bg); }
      .codex-plus-provider-card > * + * { border-top: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); }
      .codex-plus-provider-group-note { margin: 6px 4px 0; color: var(--codex-plus-text-secondary); font-size: 12px; line-height: 16px; }
      .codex-plus-provider-line { display: flex; align-items: center; justify-content: space-between; gap: 24px; box-sizing: border-box; min-height: 52px; padding: 12px 0; }
      .codex-plus-provider-line-copy { display: flex; flex: 1 1 auto; flex-direction: column; gap: 2px; min-width: 0; overflow-wrap: anywhere; }
      .codex-plus-provider-line-title { color: var(--codex-plus-text); font-size: 13px; font-weight: 500; line-height: 18px; }
      .codex-plus-provider-line-help { color: var(--codex-plus-text-secondary); font-size: 12px; line-height: 16px; }
      .codex-plus-provider-line-control { display: flex; flex: 0 1 60%; align-items: center; justify-content: flex-end; gap: 8px; min-width: 0; }
      .codex-plus-provider-line[data-stack="true"] { flex-direction: column; align-items: stretch; gap: 8px; }
      .codex-plus-provider-line[data-stack="true"] .codex-plus-provider-line-control { flex: none; width: 100%; }
      .codex-plus-provider-card .codex-plus-provider-input { min-height: 28px; padding: 4px 10px; border-color: var(--codex-plus-control-border); border-radius: var(--codex-plus-control-radius); background: var(--codex-plus-control-bg); }
      .codex-plus-provider-card .codex-plus-provider-input:focus { border-color: var(--codex-plus-focus, #2563eb); }
      .codex-plus-provider-card .codex-plus-provider-key { padding: 12px 0; border-radius: 0; }
      .codex-plus-provider-card .codex-plus-provider-key:hover { background: transparent; }
      .codex-plus-provider-card .codex-plus-provider-key-add { padding: 12px 0; }
      .codex-plus-provider-card .codex-plus-provider-help { padding: 12px 0; }
      .codex-plus-provider-page .codex-plus-action-button { box-sizing: border-box; min-height: 28px; border: 1px solid var(--codex-plus-control-border); border-radius: var(--codex-plus-control-radius); background: var(--codex-plus-button-bg); color: var(--codex-plus-text); font: inherit; font-size: 13px; cursor: pointer; }
      .codex-plus-provider-page .codex-plus-action-button:hover:not(:disabled):not([data-primary="true"]) { background: var(--codex-plus-button-hover-bg); }
      /* 重新设计：标题行吸顶，操作按钮始终可见；横向铺满详情区，内容仍对齐 760px 居中栏。 */
      .codex-plus-provider-detail > .codex-plus-provider-detail-header { position: sticky; top: -20px; z-index: 3; max-width: none; margin: 0 -32px; padding: 14px max(32px, calc((100% - 760px) / 2)); background: var(--codex-plus-settings-page-bg); border-bottom: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); }
      .codex-plus-provider-card .codex-plus-action-button[data-danger="true"]:not(:disabled) { color: var(--codex-plus-danger, #dc2626); }
      .codex-plus-provider-card .codex-plus-action-button:disabled { opacity: .5; cursor: default; }
      /* 结果提示：保存、测试、获取模型的结果贴在标题行下面，不再只在底部状态栏的小字里。 */
      .codex-plus-provider-banner { box-sizing: border-box; margin-top: 12px; padding: 8px 12px; border: 1px solid var(--codex-plus-settings-card-border); border-radius: var(--codex-plus-control-radius); background: var(--codex-plus-settings-card-bg); color: var(--codex-plus-text-secondary); font-size: 12px; line-height: 17px; white-space: pre-wrap; overflow-wrap: anywhere; }
      .codex-plus-provider-banner[data-tone="ok"] { color: var(--codex-plus-success, #10a37f); border-color: color-mix(in srgb, var(--codex-plus-success, #10a37f) 35%, transparent); }
      .codex-plus-provider-banner[data-tone="warn"] { color: var(--codex-plus-warning, #d97706); border-color: color-mix(in srgb, var(--codex-plus-warning, #d97706) 40%, transparent); }
      .codex-plus-provider-banner[data-tone="error"] { color: var(--codex-plus-danger, #dc2626); background: var(--codex-plus-danger-bg, rgba(220,38,38,.08)); border-color: color-mix(in srgb, var(--codex-plus-danger, #dc2626) 40%, transparent); }
      /* 模型行编辑器：星标 | 模型名 | 窗口下拉 | 移除。 */
      .codex-plus-provider-model-list { display: flex; flex-direction: column; gap: 6px; width: 100%; min-width: 0; }
      .codex-plus-provider-model-row { display: grid; grid-template-columns: 28px minmax(0, 1fr) 120px auto; align-items: center; gap: 8px; }
      .codex-plus-provider-star { width: 28px; height: 28px; padding: 0; border: 0; border-radius: var(--codex-plus-control-radius); background: transparent; color: var(--codex-plus-text-tertiary, var(--codex-plus-text-secondary)); font-size: 15px; line-height: 1; cursor: pointer; }
      .codex-plus-provider-star:hover { background: var(--codex-plus-bg-hover); }
      .codex-plus-provider-star[data-active="true"] { color: var(--codex-plus-warning, #d97706); }
      .codex-plus-provider-model-remove { padding: 0 4px; border: 0; background: transparent; color: var(--codex-plus-text-secondary); font: inherit; font-size: 12px; text-decoration: underline; text-underline-offset: 2px; cursor: pointer; }
      .codex-plus-provider-model-add { align-self: flex-start; }
      /* 左侧列表：名称、模式标签下面补一行「默认模型 · 接口域名」。 */
      .codex-plus-provider-item-sub { margin-top: 3px; overflow: hidden; color: var(--codex-plus-text-tertiary, var(--codex-plus-text-secondary)); font-size: 11px; line-height: 15px; text-overflow: ellipsis; white-space: nowrap; }
      /* 卡片行收尾：没有控件的行不占右半边；「配置文件」折叠标题可点；网关接入提示在详情栏里做成圆角横幅。 */
      .codex-plus-provider-line-control:empty { display: none; }
      .codex-plus-provider-line-help .codex-plus-provider-link { vertical-align: baseline; }
      /* 吸顶标题行：top 抵掉详情区 20px 的上内边距，否则滚动时标题上方会漏出一条正文。 */
      /* 只有按钮的行：控件区按内容收窄，不占 60%，说明文字不被挤断。 */
      .codex-plus-provider-line[data-kind="action"] .codex-plus-provider-line-control { flex: 0 0 auto; }
      /* 供应商页的下拉：.codex-plus-menu-* 胶囊按钮 + 菜单（与拓展页排序同一套），这里只管定位。
         菜单靠右对齐按钮，选项多（测试用模型）时限高滚动。 */
      .codex-plus-provider-menu { position: relative; display: inline-flex; min-width: 0; }
      .codex-plus-provider-menu .codex-plus-menu-trigger { max-width: 100%; }
      .codex-plus-provider-menu-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
      .codex-plus-provider-menu-popover { top: calc(100% + 4px); right: 0; max-height: 320px; overflow-y: auto; }
      .codex-plus-provider-model-row .codex-plus-provider-menu { justify-self: end; }
      /* 「配置文件」折叠标题：小节标题是 flex，原生三角会消失，自己画一个会转的箭头。 */
      details.codex-plus-provider-group > summary { cursor: pointer; list-style: none; gap: 6px; user-select: none; }
      details.codex-plus-provider-group > summary::-webkit-details-marker { display: none; }
      details.codex-plus-provider-group > summary::before { content: ""; width: 6px; height: 6px; margin: 0 2px 2px 1px; border-right: 1.5px solid var(--codex-plus-text-secondary); border-bottom: 1.5px solid var(--codex-plus-text-secondary); transform: rotate(-45deg); transition: transform .15s ease; }
      details.codex-plus-provider-group[open] > summary::before { transform: rotate(45deg); margin-bottom: 4px; }
      details.codex-plus-provider-group > summary:hover { color: var(--codex-plus-text); }
      details.codex-plus-provider-group > summary::after { content: "config.toml / auth.json"; margin-left: 4px; color: var(--codex-plus-text-tertiary, var(--codex-plus-text-secondary)); font-size: 12px; font-weight: 400; }
      details.codex-plus-provider-group:not([open]) > summary { padding-bottom: 0; }
      .codex-plus-provider-detail > .codex-plus-provider-mode-bar, .codex-plus-provider-detail > .codex-plus-provider-note { box-sizing: border-box; margin-bottom: 12px; padding: 8px 12px; border: 1px solid var(--codex-plus-settings-card-border); border-radius: var(--codex-plus-control-radius); }
      .codex-plus-provider-sidebar-header { padding: 4px 8px 10px; color: var(--codex-plus-text-secondary); font-weight: 500; letter-spacing: 0; }
      .codex-plus-provider-sidebar-header .codex-plus-action-button { min-height: 24px; padding: 0 8px; font-size: 12px; }
      .codex-plus-plugin-job-message { font-size: 12px; color: var(--codex-plus-text-secondary); margin-top: 8px; white-space: pre-wrap; overflow-wrap: anywhere; }
      .codex-plus-plugin-job-message[data-status="failed"] { color: var(--codex-plus-danger); }
      .codex-plus-plugin-pagination { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 10px; margin-top: 20px; font-size: 12px; color: var(--codex-plus-text-secondary); }
      .codex-plus-plugin-pagination > span { margin-right: auto; }
      .codex-plus-plugin-empty { padding: 40px 16px; text-align: center; color: var(--codex-plus-text-secondary); }
      @media (max-width: 600px) { .codex-plus-plugin-card { flex-direction: column; gap: 10px; } .codex-plus-plugin-install { align-self: flex-end; } }
      /*
       * 新版导航图标栏里的 Codex++ / 拓展 / 推荐内容入口：原生按钮只放图标，
       * 这里对齐它的尺寸。
       *
       * 用 [data-codex-plus-rail] 而不是逐个列 id——早先按 id 写，加第三个入口时
       * 漏掉了对应的选择器，那个图标容器就没有 20px 约束、撑成整个按钮宽，
       * 表现为图标偏左不居中。
       */
      [data-codex-plus-rail] {
        position: relative;
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: center;
      }
      [data-codex-plus-rail] > button {
        position: relative;
      }
      [data-codex-plus-rail] .codex-plus-rail-icon {
        width: 20px;
        height: 20px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
      }
      [data-codex-plus-rail] .codex-plus-rail-icon svg {
        width: 19px;
        height: 19px;
        display: block;
      }
      [data-codex-plus-rail] .codex-plus-rail-icon .codex-plus-rail-logo {
        width: 20px;
        height: 20px;
        display: block;
        pointer-events: none;
        user-select: none;
      }
      /* 图标栏是纯图标，状态点挂在按钮右上角，不占布局。 */
      #${codexPlusRailNavId} .codex-plus-sidebar-nav-status {
        position: absolute;
        top: 2px;
        right: 2px;
        margin-left: 0;
        width: 6px;
        height: 6px;
      }
      /*
       * 我们的页面是叠加在 Codex 之上的，Codex 并不知道，所以它自己那个
       * destination 的选中态会一直留着，看起来像 rail 上同时亮两个。
       * 页面打开时给根节点打标记，用 CSS 把原生选中项压成未选中；
       * 关掉页面即移除标记，原生状态自动恢复——比改它的按钮属性稳，
       * 不会和 React 的重渲染打架。
       * 颜色取自当前主题下真实未选中项（见 syncCodexPlusRailNativeSelection），
       * 主题切换时下次同步会重算。
       */
      html[data-codex-plus-page-open] nav[data-app-navigation-rail] [data-sidebar-destination][aria-current="page"] {
        color: var(--codex-plus-rail-dim, var(--codex-plus-text-tertiary, currentColor)) !important;
      }
      html[data-codex-plus-page-open] nav[data-app-navigation-rail] [data-sidebar-destination][aria-current="page"]::before {
        opacity: 0 !important;
      }
      html[data-codex-plus-page-open] nav[data-app-navigation-rail] [data-sidebar-destination][aria-current="page"] * {
        color: var(--codex-plus-rail-dim, var(--codex-plus-text-tertiary, currentColor)) !important;
      }
      /*
       * 页面 overlay 的 left 由 positionCodexPlusPage 按图标栏右边界算好写进来。
       *
       * 关键：写进来的必须是**布局坐标**（视觉值 / zoom），因为 overlay 自己在缩放
       * 空间里布局，宽度也要用同一个空间的量。width 的 calc(100vw / zoom - left - right)
       * 把右边贴到原生内容区的右边缘；三个量都除过 zoom，缩放后才正好补齐。
       * 注意：本段在 JS 模板字符串里，注释里不能出现反引号，否则会提前闭合。
       *
       * 右/下各留一圈槽（原生是 4px）并给四角加圆角，对齐官方面板的观感：
       * 官方页面面板 [_PageSurface_] 的 rect 与我们这块 overlay 完全相同（都是
       * [52, 44, 1663, 940]），四角 12px；它外面套了一个 padding: 0 4px 4px 0 的槽，
       * 右、下能看到底色。我们原先 right/bottom 贴 0 且无圆角，于是方角并且比官方
       * 多占 4px，用户反馈「少一个圆角」就是这个。
       * 圆角值由 positionCodexPlusPage 量官方面板写入，四个角一起用，不做左右区分。
       * overflow: hidden 让圆角真正裁到内容；内容都在 .codex-plus-modal-content 内，
       * 没有依赖溢出显示的全屏浮层，切掉是安全的。
       */
      .${codexPlusPageClass} {
        position: fixed;
        top: var(--codex-plus-page-top, 0px);
        right: var(--codex-plus-page-right, 0px);
        bottom: var(--codex-plus-page-bottom, 0px);
        left: var(--codex-plus-page-left, 0px);
        width: calc(100vw / var(--codex-plus-zoom, 1) - var(--codex-plus-page-left, 0px) - var(--codex-plus-page-right, 0px));
        height: calc(100vh / var(--codex-plus-zoom, 1) - var(--codex-plus-page-top, 0px) - var(--codex-plus-page-bottom, 0px));
        border-radius: var(--codex-plus-page-radius, 0px);
        overflow: hidden;
        /*
         * 功能页是覆盖在 Codex 内容区上的基础层，不能压过原生侧栏/顶部栏
         * 的 tooltip 和 menu；原生弹出层在页面打开时由下方规则提升。
         */
        z-index: 2147482000;
        display: block;
        background: var(--codex-plus-bg-primary, #fff);
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      /* 图标栏按钮提示的兜底外观：找不到原生提示样式类时使用，数值量自原生提示。 */
      .codex-plus-rail-tooltip-fallback {
        max-width: 320px;
        padding: 5px 12px;
        border: 1px solid rgba(255, 255, 255, 0.05);
        border-radius: 20px;
        background: rgb(45, 45, 45);
        box-shadow: rgba(15, 23, 42, 0.2) 0 8px 18px 0;
        color: #fff;
        font-size: 13px;
        font-weight: 430;
        line-height: 18px;
        letter-spacing: -0.15px;
        white-space: nowrap;
      }
      ${codexPlusIsWindowsPlatform ? "" : `
      html[data-codex-plus-page-open] div:has(> [data-radix-popper-content-wrapper], > [role="tooltip"], > [role="menu"]):not(.${codexPlusPageClass} *) {
        z-index: 2147483645 !important;
      }
      `}
      html[data-codex-plus-page-open] [data-radix-popper-content-wrapper]:not(.${codexPlusPageClass} *),
      html[data-codex-plus-page-open] [role="tooltip"]:not(.${codexPlusPageClass} *),
      html[data-codex-plus-page-open] [role="menu"]:not(.${codexPlusPageClass} *),
      html[data-codex-plus-page-open] [class*="ApplicationMenuTopBar"],
      html[data-codex-plus-page-open] .app-header-tint {
        z-index: 2147483645 !important;
      }
      .${codexPlusPageClass} .codex-plus-modal-content {
        width: auto;
        height: 100%;
        max-height: none;
        border: 0;
        border-radius: 0;
        background: var(--codex-plus-bg-primary, #fff);
        box-shadow: none;
      }
      .${codexPlusPageClass} .codex-plus-modal-header {
        width: 100%;
        margin: 0;
        padding: 16px 24px 10px;
      }
      .${codexPlusPageClass} .codex-plus-modal-body {
        width: 100%;
        margin: 0;
        padding: 4px 32px 32px;
      }
      /* 两栏：左侧自己的面板（导航 / 列表），右侧内容区。观感对齐 Codex 原生页面。 */
      .${codexPlusPageClass} .codex-plus-page-layout {
        display: flex;
        flex: 1 1 auto;
        min-height: 0;
        width: 100%;
      }
      .${codexPlusPageClass} .codex-plus-page-nav {
        width: 260px;
        flex: 0 0 260px;
        display: flex;
        flex-direction: column;
        min-height: 0;
        border-right: 1px solid var(--codex-plus-border-subtle);
      }
      .${codexPlusPageClass} .codex-plus-page-nav-header {
        flex: 0 0 auto;
        padding: 4px 16px 10px;
      }
      .${codexPlusPageClass} .codex-plus-page-nav-title {
        font-size: 15px;
        font-weight: 600;
        color: var(--codex-plus-text);
      }
      .${codexPlusPageClass} .codex-plus-page-nav-body {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
        overscroll-behavior: contain;
        padding: 4px 10px 16px;
        scrollbar-width: thin;
        scrollbar-color: rgba(255,255,255,.28) transparent;
        /*
         * 左面板是导航/列表，不是内容：拖动时不该把条目文字或分组标题选蓝
         * （列表项本来就是整行可点，选中态由 data-active 表达）。
         * 右侧详情区不设，那里的描述文字要能复制。
         */
        user-select: none;
        -webkit-user-select: none;
      }
      .${codexPlusPageClass} .codex-plus-page-main {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        min-height: 0;
      }
      .codex-plus-page-nav-item {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        padding: 7px 10px;
        border: 0;
        border-radius: 8px;
        background: transparent;
        color: var(--codex-plus-text-secondary);
        font: inherit;
        font-size: 13px;
        text-align: left;
        cursor: pointer;
      }
      .codex-plus-page-nav-item:hover { background: var(--codex-plus-bg-hover); }
      .codex-plus-page-nav-item[data-active="true"] {
        background: var(--codex-plus-bg-selected);
        color: var(--codex-plus-text);
      }
      .codex-plus-page-nav-item-text { flex: 1 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
      .codex-plus-page-nav-item-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-plus-page-nav-item-meta { font-size: 13px; color: var(--codex-plus-text-tertiary); }
      .codex-plus-page-nav-item-state {
        flex: 0 0 auto;
        width: 7px;
        height: 7px;
        border-radius: 999px;
        background: var(--codex-plus-text-tertiary);
      }
      .codex-plus-page-nav-item-state[data-state="on"] { background: #34d399; }
      /* 拓展条目：三行结构（名称 / 简介 / 作者+操作），对齐 VSCode 扩展列表的 .extension-list-item */
      .codex-plus-page-nav-item .codex-plus-extensions-item-body {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        /* VSCode 的 .details 垂直居中，条目高矮不一时文字块不贴顶 */
        justify-content: center;
        overflow: hidden;
      }
      .codex-plus-extensions-item-header {
        display: flex;
        align-items: center;
        gap: 8px;
        min-width: 0;
      }
      .codex-plus-extensions-icon {
        flex: 0 0 auto;
        display: flex;
        align-items: flex-start;
        padding-top: 10px;
        color: var(--codex-plus-text-secondary);
      }
      .codex-plus-extensions-icon svg { width: 40px; height: 40px; display: block; }
      /* 市场清单给的图标：正方形等比缩放，圆角与 VSCode 的扩展图标一致 */
      .codex-plus-extensions-icon .codex-plus-extensions-icon-img {
        width: 40px;
        height: 40px;
        display: block;
        object-fit: contain;
        border-radius: 6px;
      }
      .codex-plus-page-nav-item[data-active="true"] .codex-plus-extensions-icon { color: var(--codex-plus-text); }
      /* 名称：VSCode 用 semiBold，且 hover 才加下划线 */
      .codex-plus-extensions-item-name {
        flex: 1 1 auto;
        min-width: 0;
        font-weight: 600;
        color: var(--codex-plus-text);
        white-space: nowrap;
        text-overflow: ellipsis;
        overflow: hidden;
      }
      .codex-plus-page-nav-item:hover .codex-plus-extensions-item-name { text-decoration: underline; }
      .codex-plus-extensions-item-description {
        margin-top: 2px;
        padding-right: 8px;
        color: var(--codex-plus-text-tertiary);
        font-size: 13px;
        line-height: normal;
        display: -webkit-box;
        -webkit-line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
      }
      /* 底部行：作者在左、操作在右，VSCode 的 .footer 是 24px 高 */
      .codex-plus-extensions-item-footer {
        display: flex;
        align-items: center;
        justify-content: flex-end;
        gap: 6px;
        min-height: 24px;
        padding-top: 2px;
      }
      .codex-plus-extensions-item-publisher {
        flex: 1 1 auto;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-size: 13px;
        font-weight: 600;
        color: var(--codex-plus-text-tertiary);
      }
      .codex-plus-extensions-item-actions { flex: 0 0 auto; display: inline-flex; align-items: center; gap: 6px; }
      /* 行内按钮：默认低调，hover 整行时才提亮，避免列表花掉 */
      .codex-plus-extensions-item-button {
        padding: 2px 8px;
        border: 1px solid var(--codex-plus-border);
        border-radius: 6px;
        background: transparent;
        color: var(--codex-plus-text-secondary);
        font-size: 13px;
        line-height: 16px;
        white-space: nowrap;
      }
      .codex-plus-page-nav-item:hover .codex-plus-extensions-item-button {
        background: var(--codex-plus-bg-selected);
        color: var(--codex-plus-text);
      }
      /* 启用状态点挪到名称右侧，用 VSCode 那种 14px 徽标尺寸 */
      .codex-plus-page-nav-item[data-active="true"] .codex-plus-extensions-item-state {
        background: currentColor;
      }
      .codex-plus-extensions-item-state {
        flex: 0 0 auto;
        width: 7px;
        height: 7px;
        border-radius: 999px;
        background: var(--codex-plus-text-tertiary);
      }
      .codex-plus-extensions-item-state[data-state="on"] { background: #34d399; }
      .codex-plus-extensions-icon-badge {
        flex: 0 0 auto;
        width: 14px;
        height: 14px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border-radius: 999px;
        background: #10a37f;
        color: #fff;
        font-size: 13px;
        line-height: 1;
      }
      .codex-plus-page-nav-empty { padding: 8px 10px; color: var(--codex-plus-text-tertiary); font-size: 13px; }
      /* 拓展页：搜索框 + 分组标题 + 市场条目的「安装」按钮 */
      .codex-plus-page-search { padding: 0 10px 8px; }
      .codex-plus-page-search-input {
        width: 100%;
        box-sizing: border-box;
        padding: 6px 9px;
        border: 1px solid var(--codex-plus-border);
        border-radius: 8px;
        background: var(--codex-plus-bg-secondary);
        color: var(--codex-plus-text);
        font: inherit;
        font-size: 13px;
        outline: none;
      }
      .codex-plus-page-search-input:focus { border-color: var(--codex-plus-border-subtle); background: var(--codex-plus-bg-elevated); }
      .codex-plus-page-search-input::placeholder { color: var(--codex-plus-text-tertiary); }
      /* 左面板条目图标：市场里没有图标字段，统一用 VSCode 的默认扩展字形 */
      .codex-plus-page-nav-item-icon {
        flex: 0 0 auto;
        width: 22px;
        height: 22px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border-radius: 5px;
        background: var(--codex-plus-bg-hover);
        color: var(--codex-plus-text-secondary);
      }
      .codex-plus-page-nav-item-icon svg { width: 14px; height: 14px; display: block; }
      .codex-plus-page-nav-item[data-active="true"] .codex-plus-page-nav-item-icon {
        color: var(--codex-plus-text);
      }
      /* 右上角详情：形态对齐 VSCode 的扩展详情页 */
      .codex-plus-extensions-detail { padding: 4px 4px 24px; }
      .codex-plus-extensions-detail-empty {
        padding: 40px 8px;
        color: var(--codex-plus-text-tertiary);
        font-size: 13px;
        text-align: center;
      }
      .codex-plus-extensions-detail-head {
        display: flex;
        align-items: flex-start;
        gap: 14px;
        padding-bottom: 14px;
        border-bottom: 1px solid var(--codex-plus-border-subtle);
      }
      .codex-plus-extensions-detail-icon {
        flex: 0 0 auto;
        width: 64px;
        height: 64px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border-radius: 12px;
        background: var(--codex-plus-bg-hover);
        color: var(--codex-plus-text-secondary);
      }
      .codex-plus-extensions-detail-icon svg { width: 40px; height: 40px; display: block; }
      .codex-plus-extensions-detail-heading { flex: 1 1 auto; min-width: 0; }
      .codex-plus-extensions-detail-title {
        font-size: 19px;
        font-weight: 600;
        color: var(--codex-plus-text);
        line-height: 1.3;
      }
      .codex-plus-extensions-detail-meta {
        margin-top: 3px;
        color: var(--codex-plus-text-secondary);
        font-size: 13px;
      }
      .codex-plus-extensions-detail-sep { margin: 0 6px; color: var(--codex-plus-text-tertiary); }
      /* issue #2359：原 #fbbf24 在浅色主题下对比度仅 1.56:1，改为随主题走的警告色。 */
      .codex-plus-extensions-detail-update { margin-top: 5px; color: var(--codex-plus-warning); font-size: 13px; }
      .codex-plus-extensions-detail-actions {
        flex: 0 0 auto;
        display: inline-flex;
        align-items: center;
        gap: 8px;
      }
      .codex-plus-extensions-detail-button {
        padding: 5px 12px;
        border: 1px solid var(--codex-plus-border);
        border-radius: 8px;
        background: transparent;
        color: var(--codex-plus-text-secondary);
        font: inherit;
        font-size: 13px;
        cursor: pointer;
      }
      .codex-plus-extensions-detail-button:hover { background: var(--codex-plus-bg-hover); color: var(--codex-plus-text); }
      .codex-plus-extensions-detail-primary {
        background: #10a37f;
        border-color: #10a37f;
        color: #fff;
      }
      .codex-plus-extensions-detail-primary:hover { background: #0e8f70; color: #fff; }
      .codex-plus-extensions-detail-description {
        margin-top: 14px;
        color: var(--codex-plus-text-secondary);
        font-size: 13px;
        line-height: 1.6;
      }
      .codex-plus-extensions-detail-tags {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        margin-top: 12px;
      }
      .codex-plus-extensions-detail-tags span {
        padding: 2px 8px;
        border-radius: 999px;
        background: var(--codex-plus-bg-hover);
        color: var(--codex-plus-text-tertiary);
        font-size: 13px;
      }
      .codex-plus-extensions-detail-section { margin-top: 16px; }
      .codex-plus-extensions-detail-section-title {
        margin-bottom: 6px;
        font-weight: 600;
        font-size: 13px;
        color: var(--codex-plus-text);
      }
      .codex-plus-extensions-detail-section ul {
        margin: 0;
        padding-left: 18px;
        color: var(--codex-plus-text-secondary);
        font-size: 13px;
        line-height: 1.7;
      }
      .codex-plus-extensions-detail-link { margin-top: 16px; font-size: 13px; }
      .codex-plus-extensions-detail-link a { color: var(--codex-plus-success); word-break: break-all; }
      .codex-plus-extensions-detail-settings { margin-top: 22px; }
      .codex-plus-extensions-detail-settings-host { margin-top: 8px; }
      .codex-plus-extensions-detail-settings-hint { color: var(--codex-plus-text-secondary); font-size: 13px; }
      .codex-plus-extensions-detail-error {
        margin-top: 14px;
        padding: 8px 10px;
        border-radius: 8px;
        background: var(--codex-plus-danger-bg);
        color: var(--codex-plus-danger);
        font-size: 13px;
      }
      .codex-plus-page-nav-group { margin-bottom: 10px; }
      .codex-plus-page-nav-group-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 4px 10px;
        color: var(--codex-plus-text-tertiary);
        font-size: 13px;
        text-transform: uppercase;
        letter-spacing: .04em;
      }
      .codex-plus-page-nav-group-count {
        min-width: 16px;
        padding: 0 5px;
        border-radius: 999px;
        background: var(--codex-plus-bg-hover);
        text-align: center;
        font-size: 13px;
      }
      .codex-plus-page-nav-group-tail { display: inline-flex; align-items: center; gap: 6px; }
      .codex-plus-page-nav-group-action {
        border: 0;
        background: transparent;
        color: var(--codex-plus-text-tertiary);
        font: inherit;
        font-size: 13px;
        cursor: pointer;
        padding: 0 2px;
      }
      .codex-plus-page-nav-group-action:hover { color: var(--codex-plus-text); }
      .codex-plus-page-nav-item-action {
        flex: 0 0 auto;
        padding: 2px 8px;
        border: 1px solid var(--codex-plus-border);
        border-radius: 6px;
        color: var(--codex-plus-text-secondary);
        font-size: 13px;
      }
      .codex-plus-page-nav-item:hover .codex-plus-page-nav-item-action {
        background: var(--codex-plus-bg-selected);
        color: var(--codex-plus-text);
      }
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
      .codex-plus-row-description { margin-top: 2px; color: #a1a1aa; font-size: 13px; line-height: 1.4; }
      .codex-plus-row-description[data-codex-plus-native-fast-status][data-failed="true"] { color: #f59e0b; }
      .codex-plus-model-compat-warning { margin-top: 6px; color: #fbbf24; font-size: 13px; line-height: 1.45; }
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
      .codex-plus-toggle[data-pending="true"],
      .codex-plus-toggle:disabled { cursor: not-allowed; opacity: .55; }
      .codex-plus-width-control { display: flex; align-items: center; justify-content: flex-end; gap: 8px; min-width: 176px; align-self: center; }
      .codex-plus-width-input {
        width: 78px;
        height: 26px;
        box-sizing: border-box;
        border: 1px solid rgba(255,255,255,.18);
        border-radius: 7px;
        background: rgba(255,255,255,.08);
        color: #f3f4f6;
        font-size: 13px;
        font-family: inherit;
        padding: 0 8px;
      }
      .codex-plus-width-input:disabled { opacity: .55; cursor: not-allowed; }
      .codex-plus-settings input.codex-plus-daily-project-path { width: min(20em, 32%); min-width: 5em; text-align: start; }
      .codex-plus-about { color: #a1a1aa; line-height: 1.5; }
      .codex-plus-panel[hidden] { display: none; }
      .codex-plus-action-button,
      .codex-plus-issue-button { border: 1px solid rgba(255,255,255,.18); border-radius: 7px; background: #3f3f46; color: #f3f4f6; font-size: 13px;
        font-family: inherit; padding: 6px 8px; }
      .codex-plus-form-field {
        display: grid;
        gap: 4px;
        margin-top: 10px;
        /* issue #2359：原来是写死的 #d4d4d8，浅色主题下对比度只有 1.38:1（≈看不见）。
           标签文字改跟主题走。 */
        color: var(--codex-plus-text-secondary);
        font-size: 13px;
        font-family: inherit;
        text-align: left;
      }
      .codex-plus-form-field input {
        width: min(520px, 72vw);
        border: 1px solid rgba(255,255,255,.18);
        border-radius: 8px;
        background: #18181b;
        color: #f4f4f5;
        padding: 8px 10px;
        font: 13px ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      }
      .codex-plus-form-message {
        min-height: 18px;
        margin-top: 10px;
        color: #a1a1aa;
        font-size: 13px;
        font-family: inherit;
        text-align: left;
      }
      .codex-plus-form-message[data-status="ok"] { color: #34d399; }
      .codex-plus-form-message[data-status="failed"] { color: #f87171; }
      .codex-plus-form-message[data-status="loading"] { color: #fbbf24; }
      .codex-plus-backend-status { display: grid; gap: 4px; min-width: 132px; justify-items: end; }
      .codex-plus-backend-label { color: #a1a1aa; font-size: 13px; }
      .codex-plus-backend-label[data-status="ok"] { color: #34d399; }
      .codex-plus-backend-label[data-status="failed"] { color: #f87171; }
      .codex-plus-backend-label[data-status="degraded"] { color: #fbbf24; }
      .codex-plus-sponsor-text { color: #d1d5db; font-size: 13px; line-height: 1.55; margin: 4px 0 12px; }
      /*
       * 推荐内容：网格卡片。
       *
       * 一张卡 = 图标 + 名称/简介 + 右上箭头 + 底部优惠条。用 auto-fill + minmax
       * 让列数随宽度自适应（宽屏 4 列、窄屏递减），卡片等高对齐。
       */
      .codex-plus-ad-list {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(268px, 1fr));
        gap: 12px;
      }
      .codex-plus-ad-card {
        display: flex;
        flex-direction: column;
        gap: 12px;
        border: 1px solid rgba(255,255,255,.08);
        border-radius: 12px;
        background: rgba(255,255,255,.02);
        color: inherit;
        text-decoration: none;
        padding: 16px;
        transition: background .12s ease, border-color .12s ease;
      }
      .codex-plus-ad-card:hover,
      .codex-plus-ad-card:focus-visible {
        border-color: rgba(255,255,255,.18);
        background: rgba(255,255,255,.05);
        outline: none;
      }
      .codex-plus-ad-main { display: flex; align-items: flex-start; gap: 10px; min-width: 0; }
      .codex-plus-ad-icon {
        flex: 0 0 auto;
        width: 36px;
        height: 36px;
        border-radius: 9px;
        object-fit: contain;
        background: rgba(255,255,255,.06);
      }
      /* 清单没给图时用名称首字占位，比空一块整齐。 */
      .codex-plus-ad-icon-fallback {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        color: #f3f4f6;
        font-size: 15px;
        font-weight: 600;
      }
      .codex-plus-ad-text { flex: 1 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
      .codex-plus-ad-title { overflow: hidden; color: #f8fafc; font-size: 14px; font-weight: 600; line-height: 1.3; text-overflow: ellipsis; white-space: nowrap; }
      /* 简介不再压成一行：卡片按内容撑高，最多 3 行，超出才省略。
         nowrap 会让「提供 Claude 与 ...」这类较长简介只露前几个字。 */
      .codex-plus-ad-description { display: -webkit-box; overflow: hidden; color: #a1a1aa; font-size: 13px; line-height: 1.4; -webkit-box-orient: vertical; -webkit-line-clamp: 3; }
      .codex-plus-ad-arrow { flex: 0 0 auto; width: 14px; height: 14px; margin-top: 2px; color: #71717a; }
      .codex-plus-ad-arrow svg { width: 14px; height: 14px; display: block; }
      .codex-plus-ad-card:hover .codex-plus-ad-arrow,
      .codex-plus-ad-card:focus-visible .codex-plus-ad-arrow { color: #f3f4f6; }
      /* 底部优惠条：撑满卡片宽度，长文本截断。 */
      .codex-plus-ad-promo {
        display: block;
        overflow: hidden;
        border-radius: 8px;
        background: rgba(255,255,255,.05);
        color: #f5a97f;
        font-size: 13px;
        line-height: 1.35;
        padding: 7px 10px;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .codex-plus-ad-empty { border: 1px dashed rgba(255,255,255,.16); border-radius: 12px; color: #9ca3af; font-size: 13px; padding: 12px; text-align: center; }
      /*
       * 注入面板的语义色板：前景/背景必须取自同一套 token，且都要能被主题切换带走。
       *
       * issue #2359（Windows 浅色主题下黑底黑字）：旧写法在中间塞了一整层
       * --token-* 回退名（--token-bg-primary / --token-text-primary /
       * --token-border / --token-list-hover-background / --token-text-default /
       * --token-bg-fog / --token-text-tertiary …）。实测 Codex 客户端（26.9xx）
       * 的产物里这些名字**一个都不存在**——真正在用的一套是 --color-token-*。
       * 于是三级回退链的中间那级永远落空，浅色主题下只靠最后一级写死的深色值
       * 兜底，前景与背景各自独立兜底时明度撞在一起，就成了黑底黑字。
       * 这里把死名删掉，直接接到确实存在、且**由 Codex 自己按主题重算**的
       * --color-token-* 上；末级兜底仅作最后保险，不再承担主题判断。
       */
      :root {
        --codex-plus-bg-primary: var(--color-token-bg-primary, var(--color-token-main-surface-primary, #fff));
        --codex-plus-bg-secondary: var(--color-token-bg-secondary, var(--codex-plus-bg-primary));
        --codex-plus-bg-elevated: var(--color-token-dropdown-background, var(--color-token-bg-secondary, var(--codex-plus-bg-primary)));
        --codex-plus-bg-hover: var(--color-token-interactive-bg-secondary-hover, var(--color-token-list-hover-background, rgba(0,0,0,.06)));
        --codex-plus-bg-selected: var(--color-token-interactive-bg-secondary-selected, var(--codex-plus-bg-hover));
        --codex-plus-text: var(--color-token-text-primary, var(--color-token-foreground, #171717));
        --codex-plus-text-secondary: var(--color-token-text-secondary, var(--codex-plus-text));
        --codex-plus-text-tertiary: var(--color-token-text-tertiary, var(--codex-plus-text-secondary));
        --codex-plus-border: var(--color-token-border-light, var(--color-token-border, var(--color-token-border-default, rgba(0,0,0,.12))));
        --codex-plus-border-subtle: var(--codex-plus-border);
        --codex-plus-focus: var(--color-token-focus-border, var(--color-border-focus, currentColor));
        --codex-plus-danger: var(--color-text-danger, #dc2626);
        --codex-plus-danger-bg: var(--color-background-danger-soft, rgba(220,38,38,.1));
        --codex-plus-success: var(--color-text-success, #15803d);
        --codex-plus-warning: var(--color-text-warning, #a16207);
        /*
         * 设置类页面（设置主页、远程控制、拓展、供应商）直接取 Codex 原生设置页用的那几个变量：
         * 开关是原生 Switch 的 --switch-*，卡片是 --color-background-panel，内容区是 --color-surface，
         * 左栏照原生侧栏「surface 65% 叠在工作区底色上」混出来。主题、强调色切换都跟着走。
         * 这几个名字以前只有引用、从没定义过，一直落在写死的兜底色上。
         */
        --codex-plus-switch-on: var(--switch-track-color-checked, var(--color-token-focus-border, #0285ff));
        --codex-plus-switch-off: var(--switch-track-color, color-mix(in oklab, var(--codex-plus-text) 10%, transparent));
        --codex-plus-switch-thumb: var(--switch-thumb-color, #fff);
        --codex-plus-settings-page-bg: var(--color-surface, var(--color-token-main-surface-primary, var(--codex-plus-bg-primary)));
        --codex-plus-settings-card-bg: var(--color-background-panel, var(--codex-plus-bg-secondary));
        --codex-plus-settings-card-border: var(--color-token-border, var(--codex-plus-border));
        --codex-plus-nav-bg: color-mix(in oklab, var(--codex-plus-settings-page-bg) 65%, var(--app-color-background-elevated-secondary-opaque, var(--codex-plus-bg-primary)));
        --codex-plus-nav-hover-bg: var(--color-background-secondary-ghost-hover, var(--codex-plus-bg-hover));
        --codex-plus-nav-selected-bg: var(--color-background-secondary-soft-alpha, var(--codex-plus-bg-selected));
        --codex-plus-control-bg: var(--color-background-primary-soft-alpha, transparent);
        --codex-plus-control-border: var(--color-token-border, var(--codex-plus-border));
        --codex-plus-button-bg: var(--color-background-secondary-soft-alt, color-mix(in oklab, var(--codex-plus-text) 5%, transparent));
        --codex-plus-button-hover-bg: var(--color-background-secondary-soft-hover, var(--codex-plus-bg-hover));
        /* 原生圆角都乘 --codex-corner-radius-scale：按钮 / 导航项 10px、卡片 16px（缩放 1.25 时即 12.5 / 20）。 */
        --codex-plus-control-radius: calc(10px * var(--codex-corner-radius-scale, 1));
        --codex-plus-card-radius: calc(16px * var(--codex-corner-radius-scale, 1));
      }
      :where(.${moreMenuClass}, .${actionTooltipClass}, .codex-delete-toast, .codex-delete-confirm-overlay, .codex-plus-modal-overlay, .${codexPlusPageClass}) {
        color: var(--codex-plus-text);
        font-family: inherit;
      }
      .${moreMenuClass} {
        border-color: var(--codex-plus-border);
        border-radius: var(--border-radius-lg, 8px);
        background: var(--codex-plus-bg-elevated);
        color: var(--codex-plus-text);
        box-shadow: var(--ui-menu-shadow, var(--shadow-300, 0 8px 24px rgba(0,0,0,.16)));
      }
      .codex-session-more-menu-item { border-radius: var(--border-radius-sm, 6px); font-family: inherit; }
      .codex-session-more-menu-item:hover,
      .codex-session-more-menu-item:focus-visible { background: var(--codex-plus-bg-hover); }
      .${actionTooltipClass} {
        border-color: var(--codex-plus-border);
        border-radius: var(--border-radius-md, 6px);
        background: var(--codex-plus-bg-elevated);
        color: var(--codex-plus-text);
        font-family: inherit;
        font-size: 13px;
        line-height: 16px;
        padding: 6px 8px;
        box-shadow: var(--tooltip-box-shadow, var(--shadow-200, 0 4px 12px rgba(0,0,0,.14)));
      }
      .codex-delete-confirm-overlay,
      .codex-plus-modal-overlay { background: var(--color-background-surface-under, rgba(0,0,0,.32)); backdrop-filter: blur(1px); }
      .codex-delete-confirm-content,
      .codex-plus-modal-content {
        border-color: var(--codex-plus-border);
        border-radius: var(--border-radius-xl, 12px);
        background: var(--codex-plus-bg-primary);
        color: var(--codex-plus-text);
        font-family: inherit;
        box-shadow: var(--shadow-400, 0 16px 48px rgba(0,0,0,.2));
      }
      .codex-delete-confirm-message,
      .codex-plus-row-description,
      .codex-plus-about,
      .codex-plus-backend-label,
      .codex-plus-form-message,
      .codex-plus-sponsor-text { color: var(--codex-plus-text-secondary); }
      .codex-delete-confirm-actions button,
      .codex-plus-action-button,
      .codex-plus-issue-button {
        min-height: 32px;
        border: 1px solid var(--codex-plus-border);
        border-radius: var(--border-radius-lg, 8px);
        background: var(--codex-plus-bg-secondary);
        color: var(--codex-plus-text);
        font: inherit;
        font-size: 13px;
        line-height: 18px;
        padding: 5px 10px;
      }
      .codex-delete-confirm-actions button:hover,
      .codex-delete-confirm-actions button:focus-visible,
      .codex-plus-action-button:hover,
      .codex-plus-action-button:focus-visible,
      .codex-plus-issue-button:hover,
      .codex-plus-issue-button:focus-visible,
      .codex-delete-confirm-actions [data-codex-delete-confirm="true"] {
        border-color: var(--color-border-danger, #dc2626);
        background: var(--color-background-danger-solid, #dc2626);
        color: var(--color-text-danger-solid, #fff);
      }
      .codex-plus-modal-close { border-color: var(--codex-plus-border); color: var(--codex-plus-text-secondary); border-radius: var(--border-radius-lg, 8px); }
      .codex-plus-modal-close:hover,
      .codex-plus-modal-close:focus-visible { background: var(--codex-plus-bg-hover); color: var(--codex-plus-text); outline: none; }
      .${codexPlusPageClass},
      .${codexPlusPageClass} .codex-plus-modal-content {
        background: var(--codex-plus-bg-primary) !important;
        color: var(--codex-plus-text) !important;
      }
      .${codexPlusPageClass} .codex-plus-modal-content { border-color: transparent; }
      /*
       * 梦皮（Dream Skin）壁纸模式（#3152）：原生设置页在梦皮下是一层半透明遮罩叠在壁纸上，
       * 我们的页面却是实色底，整块盖住壁纸。页面是盖在内容区上的独立 fixed 层，底下不是同一页，
       * 不能直接透明；这里照梦皮给原生设置页的画法，在页面根节点自己画「遮罩 + 壁纸」，
       * 内层的页面底、左栏、内容区改透明。变量都由梦皮定义（--dream-skin-art / --ds-*），
       * 每个都带兜底：缺了就退回原来的实色底，不会变成全透明。
       * full 模式对应梦皮的 --ds-immersive-*，ambient / banner 对应更重的 --ds-task-immersive-*；
       * off 模式梦皮自己也不画壁纸，这里不管。
       */
      html[data-dream-skin="active"][data-dream-art-ready="true"]:is([data-dream-task-mode="full"], [data-dream-art-task-mode="full"]) .${codexPlusPageClass} {
        --codex-plus-dream-veil: linear-gradient(90deg, var(--ds-immersive-edge, transparent), var(--ds-immersive-mid, transparent) 64%, var(--ds-immersive-far, transparent));
      }
      html[data-dream-skin="active"][data-dream-art-ready="true"]:is([data-dream-task-mode="ambient"], [data-dream-art-task-mode="ambient"], [data-dream-task-mode="banner"], [data-dream-art-task-mode="banner"]) .${codexPlusPageClass} {
        --codex-plus-dream-veil: linear-gradient(90deg, var(--ds-task-immersive-edge, transparent), var(--ds-task-immersive-mid, transparent) 64%, var(--ds-task-immersive-far, transparent));
      }
      html[data-dream-skin="active"][data-dream-art-ready="true"]:is([data-dream-task-mode="full"], [data-dream-art-task-mode="full"], [data-dream-task-mode="ambient"], [data-dream-art-task-mode="ambient"], [data-dream-task-mode="banner"], [data-dream-art-task-mode="banner"]) .${codexPlusPageClass} {
        --codex-plus-settings-page-bg: transparent !important;
        --codex-plus-nav-bg: transparent !important;
        background:
          var(--codex-plus-dream-veil, linear-gradient(transparent, transparent)),
          var(--dream-skin-art, none) var(--ds-art-position, center) / cover no-repeat fixed,
          var(--ds-bg, var(--codex-plus-bg-primary)) !important;
      }
      html[data-dream-skin="active"][data-dream-art-ready="true"]:is([data-dream-task-mode="full"], [data-dream-art-task-mode="full"], [data-dream-task-mode="ambient"], [data-dream-art-task-mode="ambient"], [data-dream-task-mode="banner"], [data-dream-art-task-mode="banner"]) .${codexPlusPageClass} .codex-plus-modal-content {
        background: transparent !important;
      }
      /* 吸顶标题底下会滚过正文：透明后改成梦皮面板色的半透明磨砂，正文不会从标题后面透出来。 */
      html[data-dream-skin="active"][data-dream-art-ready="true"]:is([data-dream-task-mode="full"], [data-dream-art-task-mode="full"], [data-dream-task-mode="ambient"], [data-dream-art-task-mode="ambient"], [data-dream-task-mode="banner"], [data-dream-art-task-mode="banner"]) .${codexPlusPageClass} :is(.codex-plus-provider-detail > .codex-plus-provider-detail-header, .codex-plus-task-group-head) {
        background: color-mix(in srgb, var(--ds-panel, var(--codex-plus-bg-primary)) 72%, transparent);
        backdrop-filter: blur(18px) saturate(1.1);
      }
      .codex-plus-modal-body { scrollbar-color: var(--codex-plus-text-tertiary) transparent; }
      .codex-plus-modal-body::-webkit-scrollbar-thumb { background: color-mix(in srgb, var(--codex-plus-text-tertiary) 45%, transparent); background-clip: padding-box; }
      .codex-plus-modal-body::-webkit-scrollbar-thumb:hover { background: var(--codex-plus-text-tertiary); background-clip: padding-box; }
      .codex-plus-row { border-top-color: var(--codex-plus-border-subtle); }
      .codex-plus-toggle { background: var(--color-background-secondary-solid, var(--codex-plus-text-tertiary)); }
      .codex-plus-toggle span { background: var(--color-token-bg-primary, #fff); box-shadow: var(--switch-thumb-shadow, 0 1px 2px rgba(0,0,0,.16)); }
      .codex-plus-toggle[data-enabled="true"] { background: var(--color-background-primary-solid, var(--color-background-success-solid, #10a37f)); }
      .codex-plus-width-input,
      .codex-plus-form-field input {
        border: 1px solid var(--codex-plus-border);
        border-radius: var(--border-radius-lg, 8px);
        background: var(--codex-plus-bg-secondary);
        color: var(--codex-plus-text);
        font-family: inherit;
      }
      .codex-plus-width-input:focus,
      .codex-plus-form-field input:focus { border-color: var(--codex-plus-focus); outline: 2px solid color-mix(in srgb, var(--codex-plus-focus) 25%, transparent); outline-offset: 0; }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status,
      .codex-plus-backend-indicator { box-shadow: none; }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status[data-status="ok"],
      .codex-plus-backend-indicator[data-status="ok"] { background: var(--codex-plus-success); }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status[data-status="failed"],
      .codex-plus-backend-indicator[data-status="failed"] { background: var(--codex-plus-danger); }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status[data-status="checking"],
      .codex-plus-backend-indicator[data-status="checking"] { background: var(--codex-plus-warning); }
      #${codexPlusSidebarNavId} .codex-plus-sidebar-nav-status[data-status="degraded"],
      .codex-plus-backend-indicator[data-status="degraded"] { background: var(--codex-plus-warning); }
      .codex-plus-ad-card { border-color: var(--codex-plus-border-subtle); background: var(--codex-plus-bg-secondary); }
      .codex-plus-ad-card:hover,
      .codex-plus-ad-card:focus-visible { border-color: var(--codex-plus-border); background: var(--codex-plus-bg-hover); }
      .codex-plus-ad-icon { background: var(--codex-plus-bg-hover); }
      .codex-plus-ad-icon-fallback { color: var(--codex-plus-text); }
      .codex-plus-ad-title { color: var(--codex-plus-text); }
      .codex-plus-ad-description { color: var(--codex-plus-text-secondary); }
      .codex-plus-ad-arrow { color: var(--codex-plus-text-tertiary); }
      .codex-plus-ad-card:hover .codex-plus-ad-arrow,
      .codex-plus-ad-card:focus-visible .codex-plus-ad-arrow { color: var(--codex-plus-text); }
      /*
       * 优惠条：暗底配橙色文字。
       *
       * 底色不能用 --codex-plus-danger-bg —— 它在当前主题下解析成浅粉（偏浅色主题
       * 的值），压在深色卡片上非常刺眼。改成用警告色按低透明度混出来，深浅主题
       * 都成立，也和原生「需要注意」的语义色同源。
       */
      .codex-plus-ad-promo {
        background: color-mix(in srgb, var(--codex-plus-warning) 14%, transparent);
        color: var(--codex-plus-warning);
      }
      .codex-plus-ad-empty { border-color: var(--codex-plus-border); color: var(--codex-plus-text-tertiary); }
      .codex-plus-form-message[data-status="ok"], .codex-plus-backend-label[data-status="ok"] { color: var(--codex-plus-success); }
      .codex-plus-form-message[data-status="failed"], .codex-plus-backend-label[data-status="failed"] { color: var(--codex-plus-danger); }
      .codex-plus-backend-label[data-status="degraded"] { color: var(--codex-plus-warning); }
      .codex-plus-form-message[data-status="loading"], .codex-plus-model-compat-warning { color: var(--codex-plus-warning); }
      /* Codex++ 设置主页：对齐原生设置页，保持其它页面与弹窗的布局独立。 */
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] {
        background: var(--codex-plus-settings-page-bg, var(--codex-plus-bg-primary)) !important;
      }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-modal-body {
        box-sizing: border-box;
        padding: 16px 20px 64px;
      }
      /*
       * 设置页左栏：尺寸、字号、选中态照原生设置页的侧栏（288 宽、30 高的行、14px、10px 圆角乘缩放），
       * 版本号与后端状态点在顶上，位置对应原生的「设置」标题。
       */
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav {
        box-sizing: border-box;
        width: 288px;
        flex-basis: 288px;
        background: var(--codex-plus-nav-bg);
      }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-header { padding: 14px 16px 10px; }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-header > .codex-plus-modal-header { padding: 0; }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-header .codex-plus-modal-title { font-family: var(--font-openai-sans, inherit); font-synthesis: weight style small-caps; font-size: 18px; font-weight: 600; line-height: 24px; }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-body { display: flex; flex-direction: column; gap: 1px; padding: 4px 8px 8px; }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-item {
        box-sizing: border-box;
        height: 30px;
        min-height: 30px;
        padding: 5px 8px;
        border-radius: var(--codex-plus-control-radius);
        color: var(--codex-plus-text);
        font-size: 14px;
        font-weight: 400;
        line-height: 21px;
      }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-item:hover { background: var(--codex-plus-nav-hover-bg); }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-item[data-active="true"] { background: var(--codex-plus-nav-selected-bg); }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-item-icon { width: 16px; height: 16px; border-radius: 0; background: none; color: inherit; }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-item-icon svg { width: 16px; height: 16px; }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-item-name { flex: 1 1 auto; min-width: 0; }
      /* 分区小标题照原生：14px/500、次要灰、30 高；分区之间空 16px，标题和第一项之间空 4px（nav-body 自带 1px 间距）。 */
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-section-title {
        display: flex;
        align-items: center;
        box-sizing: border-box;
        min-height: 30px;
        margin: 15px 0 3px;
        padding: 0 6px 0 8px;
        color: var(--codex-plus-text-tertiary);
        font-size: 14px;
        font-weight: 500;
        line-height: 21px;
      }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav-section-title:first-child { margin-top: 0; }
      .codex-plus-settings {
        width: 100%;
        max-width: 728px;
        min-width: 0;
        margin: 0 auto;
        container: codex-plus-settings / inline-size;
      }
      .codex-plus-settings > .codex-plus-modal-header {
        width: 100%;
        box-sizing: border-box;
        padding: 0;
        margin: 0 0 48px;
        gap: 16px;
      }
      .codex-plus-settings .codex-plus-modal-title {
        min-width: 0;
        font-size: 28px;
        font-weight: 600;
        line-height: 36px;
        letter-spacing: -.02em;
        overflow-wrap: anywhere;
      }
      .codex-plus-settings-section { margin: 0 0 44px; }
      .codex-plus-settings-section:last-child { margin-bottom: 0; }
      /* 小节标题照原生：14px/500，46 高的标题行里文字居中、底下留 6px，紧贴卡片。 */
      .codex-plus-settings-section-title {
        display: flex;
        align-items: center;
        box-sizing: border-box;
        min-height: 46px;
        margin: 0;
        padding: 0 0 6px;
        color: var(--codex-plus-text);
        font-size: 14px;
        font-weight: 500;
        line-height: 21px;
      }
      .codex-plus-settings-section-description {
        margin: -4px 0 12px;
        color: var(--codex-plus-text-secondary);
        font-size: 12px;
        line-height: 16px;
      }
      .codex-plus-settings-group + .codex-plus-settings-group { margin-top: 48px; }
      .codex-plus-settings-group[data-codex-plus-settings-group="ext-menu"]:not(:has([data-codex-plus-ext-menu])) { display: none; }
      /* 分页：左栏选分组，右边只显示当前这一组；组里第一节的标题当页面标题，后面的服务配置节是小节。 */
      .codex-plus-settings[data-codex-plus-settings-paged="true"] > .codex-plus-settings-group:not([data-active="true"]) { display: none; }
      .codex-plus-settings[data-codex-plus-settings-paged="true"] > .codex-plus-settings-group { margin-top: 0; }
      .codex-plus-settings[data-codex-plus-settings-paged="true"] > .codex-plus-settings-group > .codex-plus-settings-section { margin-bottom: 44px; }
      .codex-plus-settings[data-codex-plus-settings-paged="true"] > .codex-plus-settings-group > .codex-plus-settings-section:last-child { margin-bottom: 0; }
      /*
       * 分页时组里第一节的标题当页面标题，位置照原生：60 高的标题块（上下 13px）、隔 21px 是 46 高的小节标题行。
       * 分组说明就放在那一行里（次要色），卡片和原生第一张卡片落在同一高度。
       */
      .codex-plus-settings[data-codex-plus-settings-paged="true"] > .codex-plus-settings-group > .codex-plus-settings-section:first-child > .codex-plus-settings-section-title {
        display: block;
        min-height: 0;
        padding: 13px 0;
        font-size: 28px;
        font-weight: 500;
        line-height: 1.2;
      }
      .codex-plus-settings[data-codex-plus-settings-paged="true"] > .codex-plus-settings-group > .codex-plus-settings-section:first-child > .codex-plus-settings-section-title + .codex-plus-settings-card { margin-top: 21px; }
      /* 拓展注册的设置分区（registerSettingsSection）：标题由外壳给，内容是拓展自己画的。 */
      .codex-plus-settings-section-title + .codex-plus-settings-extension-section { margin-top: 21px; }
      .codex-plus-settings-extension-section { min-width: 0; }
      .codex-plus-settings[data-codex-plus-settings-paged="true"] > .codex-plus-settings-group > .codex-plus-settings-section:first-child > .codex-plus-settings-section-description {
        display: flex;
        align-items: center;
        box-sizing: border-box;
        min-height: 46px;
        margin: 21px 0 0;
        padding: 0 0 6px;
        font-size: 13px;
        line-height: 18px;
      }
      .codex-plus-settings-card,
      .codex-plus-settings [data-codex-plus-ext-menu] {
        box-sizing: border-box;
        min-width: 0;
        padding: 0 16px;
        border: 1px solid var(--codex-plus-settings-card-border);
        border-radius: var(--codex-plus-card-radius);
        background: var(--codex-plus-settings-card-bg);
      }
      .codex-plus-settings > [data-codex-plus-ext-menu] { margin-top: 48px; }
      .codex-plus-settings .codex-plus-row {
        box-sizing: border-box;
        min-height: 52px;
        align-items: center;
        gap: 24px;
        padding: 12px 0;
        font-size: 13px;
        line-height: 18px;
      }
      .codex-plus-settings .codex-plus-row[hidden] { display: none; }
      /* 内置拓展的子选项缩进挂在所属拓展下面；被顶替时的小字用警示色。 */
      .codex-plus-settings .codex-plus-row[data-codex-plus-builtin-param] { padding-left: 20px; }
      .codex-plus-settings .codex-plus-row-description[data-codex-builtin-replaced]::after {
        content: " · " attr(data-codex-builtin-replaced);
        color: var(--color-token-text-warning, #d97706);
      }
      .codex-plus-settings .codex-plus-row > :first-child {
        flex: 1 1 auto;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .codex-plus-settings .codex-plus-row > :not(:first-child) { max-width: 48%; }
      /* 行高与原生 text-sm 一致：13px × 1.25/0.875。 */
      .codex-plus-settings .codex-plus-row-title { font-size: 13px; font-weight: 500; line-height: calc(1.25 / .875); }
      /* 设置项标题旁的小标签：实验 / 需重启，与管理器增强页一致。 */
      .codex-plus-settings .codex-plus-feature-tag {
        display: inline-flex;
        align-items: center;
        margin-left: 6px;
        padding: 0 5px;
        border: 1px solid var(--codex-plus-border);
        border-radius: 4px;
        color: var(--codex-plus-text-secondary);
        font-size: 10.5px;
        font-style: normal;
        font-weight: 500;
        line-height: 16px;
        vertical-align: 1px;
      }
      .codex-plus-settings .codex-plus-feature-tag[data-tone="experimental"] {
        border-color: color-mix(in srgb, var(--codex-plus-warning) 40%, transparent);
        color: var(--codex-plus-warning);
      }
      .codex-plus-settings .codex-plus-row-description,
      .codex-plus-settings .codex-plus-about {
        margin-top: 2px;
        color: var(--codex-plus-text-secondary);
        font-size: 12px;
        line-height: 16px;
        overflow-wrap: anywhere;
      }
      .codex-plus-settings .codex-plus-toggle {
        flex: 0 0 32px;
        width: 32px;
        height: 20px;
        min-height: 20px;
        box-sizing: border-box;
        padding: 2px;
        background: var(--codex-plus-switch-off);
        cursor: pointer;
      }
      .codex-plus-settings .codex-plus-toggle span {
        width: 16px;
        height: 16px;
        background: var(--codex-plus-switch-thumb);
        box-shadow: var(--switch-thumb-shadow, 0 1px 2px rgba(0,0,0,.2));
      }
      .codex-plus-settings .codex-plus-toggle[data-enabled="true"] { background: var(--codex-plus-switch-on); }
      .codex-plus-settings .codex-plus-toggle[data-enabled="true"] span { background: var(--switch-thumb-color-checked, var(--codex-plus-switch-thumb)); }
      .codex-plus-settings .codex-plus-toggle[data-enabled="true"] span { transform: translateX(12px); }
      .codex-plus-settings .codex-plus-toggle[data-pending="true"],
      .codex-plus-settings .codex-plus-toggle:disabled { cursor: not-allowed; opacity: .55; }
      .codex-plus-settings .codex-plus-width-control {
        flex: 0 1 auto;
        min-width: 0;
        flex-wrap: wrap;
        gap: 8px;
      }
      .codex-plus-settings .codex-plus-backend-status { overflow-wrap: anywhere; text-align: right; }
      .codex-plus-settings .codex-plus-action-button,
      .codex-plus-settings .codex-plus-issue-button,
      .codex-plus-settings .codex-plus-width-input,
      .codex-plus-settings .codex-plus-service-input {
        box-sizing: border-box;
        min-height: 28px;
        border: 1px solid var(--codex-plus-control-border);
        border-radius: var(--codex-plus-control-radius);
        background: var(--codex-plus-control-bg);
        color: var(--codex-plus-text);
        font-family: inherit;
        font-size: 13px;
        line-height: 18px;
      }
      .codex-plus-settings .codex-plus-action-button,
      .codex-plus-settings .codex-plus-issue-button {
        padding: 4px 10px;
        border-color: transparent;
        background: var(--codex-plus-button-bg);
      }
      .codex-plus-settings .codex-plus-width-input,
      .codex-plus-settings .codex-plus-service-input { height: 28px; padding: 0 10px; }
      .codex-plus-settings .codex-plus-width-input { width: 72px; min-width: 48px; }
      .codex-plus-settings .codex-plus-action-button,
      .codex-plus-settings .codex-plus-issue-button { white-space: normal; overflow-wrap: anywhere; }
      .codex-plus-settings .codex-plus-action-button:hover,
      .codex-plus-settings .codex-plus-action-button:focus-visible,
      .codex-plus-settings .codex-plus-issue-button:hover,
      .codex-plus-settings .codex-plus-issue-button:focus-visible {
        border-color: transparent;
        background: var(--codex-plus-button-hover-bg);
        color: var(--codex-plus-text);
      }
      .codex-plus-settings .codex-plus-width-input:focus,
      .codex-plus-settings .codex-plus-service-input:focus { border-color: var(--color-token-input-border, var(--codex-plus-border)); outline: none; }
      .codex-plus-settings button:focus-visible,
      .codex-plus-settings input:focus-visible,
      .codex-plus-settings select:focus-visible {
        outline: 2px solid var(--codex-plus-focus);
        outline-offset: 2px;
      }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] { container: codex-plus-settings-page / inline-size; }
      @container codex-plus-settings-page (max-width: 800px) {
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-modal-body { padding: 24px 16px 48px; }
        /* 窗口窄时左栏让出宽度，和原生侧栏可拖窄的观感一致。 */
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav { width: 220px; flex-basis: 220px; }
      }
      @container codex-plus-settings-page (max-width: 480px) {
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-modal-body { padding: 16px 12px 32px; }
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="home"] .codex-plus-page-nav { width: 168px; flex-basis: 168px; }
      }
      /* 「远程控制」页（45-remote-control-page.js）：沿用设置主页的版式，只补本页特有的小件。 */
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="remote-control"] {
        background: var(--codex-plus-settings-page-bg, var(--codex-plus-bg-primary)) !important;
        container: codex-plus-remote-page / inline-size;
      }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="remote-control"] .codex-plus-modal-body {
        box-sizing: border-box;
        padding: 72px 32px 64px;
      }
      @container codex-plus-remote-page (max-width: 800px) {
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="remote-control"] .codex-plus-modal-body { padding: 48px 24px; }
      }
      .codex-plus-remote-page .codex-plus-feature-tag { font-size: 11px; vertical-align: 6px; }
      .codex-plus-remote-page .codex-plus-row > :not(:first-child) { max-width: 60%; }
      .codex-plus-remote-controls { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 8px; }
      .codex-plus-remote-controls .codex-plus-action-button { padding: 0 12px; white-space: nowrap; }
      .codex-plus-remote-banner { margin: -24px 0 24px; padding: 8px 12px; border-radius: 8px; font-size: 13px; line-height: 18px;
        background: var(--codex-plus-bg-elevated, var(--codex-plus-bg-hover)); color: var(--codex-plus-text-secondary); }
      .codex-plus-remote-banner[data-tone="warn"] { color: var(--codex-plus-warning); }
      .codex-plus-remote-banner[data-tone="ok"] { color: var(--codex-plus-success); }
      .codex-plus-remote-url, .codex-plus-remote-fingerprint, .codex-plus-remote-address code { font-family: var(--codex-plus-font-mono, ui-monospace, Menlo, monospace); font-size: 12px; overflow-wrap: anywhere; }
      .codex-plus-remote-address { display: flex; align-items: center; gap: 6px; margin-top: 2px; }
      .codex-plus-remote-segmented { display: inline-flex; border: 1px solid var(--codex-plus-border); border-radius: 8px; overflow: hidden; }
      .codex-plus-remote-segmented button { min-height: 30px; padding: 0 10px; border: 0; border-left: 1px solid var(--codex-plus-border);
        background: transparent; color: var(--codex-plus-text-secondary); font: inherit; font-size: 12.5px; cursor: pointer; white-space: nowrap; }
      .codex-plus-remote-segmented button:first-child { border-left: 0; }
      .codex-plus-remote-segmented button[aria-checked="true"] { background: var(--codex-plus-bg-hover); color: var(--codex-plus-text); font-weight: 550; }
      .codex-plus-remote-segmented button:disabled { cursor: default; opacity: .6; }
      .codex-plus-remote-pairing { display: grid; gap: 12px; padding: 14px 0 16px; border-top: 1px solid var(--codex-plus-border-subtle, var(--codex-plus-border)); }
      .codex-plus-remote-pairing-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; font-size: 13px; }
      .codex-plus-remote-code { font-family: var(--codex-plus-font-mono, ui-monospace, Menlo, monospace); font-size: 16px; letter-spacing: 2px; }
      .codex-plus-remote-qrs { display: flex; flex-wrap: wrap; gap: 16px; }
      .codex-plus-remote-qrs figure { margin: 0; display: grid; gap: 6px; justify-items: center; color: var(--codex-plus-text-secondary); font-size: 12px; }
      .codex-plus-remote-qr { width: 180px; padding: 8px; border-radius: 8px; background: #fff; }
      .codex-plus-remote-qr svg { display: block; width: 100%; height: auto; }
      /* 「微信连接」页（45-weixin-connect-page.js）：版式同远程控制页；账号、联系人 ID、登录链接这类长文本省略号截断，不撑破布局。 */
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="weixin-connect"] {
        background: var(--codex-plus-settings-page-bg, var(--codex-plus-bg-primary)) !important;
        container: codex-plus-weixin-page / inline-size;
      }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="weixin-connect"] .codex-plus-modal-body {
        box-sizing: border-box;
        padding: 72px 32px 64px;
      }
      @container codex-plus-weixin-page (max-width: 800px) {
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="weixin-connect"] .codex-plus-modal-body { padding: 48px 24px; }
      }
      .codex-plus-weixin-page .codex-plus-row > :not(:first-child) { max-width: 60%; min-width: 0; }
      .codex-plus-weixin-controls { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 8px; min-width: 0; }
      .codex-plus-weixin-controls .codex-plus-action-button { padding: 0 12px; white-space: nowrap; }
      .codex-plus-weixin-page .codex-plus-action-button[data-primary="true"]:not(:disabled) { background: var(--codex-plus-text); color: var(--codex-plus-bg-primary); }
      .codex-plus-weixin-page .codex-plus-action-button:disabled { cursor: default; opacity: .55; }
      .codex-plus-weixin-ellipsis { display: block; min-width: 0; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-plus-weixin-meta-line { display: flex; flex-wrap: wrap; gap: 2px 12px; min-width: 0; margin-top: 2px; }
      .codex-plus-weixin-meta { display: inline-flex; align-items: baseline; gap: 4px; min-width: 0; max-width: 100%; white-space: nowrap; }
      .codex-plus-weixin-meta code, .codex-plus-weixin-qr-link { font-family: var(--codex-plus-font-mono, ui-monospace, Menlo, monospace); font-size: 12px; }
      .codex-plus-weixin-meta code { flex: 0 1 auto; max-width: 220px; }
      .codex-plus-weixin-input { width: 280px; max-width: 100%; min-width: 0; text-overflow: ellipsis; }
      .codex-plus-weixin-menu { position: relative; display: inline-flex; min-width: 0; max-width: 100%; }
      .codex-plus-weixin-menu .codex-plus-menu-trigger { max-width: 100%; }
      .codex-plus-weixin-menu .codex-plus-menu-trigger:disabled { cursor: default; opacity: .55; }
      .codex-plus-weixin-menu-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
      .codex-plus-weixin-menu-popover { top: calc(100% + 4px); right: 0; max-width: min(360px, 80vw); max-height: 320px; overflow-y: auto; }
      .codex-plus-weixin-title-badge { margin-left: 8px; font-weight: 400; }
      .codex-plus-weixin-qr-card { display: flex; flex-wrap: wrap; align-items: flex-start; gap: 16px; padding: 16px; }
      .codex-plus-weixin-qr { flex: none; }
      .codex-plus-weixin-qr-side { display: grid; flex: 1 1 220px; gap: 10px; min-width: 0; }
      .codex-plus-weixin-qr-side .codex-plus-weixin-controls { justify-content: flex-start; }
      .codex-plus-weixin-qr-link { display: block; min-width: 0; overflow: hidden; color: var(--codex-plus-text-secondary); text-overflow: ellipsis; white-space: nowrap; }
      @container codex-plus-settings (max-width: 560px) {
        .codex-plus-settings .codex-plus-row { gap: 16px; }
      }
      /* 「任务面板」页（47-task-board-page.js）：侧栏｜列表｜详情三栏铺满，各栏自己滚动。 */
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="task-board"] {
        background: var(--codex-plus-settings-page-bg, var(--codex-plus-bg-primary)) !important;
      }
      .codex-plus-task-page [hidden] { display: none !important; }
      .codex-plus-task-page {
        --codex-plus-task-line: color-mix(in oklab, var(--codex-plus-text) 9%, transparent);
        --codex-plus-task-line-strong: color-mix(in oklab, var(--codex-plus-text) 18%, transparent);
        --codex-plus-task-tint: color-mix(in oklab, var(--codex-plus-text) 3.5%, transparent);
        --codex-plus-task-page-bg: var(--codex-plus-settings-page-bg, var(--codex-plus-bg-primary));
        --codex-plus-task-sidebar-bg: color-mix(in oklab, var(--codex-plus-text) 3%, var(--codex-plus-task-page-bg));
        --codex-plus-task-accent: var(--codex-plus-focus, #3b82f6);
        --codex-plus-task-selected: color-mix(in oklab, var(--codex-plus-task-accent) 20%, transparent);
      }
      .codex-plus-task-shell:focus, .codex-plus-task-detail:focus { outline: none; }
      .codex-plus-task-shell { display: flex; flex: 1; flex-direction: column; height: 100%; min-height: 0; font-size: 13px; line-height: 18px; color: var(--codex-plus-text); -webkit-user-select: none; user-select: none; container: codex-task-page / inline-size; }
      .codex-plus-task-shell :is(h2, h3, p) { margin: 0; }
      .codex-plus-task-shell svg { flex: none; }
      .codex-plus-task-shell :is(input, textarea) { -webkit-user-select: text; user-select: text; }
      .codex-plus-task-shell :is(input, textarea)::placeholder { color: var(--codex-plus-text-tertiary); opacity: 1; }
      .codex-plus-task-spacer { flex: 1; }
      .codex-plus-task-panes { display: flex; flex: 1; min-height: 0; }
      .codex-plus-task-count { color: var(--codex-plus-text-tertiary); font-size: 12px; font-variant-numeric: tabular-nums; }
      .codex-plus-task-page code { font-family: var(--codex-plus-font-mono, ui-monospace, Menlo, monospace); font-size: 11px; color: var(--codex-plus-text-tertiary); }
      /* 状态图标：空心虚线（收集）→ 空心（待办）→ 半满（进行中）→ 四分之三（待验收）；卡住、完成实心；已取消划斜线。 */
      .codex-plus-task-dot { --codex-plus-task-dot: var(--codex-plus-text-tertiary); display: inline-block; flex: none; box-sizing: border-box; width: 12px; height: 12px; padding: 1.5px; border: 1.5px solid var(--codex-plus-task-dot); border-radius: 50%; background: transparent; background-clip: content-box; }
      .codex-plus-task-dot[data-status="backlog"] { border-style: dashed; }
      .codex-plus-task-dot[data-status="todo"] { --codex-plus-task-dot: var(--codex-plus-text-secondary); }
      .codex-plus-task-dot[data-status="in_progress"] { --codex-plus-task-dot: var(--codex-plus-task-accent); background-image: conic-gradient(var(--codex-plus-task-dot) 0 50%, transparent 50%); }
      .codex-plus-task-dot[data-status="in_review"] { --codex-plus-task-dot: var(--codex-plus-warning); background-image: conic-gradient(var(--codex-plus-task-dot) 0 75%, transparent 75%); }
      .codex-plus-task-dot[data-status="blocked"] { --codex-plus-task-dot: var(--codex-plus-danger); background-color: var(--codex-plus-task-dot); }
      .codex-plus-task-dot[data-status="done"] { --codex-plus-task-dot: var(--codex-plus-success); padding: 0; background-color: var(--codex-plus-task-dot); }
      .codex-plus-task-dot[data-status="cancelled"] { padding: 0; background: linear-gradient(135deg, transparent 42%, var(--codex-plus-task-dot) 42% 58%, transparent 58%); }
      /* 左栏：分类侧栏，和 Codex 设置页的导航同一套底色和选中态。 */
      .codex-plus-task-sidebar { display: flex; flex: none; flex-direction: column; gap: 1px; width: 196px; box-sizing: border-box; padding: 14px 10px; overflow-y: auto; border-right: 1px solid var(--codex-plus-task-line); background: var(--codex-plus-nav-bg, var(--codex-plus-task-sidebar-bg)); }
      .codex-plus-task-sidebar-title { padding: 2px 8px 12px; font-size: 14px; font-weight: 600; }
      .codex-plus-task-sidebar-section { padding: 14px 8px 4px; color: var(--codex-plus-text-tertiary); font-size: 11px; font-weight: 600; }
      .codex-plus-task-sidebar-tip { margin-top: auto !important; padding: 12px 8px 0; color: var(--codex-plus-text-tertiary); font-size: 11px; line-height: 15px; opacity: 0; transition: opacity .15s; }
      .codex-plus-task-page[data-dragging="true"] .codex-plus-task-sidebar-tip { opacity: 1; }
      .codex-plus-task-nav { display: flex; flex: none; align-items: center; gap: 9px; height: 30px; padding: 0 8px; border: 0; border-radius: 7px; background: transparent; color: var(--codex-plus-text); font: inherit; text-align: left; cursor: pointer; }
      .codex-plus-task-nav:hover { background: var(--codex-plus-nav-hover-bg, var(--codex-plus-bg-hover)); }
      .codex-plus-task-nav[aria-current="true"] { background: var(--codex-plus-nav-selected-bg, var(--codex-plus-bg-selected)); font-weight: 550; }
      .codex-plus-task-nav:focus-visible { outline: 2px solid var(--codex-plus-task-accent); outline-offset: -2px; }
      .codex-plus-task-nav[data-over="true"] { background: var(--codex-plus-task-selected); box-shadow: inset 0 0 0 1px var(--codex-plus-task-accent); }
      .codex-plus-task-nav svg { width: 15px; height: 15px; color: var(--codex-plus-text-secondary); }
      .codex-plus-task-nav .codex-plus-task-dot { margin: 0 1.5px; }
      .codex-plus-task-nav-label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-plus-task-nav-count { color: var(--codex-plus-text-tertiary); font-size: 12px; font-weight: 400; font-variant-numeric: tabular-nums; }
      /* 中栏、右栏顶上同高的工具条，横线对齐。 */
      .codex-plus-task-pane-bar { display: flex; flex: none; align-items: center; gap: 8px; height: 48px; box-sizing: border-box; padding: 0 10px 0 16px; border-bottom: 1px solid var(--codex-plus-task-line); }
      .codex-plus-task-heading { font-size: 14px; font-weight: 600; white-space: nowrap; }
      .codex-plus-task-search { display: flex; align-items: center; gap: 6px; width: 180px; min-width: 100px; flex: 0 1 auto; height: 28px; box-sizing: border-box; padding: 0 8px; border: 1px solid transparent; border-radius: 7px; background: var(--codex-plus-task-tint); color: var(--codex-plus-text-tertiary); cursor: text; transition: border-color .12s, background-color .12s; }
      .codex-plus-task-search:focus-within { border-color: var(--codex-plus-task-accent); background: transparent; }
      .codex-plus-task-search svg { width: 14px; height: 14px; }
      .codex-plus-task-search input { flex: 1; min-width: 0; height: 100%; padding: 0; border: 0; outline: 0; background: transparent; color: var(--codex-plus-text); font: inherit; font-size: 12.5px; }
      .codex-plus-task-icon-button { display: inline-flex; flex: none; align-items: center; justify-content: center; width: 28px; height: 28px; border: 0; border-radius: 6px; background: transparent; color: var(--codex-plus-text-secondary); cursor: pointer; }
      .codex-plus-task-icon-button:hover { background: var(--codex-plus-bg-hover); color: var(--codex-plus-text); }
      .codex-plus-task-icon-button:focus-visible { outline: 2px solid var(--codex-plus-task-accent); outline-offset: -2px; }
      .codex-plus-task-icon-button[data-codex-task-action="delete"]:hover { color: var(--codex-plus-danger); }
      .codex-plus-task-icon-button svg { width: 16px; height: 16px; }
      .codex-plus-task-plain-button { height: 28px; padding: 0 10px; border: 1px solid var(--codex-plus-task-line); border-radius: 7px; background: transparent; color: var(--codex-plus-text-secondary); font: inherit; font-size: 12.5px; cursor: pointer; }
      .codex-plus-task-plain-button:hover { background: var(--codex-plus-bg-hover); color: var(--codex-plus-text); }
      .codex-plus-task-empty { display: flex; flex: 1; flex-direction: column; align-items: center; justify-content: center; gap: 6px; min-height: 160px; padding: 32px 20px; color: var(--codex-plus-text-tertiary); font-size: 12.5px; text-align: center; }
      .codex-plus-task-empty svg { width: 30px; height: 30px; margin-bottom: 4px; opacity: .6; }
      .codex-plus-task-empty strong { color: var(--codex-plus-text-secondary); font-size: 13px; font-weight: 550; }
      /* 中栏：任务列表。 */
      .codex-plus-task-list-pane { display: flex; flex: 1; min-width: 280px; min-height: 0; flex-direction: column; background: var(--codex-plus-task-page-bg); }
      .codex-plus-task-list-pane .codex-plus-remote-banner { margin: 8px 12px 0; }
      .codex-plus-task-main { display: flex; flex: 1; min-height: 0; flex-direction: column; overflow-y: auto; padding: 8px 8px 16px; scrollbar-width: thin; }
      .codex-plus-task-quick-add { display: flex; flex: none; align-items: center; gap: 10px; height: 34px; box-sizing: border-box; margin-bottom: 4px; padding: 0 10px; border: 1px solid transparent; border-radius: 8px; color: var(--codex-plus-text-tertiary); cursor: text; transition: background-color .12s, border-color .12s; }
      .codex-plus-task-quick-add:hover { background: var(--codex-plus-bg-hover); }
      .codex-plus-task-quick-add:focus-within { border-color: var(--codex-plus-task-accent); background: transparent; }
      .codex-plus-task-quick-add svg { width: 14px; height: 14px; }
      .codex-plus-task-quick-add input { flex: 1; min-width: 0; padding: 0; border: 0; outline: 0; background: transparent; color: var(--codex-plus-text); font: inherit; }
      .codex-plus-task-group { display: flex; flex: none; flex-direction: column; gap: 1px; }
      .codex-plus-task-group + .codex-plus-task-group { margin-top: 10px; }
      .codex-plus-task-group-head { position: sticky; top: -8px; z-index: 1; display: flex; align-items: center; gap: 8px; height: 30px; padding: 0 10px; background: var(--codex-plus-task-page-bg); color: var(--codex-plus-text-secondary); font-size: 12px; font-weight: 600; }
      .codex-plus-task-row { display: grid; flex: none; width: 100%; grid-template-columns: 12px minmax(0, 1fr) auto; align-items: center; gap: 3px 10px; padding: 8px 10px 9px; border: 0; border-radius: 8px; background: transparent; color: var(--codex-plus-text); font: inherit; text-align: left; cursor: default; }
      .codex-plus-task-row:hover { background: var(--codex-plus-bg-hover); }
      .codex-plus-task-row:focus-visible { outline: none; }
      .codex-plus-task-row[data-selected="true"] { background: var(--codex-plus-task-selected); }
      .codex-plus-task-row[data-dragging="true"] { opacity: .4; }
      .codex-plus-task-row-title { overflow: hidden; font-weight: 500; text-overflow: ellipsis; white-space: nowrap; }
      .codex-plus-task-row-time { color: var(--codex-plus-text-tertiary); font-size: 11.5px; font-variant-numeric: tabular-nums; white-space: nowrap; }
      .codex-plus-task-row-meta { display: flex; grid-column: 2 / 4; align-items: center; gap: 10px; min-width: 0; overflow: hidden; color: var(--codex-plus-text-tertiary); font-size: 11.5px; white-space: nowrap; }
      .codex-plus-task-row-excerpt { grid-column: 2 / 4; overflow: hidden; color: var(--codex-plus-text-tertiary); font-size: 12px; text-overflow: ellipsis; white-space: nowrap; }
      .codex-plus-task-row[data-status="done"] .codex-plus-task-row-title { color: var(--codex-plus-text-secondary); }
      .codex-plus-task-row[data-status="cancelled"] .codex-plus-task-row-title { color: var(--codex-plus-text-tertiary); text-decoration: line-through; }
      .codex-plus-task-meta-item { display: inline-flex; align-items: center; gap: 4px; min-width: 0; }
      .codex-plus-task-meta-item > span { overflow: hidden; text-overflow: ellipsis; }
      .codex-plus-task-meta-item svg { width: 12px; height: 12px; }
      .codex-plus-task-meta-item[data-complete="true"] { color: var(--codex-plus-success); }
      .codex-plus-task-meta-item[data-tone="running"] { color: var(--codex-plus-task-accent); }
      .codex-plus-task-meta-item i { width: 6px; height: 6px; border-radius: 50%; background: currentColor; animation: codex-plus-task-pulse 1.4s ease-in-out infinite; }
      @keyframes codex-plus-task-pulse { 50% { opacity: .3; } }
      .codex-plus-task-more { align-self: center; margin-top: 8px; }
      /* 右栏：详情。 */
      .codex-plus-task-detail { display: flex; flex: none; flex-direction: column; width: clamp(300px, 36%, 440px); min-height: 0; border-left: 1px solid var(--codex-plus-task-line); background: var(--codex-plus-task-page-bg); }
      .codex-plus-task-detail .codex-plus-task-pane-bar { gap: 2px; padding: 0 8px 0 18px; }
      .codex-plus-task-detail .codex-plus-task-pane-bar code { font-size: 12px; color: var(--codex-plus-text-secondary); }
      .codex-plus-task-detail-body { display: flex; flex: 1; min-height: 0; flex-direction: column; gap: 16px; overflow-y: auto; padding: 16px 18px 20px; scrollbar-width: thin; }
      .codex-plus-task-title-input { display: block; box-sizing: border-box; width: calc(100% + 12px); min-height: 34px; margin: 0 -6px -6px; padding: 4px 6px; border: 1px solid transparent; border-radius: 6px; background: transparent; color: var(--codex-plus-text); font: inherit; font-size: 18px; font-weight: 600; line-height: 1.4; overflow: hidden; resize: none; field-sizing: content; }
      .codex-plus-task-title-input:hover { border-color: var(--codex-plus-task-line); }
      .codex-plus-task-title-input:focus { border-color: var(--codex-plus-task-accent); outline: none; }
      .codex-plus-task-props { display: grid; grid-template-columns: 44px minmax(0, 1fr); align-items: center; gap: 4px 12px; }
      .codex-plus-task-prop-label { color: var(--codex-plus-text-tertiary); font-size: 12px; }
      .codex-plus-task-prop-value { position: relative; display: flex; align-items: center; gap: 8px; min-width: 0; min-height: 28px; }
      .codex-plus-task-prop-note { gap: 6px; color: var(--codex-plus-text-tertiary); font-size: 12px; white-space: nowrap; }
      .codex-plus-task-select { display: inline-flex; max-width: calc(100% + 8px); min-width: 0; align-items: center; gap: 7px; height: 28px; margin-left: -8px; padding: 0 8px; border: 0; border-radius: 6px; background: transparent; color: var(--codex-plus-text); font: inherit; font-size: 12.5px; cursor: pointer; }
      .codex-plus-task-select:is(:hover, [aria-expanded="true"]) { background: var(--codex-plus-bg-hover); }
      .codex-plus-task-select:focus-visible { outline: 2px solid var(--codex-plus-task-accent); outline-offset: 0; }
      .codex-plus-task-select svg { width: 13px; height: 13px; color: var(--codex-plus-text-tertiary); }
      .codex-plus-task-menu { position: absolute; top: calc(100% + 4px); left: -8px; z-index: 5; display: flex; flex-direction: column; width: 280px; padding: 4px; border: 1px solid var(--codex-plus-task-line-strong); border-radius: 9px; background: var(--codex-plus-bg-elevated); box-shadow: 0 12px 32px rgba(0,0,0,.28), 0 2px 6px rgba(0,0,0,.14); }
      .codex-plus-task-menu button { display: grid; grid-template-columns: 12px auto minmax(0, 1fr); align-items: center; gap: 8px; height: 30px; padding: 0 8px; border: 0; border-radius: 6px; background: transparent; color: var(--codex-plus-text); font: inherit; font-size: 12.5px; text-align: left; cursor: pointer; }
      .codex-plus-task-menu button:is(:hover, :focus-visible) { background: var(--codex-plus-bg-hover); outline: none; }
      .codex-plus-task-menu button[aria-checked="true"] { background: var(--codex-plus-task-selected); font-weight: 550; }
      .codex-plus-task-menu-hint { overflow: hidden; color: var(--codex-plus-text-tertiary); font-size: 11.5px; font-weight: 400; text-align: right; text-overflow: ellipsis; white-space: nowrap; }
      .codex-plus-task-menu svg { width: 13px; height: 13px; color: var(--codex-plus-text-secondary); }
      .codex-plus-task-project-menu { width: min(360px, calc(100vw - 80px)); max-height: 320px; overflow-y: auto; }
      .codex-plus-task-menu-separator { flex: none; height: 1px; margin: 4px 6px; background: var(--codex-plus-task-line); }
      .codex-plus-task-menu-empty { padding: 8px; color: var(--codex-plus-text-tertiary); font-size: 12px; }
      .codex-plus-task-select > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-plus-task-select > svg:first-child { color: var(--codex-plus-text-secondary); }
      .codex-plus-task-placeholder { color: var(--codex-plus-text-tertiary); }
      .codex-plus-task-primary { display: inline-flex; align-items: center; justify-content: center; gap: 6px; height: 32px; box-sizing: border-box; padding: 0 12px; border: 0; border-radius: 7px; background: var(--codex-plus-task-accent); color: #fff; font: inherit; font-size: 12.5px; font-weight: 550; white-space: nowrap; cursor: pointer; transition: filter .12s; }
      .codex-plus-task-primary:hover { filter: brightness(1.1); }
      .codex-plus-task-primary:focus-visible { outline: 2px solid var(--codex-plus-task-accent); outline-offset: 2px; }
      .codex-plus-task-primary:disabled { opacity: .5; cursor: default; }
      .codex-plus-task-primary svg { width: 11px; height: 11px; fill: currentColor; }
      .codex-plus-task-start { display: flex; flex-direction: column; gap: 6px; }
      .codex-plus-task-start-hint { color: var(--codex-plus-text-tertiary); font-size: 11.5px; line-height: 16px; }
      .codex-plus-task-section-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: -10px; padding-top: 14px; border-top: 1px solid var(--codex-plus-task-line); color: var(--codex-plus-text-secondary); font-size: 12px; font-weight: 550; }
      .codex-plus-task-section-head > :nth-child(2) { color: var(--codex-plus-text-tertiary); font-weight: 400; font-variant-numeric: tabular-nums; }
      .codex-plus-task-body-input { display: block; flex: none; box-sizing: border-box; width: calc(100% + 16px); min-height: 160px; margin: 0 -8px; padding: 8px; border: 1px solid transparent; border-radius: 7px; background: transparent; color: var(--codex-plus-text); font: inherit; font-size: 13px; line-height: 1.65; resize: none; field-sizing: content; }
      .codex-plus-task-body-input:hover { border-color: var(--codex-plus-task-line); }
      .codex-plus-task-body-input:focus { border-color: var(--codex-plus-task-accent); outline: none; }
      .codex-plus-task-runs { display: flex; flex: none; flex-direction: column; overflow: hidden; border: 1px solid var(--codex-plus-task-line); border-radius: 8px; }
      .codex-plus-task-run { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 4px 8px; padding: 8px 10px; border: 0; border-bottom: 1px solid var(--codex-plus-task-line); background: transparent; color: inherit; font: inherit; text-align: left; cursor: pointer; }
      .codex-plus-task-run:last-child { border-bottom: 0; }
      .codex-plus-task-run:hover { background: var(--codex-plus-bg-hover); }
      .codex-plus-task-run-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-plus-task-run-time { color: var(--codex-plus-text-tertiary); font-size: 11.5px; white-space: nowrap; }
      .codex-plus-task-run-last { display: -webkit-box; grid-column: 1 / -1; overflow: hidden; color: var(--codex-plus-text-secondary); font-size: 12px; line-height: 1.45; overflow-wrap: anywhere; -webkit-box-orient: vertical; -webkit-line-clamp: 2; }
      .codex-plus-task-run-badge { display: inline-flex; flex: none; align-items: center; height: 20px; box-sizing: border-box; padding: 0 7px; border-radius: 5px; background: var(--codex-plus-bg-hover); color: var(--codex-plus-text-secondary); font-size: 11px; white-space: nowrap; }
      .codex-plus-task-run-badge[data-tone="running"] { background: color-mix(in srgb, var(--codex-plus-task-accent) 15%, transparent); color: var(--codex-plus-task-accent); }
      .codex-plus-task-run-badge[data-tone="done"] { background: color-mix(in srgb, var(--codex-plus-success) 13%, transparent); color: var(--codex-plus-success); }
      .codex-plus-task-run-badge[data-tone="warn"] { background: color-mix(in srgb, var(--codex-plus-warning) 13%, transparent); color: var(--codex-plus-warning); }
      /* 底部状态栏横跨三栏。 */
      .codex-plus-task-status { display: flex; flex: none; align-items: center; gap: 16px; height: 26px; padding: 0 14px; border-top: 1px solid var(--codex-plus-task-line); background: var(--codex-plus-nav-bg, var(--codex-plus-task-sidebar-bg)); color: var(--codex-plus-text-tertiary); font-size: 11.5px; font-variant-numeric: tabular-nums; }
      .codex-plus-task-status [data-tone="warn"] { color: var(--codex-plus-warning); }
      .codex-plus-task-status [data-tone="danger"] { color: var(--codex-plus-danger); }
      .codex-plus-task-dir { min-width: 0; margin-left: auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      @container codex-task-page (max-width: 820px) {
        .codex-plus-task-sidebar { width: 52px; padding: 14px 6px; }
        .codex-plus-task-sidebar :is(.codex-plus-task-sidebar-title, .codex-plus-task-sidebar-section, .codex-plus-task-sidebar-tip, .codex-plus-task-nav-label, .codex-plus-task-nav-count) { display: none; }
        .codex-plus-task-nav { justify-content: center; }
      }
      /* 拓展与推荐内容使用原生插件页面的导航、标题和轻量列表布局。 */
      .${codexPlusPageClass} .codex-plus-modal-content:is([data-codex-plus-active-tab="extensions"], [data-codex-plus-active-tab="sponsor"]) {
        background: var(--codex-plus-settings-page-bg, var(--codex-plus-bg-primary)) !important;
        container: codex-plus-discovery-page / inline-size;
      }
      .${codexPlusPageClass} .codex-plus-modal-content:is([data-codex-plus-active-tab="extensions"], [data-codex-plus-active-tab="sponsor"]) > .codex-plus-modal-header { display: none; }
      .${codexPlusPageClass} .codex-plus-modal-content:is([data-codex-plus-active-tab="extensions"], [data-codex-plus-active-tab="sponsor"]) .codex-plus-modal-body {
        box-sizing: border-box;
        padding: 32px 32px 56px;
      }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav {
        box-sizing: border-box;
        width: 312px;
        flex-basis: 312px;
        background: var(--codex-plus-nav-bg, var(--codex-plus-bg-secondary));
      }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav-item[data-active="true"] { background: var(--codex-plus-nav-selected-bg, var(--codex-plus-bg-selected)); }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav-header { padding: 16px; }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav-title { font-size: 18px; line-height: 24px; }
      .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav-body { padding: 0 12px 20px; }
      .codex-plus-extensions-nav-entry {
        min-height: 36px;
        gap: 10px;
        box-sizing: border-box;
        padding: 8px 12px;
        font-size: 14px;
        font-weight: 400;
        line-height: 20px;
      }
      .codex-plus-extensions-nav-entry .codex-plus-page-nav-item-icon,
      .codex-plus-extensions-nav-entry .codex-plus-extensions-icon {
        flex: 0 0 20px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 20px;
        height: 20px;
        padding: 0;
        background: transparent;
      }
      .codex-plus-extensions-nav-entry .codex-plus-page-nav-item-icon svg,
      .codex-plus-extensions-nav-entry .codex-plus-extensions-icon svg,
      .codex-plus-extensions-nav-entry .codex-plus-extensions-icon-img { width: 20px; height: 20px; }
      .codex-plus-extensions-nav-entry .codex-plus-extensions-icon-img { display: block; object-fit: contain; }
      .codex-plus-extensions-nav-entry .codex-plus-extensions-item-name { font-weight: 400; }
      .codex-plus-extensions-nav-entry + .codex-plus-page-nav-group { margin-top: 24px; }
      .codex-plus-extensions-nav-entry:hover .codex-plus-extensions-item-name { text-decoration: none; }
      .codex-plus-discovery {
        width: 100%;
        max-width: 900px;
        min-width: 0;
        margin: 0 auto;
        padding: 0;
        container: codex-plus-discovery / inline-size;
      }
      .codex-plus-discovery-header {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 24px;
      }
      .codex-plus-discovery-header > :first-child { flex: 1 1 auto; min-width: 0; }
      .codex-plus-discovery-title {
        margin: 0;
        color: var(--codex-plus-text);
        font-size: 26px;
        font-weight: 600;
        line-height: 34px;
        letter-spacing: -.02em;
        overflow-wrap: anywhere;
      }
      .codex-plus-discovery-description {
        margin: 12px 0 0;
        color: var(--codex-plus-text-secondary);
        font-size: 14px;
        line-height: 20px;
        overflow-wrap: anywhere;
      }
      .codex-plus-discovery-description strong { color: var(--codex-plus-text); font-weight: 650; }
      .codex-plus-discovery-toolbar { display: flex; flex: 0 1 auto; align-items: center; gap: 8px; min-width: 0; }
      .codex-plus-discovery-search {
        display: flex;
        flex: 0 1 240px;
        align-items: center;
        gap: 8px;
        width: 240px;
        max-width: 100%;
        min-width: 0;
        box-sizing: border-box;
        border: 1px solid var(--codex-plus-border);
        border-radius: 999px;
        padding: 8px 12px;
        background: transparent;
        color: var(--codex-plus-text-secondary);
      }
      .codex-plus-discovery-search > svg { flex: 0 0 16px; width: 16px; height: 16px; }
      .codex-plus-discovery-search input {
        width: 100%;
        min-width: 0;
        border: 0;
        padding: 0;
        outline: none;
        background: transparent;
        color: var(--codex-plus-text);
        font: inherit;
        font-size: 13px;
        line-height: 18px;
      }
      .codex-plus-discovery-search input::placeholder { color: var(--codex-plus-text-tertiary); }
      .codex-plus-discovery-search:focus-within {
        outline: 2px solid var(--codex-plus-switch-on, #3485ff);
        outline-offset: 2px;
      }
      .codex-plus-discovery-icon-button {
        display: inline-flex;
        flex: 0 0 32px;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        box-sizing: border-box;
        border: 0;
        border-radius: 8px;
        padding: 0;
        background: transparent;
        color: var(--codex-plus-text-secondary);
        cursor: pointer;
      }
      .codex-plus-discovery-icon-button svg { width: 16px; height: 16px; }
      .codex-plus-discovery-icon-button:hover { background: var(--codex-plus-bg-hover); color: var(--codex-plus-text); }
      .codex-plus-discovery-icon-button:disabled { opacity: .55; cursor: not-allowed; }
      .codex-plus-extensions-filter { display: flex; align-items: center; gap: 8px; margin-top: 16px; }
      .codex-plus-extensions-filter button {
        border: 0;
        border-radius: 999px;
        padding: 8px 16px;
        background: transparent;
        color: var(--codex-plus-text-secondary);
        font: inherit;
        font-size: 13px;
        line-height: 20px;
        cursor: pointer;
      }
      .codex-plus-extensions-filter button[data-active="true"] { background: var(--codex-plus-bg-selected); color: var(--codex-plus-text); }
      .codex-plus-extensions-filter button:hover { background: var(--codex-plus-bg-hover); }
      /* 分栏条（精选 / 社区 / 本地 / 我发布的 / dsh 插件）与右侧排序，对齐管理器「拓展」页。 */
      .codex-plus-extensions-filterbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
      .codex-plus-extensions-filterbar .codex-plus-extensions-filter { flex-wrap: wrap; }
      .codex-plus-extensions-filter-count, .codex-plus-discovery-section-count { color: var(--codex-plus-text-secondary); font-weight: 400; }
      /* 和 Codex 原生下拉一致的胶囊按钮 + 菜单（实测 Codex 的「更改权限」「推理强度」菜单）：
         按钮 28px 高、13px 次要色、悬停 / 展开淡底；菜单半透明浮层 + 8px 模糊、20px 圆角、4px 内边距；
         选项 5px 8px、15px 圆角、悬停用列表悬停色，选中项右侧对勾。颜色都取 Codex 自己的变量，浅色主题跟着变。 */
      .codex-plus-extensions-sort { position: relative; margin-top: 16px; }
      .codex-plus-menu-trigger {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        height: 28px;
        border: 1px solid transparent;
        border-radius: 9999px;
        padding: 0 8px 0 10px;
        background: transparent;
        color: var(--color-text-tertiary, var(--codex-plus-text-secondary));
        font: inherit;
        font-size: 13px;
        line-height: 18px;
        white-space: nowrap;
        cursor: pointer;
      }
      .codex-plus-menu-trigger:is(:hover, [aria-expanded="true"]) { background: var(--color-token-list-hover-background, var(--codex-plus-bg-hover)); color: var(--codex-plus-text); }
      .codex-plus-menu-trigger:focus-visible { outline: 2px solid var(--codex-plus-switch-on, #3485ff); outline-offset: 0; }
      .codex-plus-menu-trigger svg { flex: 0 0 auto; opacity: .75; }
      .codex-plus-menu-popover {
        position: absolute;
        z-index: 50;
        display: flex;
        flex-direction: column;
        min-width: 160px;
        box-sizing: border-box;
        border-radius: 20px;
        padding: 4px;
        background: color-mix(in srgb, var(--color-surface-elevated-secondary, var(--codex-plus-bg-elevated)) 90%, transparent);
        color: var(--codex-plus-text);
        box-shadow: var(--shadow-dropdown-overlay, 0 0 0 .5px var(--codex-plus-border), 0 8px 16px -4px rgba(0, 0, 0, .12));
        backdrop-filter: blur(8px);
        -webkit-backdrop-filter: blur(8px);
      }
      .codex-plus-extensions-sort-menu { top: calc(100% + 4px); right: 0; }
      .codex-plus-menu-item {
        display: flex;
        align-items: center;
        gap: 6px;
        width: 100%;
        border: 0;
        border-radius: 15px;
        padding: 5px 8px;
        background: transparent;
        color: var(--codex-plus-text);
        font: inherit;
        font-size: 13px;
        line-height: 18.5px;
        text-align: left;
        white-space: nowrap;
        cursor: pointer;
      }
      .codex-plus-menu-item:is(:hover, :focus-visible) { background: var(--color-token-list-hover-background, var(--codex-plus-bg-hover)); outline: none; }
      .codex-plus-menu-item-label { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
      .codex-plus-menu-item svg { flex: 0 0 auto; opacity: .75; }
      .codex-plus-menu-item:is(:hover, :focus-visible) svg { opacity: 1; }
      /* 原生下拉的弹出层不跟主题：深色下选项是浅色字配白底，没选中的项几乎看不见。
         选项显式用 Codex 下拉菜单的底色与正文色（排序、供应商页、拓展设置表单的下拉都适用）。 */
      :is(.${codexPlusPageClass}, .codex-plus-modal-overlay) select option {
        background-color: var(--codex-plus-bg-elevated);
        color: var(--codex-plus-text);
      }
      .codex-plus-extension-badge {
        display: inline-flex;
        flex: 0 0 auto;
        align-items: center;
        gap: 3px;
        border-radius: 4px;
        padding: 0 5px;
        background: var(--codex-plus-bg-hover);
        color: var(--codex-plus-text-secondary);
        font-size: 11px;
        font-weight: 500;
        line-height: 16px;
        white-space: nowrap;
      }
      .codex-plus-extension-badge svg { width: 11px; height: 11px; }
      .codex-plus-extension-badge[data-tone="ok"] { background: color-mix(in srgb, #22c55e 14%, transparent); color: #22c55e; }
      .codex-plus-extension-badge[data-tone="warn"] { background: color-mix(in srgb, #f59e0b 16%, transparent); color: #d97706; }
      .codex-plus-extension-badge[data-tone="danger"] { background: color-mix(in srgb, #ef4444 14%, transparent); color: #ef4444; }
      .codex-plus-extension-card[data-dimmed="true"] { opacity: .6; }
      a.codex-plus-extension-card-select, span.codex-plus-extension-card-select { text-decoration: none; }
      span.codex-plus-extension-card-select { cursor: default; }
      .codex-plus-extensions-notice { margin-top: 12px; border-radius: 8px; padding: 8px 12px; background: var(--codex-plus-bg-hover); color: var(--codex-plus-text-secondary); font-size: 13px; line-height: 18px; white-space: pre-wrap; overflow-wrap: anywhere; }
      .codex-plus-extensions-notice[data-tone="error"] { background: color-mix(in srgb, #ef4444 12%, transparent); color: #ef4444; }
      .codex-plus-extensions-banner { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 20px; border: 1px solid color-mix(in srgb, #f59e0b 40%, transparent); border-radius: 10px; padding: 10px 14px; background: color-mix(in srgb, #f59e0b 8%, transparent); }
      .codex-plus-extensions-banner-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; color: var(--codex-plus-text-secondary); font-size: 13px; line-height: 18px; }
      .codex-plus-extensions-banner-text strong { color: var(--codex-plus-text); font-weight: 500; }
      .codex-plus-extensions-banner-switch { display: inline-flex; flex: 0 0 auto; align-items: center; gap: 8px; color: var(--codex-plus-text-secondary); font-size: 13px; }
      .codex-plus-extensions-pane-meta { margin-top: 12px; color: var(--codex-plus-text-secondary); font-size: 12px; line-height: 16px; }
      .codex-plus-discovery-section-title .codex-plus-extensions-pane-meta { display: inline; margin: 0 0 0 8px; font-weight: 400; }
      .codex-plus-extensions-text-button { border: 0; border-radius: 6px; padding: 4px 8px; background: transparent; color: var(--codex-plus-text-secondary); font: inherit; font-size: 13px; cursor: pointer; white-space: nowrap; }
      .codex-plus-extensions-text-button:hover:not(:disabled) { background: var(--codex-plus-bg-hover); color: var(--codex-plus-text); }
      .codex-plus-extensions-text-button[data-tone="danger"] { color: #ef4444; }
      .codex-plus-extensions-text-button:disabled { cursor: default; opacity: .5; }
      /* dsh 插件：一列，每个插件一行，点开看各行明细。 */
      .codex-plus-dsh-list { display: flex; flex-direction: column; gap: 4px; }
      .codex-plus-extension-card-icon[data-tone="ok"] { color: #22c55e; }
      .codex-plus-dsh-chevron { display: inline-flex; flex: 0 0 auto; color: var(--codex-plus-text-secondary); transition: transform .15s; }
      .codex-plus-dsh-chevron svg { width: 16px; height: 16px; }
      .codex-plus-dsh-chevron[data-open="true"] { transform: rotate(180deg); }
      .codex-plus-dsh-detail { display: flex; flex-direction: column; gap: 8px; margin: 0 0 12px 48px; border-left: 2px solid var(--codex-plus-border); padding: 4px 0 4px 14px; font-size: 13px; line-height: 18px; }
      .codex-plus-dsh-source { color: var(--codex-plus-text-secondary); font-size: 12px; overflow-wrap: anywhere; }
      .codex-plus-dsh-actions { display: flex; flex-wrap: wrap; gap: 4px; margin-left: -8px; }
      .codex-plus-dsh-row { display: flex; flex-direction: column; gap: 6px; border-radius: 8px; padding: 8px 10px; background: var(--codex-plus-bg-hover); }
      .codex-plus-dsh-row-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
      .codex-plus-dsh-row-head code { color: var(--codex-plus-text-secondary); font-size: 12px; }
      .codex-plus-dsh-row-actions { display: inline-flex; gap: 2px; margin-left: auto; }
      .codex-plus-dsh-status { border-radius: 4px; padding: 0 5px; background: var(--codex-plus-bg-selected); color: var(--codex-plus-text-secondary); font-size: 11px; line-height: 16px; }
      .codex-plus-dsh-status[data-status="compatible"] { color: #22c55e; }
      .codex-plus-dsh-status[data-status="partial"] { color: #d97706; }
      .codex-plus-dsh-status[data-status="incompatible"], .codex-plus-dsh-status[data-status="error"] { color: #ef4444; }
      .codex-plus-dsh-mappings { display: flex; flex-wrap: wrap; gap: 4px; }
      .codex-plus-dsh-mappings span { border-radius: 4px; padding: 0 6px; background: var(--codex-plus-bg-selected); color: var(--codex-plus-text-secondary); font-size: 12px; }
      .codex-plus-dsh-tools { display: flex; flex-direction: column; gap: 4px; margin: 0; padding: 0; list-style: none; }
      .codex-plus-dsh-tools li { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
      .codex-plus-dsh-tools li span { flex: 1 1 auto; min-width: 0; color: var(--codex-plus-text-secondary); overflow-wrap: anywhere; }
      .codex-plus-dsh-reason, .codex-plus-dsh-line, .codex-plus-dsh-companion { color: var(--codex-plus-text-secondary); overflow-wrap: anywhere; }
      .codex-plus-dsh-warning { color: #d97706; overflow-wrap: anywhere; }
      .codex-plus-dsh-config { display: flex; flex-direction: column; gap: 6px; }
      .codex-plus-dsh-config textarea { box-sizing: border-box; width: 100%; border: 1px solid var(--codex-plus-border); border-radius: 8px; padding: 8px; background: var(--codex-plus-bg-primary, transparent); color: var(--codex-plus-text); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
      .codex-plus-dsh-config small { color: var(--codex-plus-text-secondary); font-size: 12px; }
      .codex-plus-dsh-config-actions { display: flex; gap: 8px; }
      .codex-plus-dsh-import { display: flex; gap: 8px; margin-top: 8px; }
      .codex-plus-dsh-import input { flex: 1 1 auto; min-width: 0; border: 1px solid var(--codex-plus-border); border-radius: 8px; padding: 6px 10px; background: transparent; color: var(--codex-plus-text); font: inherit; font-size: 13px; }
      .codex-plus-dsh-index { margin-top: 12px; color: var(--codex-plus-text-secondary); font-size: 13px; }
      .codex-plus-dsh-index summary { cursor: pointer; }
      .codex-plus-dsh-market { margin-top: 8px; }
      .codex-plus-discovery-section-title {
        margin: 32px 0 16px;
        color: var(--codex-plus-text);
        font-size: 16px;
        font-weight: 550;
        line-height: 24px;
      }
      .codex-plus-extensions-grid,
      .codex-plus-discovery .codex-plus-ad-list { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px 32px; }
      .codex-plus-extensions-grid > .codex-plus-extensions-detail-empty,
      .codex-plus-discovery .codex-plus-ad-empty { grid-column: 1 / -1; }
      .codex-plus-extension-card {
        display: flex;
        align-items: center;
        gap: 16px;
        min-width: 0;
        min-height: 56px;
        box-sizing: border-box;
        border: 0;
        padding: 8px 0;
        background: transparent;
      }
      .codex-plus-extension-card-select {
        display: flex;
        flex: 1 1 auto;
        align-items: center;
        gap: 12px;
        min-width: 0;
        border: 0;
        border-radius: 8px;
        padding: 0;
        background: transparent;
        color: var(--codex-plus-text);
        font: inherit;
        text-align: left;
        cursor: pointer;
      }
      .codex-plus-extension-card-icon {
        display: inline-flex;
        flex: 0 0 36px;
        align-items: center;
        justify-content: center;
        width: 36px;
        height: 36px;
        box-sizing: border-box;
        border: 1px solid var(--codex-plus-border);
        border-radius: 10px;
        color: var(--codex-plus-text-secondary);
      }
      .codex-plus-extension-card-icon svg { width: 24px; height: 24px; }
      .codex-plus-extension-card-icon img { width: 32px; height: 32px; object-fit: contain; border-radius: 8px; }
      .codex-plus-extension-card-copy { display: flex; flex: 1 1 auto; flex-direction: column; gap: 2px; min-width: 0; }
      .codex-plus-extension-card-title {
        overflow: hidden;
        color: var(--codex-plus-text);
        font-size: 14px;
        font-weight: 500;
        line-height: 20px;
        white-space: nowrap;
        text-overflow: ellipsis;
      }
      .codex-plus-extension-card-heading { display: flex; align-items: center; gap: 6px; min-width: 0; }
      .codex-plus-extension-card-heading .codex-plus-extension-card-title { min-width: 0; }
      /* 「官方」徽标：与管理器精选栏的绿色小标签同义。 */
      .codex-plus-extension-official {
        display: inline-flex;
        flex: 0 0 auto;
        align-items: center;
        border-radius: 4px;
        padding: 0 5px;
        background: color-mix(in srgb, #22c55e 14%, transparent);
        color: #22c55e;
        font-size: 11px;
        font-weight: 500;
        line-height: 16px;
        vertical-align: middle;
      }
      .codex-plus-extensions-detail-title .codex-plus-extension-official { margin-left: 8px; font-size: 12px; line-height: 18px; }
      .codex-plus-discovery-icon-button[data-busy="true"] svg { animation: codex-plus-ext-install-spin 1s linear infinite; }
      @keyframes codex-plus-ext-install-spin { to { transform: rotate(360deg); } }
      .codex-plus-discovery-icon-button:disabled { cursor: default; opacity: .5; }
      .codex-plus-extension-card-description {
        display: block;
        overflow: hidden;
        color: var(--codex-plus-text-secondary);
        font-size: 13px;
        line-height: 18px;
        white-space: nowrap;
        text-overflow: ellipsis;
      }
      .codex-plus-extension-card-actions { display: inline-flex; flex: 0 0 auto; align-items: center; gap: 4px; }
      .codex-plus-discovery .codex-plus-extensions-detail { max-width: 900px; margin: 0 auto; padding: 0 0 24px; }
      .codex-plus-discovery .codex-plus-extensions-detail-head { align-items: center; border-bottom: 0; gap: 20px; padding-bottom: 20px; }
      .codex-plus-discovery .codex-plus-extensions-detail-title { font-size: 26px; line-height: 34px; overflow-wrap: anywhere; }
      .codex-plus-discovery .codex-plus-extensions-detail-icon img { width: 40px; height: 40px; object-fit: contain; border-radius: 8px; }
      .codex-plus-discovery .codex-plus-extensions-detail-description { font-size: 14px; line-height: 20px; overflow-wrap: anywhere; }
      .codex-plus-discovery .codex-plus-extensions-detail-actions { flex-wrap: wrap; }
      .codex-plus-discovery .codex-plus-extensions-detail-button { min-height: 32px; box-sizing: border-box; }
      .codex-plus-discovery .codex-plus-extensions-detail-primary {
        border-color: var(--codex-plus-text);
        background: var(--codex-plus-text);
        color: var(--codex-plus-settings-page-bg, var(--codex-plus-bg-primary));
      }
      .codex-plus-discovery .codex-plus-extensions-detail-primary:hover {
        background: color-mix(in srgb, var(--codex-plus-text) 90%, transparent);
        color: var(--codex-plus-settings-page-bg, var(--codex-plus-bg-primary));
      }
      .codex-plus-discovery .codex-plus-extensions-detail-link a { color: var(--color-text-link, var(--color-token-text-link, var(--codex-plus-switch-on, #3485ff))); }
      /* 详情页签与两栏（对齐 VS Code 扩展详情）：左边细节 / 设置 / 功能，右边安装与市场信息。 */
      .codex-plus-discovery .codex-plus-extensions-detail { max-width: 1120px; }
      .codex-plus-discovery .codex-plus-extensions-detail-heading .codex-plus-extensions-detail-description { margin-top: 6px; }
      .codex-plus-extensions-detail-gear { display: inline-flex; align-items: center; justify-content: center; width: 32px; padding: 0; }
      .codex-plus-extensions-detail-gear svg { width: 16px; height: 16px; }
      .codex-plus-extensions-detail-tabs { display: flex; gap: 20px; border-bottom: 1px solid var(--codex-plus-border-subtle); }
      .codex-plus-extensions-detail-tabs button {
        margin-bottom: -1px; padding: 8px 2px; border: 0; border-bottom: 2px solid transparent; background: transparent;
        color: var(--codex-plus-text-secondary); font: inherit; font-size: 13px; line-height: 20px; cursor: pointer;
      }
      .codex-plus-extensions-detail-tabs button:hover { color: var(--codex-plus-text); }
      .codex-plus-extensions-detail-tabs button[aria-selected="true"] { border-bottom-color: var(--codex-plus-switch-on, #3485ff); color: var(--codex-plus-text); }
      .codex-plus-extensions-detail-columns { display: grid; grid-template-columns: minmax(0, 1fr) 240px; gap: 32px; padding-top: 20px; }
      .codex-plus-extensions-detail-main { min-width: 0; }
      .codex-plus-extensions-detail-tabpanel[hidden] { display: none !important; }
      .codex-plus-extensions-readme-hint { color: var(--codex-plus-text-tertiary); font-size: 13px; line-height: 20px; }
      .codex-plus-extensions-detail-aside { min-width: 0; font-size: 12px; line-height: 18px; color: var(--codex-plus-text-secondary); }
      .codex-plus-extensions-detail-aside section { padding-bottom: 14px; margin-bottom: 14px; border-bottom: 1px solid var(--codex-plus-border-subtle); }
      .codex-plus-extensions-detail-aside section:last-child { border-bottom: 0; }
      .codex-plus-extensions-detail-aside h3 { margin: 0 0 8px; color: var(--codex-plus-text); font-size: 13px; font-weight: 600; }
      .codex-plus-extensions-detail-aside dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 4px 12px; margin: 0; }
      .codex-plus-extensions-detail-aside dt { color: var(--codex-plus-text-tertiary); }
      .codex-plus-extensions-detail-aside dd { margin: 0; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
      .codex-plus-extensions-aside-links { margin: 0; padding: 0; list-style: none; display: grid; gap: 4px; }
      .codex-plus-extensions-detail-aside a { color: var(--color-text-link, var(--color-token-text-link, var(--codex-plus-switch-on, #3485ff))); text-decoration: none; }
      .codex-plus-extensions-detail-aside a:hover { text-decoration: underline; }
      .codex-plus-extensions-detail-aside .codex-plus-extensions-detail-tags { margin-top: 0; }
      .codex-plus-extensions-features { width: 100%; border-collapse: collapse; font-size: 13px; line-height: 20px; }
      .codex-plus-extensions-features th, .codex-plus-extensions-features td { padding: 8px 0; border-bottom: 1px solid var(--codex-plus-border-subtle); text-align: left; vertical-align: top; }
      .codex-plus-extensions-features th { width: 160px; padding-right: 16px; color: var(--codex-plus-text-secondary); font-weight: 500; }
      .codex-plus-extensions-features td { color: var(--codex-plus-text); overflow-wrap: anywhere; }
      /* 通用设置表单（按清单 config 生成）。 */
      .codex-plus-ext-config { display: grid; gap: 2px; }
      .codex-plus-extensions-detail-settings-hint + .codex-plus-ext-config { margin-top: 12px; }
      .codex-plus-ext-config-row { display: grid; grid-template-columns: minmax(0, 1fr) minmax(160px, 260px); align-items: center; gap: 8px 24px; padding: 10px 0; border-bottom: 1px solid var(--codex-plus-border-subtle); }
      .codex-plus-ext-config-row[data-wide="true"] { grid-template-columns: minmax(0, 1fr); }
      .codex-plus-ext-config-label label { color: var(--codex-plus-text); font-size: 13px; font-weight: 500; line-height: 20px; }
      .codex-plus-ext-config-description { color: var(--codex-plus-text-tertiary); font-size: 12px; line-height: 18px; }
      .codex-plus-ext-config-control { display: flex; justify-content: flex-end; min-width: 0; }
      .codex-plus-ext-config-row[data-wide="true"] .codex-plus-ext-config-control { justify-content: stretch; }
      .codex-plus-ext-config-input {
        width: 100%; box-sizing: border-box; min-height: 30px; padding: 5px 8px; border: 1px solid var(--codex-plus-border); border-radius: 6px;
        background: var(--codex-plus-bg-secondary, transparent); color: var(--codex-plus-text); font: inherit; font-size: 13px; line-height: 18px;
      }
      textarea.codex-plus-ext-config-input { resize: vertical; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
      .codex-plus-ext-config-input:focus { outline: 2px solid var(--codex-plus-switch-on, #3485ff); outline-offset: -1px; }
      .codex-plus-ext-config-status { min-height: 18px; padding-top: 8px; color: var(--codex-plus-text-tertiary); font-size: 12px; }
      .codex-plus-ext-config-status[data-kind="ok"] { color: var(--codex-plus-success); }
      .codex-plus-ext-config-status[data-kind="error"] { color: var(--codex-plus-danger); }
      /* README 的 Markdown 正文。 */
      .codex-plus-markdown { color: var(--codex-plus-text); font-size: 14px; line-height: 1.65; overflow-wrap: anywhere; }
      .codex-plus-markdown > :first-child { margin-top: 0; }
      .codex-plus-markdown h1, .codex-plus-markdown h2, .codex-plus-markdown h3, .codex-plus-markdown h4, .codex-plus-markdown h5, .codex-plus-markdown h6 { margin: 1.4em 0 .6em; font-weight: 600; line-height: 1.3; }
      .codex-plus-markdown h1 { font-size: 24px; padding-bottom: .3em; border-bottom: 1px solid var(--codex-plus-border-subtle); }
      .codex-plus-markdown h2 { font-size: 19px; padding-bottom: .3em; border-bottom: 1px solid var(--codex-plus-border-subtle); }
      .codex-plus-markdown h3 { font-size: 16px; }
      .codex-plus-markdown h4, .codex-plus-markdown h5, .codex-plus-markdown h6 { font-size: 14px; }
      .codex-plus-markdown p, .codex-plus-markdown ul, .codex-plus-markdown ol, .codex-plus-markdown blockquote, .codex-plus-markdown pre, .codex-plus-markdown .codex-plus-markdown-table { margin: 0 0 12px; }
      /* Codex 自己的样式把列表符号清掉了，这里补回。 */
      .codex-plus-markdown ul, .codex-plus-markdown ol { padding-left: 22px; }
      .codex-plus-markdown ul { list-style: disc; }
      .codex-plus-markdown ul ul { list-style: circle; }
      .codex-plus-markdown ol { list-style: decimal; }
      .codex-plus-markdown li { display: list-item; }
      .codex-plus-markdown li + li { margin-top: 2px; }
      .codex-plus-markdown li > ul, .codex-plus-markdown li > ol { margin: 2px 0 0; }
      .codex-plus-markdown li.codex-plus-markdown-task { list-style: none; margin-left: -20px; }
      .codex-plus-markdown li.codex-plus-markdown-task input { margin: 0 6px 0 0; vertical-align: -1px; }
      .codex-plus-markdown a { color: var(--color-text-link, var(--color-token-text-link, var(--codex-plus-switch-on, #3485ff))); text-decoration: none; }
      .codex-plus-markdown a:hover { text-decoration: underline; }
      .codex-plus-markdown code { padding: 1px 5px; border-radius: 4px; background: var(--codex-plus-bg-hover); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .88em; }
      .codex-plus-markdown pre { overflow: auto; padding: 12px 14px; border-radius: 8px; background: var(--codex-plus-bg-hover); line-height: 1.5; }
      .codex-plus-markdown pre code { padding: 0; background: transparent; font-size: 12px; white-space: pre; }
      .codex-plus-markdown blockquote { padding: 0 0 0 12px; border-left: 3px solid var(--codex-plus-border); color: var(--codex-plus-text-secondary); }
      .codex-plus-markdown hr { height: 1px; margin: 20px 0; border: 0; background: var(--codex-plus-border-subtle); }
      .codex-plus-markdown img { max-width: 100%; vertical-align: middle; }
      .codex-plus-markdown .codex-plus-markdown-table { overflow-x: auto; }
      .codex-plus-markdown table { border-collapse: collapse; font-size: 13px; }
      .codex-plus-markdown th, .codex-plus-markdown td { padding: 6px 12px; border: 1px solid var(--codex-plus-border-subtle); }
      .codex-plus-markdown th { background: var(--codex-plus-bg-hover); font-weight: 600; }
      .codex-plus-discovery .codex-plus-toggle { flex: 0 0 32px; width: 32px; height: 20px; min-height: 20px; box-sizing: border-box; padding: 2px; background: var(--codex-plus-switch-off, #3a3a3a); cursor: pointer; }
      .codex-plus-discovery .codex-plus-toggle span { width: 16px; height: 16px; background: var(--codex-plus-switch-thumb, #fff); }
      .codex-plus-discovery .codex-plus-toggle[data-enabled="true"] { background: var(--codex-plus-switch-on, #3485ff); }
      .codex-plus-discovery .codex-plus-toggle[data-enabled="true"] span { transform: translateX(12px); }
      .codex-plus-discovery .codex-plus-toggle[data-pending="true"],
      .codex-plus-discovery .codex-plus-toggle:disabled { cursor: not-allowed; opacity: .55; }
      .codex-plus-extensions-back {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        margin: 0 0 24px;
        border: 0;
        border-radius: 8px;
        padding: 4px 0;
        background: transparent;
        color: var(--codex-plus-text-secondary);
        font: inherit;
        font-size: 13px;
        line-height: 20px;
        cursor: pointer;
      }
      .codex-plus-extensions-back svg { width: 16px; height: 16px; }
      .codex-plus-extensions-back:hover { color: var(--codex-plus-text); }
      .codex-plus-discovery button:focus-visible,
      .codex-plus-discovery a:focus-visible,
      .codex-plus-extensions-nav-entry:focus-visible { outline: 2px solid var(--codex-plus-switch-on, #3485ff); outline-offset: 3px; }
      .codex-plus-discovery .codex-plus-ad-card {
        flex-direction: row;
        align-items: center;
        gap: 16px;
        min-height: 72px;
        min-width: 0;
        box-sizing: border-box;
        border: 0;
        border-radius: 8px;
        padding: 8px 0;
        background: transparent;
      }
      .codex-plus-discovery .codex-plus-ad-card:hover,
      .codex-plus-discovery .codex-plus-ad-card:focus-visible { background: transparent; }
      .codex-plus-discovery .codex-plus-ad-main { flex: 1 1 auto; align-items: center; gap: 12px; }
      .codex-plus-discovery .codex-plus-ad-icon { flex: 0 0 36px; border: 1px solid var(--codex-plus-border); border-radius: 10px; background: transparent; box-sizing: border-box; }
      .codex-plus-discovery .codex-plus-ad-text { gap: 4px; }
      .codex-plus-discovery .codex-plus-ad-title { font-size: 14px; font-weight: 500; line-height: 20px; }
      .codex-plus-discovery .codex-plus-ad-description { font-size: 13px; line-height: 18px; -webkit-line-clamp: 2; overflow-wrap: anywhere; }
      .codex-plus-discovery .codex-plus-ad-promo { border-radius: 0; padding: 0; background: transparent; color: var(--codex-plus-text-secondary); font-size: 12px; line-height: 16px; white-space: normal; overflow-wrap: anywhere; }
      .codex-plus-discovery .codex-plus-ad-arrow { display: inline-flex; align-items: center; justify-content: center; width: 32px; height: 32px; margin: 0; color: var(--codex-plus-text-secondary); }
      .codex-plus-discovery .codex-plus-ad-arrow svg { width: 16px; height: 16px; }
      @container codex-plus-discovery-page (max-width: 900px) {
        .codex-plus-discovery-header { flex-wrap: wrap; gap: 20px; }
        .codex-plus-discovery-toolbar { flex-basis: 100%; }
        .codex-plus-discovery .codex-plus-extensions-detail-head { flex-wrap: wrap; }
        .codex-plus-discovery .codex-plus-extensions-detail-actions { flex-basis: 100%; }
        .codex-plus-extensions-detail-columns { grid-template-columns: minmax(0, 1fr); }
      }
      @container codex-plus-discovery (max-width: 600px) {
        .codex-plus-extensions-grid,
        .codex-plus-discovery .codex-plus-ad-list { grid-template-columns: minmax(0, 1fr); }
      }
      @container codex-plus-discovery-page (max-width: 700px) {
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav { width: 180px; flex-basis: 180px; }
        .${codexPlusPageClass} .codex-plus-modal-content:is([data-codex-plus-active-tab="extensions"], [data-codex-plus-active-tab="sponsor"]) .codex-plus-modal-body { padding: 24px 20px 40px; }
      }
      @container codex-plus-discovery-page (max-width: 480px) {
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav { width: 56px; flex-basis: 56px; }
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav-header { display: none; }
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav-body { padding: 12px 8px; }
        .codex-plus-extensions-nav-entry { justify-content: center; padding: 8px; }
        .codex-plus-extensions-nav-entry .codex-plus-page-nav-item-text,
        .codex-plus-extensions-nav-entry .codex-plus-page-nav-item-name,
        .codex-plus-extensions-nav-entry .codex-plus-extensions-item-name,
        .codex-plus-extensions-nav-entry .codex-plus-page-nav-item-state,
        .codex-plus-extensions-nav-entry .codex-plus-extensions-item-state { display: none; }
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav-group-head,
        .${codexPlusPageClass} .codex-plus-modal-content[data-codex-plus-active-tab="extensions"] .codex-plus-page-nav-empty { display: none; }
        .${codexPlusPageClass} .codex-plus-modal-content:is([data-codex-plus-active-tab="extensions"], [data-codex-plus-active-tab="sponsor"]) .codex-plus-modal-body { padding: 24px 16px 40px; }
      }
    `;
      // 32 位 FNV-1a，只用来区分样式表内容。
      let hash = 0x811c9dc5;
      for (let index = 0; index < text.length; index += 1) {
        hash ^= text.charCodeAt(index);
        hash = Math.imul(hash, 0x01000193);
      }
      installStyle.sheet = { text, version: `${codexDeleteStyleVersion}-${(hash >>> 0).toString(36)}` };
    }
    const { text, version } = installStyle.sheet;
    const existingStyle = document.getElementById(styleId);
    if (existingStyle?.dataset.codexDeleteStyleVersion === version) return;
    existingStyle?.remove();
    const style = document.createElement("style");
    style.id = styleId;
    style.dataset.codexDeleteStyleVersion = version;
    style.textContent = text;
    document.documentElement.appendChild(style);
  }

  function syncCodexPlusPagePopupStyle(open) {
    const id = `${styleId}-page-popups`;
    const existing = document.getElementById(id);
    if (!codexPlusIsWindowsPlatform || !open) {
      existing?.remove();
      return;
    }
    if (existing) return;
    // Windows 仅在管理页打开时提升原生弹层的直接定位容器。即使根条件不命中，
    // 常驻的 :has() 仍会增加聊天 DOM 变化时的样式失效开销。
    const style = document.createElement("style");
    style.id = id;
    style.setAttribute("data-codex-plus-ext", "page-popup-style");
    style.textContent = `
      html[data-codex-plus-page-open] div:has(> [data-radix-popper-content-wrapper], > [role="tooltip"], > [role="menu"]):not(.${codexPlusPageClass} *) {
        z-index: 2147483645 !important;
      }
    `;
    document.documentElement.appendChild(style);
  }

  function defaultCodexPlusSettings() {
    // capacityAutoRetry* 必须写成字面量：多个测试（agent-pet、
    // backend-settings-sync…）把 defaultCodexPlusSettings 切片到不带 prelude 的沙箱里执行，
    // 引用 prelude 常量会直接 ReferenceError。与 00-prelude.js 里
    // capacityAutoRetryDefaultMaxAttempts / capacityAutoRetryDefaultIntervalSeconds 的一致性
    // 由 capacity-auto-retry.test.ts 的防漂移用例守住。
    return { sideChatSave: false, pluginMarketplaceUnlock: true, modelWhitelistUnlock: true, sessionDelete: true, sessionDeleteSkipConfirm: false, markdownExport: true, pasteFix: false, promptArgs: true, offlineAutoReview: false, statsigOverrides: false, reportedUsageCompaction: false, threadIdBadge: false, conversationView: false, conversationViewMaxWidth: conversationViewDefaultWidth, threadScrollRestore: true, historyTurnRecovery: false, nativeMenuPlacement: true, nativeFastUnlock: false, petRealMouseLook: false, activityOverlayRight: false, stepwise: false, answerOutline: false, turnRewind: false, capacityAutoRetry: false, capacityAutoRetryMaxAttempts: 15, capacityAutoRetryIntervalSeconds: 0, dreamSkinEnabled: false, dreamSkinPaused: false, dreamSkinThemeConfig: window.__CODEX_PLUS_DREAM_SKIN_THEME__ || {}, dreamSkinImagePath: "" };
  }

  const codexPlusBackendSettingMap = {
    pluginMarketplaceUnlock: "codexAppPluginMarketplaceUnlock",
    modelWhitelistUnlock: "codexAppModelWhitelistUnlock",
    sessionDelete: "codexAppSessionDelete",
    sessionDeleteSkipConfirm: "codexAppSessionDeleteSkipConfirm",
    sideChatSave: "codexAppSideChatSave",
    markdownExport: "codexAppMarkdownExport",
    threadIdBadge: "codexAppThreadIdBadge",
    conversationView: "codexAppConversationView",
    threadScrollRestore: "codexAppThreadScrollRestore",
    historyTurnRecovery: "codexAppHistoryTurnRecovery",
    nativeMenuPlacement: "codexAppNativeMenuPlacement",
    nativeFastUnlock: "codexAppNativeFastUnlock",
    petRealMouseLook: "codexAppPetRealMouseLook",
    activityOverlayRight: "codexAppActivityOverlayRight",
    stepwise: "codexAppStepwiseEnabled",
    answerOutline: "codexAppAnswerOutlineEnabled",
    capacityAutoRetry: "codexAppCapacityAutoRetry",
    turnRewind: "codexAppTurnRewind",
    pasteFix: "codexAppPasteFix",
    promptArgs: "codexAppPromptArgs",
    offlineAutoReview: "codexAppOfflineAutoReview",
    statsigOverrides: "codexAppStatsigOverrides",
    reportedUsageCompaction: "codexAppReportedUsageCompaction",
    dreamSkinEnabled: "codexAppDreamSkinEnabled",
    dreamSkinPaused: "codexAppDreamSkinPaused",
    dreamSkinThemeConfig: "codexAppDreamSkinThemeConfig",
    dreamSkinImagePath: "codexAppDreamSkinImagePath",
  };
  const codexPlusBackendMappedSettings = new Set(Object.keys(codexPlusBackendSettingMap));

  function backendCodexPlusSettings() {
    const settings = {};
    Object.entries(codexPlusBackendSettingMap).forEach(([localKey, backendKey]) => {
      const value = codexPlusBackendSettings[backendKey];
      if (typeof value === "boolean" || typeof value === "string" || (value && typeof value === "object" && !Array.isArray(value))) {
        settings[localKey] = value;
      }
    });
    return settings;
  }

  /**
   * 功能侧读到的设置：设置项开着、且对应的内置拓展在用（没停用、没被顶替）才算开。
   * 设置页的开关要显示 / 切换用户自己存的值，传 { ignoreBuiltins: true }。
   * 映射写在函数里：多个测试把本函数单独切片到沙箱里执行。
   */
  function codexPlusSettings(options) {
    const withBuiltins = (settings) => {
      const inactive = globalThis.__codexPlusBuiltinInactive;
      if (options?.ignoreBuiltins || !inactive?.size) return settings;
      const builtinSettings = {
        sessionDelete: "session-delete",
        markdownExport: "markdown-export",
        threadIdBadge: "thread-id-badge",
        threadScrollRestore: "thread-scroll-restore",
        conversationView: "conversation-view",
        activityOverlayRight: "activity-overlay-right",
        pasteFix: "paste-fix",
        promptArgs: "prompt-args",
        offlineAutoReview: "offline-auto-review",
        statsigOverrides: "statsig-overrides",
        reportedUsageCompaction: "reported-usage-compaction",
        stepwise: "stepwise",
        answerOutline: "answer-outline",
        petRealMouseLook: "pet-real-mouse",
      };
      Object.entries(builtinSettings).forEach(([key, id]) => {
        if (inactive.has(id)) settings[key] = false;
      });
      return settings;
    };
    if (codexPlusBackendSettings.enhancementsEnabled === false) {
      return {
        pluginMarketplaceUnlock: false,
        modelWhitelistUnlock: false,
        sessionDelete: false,
        sessionDeleteSkipConfirm: false,
        markdownExport: false,
        sideChatSave: false,
        pasteFix: false,
        promptArgs: false,
        offlineAutoReview: false,
        statsigOverrides: false,
        reportedUsageCompaction: false,
        threadIdBadge: false,
        conversationView: false,
        conversationViewMaxWidth: conversationViewDefaultWidth,
        threadScrollRestore: false,
        historyTurnRecovery: false,
        nativeMenuPlacement: false,
        nativeFastUnlock: false,
        petRealMouseLook: false,
        activityOverlayRight: false,
        stepwise: false,
        answerOutline: false,
        capacityAutoRetry: false,
        turnRewind: false,
        capacityAutoRetryMaxAttempts: 15,
        capacityAutoRetryIntervalSeconds: 0,
        dreamSkinEnabled: false,
        dreamSkinPaused: false,
        dreamSkinThemeConfig: window.__CODEX_PLUS_DREAM_SKIN_THEME__ || {},
        dreamSkinImagePath: "",
      };
    }
    try {
      const settings = { ...defaultCodexPlusSettings(), ...JSON.parse(localStorage.getItem(codexPlusSettingsKey) || "{}"), ...backendCodexPlusSettings() };
      delete settings.customLayout;
      delete settings.serviceTierControls;
      return withBuiltins(settings);
    } catch {
      const settings = { ...defaultCodexPlusSettings(), ...backendCodexPlusSettings() };
      return withBuiltins(settings);
    }
  }

  // Dream skin runtime is adapted from Fei-Away/Codex-Dream-Skin's renderer injection.
  function dreamSkinStylePreset(id, stylePreset) {
    const preset = String(stylePreset || "").trim();
    if (preset && preset !== "dream-original") return preset;
    return ({
      "caishen-lite": "caishen-lite",
      "caishen-max": "caishen-max",
      "caishen-readable": "caishen-readable",
      "export-night": "export-night",
      "global-founder-bright": "global-founder-bright",
      "mythic-guardian-noir": "mythic-guardian-noir",
      "codex-snow-skin": "codex-snow",
      "glass-vision": "glass-vision",
      "preset-midnight-aurora": "midnight-aurora",
      "preset-amber-dusk": "amber-dusk",
      "preset-forest-mist": "forest-mist",
      "preset-cyber-neon": "cyber-neon",
      "preset-sakura-dawn": "sakura-dawn",
    })[String(id || "").trim()] || "dream-original";
  }

  function dreamSkinThemeConfig(theme) {
    const fallback = window.__CODEX_PLUS_DREAM_SKIN_THEME__ || {};
    const value = theme && typeof theme === "object" ? theme : fallback;
    const colors = value.colors && typeof value.colors === "object" ? value.colors : fallback.colors || {};
    return {
      schemaVersion: value.schemaVersion === 1 ? 1 : 1,
      id: String(value.id || fallback.id || "custom"),
      name: String(value.name || fallback.name || "Dream Skin"),
      stylePreset: dreamSkinStylePreset(
        value.id || fallback.id,
        value.stylePreset || fallback.stylePreset,
      ),
      brandSubtitle: String(value.brandSubtitle || fallback.brandSubtitle || "CODEX DREAM SKIN"),
      statusText: String(value.statusText || fallback.statusText || "DREAM SKIN ONLINE"),
      quote: String(value.quote || fallback.quote || "MAKE SOMETHING WONDERFUL"),
      tagline: String(value.tagline || fallback.tagline || "把喜欢的画面变成可交互的 Codex 工作台。"),
      projectPrefix: String(value.projectPrefix || fallback.projectPrefix || "选择项目 · "),
      projectLabel: String(value.projectLabel || fallback.projectLabel || "◉  选择项目"),
      colors: { ...(fallback.colors || {}), ...colors },
    };
  }

  function dreamSkinCssString(value) {
    return JSON.stringify(String(value ?? ""));
  }

  function dreamSkinParseRgb(value) {
    if (!value || value === "transparent") return null;
    const text = String(value).trim();
    const hex = text.match(/^#([\da-f]{3}|[\da-f]{6}|[\da-f]{8})$/i)?.[1];
    if (hex) {
      const normalized = hex.length === 3
        ? hex.split("").map((part) => `${part}${part}`).join("")
        : hex.slice(0, 6);
      return {
        r: Number.parseInt(normalized.slice(0, 2), 16),
        g: Number.parseInt(normalized.slice(2, 4), 16),
        b: Number.parseInt(normalized.slice(4, 6), 16),
      };
    }
    const match = text.match(/rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)/i);
    if (!match) return null;
    return { r: Number(match[1]), g: Number(match[2]), b: Number(match[3]) };
  }

  function dreamSkinLuminance({ r, g, b }) {
    const linear = [r, g, b].map((color) => {
      const value = color / 255;
      return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
  }

  const codexPlusDreamSkinMainSurfaceMarker = "data-codex-plus-dream-skin-main-surface";

  function ensureDreamSkinMainSurface() {
    const existing = document.querySelector("main.main-surface");
    if (existing) return existing;

    // 优先按 `data-app-shell-main-surface` 属性找，和上游梦皮的选择器契约（shell-main）一致；
    // 属性不受 CSS Modules 哈希格式变化影响（26.1029 把类名改成了 `MainContentSurface-哈希`，属性仍在）。
    // 属性也没有时，再按类名找，新旧格式都认（#2893）。
    const modularSurface = document.querySelector("main[data-app-shell-main-surface]")
      || document.querySelector('main[class*="_MainContentSurface_"], main[class^="MainContentSurface-"], main[class*=" MainContentSurface-"]');
    const mainCandidates = modularSurface ? [] : [...document.querySelectorAll("main")];
    const shellMain = modularSurface || (mainCandidates.length === 1 ? mainCandidates[0] : null);
    if (!shellMain) return null;

    shellMain.classList.add("main-surface");
    shellMain.setAttribute(codexPlusDreamSkinMainSurfaceMarker, "true");
    return shellMain;
  }

  function clearDreamSkinMainSurfaceCompatibility() {
    document.querySelectorAll(`main[${codexPlusDreamSkinMainSurfaceMarker}="true"]`).forEach((node) => {
      node.classList.remove("main-surface");
      node.removeAttribute(codexPlusDreamSkinMainSurfaceMarker);
    });
  }

  function detectDreamSkinShellMode() {
    const root = document.documentElement;
    const body = document.body;
    const classText = `${root?.className || ""} ${body?.className || ""}`.toLowerCase();

    if (/\b(dark|theme-dark|appearance-dark)\b/.test(classText)) return "dark";
    if (/\b(light|theme-light|appearance-light)\b/.test(classText)) return "light";

    const dataTheme = (
      root?.getAttribute("data-theme") ||
      root?.getAttribute("data-appearance") ||
      root?.getAttribute("data-color-mode") ||
      body?.getAttribute("data-theme") ||
      body?.getAttribute("data-appearance") ||
      ""
    ).toLowerCase();
    if (dataTheme.includes("dark")) return "dark";
    if (dataTheme.includes("light")) return "light";

    const checked = document.querySelector('input[name="appearance-theme"]:checked');
    if (checked) {
      const label = (checked.getAttribute("aria-label") || checked.value || "").toLowerCase();
      if (label.includes("暗") || label.includes("dark")) return "dark";
      if (label.includes("浅") || label.includes("light")) return "light";
      if (label.includes("系统") || label.includes("system")) {
        return window.matchMedia?.("(prefers-color-scheme: dark)")?.matches ? "dark" : "light";
      }
    }

    try {
      const colorScheme = getComputedStyle(root).colorScheme || "";
      if (colorScheme.includes("dark") && !colorScheme.includes("light")) return "dark";
      if (colorScheme.includes("light") && !colorScheme.includes("dark")) return "light";
    } catch {
    }

    const samples = [
      body,
      ensureDreamSkinMainSurface(),
      document.querySelector("aside.app-shell-left-panel"),
    ].filter(Boolean);
    let lightVotes = 0;
    let darkVotes = 0;
    for (const element of samples) {
      try {
        const rgb = dreamSkinParseRgb(getComputedStyle(element).backgroundColor);
        if (!rgb) continue;
        const luminance = dreamSkinLuminance(rgb);
        if (luminance >= 0.55) lightVotes += 1;
        else if (luminance <= 0.25) darkVotes += 1;
      } catch {
      }
    }
    if (lightVotes > darkVotes) return "light";
    if (darkVotes > lightVotes) return "dark";

    try {
      if (window.matchMedia("(prefers-color-scheme: dark)").matches) return "dark";
    } catch {
    }
    return "light";
  }

  function dreamSkinThemeShellMode(theme) {
    const background = dreamSkinParseRgb(theme?.colors?.background);
    if (background) return dreamSkinLuminance(background) < 0.36 ? "dark" : "light";
    return detectDreamSkinShellMode();
  }

  function dreamSkinArtBlobUrl(artDataUrl) {
    if (!artDataUrl || !artDataUrl.startsWith("data:")) return "";
    const comma = artDataUrl.indexOf(",");
    if (comma < 0) return "";
    const mime = /^data:([^;,]+)/.exec(artDataUrl)?.[1] || "image/png";
    const binary = atob(artDataUrl.slice(comma + 1));
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return URL.createObjectURL(new Blob([bytes], { type: mime }));
  }

  function independentThemeDescriptor(stylePreset) {
    const custom = (name, chromeMarkup) => ({
      rootClass: `codex-theme-${name}`,
      homeClass: `theme-${name}-home`,
      shellClass: `theme-${name}-home-shell`,
      chromeId: "codex-theme-chrome",
      chromeClass: `theme-chrome-${name}`,
      chromeMarkup,
    });
    const descriptors = {
      "caishen-lite": custom("caishen-lite", `
        <div class="csl-caption" data-theme-field="name"></div><div class="csl-seal">吉</div>`),
      "caishen-max": custom("caishen-max", `
        <div class="csm-banner" data-theme-field="name"></div><div class="csm-coins">◇ ◇ ◇</div>`),
      "caishen-readable": custom("caishen-readable", ""),
      "export-night": custom("export-night", `
        <div class="exn-titlebar"><span data-theme-field="name"></span><span class="exn-cursor">█</span></div>`),
      "global-founder-bright": custom("global-founder-bright", `
        <div class="gfb-masthead"><span data-theme-field="name"></span><small data-theme-field="status"></small></div>`),
      "mythic-guardian-noir": custom("mythic-guardian-noir", `
        <div class="mgn-sigil"></div><div class="mgn-line"></div>`),
      "midnight-aurora": custom("midnight-aurora", `
        <div class="mda-arc"></div><div class="mda-star">✦</div>`),
      "amber-dusk": custom("amber-dusk", `
        <div class="abd-sun"></div><div class="abd-horizon"></div>`),
      "forest-mist": custom("forest-mist", `
        <div class="fm-branch"></div><div class="fm-leaf">⌁</div>`),
      "cyber-neon": custom("cyber-neon", `
        <div class="cn-index" data-theme-field="status"></div><div class="cn-scan"></div>`),
      "sakura-dawn": custom("sakura-dawn", `
        <div class="sd-petal">✿</div><div class="sd-rule"></div>`),
      "codex-snow": {
        rootClass: "codex-dream-skin",
        homeClass: "dream-home",
        shellClass: "dream-home-shell",
        chromeId: "codex-dream-skin-chrome",
        chromeClass: "",
        chromeMarkup: `
          <div class="dream-brand"><span class="dream-note">SKI</span><span><b>Snowline Codex</b><small>ice-blue training mode</small></span></div>
          <div class="dream-signature">Freeski focus</div>
          <div class="dream-sparkles"><i></i><i></i><i></i><i></i><i></i><i></i></div>
          <div class="dream-ribbon"><span>slopestyle</span><strong>double cork energy</strong><span>halfpipe</span></div>
          <div class="dream-polaroid"></div>`,
      },
      "glass-vision": {
        rootClass: "codex-glass-vision-skin",
        homeClass: "glass-vision-home",
        shellClass: "glass-vision-home-shell",
        taskShellClass: "glass-vision-task-shell",
        chromeId: "codex-glass-vision-skin-chrome",
        chromeClass: "",
        chromeMarkup: `
          <div class="glass-vision-brand"><span class="glass-vision-orbit-mark"><i></i></span><span><b>GLASS VISION</b><small>SILVER BLUE · CELESTIAL</small></span></div>
          <div class="glass-vision-status"><i></i><span>CRYSTAL FIELD</span></div>
          <div class="glass-vision-atmosphere"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div>
          <div class="glass-vision-orbit-lines"><i></i><i></i><i></i></div><div class="glass-vision-prism"></div>`,
      },
    };
    if (descriptors[stylePreset]) return descriptors[stylePreset];
    if (codexPlusDreamSkinPlatform === "windows") {
      return {
        rootClass: "codex-dream-skin",
        homeClass: "dream-home",
        shellClass: "dream-home-shell",
        taskClass: "dream-task",
        chromeId: "codex-dream-skin-chrome",
        chromeClass: "",
        chromeMarkup: "",
      };
    }
    return {
      rootClass: "codex-dream-skin",
      homeClass: "dream-skin-home",
      shellClass: "dream-skin-home-shell",
      chromeId: "codex-dream-skin-chrome",
      chromeClass: "",
      chromeMarkup: `
        <div class="dream-skin-brand"><span class="dream-skin-portal-mark">◉</span><span><b data-theme-field="name"></b><small data-theme-field="subtitle"></small></span></div>
        <div class="dream-skin-status"><i></i><span data-theme-field="status"></span></div>
        <div class="dream-skin-quote" data-theme-field="quote"></div>
        <div class="dream-skin-particles"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div><div class="dream-skin-orbit"></div>`,
    };
  }

  const dreamSkinCompanionId = "codex-dream-skin-companion";
  const dreamSkinCompanionDataUrlPrefixes = [
    "data:image/png;base64,",
    "data:image/jpeg;base64,",
    "data:image/webp;base64,",
    "data:image/gif;base64,",
  ];
  const dreamSkinCompanionBase64Pattern = /^[a-z0-9+/=\s]+$/i;

  function removeDreamSkinCompanion() {
    document.getElementById(dreamSkinCompanionId)?.remove();
  }

  function dreamSkinCompanionConfig(theme) {
    const companion = theme && theme.companion;
    if (!companion || typeof companion !== "object" || companion.enabled === false) return null;
    const dataUrl = typeof companion.dataUrl === "string" ? companion.dataUrl.trim() : "";
    const prefix = dreamSkinCompanionDataUrlPrefixes.find((candidate) =>
      dataUrl.toLowerCase().startsWith(candidate));
    if (
      !dataUrl
      || dataUrl.length > 240_000
      || !prefix
      || !dreamSkinCompanionBase64Pattern.test(dataUrl.slice(prefix.length))
    ) {
      return null;
    }
    const width = Math.max(48, Math.min(Number(companion.width) || 96, 160));
    const side = ["left", "right"].includes(companion.side) ? companion.side : "auto";
    const offsetX = Math.max(-48, Math.min(Number(companion.offsetX) || 0, 48));
    const offsetY = Math.max(-160, Math.min(Number(companion.offsetY) || 0, 160));
    return { dataUrl, width, side, offsetX, offsetY };
  }

  function visibleDreamSkinComposer() {
    return [...document.querySelectorAll(".composer-footer, .composer-surface-chrome")]
      .map((node) => ({ node, rect: node.getBoundingClientRect?.() }))
      .filter(({ rect }) => rect && rect.width > 200 && rect.height > 0)
      .sort((left, right) => right.rect.bottom - left.rect.bottom)[0] || null;
  }

  function ensureDreamSkinCompanion(theme) {
    const config = dreamSkinCompanionConfig(theme);
    const composer = visibleDreamSkinComposer();
    if (!config || !composer) {
      removeDreamSkinCompanion();
      return;
    }

    let companion = document.getElementById(dreamSkinCompanionId);
    if (!companion) {
      companion = document.createElement("img");
      companion.id = dreamSkinCompanionId;
      companion.alt = "";
      companion.setAttribute("aria-hidden", "true");
      Object.assign(companion.style, {
        position: "fixed",
        zIndex: "39",
        height: "auto",
        maxHeight: "160px",
        objectFit: "contain",
        pointerEvents: "none",
        userSelect: "none",
        filter: "drop-shadow(0 8px 14px rgba(0, 0, 0, .18))",
        transition: "left 160ms ease, top 160ms ease, opacity 160ms ease",
      });
      document.body.appendChild(companion);
    }
    if (companion.src !== config.dataUrl) {
      companion.onload = () => ensureDreamSkinCompanion(theme);
      companion.src = config.dataUrl;
    }

    const renderedHeight = companion.naturalWidth > 0 && companion.naturalHeight > 0
      ? Math.min(160, config.width * companion.naturalHeight / companion.naturalWidth)
      : config.width;

    const gap = 12;
    const edge = 8;
    const right = composer.rect.right + gap + config.offsetX;
    const left = composer.rect.left - config.width - gap + config.offsetX;
    const fitsRight = right + config.width <= window.innerWidth - edge;
    const fitsLeft = left >= edge;
    const useRight = config.side === "right"
      ? fitsRight
      : config.side === "left"
        ? !fitsLeft && fitsRight
        : fitsRight || !fitsLeft;

    if (!fitsRight && !fitsLeft) {
      companion.style.opacity = "0";
      return;
    }

    const top = Math.max(
      edge,
      Math.min(
        composer.rect.bottom - renderedHeight + config.offsetY,
        window.innerHeight - renderedHeight - edge,
      ),
    );
    companion.style.width = `${config.width}px`;
    companion.style.left = `${Math.round(useRight ? right : left)}px`;
    companion.style.top = `${Math.round(top)}px`;
    companion.style.opacity = "1";
  }

  function clearDreamSkinPresentation() {
    const root = document.documentElement;
    for (const className of [...(root?.classList || [])]) {
      if (
        className === "codex-dream-skin"
        || className === "codex-glass-vision-skin"
        || className.startsWith("codex-theme-")
      ) {
        root?.classList.remove(className);
      }
    }
    root?.removeAttribute("data-dream-shell");
    root?.removeAttribute("data-codex-plus-dream-skin");
    root?.style.removeProperty("--dream-art");
    root?.style.removeProperty("--dream-skin-art");
    [
      "--ds-bg",
      "--ds-panel",
      "--ds-panel-2",
      "--ds-green",
      "--ds-lime",
      "--ds-cyan",
      "--ds-purple",
      "--ds-text",
      "--ds-muted",
      "--ds-line",
      "--dream-ink",
      "--dream-purple",
      "--dream-violet",
      "--dream-pink",
      "--dream-blush",
      "--dream-pearl",
      "--dream-line",
      "--dream-skin-name",
      "--dream-skin-tagline",
      "--dream-skin-project-prefix",
      "--dream-skin-project-label",
    ].forEach((name) => root?.style.removeProperty(name));
    document.querySelectorAll(".dream-home").forEach((node) => node.classList.remove("dream-home"));
    document.querySelectorAll('[role="main"][data-dream-home-layout]').forEach((node) => {
      node.removeAttribute("data-dream-home-layout");
    });
    document.querySelectorAll(".dream-home-shell").forEach((node) => node.classList.remove("dream-home-shell"));
    document.querySelectorAll(".dream-skin-home").forEach((node) => node.classList.remove("dream-skin-home"));
    document.querySelectorAll(".dream-skin-home-shell").forEach((node) => node.classList.remove("dream-skin-home-shell"));
    document.querySelectorAll("[class]").forEach((node) => {
      for (const className of [...node.classList]) {
        if (
          /^theme-[a-z0-9-]+-(?:home|home-shell|task|task-shell)$/.test(className)
          || /^glass-vision-(?:home|home-shell|task|task-shell)$/.test(className)
        ) {
          node.classList.remove(className);
        }
      }
    });
    document.getElementById(codexPlusDreamSkinStyleId)?.remove();
    document.getElementById("codex-plus-dream-skin-style")?.remove();
    document.getElementById("codex-dream-skin-chrome")?.remove();
    document.getElementById("codex-glass-vision-skin-chrome")?.remove();
    document.getElementById("codex-theme-chrome")?.remove();
    removeDreamSkinCompanion();
    clearDreamSkinMainSurfaceCompatibility();
    const state = window.__CODEX_DREAM_SKIN_STATE__;
    const descriptor = state?.descriptor;
    if (descriptor) {
      root?.classList.remove(descriptor.rootClass);
      for (const className of [descriptor.homeClass, descriptor.shellClass, descriptor.taskClass, descriptor.taskShellClass]) {
        if (!className) continue;
        document.querySelectorAll(`.${className}`).forEach((node) => node.classList.remove(className));
      }
      document.getElementById(descriptor.chromeId)?.remove();
    }
    root?.classList.remove("dream-theme-dark", "dream-theme-light");
    root?.removeAttribute("data-codex-theme");
    root?.removeAttribute("data-codex-theme-root");
    [
      "--theme-bg", "--theme-panel", "--theme-panel-alt", "--theme-accent",
      "--theme-accent-alt", "--theme-secondary", "--theme-highlight", "--theme-text",
      "--theme-muted", "--theme-line", "--theme-art", "--glass-vision-art",
      "--dream-accent", "--dream-accent-ink",
    ].forEach((name) => root?.style.removeProperty(name));
  }

  function cleanupDreamSkin() {
    window.__CODEX_DREAM_SKIN_DISABLED__ = true;
    const state = window.__CODEX_DREAM_SKIN_STATE__;
    if (typeof state?.cleanup === "function" && state.cleanup !== cleanupDreamSkin) {
      try {
        state.cleanup();
      } catch {
      }
    }
    const remainingState = window.__CODEX_DREAM_SKIN_STATE__;
    remainingState?.observer?.disconnect();
    if (remainingState?.timer) clearInterval(remainingState.timer);
    if (remainingState?.scheduler?.timeout) clearTimeout(remainingState.scheduler.timeout);
    if (remainingState?.resizeHandler) window.removeEventListener("resize", remainingState.resizeHandler);
    if (remainingState?.mediaHandler && remainingState?.mediaQuery) {
      try {
        remainingState.mediaQuery.removeEventListener("change", remainingState.mediaHandler);
      } catch {
      }
    }
    if (remainingState?.artUrl) URL.revokeObjectURL(remainingState.artUrl);
    delete window.__CODEX_DREAM_SKIN_STATE__;
    window.__CODEX_GLASS_VISION_SKIN_DISABLED__ = true;
    const glassState = window.__CODEX_GLASS_VISION_SKIN_STATE__;
    try {
      glassState?.cleanup?.();
    } catch {
    }
    delete window.__CODEX_GLASS_VISION_SKIN_STATE__;
    clearDreamSkinPresentation();
  }

  window.__CODEX_PLUS_CLEAR_DREAM_SKIN__ = cleanupDreamSkin;

  function dreamSkinContentSignature(value) {
    const text = String(value || "");
    let hash = 2166136261;
    for (let index = 0; index < text.length; index += 1) {
      hash ^= text.charCodeAt(index);
      hash = Math.imul(hash, 16777619);
    }
    return `${text.length}-${(hash >>> 0).toString(16)}`;
  }

  function applyIndependentThemeVariables(root, shell, theme, descriptor, artSource) {
    const colors = theme.colors || {};
    const accent = colors.accent || (shell === "light" ? "#d85c6c" : "#76e6cc");
    const accentAlt = colors.accentAlt || accent;
    const secondary = colors.secondary || (shell === "light" ? "#e7a3ad" : "#65bde8");
    const variables = {
      "--theme-bg": colors.background || (shell === "light" ? "#f6f3f4" : "#071116"),
      "--theme-panel": colors.panel || (shell === "light" ? "#ffffff" : "#0b1a20"),
      "--theme-panel-alt": colors.panelAlt || (shell === "light" ? "#fff8f9" : "#10272c"),
      "--theme-accent": accent,
      "--theme-accent-alt": accentAlt,
      "--theme-secondary": secondary,
      "--theme-highlight": colors.highlight || accentAlt,
      "--theme-text": colors.text || (shell === "light" ? "#201b1c" : "#edf7f3"),
      "--theme-muted": colors.muted || (shell === "light" ? "#6c6062" : "#9db7ae"),
      "--theme-line": colors.line || (shell === "light" ? "rgba(90, 64, 68, .18)" : "rgba(150, 220, 200, .24)"),
      "--theme-art": artSource,
      "--dream-art": artSource,
      "--dream-skin-art": artSource,
      "--glass-vision-art": artSource,
      "--dream-accent": accent,
      "--dream-accent-ink": colors.panel || "#ffffff",
    };
    for (const [name, value] of Object.entries(variables)) {
      if (typeof value === "string" && value) root.style.setProperty(name, value);
    }
    root.style.setProperty("--dream-skin-name", dreamSkinCssString(theme.name || "Codex Dream Skin"));
    root.style.setProperty("--dream-skin-tagline", dreamSkinCssString(theme.tagline || "把喜欢的画面变成可交互的 Codex 工作台。"));
    root.style.setProperty("--dream-skin-project-prefix", dreamSkinCssString(theme.projectPrefix || "选择项目 · "));
    root.style.setProperty("--dream-skin-project-label", dreamSkinCssString(theme.projectLabel || "◉  选择项目"));
    root.classList.toggle("dream-theme-dark", shell === "dark");
    root.classList.toggle("dream-theme-light", shell === "light");
    const preset = theme.stylePreset || "dream-original";
    if (root.getAttribute("data-codex-theme") !== preset) root.setAttribute("data-codex-theme", preset);
    if (root.getAttribute("data-codex-theme-root") !== descriptor.rootClass) {
      root.setAttribute("data-codex-theme-root", descriptor.rootClass);
    }
  }

  function installDreamSkin(settings) {
    const theme = dreamSkinThemeConfig(settings.dreamSkinThemeConfig);
    const styles = window.__CODEX_PLUS_DREAM_SKIN_STYLES__ || {};
    const descriptor = independentThemeDescriptor(theme.stylePreset);
    const cssText = String(styles[theme.stylePreset] || styles["dream-original"] || "");
    const artDataUrl = String(window.__CODEX_PLUS_DREAM_SKIN_ART__ || "");
    const themeSignature = dreamSkinContentSignature(JSON.stringify(theme));
    const artSignature = String(window.__CODEX_PLUS_DREAM_SKIN_ART_SIGNATURE__ || dreamSkinContentSignature(artDataUrl));
    const version = `codex-plus:independent:${codexPlusDreamSkinPlatform}:r${codexPlusDreamSkinRevision}:${theme.stylePreset}:${themeSignature}:${artSignature}:${cssText.length}`;
    const existingState = window.__CODEX_DREAM_SKIN_STATE__;
    if (existingState?.version === version && typeof existingState.ensure === "function") {
      window.__CODEX_DREAM_SKIN_DISABLED__ = false;
      existingState.ensure();
      return;
    }

    cleanupDreamSkin();
    window.__CODEX_DREAM_SKIN_DISABLED__ = false;
    const artUrl = dreamSkinArtBlobUrl(artDataUrl);
    const artSource = artUrl ? `url("${artUrl}")` : "none";

    const ensureStyle = (root) => {
      let style = document.getElementById(codexPlusDreamSkinStyleId);
      if (!style) {
        style = document.createElement("style");
        style.id = codexPlusDreamSkinStyleId;
        (document.head || root).appendChild(style);
      }
      if (style.dataset.independentThemeVersion !== version) {
        style.textContent = cssText;
        style.dataset.independentThemeVersion = version;
      }
    };

    const ensure = () => {
      if (window.__CODEX_DREAM_SKIN_DISABLED__) return;
      const root = document.documentElement;
      if (!root || !document.body) return;
      const shellMain = ensureDreamSkinMainSurface();
      if (!shellMain) {
        clearDreamSkinPresentation();
        return;
      }

      root.classList.add(descriptor.rootClass);
      root.setAttribute("data-codex-plus-dream-skin", "true");
      const shell = dreamSkinThemeShellMode(theme);
      root.setAttribute("data-dream-shell", shell);
      applyIndependentThemeVariables(root, shell, theme, descriptor, artSource);
      ensureStyle(root);
      ensureDreamSkinCompanion(theme);

      const homeIndicator = document.querySelector('[data-testid="home-icon"]');
      const homeCandidate = homeIndicator?.closest('[role="main"]')
        || [...document.querySelectorAll('[role="main"]')].find((candidate) =>
          candidate.querySelector('[data-feature="game-source"]')
          && candidate.querySelector('.group\\/home-suggestions'))
        || null;
      const homeHasClassicChrome = !!(
        homeCandidate
        && homeCandidate.querySelector('[data-feature="game-source"]')
        && (
          homeCandidate.querySelector('.group\\/home-suggestions')
          || homeCandidate.querySelector('[class*="home-suggestions"]')
          || homeCandidate.querySelector('[class*="_homeUtilityBar_"], [class^="homeUtilityBar-"], [class*=" homeUtilityBar-"]')
        )
      );
      const home = homeHasClassicChrome ? homeCandidate : null;
      for (const candidate of document.querySelectorAll(`[role="main"].${descriptor.homeClass}`)) {
        if (candidate !== home && candidate !== homeCandidate) candidate.classList.remove(descriptor.homeClass);
      }
      if (home) home.classList.add(descriptor.homeClass);
      else if (homeCandidate && descriptor.homeClass) homeCandidate.classList.add(descriptor.homeClass);
      if (descriptor.taskClass) {
        for (const candidate of document.querySelectorAll('[role="main"]')) {
          candidate.classList.toggle(descriptor.taskClass, candidate !== home && candidate !== homeCandidate);
        }
      }
      for (const candidate of document.querySelectorAll('[role="main"]')) {
        if (candidate === home) {
          const hero = candidate.querySelector(':scope > div > div > div');
          const structured = !!(hero && hero.querySelector('[data-feature="game-source"], [data-testid="home-icon"]'));
          candidate.setAttribute('data-dream-home-layout', structured ? 'structured' : 'soft');
        } else {
          candidate.setAttribute('data-dream-home-layout', 'soft');
        }
      }
      shellMain.classList.toggle(descriptor.shellClass, Boolean(homeCandidate));
      if (descriptor.taskShellClass) shellMain.classList.toggle(descriptor.taskShellClass, !home);

      let chrome = document.getElementById(descriptor.chromeId);
      if (!chrome || chrome.parentElement !== document.body) {
        chrome?.remove();
        chrome = document.createElement("div");
        chrome.id = descriptor.chromeId;
        chrome.setAttribute("aria-hidden", "true");
        chrome.innerHTML = descriptor.chromeMarkup;
        document.body.appendChild(chrome);
      }
      if (chrome.className !== descriptor.chromeClass) chrome.className = descriptor.chromeClass;
      const fields = {
        name: theme.name || "Codex Dream Skin",
        subtitle: theme.brandSubtitle || "CODEX DREAM SKIN",
        status: theme.statusText || "THEME ONLINE",
        quote: theme.quote || "MAKE SOMETHING WONDERFUL",
      };
      for (const [field, value] of Object.entries(fields)) {
        const target = chrome.querySelector(`[data-theme-field="${field}"]`);
        if (target && target.textContent !== value) target.textContent = value;
      }
      const shellBox = shellMain.getBoundingClientRect();
      chrome.style.left = `${Math.round(shellBox.left)}px`;
      chrome.style.top = `${Math.round(shellBox.top)}px`;
      chrome.style.width = `${Math.round(shellBox.width)}px`;
      chrome.style.height = `${Math.round(shellBox.height)}px`;
      chrome.classList.toggle(descriptor.shellClass, Boolean(home));
      if (descriptor.taskShellClass) chrome.classList.toggle(descriptor.taskShellClass, !home);
      chrome.dataset.dreamShell = shell;
    };

    const scheduler = { timeout: null };
    const scheduleEnsure = () => {
      if (scheduler.timeout) clearTimeout(scheduler.timeout);
      scheduler.timeout = setTimeout(() => {
        scheduler.timeout = null;
        ensure();
      }, 180);
    };
    const observer = new MutationObserver(scheduleEnsure);
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "data-theme", "data-appearance", "data-color-mode"],
    });
    const timer = setInterval(ensure, 4000);
    const resizeHandler = scheduleEnsure;
    window.addEventListener("resize", resizeHandler, { passive: true });

    let mediaQuery = null;
    let mediaHandler = null;
    try {
      mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
      mediaHandler = scheduleEnsure;
      mediaQuery.addEventListener("change", mediaHandler);
    } catch {
    }

    window.__CODEX_DREAM_SKIN_STATE__ = {
      ensure,
      cleanup: cleanupDreamSkin,
      observer,
      timer,
      scheduler,
      resizeHandler,
      mediaQuery,
      mediaHandler,
      artUrl,
      version,
      descriptor,
      themeId: theme.id || "custom",
      detectShellMode: detectDreamSkinShellMode,
    };
    ensure();
  }

  /**
   * 本轮皮肤刷新该做什么（纯函数，供测试切片）：
   *   - "cleanup"：设置明确关闭或暂停皮肤；
   *   - "install"：内置运行时按设置安装；
   *   - "ensure"：外部运行时已在页面里，补主区域兼容标记并刷新；
   *   - "idle"：外部运行时未安装、设置也还没读到，只做无副作用的刷新。
   * 外部运行时由注入脚本按注入时的设置直接安装。重启 Codex 后首次 /backend/status
   * 可能超时、页面还会被启动壳自救重载（#2680），这期间设置读不到，不能因此不补
   * 主区域兼容标记，否则皮肤选择器对不上主区域，整页看起来没换肤。
   */
  function dreamSkinRefreshAction({ enabled, paused, backendLoaded, externalRuntime, runtimeInstalled }) {
    if (externalRuntime) {
      if (backendLoaded) return enabled && !paused ? "ensure" : "cleanup";
      return runtimeInstalled ? "ensure" : "idle";
    }
    return enabled && !paused ? "install" : "cleanup";
  }

  let dreamSkinCleanupRoot = null;

  function refreshDreamSkin() {
    const settings = codexPlusSettings();
    const runtimeState = window.__CODEX_DREAM_SKIN_STATE__ || window.__CODEX_GLASS_VISION_SKIN_STATE__;
    const action = dreamSkinRefreshAction({
      enabled: !!settings.dreamSkinEnabled && !globalThis.__codexPlusBuiltinInactive?.has("dream-skin"),
      paused: !!settings.dreamSkinPaused,
      backendLoaded: !!codexPlusBackendSettingsLoaded,
      externalRuntime: !!window.__CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__,
      runtimeInstalled: !!runtimeState,
    });
    if (action === "cleanup") {
      const root = document.documentElement;
      // 首次仍清旧残留；关闭后不再每轮扫描全页，新运行时或根节点出现时重新清理。
      if (!root || dreamSkinCleanupRoot !== root || runtimeState) {
        cleanupDreamSkin();
        dreamSkinCleanupRoot = root;
      }
      return;
    }
    dreamSkinCleanupRoot = null;
    if (action === "ensure" || action === "install") ensureDreamSkinMainSurface();
    if (window.__CODEX_PLUS_EXTERNAL_DREAM_SKIN_RUNTIME__) {
      const state = runtimeState;
      state?.ensure?.();
      ensureDreamSkinCompanion(
        window.__CODEX_PLUS_DREAM_SKIN_THEME__ || settings.dreamSkinThemeConfig,
      );
      return;
    }
    installDreamSkin(settings);
  }

  function applyDreamSkinLiveUpdate(payload) {
    if (!payload || String(payload.revision || "") !== codexPlusDreamSkinRevision) return false;
    if (typeof payload.artDataUrl === "string" && payload.artDataUrl) {
      window.__CODEX_PLUS_DREAM_SKIN_ART__ = payload.artDataUrl;
    }
    window.__CODEX_PLUS_DREAM_SKIN_ART_SIGNATURE__ = String(payload.artSignature || "");
    window.__CODEX_PLUS_DREAM_SKIN_THEME__ = payload.theme && typeof payload.theme === "object" ? payload.theme : {};
    codexPlusBackendSettings.codexAppDreamSkinEnabled = true;
    codexPlusBackendSettings.codexAppDreamSkinPaused = false;
    codexPlusBackendSettings.codexAppDreamSkinThemeConfig = window.__CODEX_PLUS_DREAM_SKIN_THEME__;
    refreshDreamSkin();
    return true;
  }

  window.__CODEX_PLUS_DREAM_SKIN_RUNTIME_REVISION__ = codexPlusDreamSkinRevision;
  window.__CODEX_PLUS_APPLY_DREAM_SKIN__ = applyDreamSkinLiveUpdate;

  function setCodexPlusSetting(key, value) {
    const backendKey = codexPlusBackendSettingMap[key];
    if (backendKey) {
      // 跟着开关走的功能由各自的内置拓展订阅 setBackendSetting 发的 settings/changed，这里不点名。
      void setBackendSetting(backendKey, value).catch(() => {
        void loadBackendSettings();
      });
      return;
    }
    let stored = {};
    try {
      stored = JSON.parse(localStorage.getItem(codexPlusSettingsKey) || "{}");
    } catch {
      stored = {};
    }
    const next = { ...stored, [key]: value };
    localStorage.setItem(codexPlusSettingsKey, JSON.stringify(next));
    if (typeof emitCodexPlusSettingsChanged === "function") emitCodexPlusSettingsChanged(key, value, true);
    renderCodexPlusMenu();
    scan();
  }

  /** 滚动位置恢复关掉时（设置关、或内置拓展停用）撤掉排着的保存 / 恢复定时器，作废进行中的轮次。 */
  function stopThreadScrollRestore() {
    if (window.__codexThreadScrollRuntime?.activeScroller) bindThreadScrollListener(null);
    clearTimeout(window.__codexThreadScrollSaveTimer);
    window.__codexThreadScrollSaveTimer = null;
    window.__codexThreadScrollRestoreRevision = (window.__codexThreadScrollRestoreRevision || 0) + 1;
    window.__codexThreadScrollSyncRevision = (window.__codexThreadScrollSyncRevision || 0) + 1;
    (window.__codexThreadScrollRestoreTimers || []).forEach((timer) => clearTimeout(timer));
    window.__codexThreadScrollRestoreTimers = [];
    (window.__codexThreadScrollSyncTimers || []).forEach((timer) => clearTimeout(timer));
    window.__codexThreadScrollSyncTimers = [];
    window.__codexThreadScrollRuntime = null;
  }

  function syncStepwisePanel(
    enabled = codexPlusSettings().stepwise,
    answerOutlineEnabled = codexPlusSettings().answerOutline
  ) {
    try {
      window.__codexStepwisePanel?.syncSettings?.({
        enabled: !!enabled,
        answerOutlineEnabled: !!answerOutlineEnabled,
      });
    } catch (error) {
      sendCodexPlusDiagnostic("stepwise_sync_failed", {
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
    }
  }

  function normalizeConversationViewWidth(value) {
    if (value === null || value === undefined || String(value).trim() === "") return null;
    const number = Number(value);
    if (!Number.isFinite(number)) return null;
    return Math.max(conversationViewMinWidth, Math.min(conversationViewMaxAllowedWidth, Math.round(number)));
  }

  function conversationViewWidth() {
    const settingsWidth = normalizeConversationViewWidth(codexPlusSettings().conversationViewMaxWidth);
    if (settingsWidth) return settingsWidth;
    const legacyWidth = normalizeConversationViewWidth(localStorage.getItem(conversationViewLegacyWidthKey));
    return legacyWidth || conversationViewDefaultWidth;
  }

  function refreshConversationViewControls() {
    const enabled = !!codexPlusSettings().conversationView;
    const width = conversationViewWidth();
    document.querySelectorAll("[data-codex-plus-conversation-view-width]").forEach((input) => {
      input.value = String(width);
      input.disabled = !enabled;
    });
  }

  function setConversationViewWidth(value) {
    const width = normalizeConversationViewWidth(value);
    if (!width) return;
    setCodexPlusSetting("conversationViewMaxWidth", width);
  }
