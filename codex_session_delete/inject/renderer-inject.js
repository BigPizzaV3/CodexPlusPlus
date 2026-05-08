(() => {
  const helperBase = window.__CODEX_SESSION_DELETE_HELPER__ || "http://127.0.0.1:57321";
  const buttonClass = "codex-delete-button";
  const styleId = "codex-delete-style";
  const codexDeleteStyleVersion = "4";
  const codexPlusMenuId = "codex-plus-menu";
  const codexDeleteVersion = "5";
  const codexArchiveDeleteAllVersion = "2";
  const codexPlusVersion = "1.0.0";
  const codexPlusSettingsKey = "codexPlusSettings";

  function installStyle() {
    const existingStyle = document.getElementById(styleId);
    if (existingStyle?.dataset.codexDeleteStyleVersion === codexDeleteStyleVersion) return;
    existingStyle?.remove();
    const style = document.createElement("style");
    style.id = styleId;
    style.dataset.codexDeleteStyleVersion = codexDeleteStyleVersion;
    style.textContent = `
      .${buttonClass} {
        position: absolute;
        right: 28px;
        top: 50%;
        transform: translateY(-50%);
        z-index: 20;
        opacity: 0;
        border: 1px solid #ef4444;
        border-radius: 6px;
        background: #fee2e2;
        color: #991b1b;
        font-size: 12px;
        line-height: 16px;
        padding: 1px 6px;
        cursor: pointer;
      }
      [data-codex-delete-row="true"]:hover .${buttonClass} { opacity: 1; }
      [data-codex-delete-row="true"].codex-archive-confirm-visible .${buttonClass} { right: 66px; }
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
        left: 240px;
        z-index: 2147483645;
        height: 30px;
        color: #d1d5db;
        font: 13px system-ui, sans-serif;
      }
      #${codexPlusMenuId} {
        display: inline-flex;
        align-items: center;
        height: 100%;
        flex: 0 0 auto;
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
      .codex-plus-modal-title { font-size: 18px; font-weight: 650; }
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
    `;
    document.documentElement.appendChild(style);
  }

  function defaultCodexPlusSettings() {
    return { pluginEntryUnlock: true, forcePluginInstall: true, sessionDelete: true };
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

  function openCodexPlusModal() {
    document.querySelectorAll(".codex-plus-modal-overlay").forEach((node) => node.remove());
    document.querySelectorAll('[data-codex-plus-dialog="true"]').forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = "codex-plus-modal-overlay";
    overlay.innerHTML = `
      <div class="codex-plus-modal-content" role="dialog" aria-modal="true" aria-label="Codex++">
        <div class="codex-plus-modal-header">
          <div class="codex-plus-modal-title">Codex++ ${codexPlusVersion}</div>
          <button type="button" class="codex-plus-modal-close" aria-label="关闭">×</button>
        </div>
        <div class="codex-plus-modal-body">
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
            <div><div class="codex-plus-row-title">关于 Codex++</div><div class="codex-plus-about">Codex++ 是通过外部 launcher 注入的增强菜单，不修改 Codex App 原始安装文件。<br>GitHub: <a href="https://github.com/BigPizzaV3/CodexPlusPlus" target="_blank" rel="noreferrer">https://github.com/BigPizzaV3/CodexPlusPlus</a></div></div>
          </div>
          <div class="codex-plus-row">
            <div><div class="codex-plus-row-title">提出问题</div><div class="codex-plus-row-description">打开 GitHub Issues 反馈问题或建议。</div></div>
            <button type="button" class="codex-plus-issue-button" data-codex-plus-issue="true">提出问题</button>
          </div>
        </div>
      </div>
    `;
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay || event.target.closest(".codex-plus-modal-close")) {
        overlay.remove();
        return;
      }
      const issueButton = event.target.closest("[data-codex-plus-issue]");
      if (issueButton) {
        const issueUrl = "https://github.com/BigPizzaV3/CodexPlusPlus/issues";
        window.open(issueUrl, "_blank");
        return;
      }
      const toggle = event.target.closest("[data-codex-plus-setting]");
      if (!toggle) return;
      const key = toggle.getAttribute("data-codex-plus-setting");
      setCodexPlusSetting(key, !codexPlusSettings()[key]);
    }, true);
    document.body.appendChild(overlay);
    renderCodexPlusMenu();
  }

  function findNativeMenuInsertionPoint() {
    const header = document.querySelector(".app-header-tint");
    const menuBar = header?.querySelector(".flex.items-center.gap-0\\.5") || header?.querySelector('[class*="flex items-center gap-0.5"]');
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

  function pluginEntryLabel(element) {
    return [element.textContent, element.getAttribute("aria-label"), element.getAttribute("title")]
      .filter(Boolean)
      .join(" ")
      .trim();
  }

  function isPluginEntry(element) {
    return /^(插件|Plugins?)$/i.test(pluginEntryLabel(element));
  }

  function pluginEntryButton() {
    return Array.from(document.querySelectorAll("button, [role='button']")).find(isPluginEntry);
  }

  function unlockPluginButton(pluginButton) {
    pluginButton.disabled = false;
    pluginButton.removeAttribute("disabled");
    pluginButton.removeAttribute("aria-disabled");
    pluginButton.style.display = "";
    pluginButton.style.pointerEvents = "auto";
    pluginButton.classList.remove("disabled", "opacity-50", "cursor-not-allowed", "pointer-events-none");
    pluginButton.querySelectorAll("*").forEach((node) => {
      node.style.display = "";
      node.style.pointerEvents = "";
    });
    const reactPropsKey = Object.keys(pluginButton).find((key) => key.startsWith("__reactProps"));
    if (reactPropsKey) {
      pluginButton[reactPropsKey].disabled = false;
      pluginButton[reactPropsKey]["aria-disabled"] = false;
    }
  }

  function enablePluginEntry() {
    if (!codexPlusSettings().pluginEntryUnlock) return;
    const pluginButton = pluginEntryButton();
    if (!pluginButton) return;
    unlockPluginButton(pluginButton);
    spoofChatGPTAuthMethod(pluginButton);
    requestAnimationFrame(() => {
      const currentPluginButton = pluginEntryButton();
      if (currentPluginButton) unlockPluginButton(currentPluginButton);
    });
    if (pluginButton.dataset.codexPluginEnabled === "true") return;
    pluginButton.dataset.codexPluginEnabled = "true";
    pluginButton.addEventListener("click", () => {
      spoofChatGPTAuthMethod(pluginButton);
      requestAnimationFrame(() => {
        const currentPluginButton = pluginEntryButton();
        if (currentPluginButton) unlockPluginButton(currentPluginButton);
      });
    }, true);
  }

  function unblockPluginInstallButtons() {
    if (!codexPlusSettings().forcePluginInstall) return;
    const unavailableLabels = ["App unavailable", "应用不可用"];
    const pageText = document.body.textContent || "";
    if (!unavailableLabels.some((label) => pageText.includes(label))) return;
    document.querySelectorAll("button:disabled").forEach((button) => {
      const text = (button.textContent || "").trim();
      if (!/^安装\s/.test(text) && !/^Install\s/.test(text)) return;
      button.disabled = false;
      button.removeAttribute("disabled");
      button.removeAttribute("aria-disabled");
      button.classList.remove("disabled", "opacity-50", "cursor-not-allowed", "pointer-events-none");
      button.style.pointerEvents = "auto";
      const reactPropsKey = Object.keys(button).find((key) => key.startsWith("__reactProps"));
      if (reactPropsKey) {
        button[reactPropsKey].disabled = false;
        button[reactPropsKey]["aria-disabled"] = false;
      }
    });
  }

  function sessionRows() {
    const codexThreads = Array.from(document.querySelectorAll('[data-app-action-sidebar-thread-id]'));
    if (codexThreads.length > 0) return codexThreads;

    const candidates = Array.from(document.querySelectorAll("a, button, [role='button'], [data-testid], li, div"));
    return candidates.filter((element) => {
      const text = (element.textContent || "").trim();
      const href = element.getAttribute("href") || "";
      const hasSessionHint = /session|conversation|thread/i.test(href + " " + element.outerHTML.slice(0, 400));
      return text.length > 0 && text.length < 200 && hasSessionHint;
    });
  }

  function archivedPageRows() {
    const rows = Array.from(document.querySelectorAll("button")).filter((button) => (button.textContent || "").trim() === "取消归档").map((button) => button.closest(".flex.w-full.items-center.justify-between") || button.parentElement).filter(Boolean);
    rows.forEach((row) => {
      row.dataset.codexArchivePageRow = "true";
      row.setAttribute("data-codex-archive-page-row", "true");
    });
    return rows;
  }

  function archivedSessionRows() {
    return sessionRows().filter((row) => row.querySelector('button[aria-label="取消归档对话"]') || row.outerHTML.includes("取消归档") || row.outerHTML.includes("unarchive"));
  }

  function archivedRows() {
    return [...archivedSessionRows(), ...archivedPageRows()];
  }

  function archivedPageVisible() {
    return archivedRows().length > 0 || window.location.href.includes("archive") || (document.body.textContent || "").includes("已归档对话");
  }

  function sessionRefFromRow(row) {
    const href = row.getAttribute("href") || row.querySelector("a")?.getAttribute("href") || "";
    const idMatch = href.match(/(?:session|conversation|thread)[=/:-]([A-Za-z0-9_.-]+)/i) || href.match(/([A-Za-z0-9_-]{8,})$/);
    const codexThreadId = row.getAttribute("data-app-action-sidebar-thread-id") || "";
    const fallbackId = row.getAttribute("data-session-id") || row.getAttribute("data-testid") || "";
    const sessionId = codexThreadId || (idMatch && idMatch[1]) || fallbackId;
    const titleNode = row.querySelector('[data-thread-title]');
    const title = ((titleNode || row).textContent || "Untitled session").replace("删除", "").trim().slice(0, 160);
    return { session_id: sessionId, title };
  }

  async function postJson(path, payload) {
    if (!window.__codexSessionDeleteBridge) {
      return { status: "failed", message: "Delete bridge unavailable. Restart the launcher." };
    }
    return await window.__codexSessionDeleteBridge(path, payload);
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
        toast.textContent = result.message || "Undo finished";
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
        if (button.classList.contains(buttonClass) || label === "归档对话" || label === "置顶对话") return false;
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
        showToast(result.message || "Deleted", result.undo_token);
      } else {
        showToast(result.message || "Delete failed", null);
      }
    });
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

  function attachButton(row) {
    if (!codexPlusSettings().sessionDelete) return;
    const existingDeleteButtons = Array.from(row.querySelectorAll(`.${buttonClass}`));
    if (existingDeleteButtons.length === 1 && existingDeleteButtons[0].dataset.codexDeleteVersion === codexDeleteVersion) return;
    existingDeleteButtons.forEach((button) => button.remove());
    row.dataset.codexDeleteRow = "false";
    const ref = sessionRefFromRow(row);
    if (!ref.session_id) return;
    row.dataset.codexDeleteRow = "true";
    const button = document.createElement("button");
    button.type = "button";
    button.className = buttonClass;
    button.dataset.codexDeleteVersion = codexDeleteVersion;
    button.textContent = "删除";
    const stopDeleteButtonEvent = (event) => {
      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation?.();
      releaseDeleteFocus(row, button);
    };
    ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
      button.addEventListener(eventName, stopDeleteButtonEvent, true);
    });
    const openDeleteConfirm = (event) => openDeleteConfirmForRow(row, button, ref, event);
    button.addEventListener("pointerup", openDeleteConfirm, true);
    button.addEventListener("click", openDeleteConfirm, true);
    row.appendChild(button);
    const refreshDeleteButton = (originalButton) => {
      if (!originalButton.isConnected) return;
      const replacement = originalButton.cloneNode(true);
      ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
        replacement.addEventListener(eventName, stopDeleteButtonEvent, true);
      });
      replacement.addEventListener("pointerup", openDeleteConfirm, true);
      replacement.addEventListener("click", openDeleteConfirm, true);
      originalButton.replaceWith(replacement);
    };
    setTimeout(() => refreshDeleteButton(button), 0);
  }

  function tryAttachButton(row) {
    try {
      attachButton(row);
    } catch (error) {
      window.__codexSessionDeleteAttachButtonFailures = window.__codexSessionDeleteAttachButtonFailures || [];
      window.__codexSessionDeleteAttachButtonFailures.push(String(error?.stack || error));
    }
  }

  function archivedRefFromRow(row) {
    const sidebarRef = sessionRefFromRow(row);
    if (sidebarRef.session_id) return sidebarRef;
    const title = (row.querySelector(".truncate.text-base")?.textContent || row.textContent || "Untitled session").replace("取消归档", "").replace("删除", "").trim().slice(0, 160);
    const titleMatches = sessionRows().map(sessionRefFromRow).filter((ref) => ref.session_id && ref.title && title.includes(ref.title.slice(0, 24)));
    return titleMatches[0] || { session_id: "", title };
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

  function archiveTitleContainer() {
    return Array.from(document.querySelectorAll("h1, h2, h3, div, span"))
      .find((element) => (element.textContent || "").trim() === "已归档对话" && element.getBoundingClientRect().x > 350);
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
    if (!codexPlusSettings().sessionDelete) return;
    if (row.dataset.codexArchiveDeleteRow === "true") return;
    row.dataset.codexArchiveDeleteRow = "true";
    const unarchiveButton = Array.from(row.querySelectorAll("button")).find((button) => (button.textContent || "").trim() === "取消归档");
    if (!unarchiveButton) return;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "codex-archive-delete-all";
    button.textContent = "删除";
    ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
      button.addEventListener(eventName, stopArchivedButtonEvent, true);
    });
    button.addEventListener("click", async (event) => {
      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation?.();
      const ref = await resolveArchivedThread(row);
      if (!ref.session_id) {
        showToast("Delete failed: archived session id not found", null);
        return;
      }
      if (!(await confirmDelete(ref.title))) return;
      const result = await postJson("/delete", ref);
      if (result.status === "server_deleted" || result.status === "local_deleted") {
        row.remove();
        showToast(result.message || "Deleted", result.undo_token);
      } else {
        showToast(result.message || "Delete failed", null);
      }
    }, true);
    unarchiveButton.insertAdjacentElement("afterend", button);
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
    installDeleteButtonEventDelegation();
  }

  function scanDeferred() {
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
    setTimeout(() => runScanStep(scanDeferred), 50);
  }

  function runScheduledScan() {
    if (window.__codexSessionDeleteScanPending) {
      window.__codexSessionDeleteScanPending = false;
    }
    scan();
  }

  function scheduleScan() {
    if (window.__codexSessionDeleteScanPending) return;
    window.__codexSessionDeleteScanPending = true;
    requestAnimationFrame(runScheduledScan);
    setTimeout(runScheduledScan, 50);
  }

  scheduleScan();
  window.__codexSessionDeleteObserver?.disconnect();
  window.__codexSessionDeleteObserver = new MutationObserver(scheduleScan);
  window.__codexSessionDeleteObserver.observe(document.documentElement, { childList: true, subtree: true });
})();
