(() => {
  const helperBase = window.__CODEX_SESSION_DELETE_HELPER__ || "http://127.0.0.1:57321";
  const buttonClass = "codex-delete-button";
  const exportButtonClass = "codex-export-button";
  const actionButtonClass = "codex-session-action-button";
  const actionGroupClass = "codex-session-actions";
  const styleId = "codex-delete-style";
  const codexDeleteStyleVersion = "5";
  const codexPlusMenuId = "codex-plus-menu";
  const codexDeleteVersion = "6";
  const codexExportVersion = "1";
  const codexActionGroupVersion = "1";
  const codexArchiveRowActionsVersion = "1";
  const codexArchiveDeleteAllVersion = "2";
  const codexPlusVersion = "1.0.5";
  const codexPlusSettingsKey = "codexPlusSettings";
  const codexPlusBridgePatchVersion = "1";
  const codexThreadListPageLimit = 1000;
  const codexThreadListMaxPages = 20;
  const codexFastModeUnlockVersion = "1";
  const codexStatsigUnlockVersion = "1";
  const codexPlusStatsigGateOverrides = {
    "410262010": true,
    "410065390": true,
    "2929582856": false,
  };
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
      [data-codex-delete-row="true"]:hover .${actionGroupClass} { opacity: 1; }
      [data-codex-delete-row="true"].codex-archive-confirm-visible .${actionGroupClass} { right: 66px; }
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
      }
      .codex-delete-confirm-actions [data-codex-delete-confirm="true"] {
        border-color: #ef4444;
        background: #dc2626;
      }
      #${codexPlusMenuId}.codex-plus-menu-floating {
        position: fixed;
        top: 0;
        right: 140px;
        left: auto;
        z-index: 2147483645;
        height: 30px;
        color: #d1d5db;
        font: 13px system-ui, sans-serif;
        text-align: right;
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
        border: 0;
        background: transparent;
        color: inherit;
        font: inherit;
        height: 100%;
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
      }
      .codex-plus-modal-content {
        width: min(520px, calc(100vw - 48px));
        border: 1px solid rgba(255,255,255,.12);
        border-radius: 18px;
        background: #2b2b2b;
        color: #f3f4f6;
        font: 14px system-ui, sans-serif;
        box-shadow: 0 24px 80px rgba(0,0,0,.45);
      }
      .codex-plus-modal-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 18px 20px 10px;
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
        cursor: default;
      }
      .codex-plus-modal-body { padding: 8px 20px 20px; }
      .codex-plus-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        padding: 12px 0;
        border-top: 1px solid rgba(255,255,255,.1);
      }
      .codex-plus-row:first-child { border-top: 0; }
      .codex-plus-row-title { font-weight: 550; }
      .codex-plus-row-description { margin-top: 3px; color: #a1a1aa; font-size: 12px; }
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
      .codex-plus-toggle[data-enabled="true"] { background: #10a37f; }
      .codex-plus-toggle[data-enabled="true"] span { transform: translateX(18px); }
      .codex-plus-about { color: #a1a1aa; line-height: 1.5; }
      .codex-plus-tabs { display: flex; gap: 8px; padding: 0 20px 8px; }
      .codex-plus-tab-button { border: 1px solid rgba(255,255,255,.14); border-radius: 999px; background: transparent; color: #d1d5db; font: 12px system-ui, sans-serif; padding: 5px 10px; }
      .codex-plus-tab-button[data-active="true"] { background: #10a37f; color: white; border-color: #10a37f; }
      .codex-plus-panel[hidden] { display: none; }
      .codex-plus-action-button { border: 1px solid rgba(255,255,255,.18); border-radius: 7px; background: #3f3f46; color: #f3f4f6; font: 12px system-ui, sans-serif; padding: 6px 8px; }
      .codex-plus-backend-status { display: grid; gap: 4px; min-width: 132px; justify-items: end; }
      .codex-plus-backend-label { color: #a1a1aa; font-size: 12px; }
      .codex-plus-backend-label[data-status="ok"] { color: #34d399; }
      .codex-plus-backend-label[data-status="failed"] { color: #f87171; }
      .codex-plus-backend-repair { border: 1px solid rgba(255,255,255,.18); border-radius: 7px; background: #3f3f46; color: #f3f4f6; font: 12px system-ui, sans-serif; padding: 6px 8px; }
      .codex-plus-backend-repair[hidden] { display: none; }
      .codex-plus-user-script-warning { margin-top: 4px; color: #fbbf24; font-size: 12px; }
      .codex-plus-user-script-dirs { margin-top: 6px; color: #a1a1aa; font-size: 11px; line-height: 1.4; word-break: break-all; }
      .codex-plus-user-script-list { margin-top: 8px; display: grid; gap: 6px; }
      .codex-plus-user-script-item { display: flex; align-items: center; justify-content: space-between; gap: 8px; border: 1px solid rgba(255,255,255,.08); border-radius: 8px; padding: 6px 8px; }
      .codex-plus-user-script-name { font-size: 12px; }
      .codex-plus-user-script-meta { margin-top: 2px; color: #a1a1aa; font-size: 11px; }
      .codex-plus-user-script-error { margin-top: 2px; color: #f87171; font-size: 11px; word-break: break-all; }
      .codex-plus-user-script-actions { display: grid; justify-items: end; gap: 8px; min-width: 120px; }
      .codex-plus-user-script-reload { border: 1px solid rgba(255,255,255,.18); border-radius: 7px; background: #3f3f46; color: #f3f4f6; font: 12px system-ui, sans-serif; padding: 6px 8px; }
    `;
    document.documentElement.appendChild(style);
  }

  function defaultCodexPlusSettings() {
    return {
      pluginEntryUnlock: true,
      forcePluginInstall: true,
      sessionDelete: true,
      markdownExport: true,
      sessionLimitUnlock: true,
      fastModeUnlock: true,
      haleclipseCompatUnlock: true,
      nativeMenuPlacement: true,
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
    renderCodexPlusMenu();
    scan();
  }

  function renderCodexPlusMenu() {
    document.querySelectorAll(".codex-plus-toggle[data-codex-plus-setting]").forEach((button) => {
      const key = button.getAttribute("data-codex-plus-setting");
      button.dataset.enabled = String(!!codexPlusSettings()[key]);
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

  function selectCodexPlusTab(tab) {
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
              <div><div class="codex-plus-row-title">会话列表扩容</div><div class="codex-plus-row-description">自动分页合并最近会话请求，避免侧边栏只加载前 50 条。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="sessionLimitUnlock"><span></span></button>
            </div>
            <div class="codex-plus-row codex-fast-mode-row">
              <div><div class="codex-plus-row-title">Fast 模式解锁</div><div class="codex-plus-row-description">允许 API Key / 第三方 API 模式显示并使用 Fast 速度选项。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="fastModeUnlock"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">哈雷彗星兼容补丁</div><div class="codex-plus-row-description">外置启用 i18n、Browser-use 相关 gate，并关闭 sunset 强制更新遮罩。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="haleclipseCompatUnlock"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">原生菜单栏位置</div><div class="codex-plus-row-description">把 Codex++ 菜单插入顶部原生菜单栏；默认关闭以避免页面重渲染冲突。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="nativeMenuPlacement"><span></span></button>
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
        </div>
      </div>
    `;
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay || event.target.closest(".codex-plus-modal-close")) {
        overlay.remove();
        return;
      }
      const tabButton = event.target.closest("[data-codex-plus-tab]");
      if (tabButton) {
        selectCodexPlusTab(tabButton.getAttribute("data-codex-plus-tab"));
        return;
      }
      if (event.target.closest("[data-codex-open-devtools]")) {
        postJson("/devtools/open", {});
        return;
      }
      if (event.target.closest("[data-codex-backend-repair]")) {
        repairBackend();
        return;
      }
      const issueButton = event.target.closest("[data-codex-plus-issue]");
      if (issueButton) {
        const issueUrl = "https://github.com/BigPizzaV3/CodexPlusPlus/issues";
        window.open(issueUrl, "_blank");
        return;
      }
      const userScriptsEnabled = event.target.closest("[data-codex-user-scripts-enabled]");
      if (userScriptsEnabled) {
        loadUserScripts("/user-scripts/set-enabled", { enabled: userScriptsEnabled.dataset.enabled !== "true" });
        return;
      }
      const userScriptToggle = event.target.closest("[data-codex-user-script-key]");
      if (userScriptToggle) {
        loadUserScripts("/user-scripts/set-script-enabled", { key: userScriptToggle.getAttribute("data-codex-user-script-key"), enabled: userScriptToggle.dataset.enabled !== "true" });
        return;
      }
      if (event.target.closest("[data-codex-user-scripts-reload]")) {
        loadUserScripts("/user-scripts/reload", {});
        return;
      }
      const toggle = event.target.closest("[data-codex-plus-setting]");
      if (!toggle) return;
      const key = toggle.getAttribute("data-codex-plus-setting");
      setCodexPlusSetting(key, !codexPlusSettings()[key]);
    }, true);
    document.body.appendChild(overlay);
    renderCodexPlusMenu();
    renderBackendStatus();
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

  function removeDuplicateCodexPlusMenus(keep) {
    document.querySelectorAll(`#${codexPlusMenuId}, [data-codex-plus-menu="true"]`).forEach((node) => {
      if (node !== keep) node.remove();
    });
    Array.from(document.querySelectorAll("button")).forEach((button) => {
      if ((button.textContent || "").trim() === `Codex++ ${codexPlusVersion}` && !button.closest(`#${codexPlusMenuId}`)) {
        button.remove();
      }
    });
  }

  function configureCodexPlusTrigger(menu, trigger, nativeButtonClass) {
    if (!trigger) return;
    if (nativeButtonClass) trigger.className = nativeButtonClass;
    if (trigger.dataset.codexPlusTriggerInstalled === "5") return;
    trigger.dataset.codexPlusTriggerInstalled = "5";
    trigger.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      openCodexPlusModal();
    }, true);
  }

  function installCodexPlusMenu() {
    const existing = document.getElementById(codexPlusMenuId);
    removeDuplicateCodexPlusMenus(existing);
    let insertionPoint = findNativeMenuInsertionPoint();
    if (existing && existing.dataset.codexPlusMenuVersion !== "5") {
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
    menu.dataset.codexPlusMenuVersion = "5";
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
      menu.className = "codex-plus-menu-floating";
      document.documentElement.appendChild(menu);
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

  function codexPlusDiagnostics() {
    window.__codexPlusPlusDiagnostics = window.__codexPlusPlusDiagnostics || {
      sessionList: {},
      fastMode: {},
      statsig: {},
      bridgeErrors: [],
    };
    return window.__codexPlusPlusDiagnostics;
  }

  function recordCodexPlusDiagnostic(scope, detail) {
    const diagnostics = codexPlusDiagnostics();
    diagnostics[scope] = { ...(diagnostics[scope] || {}), ...detail, updatedAt: Date.now() };
  }

  function recordFastModeDiagnostic(detail) {
    recordCodexPlusDiagnostic("fastMode", detail);
  }

  function clonePlainObject(value) {
    try {
      if (typeof structuredClone === "function") return structuredClone(value);
    } catch {}
    try {
      return JSON.parse(JSON.stringify(value));
    } catch {
      return value;
    }
  }

  function isPlainObject(value) {
    return !!value && typeof value === "object";
  }

  function requestFromCodexMessage(message) {
    const request = message?.request;
    return request && typeof request === "object" ? request : null;
  }

  function isRecentThreadListRequest(message) {
    if (!codexPlusSettings().sessionLimitUnlock) return false;
    if (message?.type !== "mcp-request") return false;
    const request = requestFromCodexMessage(message);
    if (request?.method !== "thread/list") return false;
    const params = request.params || {};
    if (params.archived === true) return false;
    const limit = Number(params.limit || 0);
    return !limit || limit <= codexThreadListPageLimit;
  }

  function responseEnvelopeFromMessageEvent(event) {
    const data = event?.data;
    if (!data || data.type !== "mcp-response") return null;
    const message = data.message || data.response;
    if (!message || typeof message !== "object") return null;
    const id = message.id ?? message.requestId;
    if (id == null) return null;
    return { data, message, id: String(id) };
  }

  function postSyntheticMcpResponse(hostId, id, result, error = null) {
    const message = error ? { id, error } : { id, result };
    const payload = { type: "mcp-response", hostId, message };
    const targetOrigin = window.location.origin && window.location.origin !== "null" ? window.location.origin : "*";
    setTimeout(() => window.postMessage(payload, targetOrigin), 0);
  }

  function mergeThreadListPages(pages) {
    const merged = [];
    const seen = new Set();
    for (const page of pages) {
      for (const thread of page?.data || []) {
        const key = thread?.id || thread?.sessionId || JSON.stringify(thread);
        if (seen.has(key)) continue;
        seen.add(key);
        merged.push(thread);
      }
    }
    const lastPage = pages[pages.length - 1] || {};
    return { ...lastPage, data: merged, nextCursor: lastPage.nextCursor || null };
  }

  function sendThreadListPage(job, cursor) {
    if (typeof job.originalSend !== "function") {
      postSyntheticMcpResponse(job.hostId, job.originalId, mergeThreadListPages(job.pages));
      recordCodexPlusDiagnostic("sessionList", { active: false, error: "electronBridge.sendMessageFromView unavailable" });
      return;
    }
    const pageIndex = job.pages.length + 1;
    const shadowId = `${job.originalId}:codex-plus-thread-list:${pageIndex}`;
    const pageMessage = clonePlainObject(job.originalMessage);
    pageMessage.request = {
      ...pageMessage.request,
      id: shadowId,
      params: {
        ...(pageMessage.request?.params || {}),
        cursor,
        limit: codexThreadListPageLimit,
      },
    };
    window.__codexPlusThreadListRequests.set(shadowId, job);
    recordCodexPlusDiagnostic("sessionList", {
      active: true,
      lastShadowId: shadowId,
      pageLimit: codexThreadListPageLimit,
      maxPages: codexThreadListMaxPages,
    });
    Promise.resolve(job.originalSend.call(job.thisArg, pageMessage)).catch((error) => {
      window.__codexPlusThreadListRequests.delete(shadowId);
      postSyntheticMcpResponse(job.hostId, job.originalId, null, { message: String(error?.message || error) });
      recordCodexPlusDiagnostic("sessionList", { active: false, error: String(error?.stack || error) });
    });
  }

  function startThreadListPagination(message, originalSend, thisArg) {
    const request = requestFromCodexMessage(message);
    const originalId = String(request.id);
    const params = request.params || {};
    const job = {
      originalId,
      originalMessage: clonePlainObject(message),
      hostId: message.hostId,
      originalSend,
      thisArg,
      pages: [],
    };
    sendThreadListPage(job, params.cursor ?? null);
    return Promise.resolve();
  }

  function trackThreadListPagination(message) {
    const request = requestFromCodexMessage(message);
    if (!request?.id || window.__codexPlusThreadListRequests?.has(String(request.id))) return;
    window.__codexPlusThreadListRequests = window.__codexPlusThreadListRequests || new Map();
    window.__codexPlusThreadListRequests.set(String(request.id), {
      originalId: String(request.id),
      originalMessage: clonePlainObject(message),
      hostId: message.hostId,
      originalSend: window.electronBridge?.sendMessageFromView,
      thisArg: window.electronBridge,
      pages: [],
    });
    recordCodexPlusDiagnostic("sessionList", {
      active: true,
      trackedOriginalId: String(request.id),
      pageLimit: codexThreadListPageLimit,
      maxPages: codexThreadListMaxPages,
    });
  }

  function handleThreadListPageResponse(event, envelope) {
    const job = window.__codexPlusThreadListRequests?.get(envelope.id);
    if (!job) return false;
    event.stopImmediatePropagation?.();
    event.stopPropagation?.();
    window.__codexPlusThreadListRequests.delete(envelope.id);
    if (envelope.message.error) {
      postSyntheticMcpResponse(job.hostId, job.originalId, null, envelope.message.error);
      recordCodexPlusDiagnostic("sessionList", { active: false, error: String(envelope.message.error?.message || envelope.message.error) });
      return true;
    }
    const result = envelope.message.result;
    if (!result || !Array.isArray(result.data)) {
      postSyntheticMcpResponse(job.hostId, job.originalId, result);
      recordCodexPlusDiagnostic("sessionList", { active: false, bypassed: true });
      return true;
    }
    job.pages.push(result);
    const nextCursor = result.nextCursor ?? null;
    if (nextCursor && job.pages.length < codexThreadListMaxPages) {
      sendThreadListPage(job, nextCursor);
      return true;
    }
    const merged = mergeThreadListPages(job.pages);
    merged.nextCursor = nextCursor && job.pages.length >= codexThreadListMaxPages ? nextCursor : null;
    postSyntheticMcpResponse(job.hostId, job.originalId, merged);
    recordCodexPlusDiagnostic("sessionList", {
      active: false,
      pages: job.pages.length,
      threads: merged.data.length,
      hasMore: !!merged.nextCursor,
    });
    return true;
  }

  function trackCodexRequest(message) {
    const request = requestFromCodexMessage(message);
    if (!request?.id) return;
    window.__codexPlusRequestMethods = window.__codexPlusRequestMethods || new Map();
    window.__codexPlusRequestMethods.set(String(request.id), request.method || "");
  }

  function ensureFastTier(model) {
    if (!model || typeof model !== "object") return 0;
    const tiers = Array.isArray(model.additionalSpeedTiers) ? model.additionalSpeedTiers : [];
    if (!tiers.includes("fast")) {
      model.additionalSpeedTiers = [...tiers, "fast"];
      return 1;
    }
    return 0;
  }

  function patchFastModeGateOnObject(value, seen = new WeakSet(), depth = 0) {
    if (!codexPlusSettings().fastModeUnlock) return 0;
    if (!value || typeof value !== "object" || seen.has(value) || depth > 5) return 0;
    seen.add(value);
    let patches = 0;
    if (Array.isArray(value)) {
      for (const item of value.slice(0, 120)) patches += patchFastModeGateOnObject(item, seen, depth + 1);
      return patches;
    }
    if ("additionalSpeedTiers" in value || ("model" in value && ("id" in value || "slug" in value || "name" in value))) {
      patches += ensureFastTier(value);
    }
    if (value.canUseFastMode === false) {
      value.canUseFastMode = true;
      patches += 1;
    }
    if (value.isDisabledByRequirement === true) {
      value.isDisabledByRequirement = false;
      patches += 1;
    }
    if (value.featureRequirements && typeof value.featureRequirements === "object" && value.featureRequirements.fast_mode === false) {
      value.featureRequirements.fast_mode = true;
      patches += 1;
    }
    if (value.requirements?.featureRequirements?.fast_mode === false) {
      value.requirements.featureRequirements.fast_mode = true;
      patches += 1;
    }
    const models = value.models || value.availableModels;
    if (Array.isArray(models)) {
      for (const model of models) patches += ensureFastTier(model);
    }
    for (const key of ["data", "result", "value", "model", "models", "requirements", "featureRequirements", "serviceTierSettings"]) {
      patches += patchFastModeGateOnObject(value[key], seen, depth + 1);
    }
    return patches;
  }

  function patchExperimentalFeatures(value) {
    if (!codexPlusSettings().haleclipseCompatUnlock) return 0;
    if (!value || typeof value !== "object") return 0;
    const features = Array.isArray(value) ? value : Array.isArray(value.data) ? value.data : Array.isArray(value.features) ? value.features : null;
    if (!features) return 0;
    let patches = 0;
    const names = new Set(features.map((feature) => feature?.name).filter(Boolean));
    for (const feature of features) {
      if (!feature || typeof feature !== "object") continue;
      if (["plugins", "browser_use", "browser_use_external", "in_app_browser"].includes(feature.name) && feature.enabled !== true) {
        feature.enabled = true;
        patches += 1;
      }
    }
    for (const name of ["plugins", "browser_use", "browser_use_external", "in_app_browser"]) {
      if (!names.has(name)) {
        features.push({ name, enabled: true });
        patches += 1;
      }
    }
    return patches;
  }

  function patchCodexResponseForUnlocks(result, method = "") {
    let patches = 0;
    if (method === "model/list" || method === "configRequirements/read" || method === "config/read" || method === "") {
      patches += patchFastModeGateOnObject(result);
    }
    if (method === "experimentalFeature/list" || method === "") {
      patches += patchExperimentalFeatures(result);
    }
    if (patches > 0) recordFastModeDiagnostic({ lastResponseMethod: method || "unknown", responsePatches: patches, version: codexFastModeUnlockVersion });
    return patches;
  }

  function patchMcpResponseBeforeCodex(event, envelope) {
    const method = window.__codexPlusRequestMethods?.get(envelope.id) || "";
    if (method) window.__codexPlusRequestMethods.delete(envelope.id);
    if (envelope.message?.result) patchCodexResponseForUnlocks(envelope.message.result, method);
  }

  function installMcpResponseInterceptor() {
    if (window.__codexPlusMcpResponseInterceptorVersion === codexPlusBridgePatchVersion) return;
    window.__codexPlusThreadListRequests = window.__codexPlusThreadListRequests || new Map();
    window.addEventListener("message", (event) => {
      try {
        const envelope = responseEnvelopeFromMessageEvent(event);
        if (!envelope) return;
        if (handleThreadListPageResponse(event, envelope)) return;
        patchMcpResponseBeforeCodex(event, envelope);
      } catch (error) {
        const diagnostics = codexPlusDiagnostics();
        diagnostics.bridgeErrors.push(String(error?.stack || error));
      }
    }, true);
    window.__codexPlusMcpResponseInterceptorVersion = codexPlusBridgePatchVersion;
  }

  function installCodexMessageFromViewInterceptor() {
    if (window.__codexPlusMessageFromViewInterceptorVersion === codexPlusBridgePatchVersion) return;
    window.addEventListener("codex-message-from-view", (event) => {
      try {
        const message = event?.detail;
        const request = requestFromCodexMessage(message);
        if (!request) return;
        trackCodexRequest(message);
        if (isRecentThreadListRequest(message)) {
          trackThreadListPagination(message);
        }
      } catch (error) {
        const diagnostics = codexPlusDiagnostics();
        diagnostics.bridgeErrors.push(String(error?.stack || error));
      }
    }, true);
    window.__codexPlusMessageFromViewInterceptorVersion = codexPlusBridgePatchVersion;
  }

  function patchCodexOutboundRequest(message) {
    const next = clonePlainObject(message);
    const request = requestFromCodexMessage(next);
    if (!request) return next;
    trackCodexRequest(next);
    if (codexPlusSettings().fastModeUnlock && ["thread/start", "thread/resume", "turn/start"].includes(request.method)) {
      const params = request.params || {};
      if (params.serviceTier === "fast" || params.service_tier === "fast") {
        recordFastModeDiagnostic({ lastFastServiceTierRequest: request.method, version: codexFastModeUnlockVersion });
      }
    }
    return next;
  }

  function installCodexBridgeRequestPatches() {
    installMcpResponseInterceptor();
    installCodexMessageFromViewInterceptor();
    const descriptor = Object.getOwnPropertyDescriptor(window.electronBridge || {}, "sendMessageFromView");
    recordCodexPlusDiagnostic("sessionList", {
      installed: true,
      strategy: "codex-message-from-view",
      bridgeWritable: descriptor ? descriptor.writable !== false || !!descriptor.set : null,
      pageLimit: codexThreadListPageLimit,
      maxPages: codexThreadListMaxPages,
    });
    return true;
  }

  function statsigGlobal() {
    return window.__STATSIG__ || globalThis.__STATSIG__ || null;
  }

  function statsigClients() {
    const statsig = statsigGlobal();
    if (!statsig) return [];
    const clients = [];
    if (statsig.firstInstance) clients.push(statsig.firstInstance);
    if (typeof statsig.instance === "function") {
      try {
        const client = statsig.instance();
        if (client) clients.push(client);
      } catch {}
    }
    if (statsig.instances && typeof statsig.instances === "object") {
      clients.push(...Object.values(statsig.instances).filter(Boolean));
    }
    return [...new Set(clients)];
  }

  function patchStatsigLayer(layer, name) {
    const originalLayer = layer || { name };
    if (originalLayer.__codexPlusStatsigLayerVersion === codexStatsigUnlockVersion) return originalLayer;
    const originalGet = typeof originalLayer.get === "function" ? originalLayer.get : null;
    originalLayer.get = function(field, fallback) {
      if (name === "72216192" && field === "enable_i18n" && codexPlusSettings().haleclipseCompatUnlock) return true;
      return originalGet ? originalGet.call(this, field, fallback) : fallback;
    };
    originalLayer.__codexPlusStatsigLayerVersion = codexStatsigUnlockVersion;
    return originalLayer;
  }

  function patchStatsigClient(client) {
    if (!client || typeof client !== "object" || client.__codexPlusStatsigUnlockVersion === codexStatsigUnlockVersion) return false;
    const originalCheckGate = typeof client.checkGate === "function" ? client.checkGate : null;
    client.checkGate = function(name, ...args) {
      const key = String(name);
      if (codexPlusSettings().haleclipseCompatUnlock && key in codexPlusStatsigGateOverrides) {
        return codexPlusStatsigGateOverrides[key];
      }
      return originalCheckGate ? originalCheckGate.call(this, name, ...args) : false;
    };
    const originalFeatureGate = typeof client.getFeatureGate === "function" ? client.getFeatureGate : null;
    client.getFeatureGate = function(name, ...args) {
      const key = String(name);
      const original = originalFeatureGate ? originalFeatureGate.call(this, name, ...args) : null;
      if (codexPlusSettings().haleclipseCompatUnlock && key in codexPlusStatsigGateOverrides) {
        return { ...(original || {}), name, value: codexPlusStatsigGateOverrides[key], rule_id: "codex_plus_plus" };
      }
      return original;
    };
    const originalGetLayer = typeof client.getLayer === "function" ? client.getLayer : null;
    client.getLayer = function(name, ...args) {
      const layer = originalGetLayer ? originalGetLayer.call(this, name, ...args) : null;
      return patchStatsigLayer(layer, String(name));
    };
    client.__codexPlusStatsigUnlockVersion = codexStatsigUnlockVersion;
    return true;
  }

  function patchHaleclipseCompatGates() {
    if (!codexPlusSettings().haleclipseCompatUnlock) return;
    const patched = statsigClients().map(patchStatsigClient).filter(Boolean).length;
    recordCodexPlusDiagnostic("statsig", {
      version: codexStatsigUnlockVersion,
      patchedClients: patched,
      gates: Object.keys(codexPlusStatsigGateOverrides),
      i18nLayer: "72216192",
    });
  }

  function forceChatGPTAuthForFastMode() {
    if (!codexPlusSettings().fastModeUnlock) return false;
    const nodes = [pluginEntryButton(), document.body, ...Array.from(document.querySelectorAll("button, nav, main, [data-testid], [role='dialog']")).slice(0, 200)].filter(Boolean);
    for (const node of nodes) {
      const auth = authContextValueFrom(node);
      if (!auth || auth.authMethod === "chatgpt" || typeof auth.setAuthMethod !== "function") continue;
      auth.setAuthMethod("chatgpt");
      recordFastModeDiagnostic({ authMethodSpoofed: true, version: codexFastModeUnlockVersion });
      return true;
    }
    return false;
  }

  function patchFastModeGates() {
    if (!codexPlusSettings().fastModeUnlock) return;
    forceChatGPTAuthForFastMode();
    let patches = 0;
    const nodes = [document.body, ...Array.from(document.querySelectorAll("button, nav, main, [data-testid], [role='dialog']")).slice(0, 250)].filter(Boolean);
    const seenObjects = new WeakSet();
    for (const node of nodes) {
      for (let fiber = reactFiberFrom(node), depth = 0; fiber && depth < 20; fiber = fiber.return, depth += 1) {
        patches += patchFastModeGateOnObject(fiber.memoizedProps, seenObjects);
        patches += patchFastModeGateOnObject(fiber.pendingProps, seenObjects);
        patches += patchFastModeGateOnObject(fiber.memoizedState, seenObjects);
      }
    }
    if (patches > 0) recordFastModeDiagnostic({ fiberPatches: patches, version: codexFastModeUnlockVersion });
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
    const titleNode = row.querySelector(selectors.threadTitle);
    const rawTitle = (titleNode?.textContent || (titleNode ? "" : (row.textContent || "Untitled session")));
    const title = (titleNode ? rawTitle : rawTitle.replace("导出", "").replace("删除", "")).trim().slice(0, 160);
    return { session_id: sessionId, title };
  }

  async function postJson(path, payload) {
    if (!window.__codexSessionDeleteBridge) {
      return { status: "failed", message: "桥接不可用，请重启启动器" };
    }
    return await window.__codexSessionDeleteBridge(path, payload);
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
    if (!settings.sessionDelete && !settings.markdownExport) {
      removeActionGroups(row);
      row.dataset.codexDeleteRow = "false";
      return;
    }
    const existingGroup = actionGroupFromRow(row);
    const existingDeleteButton = existingGroup?.querySelector(`.${buttonClass}`);
    const existingExportButton = existingGroup?.querySelector(`.${exportButtonClass}`);
    const hasUnexpectedDelete = !settings.sessionDelete && !!existingDeleteButton;
    const hasUnexpectedExport = !settings.markdownExport && !!existingExportButton;
    const missingDelete = settings.sessionDelete && !existingDeleteButton;
    const missingExport = settings.markdownExport && !existingExportButton;
    const deleteReady = !settings.sessionDelete || existingDeleteButton?.dataset.codexDeleteVersion === codexDeleteVersion;
    const exportReady = !settings.markdownExport || existingExportButton?.dataset.codexExportVersion === codexExportVersion;
    const groupReady = existingGroup?.dataset.codexActionGroupVersion === codexActionGroupVersion;
    if (groupReady && deleteReady && exportReady && !hasUnexpectedDelete && !hasUnexpectedExport && !missingDelete && !missingExport) return;
    removeActionGroups(row);
    row.dataset.codexDeleteRow = "false";
    const ref = sessionRefFromRow(row);
    if (!ref.session_id) return;
    row.dataset.codexDeleteRow = "true";
    const group = document.createElement("div");
    group.className = actionGroupClass;
    group.dataset.codexActionGroupVersion = codexActionGroupVersion;
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

  function scanLightweight() {
    installStyle();
    installCodexPlusMenu();
    scheduleBackendHeartbeat();
    installCodexBridgeRequestPatches();
    patchHaleclipseCompatGates();
    installDeleteButtonEventDelegation();
  }

  function scanDeferred() {
    patchFastModeGates();
    enablePluginEntry();
    unblockPluginInstallButtons();
    sessionRows().forEach(tryAttachButton);
    updateDeleteButtonOffsets();
    archivedPageRows().forEach(attachArchivedPageDeleteButton);
    installArchivedDeleteAllButton();
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
    return !!node?.closest?.(".codex-delete-toast, .codex-delete-confirm-overlay, .codex-plus-modal-overlay, #codex-plus-menu");
  }

  const scanRelevantSelector = [
    selectors.sidebarThread,
    '[data-codex-archive-page-row="true"]',
    "[data-codex-archive-delete-all]",
    selectors.appHeader,
    selectors.archiveNav,
    selectors.disabledInstallButton,
  ].join(", ");

  function isScanRelevantNode(node) {
    if (node.nodeType !== 1) return false;
    if (isExtensionUiNode(node)) return false;
    return !!node.matches?.(scanRelevantSelector) || !!node.closest?.(scanRelevantSelector) || !!node.querySelector?.(scanRelevantSelector);
  }

  function isChatContentMutation(mutation) {
    const target = mutation.target;
    if (target?.closest?.('[data-message-author-role], [data-testid="conversation-turn"], main .prose')) {
      return !Array.from(mutation.addedNodes).some(isScanRelevantNode) && !Array.from(mutation.removedNodes).some(isScanRelevantNode);
    }
    return false;
  }

  function shouldScheduleScan(mutations) {
    if (!mutations) return true;
    return mutations.some((mutation) => {
      if (isChatContentMutation(mutation)) return false;
      const target = mutation.target;
      if (isExtensionUiNode(target)) return false;
      return Array.from(mutation.addedNodes).some((node) => node.nodeType === 1 && !isExtensionUiNode(node)) || Array.from(mutation.removedNodes).some((node) => node.nodeType === 1);
    });
  }

  function runScheduledScan() {
    window.__codexSessionDeleteScanPending = false;
    clearTimeout(window.__codexSessionDeleteScanTimer);
    window.__codexSessionDeleteScanTimer = null;
    scan();
  }

  function scheduleScan(mutations) {
    if (!shouldScheduleScan(mutations)) return;
    if (window.__codexSessionDeleteScanPending) return;
    window.__codexSessionDeleteScanPending = true;
    window.__codexSessionDeleteScanTimer = setTimeout(runScheduledScan, 200);
  }

  scan();
  window.__codexSessionDeleteObserver?.disconnect();
  window.__codexSessionDeleteObserver = new MutationObserver(scheduleScan);
  window.__codexSessionDeleteObserver.observe(document.body || document.documentElement, { childList: true, subtree: true });
})();
